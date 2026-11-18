#!/bin/bash
# Keep the zyncAPP fact service (zync-serve) on the board's newest genesis-anchored proof (ROADMAP O2).
#
# Coin check (hazync.org/zapp/) checks every fact against a proof of blocks 1..H, so the history, the trie snapshot
# and the proof must all be at the SAME H. The board's spine is that proof. Each run:
#   1. copies the spine and asks the host what it proves (`verify-any <spine> 1` -> RANGE-OK lo=1 hi=H anchored=1);
#   2. if H is past the last height served, fetches blocks last+1..H from the box's node and runs prep over them with
#      HAZYNC_PREP_HISTORY=1 on this follower's own disk stores (forward only; prep always saves at its last block,
#      so state_H.trie is written and history.bin ends at H);
#   3. publishes history.bin, state.trie (= state_H.trie) and proof.rcpt (= that spine) into the zync-serve directory
#      and restarts zync-serve, whose relay and /api/zync/proof the coordinator serves.
# It proves nothing and is trusted for nothing: the page verifies the proof and checks every fact against it.
#
# A REORG (docs/libzync/REORG.md §10, ZAPP_FOLLOWER.md §7). The coordinator sets the spine back to a kept head H' at or
# below the fork; this follower's stores may then be above H', or at H' on the orphaned block. Its trie is in memory and
# its coin log is changed in place, with no undo records, so its stores cannot go back: they are moved aside
# (`$ZAPP_ROOT/orphaned/<time>/`, only the newest kept) and prep runs again from genesis to H' (cheap while H' is low,
# and today's in-memory follower stops scaling near ~140,000 anyway, ZAPP_FOLLOWER.md §1). The previous set stays
# published until the new one is ready. Only when the node agrees with the spine at H' (its block hash): a node on
# another branch would rebuild the same wrong stores every run. Tip mode (ZAPP_FOLLOWER Z2-Z5, a reader of the tip
# run's stores with pinned roots) replaces this; it is not built.
#
#   hazync-lz-zapp-follow            (the timer runs this)
# Env: ZAPP_ROOT (stores, blocks, states), ZAPP_SERVE_DIR (zync-serve's files), COORD_SPINE, ZAPP_HOST (a libzync host
# with HAZYNC_PREP_HISTORY: v0.24.0's host predates it), ZAPP_FETCH (fetch_block_rpc.py), HAZYNC_RPC_COOKIE.
# For tests only: ZAPP_OWNER (the served files' owner, default hazync; empty: as is), ZAPP_RESTART / ZAPP_ACTIVE (the
# commands that restart zync-serve and ask whether it is up), ZAPP_SETTLE_S (5).
set -euo pipefail
Z=${ZAPP_ROOT:-/srv/bulk/libzync-zapp}
SERVE=${ZAPP_SERVE_DIR:-/var/lib/hazync-lz/zapp}
SPINE=${COORD_SPINE:-/var/lib/hazync-lz/spine}
HOST=${ZAPP_HOST:-$Z/bin/host}
FETCH=${ZAPP_FETCH:-/srv/bulk/libzync-board/bin/fetch_block_rpc.py}
OWNER=${ZAPP_OWNER-hazync}
RESTART=${ZAPP_RESTART:-systemctl restart hazync-lz-zync-serve}
ACTIVE=${ZAPP_ACTIVE:-systemctl is-active --quiet hazync-lz-zync-serve}
own() { if [ -n "$OWNER" ]; then echo "-o $OWNER -g $OWNER"; fi; }
log() { echo "[$(date -u +%FT%TZ)] $*"; }
die() { log "ERROR: $*"; exit 1; }

mkdir -p "$Z/blocks" "$Z/cl" "$Z/out" "$Z/tmp"
exec 9>"$Z/.lock"; flock -n 9 || { log "another run holds the lock"; exit 0; }

# 1. the proof, and what it proves (asked of the host, from the copied bytes: the spine may move under us)
if [ ! -s "$SPINE/spine.bin" ]; then
    # no spine: none yet (nothing served either), or a reorg below every kept head took it away (REORG §7) while facts
    # of the orphaned branch are still served: that is a failure for a person, not "nothing to do" (round 3, LOW)
    [ -s "$SERVE/proof.rcpt" ] && die "no spine, but facts are served at the proof in $SERVE: a reorg below every kept spine head? REORG.md §11"
    log "no spine yet"; exit 0
fi
cp "$SPINE/spine.bin" "$Z/tmp/spine.bin"
OKLINE=$("$HOST" verify-any "$Z/tmp/spine.bin" 1 2>/dev/null | grep -m1 '^RANGE-OK lo=1 .* anchored=1$' || true)
H=$(echo "$OKLINE" | tr ' ' '\n' | sed -n 's/^hi=//p')
BH=$(echo "$OKLINE" | tr ' ' '\n' | sed -n 's/^out_bhash=//p')
[ -n "$H" ] && [ -n "$BH" ] || die "the spine does not verify as a genesis-anchored proof under this host's image"

# 2. the last height this follower's stores are at (a save point with every file prep resumes from), else genesis
last=0
for h in $(find "$Z/out/" -maxdepth 1 -name 'state_*.smt' -printf '%f\n' | sed -n 's/^state_\([0-9]*\)\.smt$/\1/p' | sort -rn); do
    if [ -s "$Z/out/state_$h.json" ] && [ -e "$Z/out/state_$h.coinlog" ] && [ -e "$Z/out/state_$h.trie" ]; then last=$h; break; fi
done
if [ "$H" -le "$last" ]; then
    [ -s "$SERVE/proof.rcpt" ] && cmp -s "$Z/tmp/spine.bin" "$SERVE/proof.rcpt" && { log "at $H, nothing new"; exit 0; }
fi
# REORG §10: the spine below the stores, at a height whose block the stores have another of, or above stores whose
# own last block the node no longer has (the spine fell back and advanced again between two runs, round 3 M1)
stip() { python3 -c 'import json,sys;print(bytes(json.load(open(sys.argv[1]))["tip_hash"]).hex())' "$Z/out/state_$1.json" 2>/dev/null || true; }
node_hash() {  # node_hash <h>: the node's block at h, from its header (internal order, as out_bhash), or empty
    python3 "$FETCH" "$1" "$Z/tmp/block_$1.json" >/dev/null 2>&1 || { echo; return; }
    python3 -c '
import hashlib, json, struct, sys
j = json.load(open(sys.argv[1]))
h = (struct.pack("<I", int(j["version"]) & 0xffffffff) + bytes.fromhex(j["prev"])[::-1] + bytes.fromhex(j["merkle"])[::-1]
     + struct.pack("<III", int(j["time"]), int(j["bits"]), int(j["nonce"])))
print(hashlib.sha256(hashlib.sha256(h).digest()).digest().hex())' "$Z/tmp/block_$1.json"
    rm -f "$Z/tmp/block_$1.json"
}
mine=$(stip "$H")
why=""
if [ "$last" -gt 0 ]; then
    if [ "$H" -lt "$last" ]; then why="the spine is below them"
    elif [ -n "$mine" ] && [ "$mine" != "$BH" ]; then why="they have another block at $H"
    elif [ "$H" -gt "$last" ]; then
        NL=$(node_hash "$last"); [ -n "$NL" ] || die "cannot fetch block $last from the node to check this follower's stores"
        [ "$NL" = "$(stip "$last")" ] || why="the node no longer has their block $last"
    fi
fi
if [ -n "$why" ]; then
    # only for a spine the node agrees with: its block at H is the spine's (a node on another branch would rebuild the
    # same wrong stores every run)
    NH=$(node_hash "$H")
    [ "$NH" = "$BH" ] || die "the spine ($H, block $BH) is not the node's block at $H (${NH:-none}): this follower's stores are left as they are"
    ts=$(date -u +%Y%m%dT%H%M%SZ)
    log "a reorg: the spine is at $H (block ${BH:0:16}), this follower's stores at $last ($why): moved to orphaned/$ts, prep again from genesis"
    mkdir -p "$Z/orphaned/$ts"
    for d in cl out blocks; do if [ -e "$Z/$d" ]; then mv "$Z/$d" "$Z/orphaned/$ts/"; fi; done
    find "$Z/orphaned/" -mindepth 1 -maxdepth 1 -type d ! -name "$ts" -exec rm -rf {} +    # only the newest kept
    mkdir -p "$Z/blocks" "$Z/cl" "$Z/out"
    last=0
fi
if [ "$H" -gt "$last" ]; then
    log "prep $((last + 1))..$H (history and trie for the spine at $H)"
    python3 "$FETCH" range $((last + 1)) "$H" "$Z/blocks" >/dev/null || die "fetching blocks $((last + 1))..$H failed"
    # from genesis: a rebuild that died before its first save point leaves a coin log a run from genesis refuses;
    # nothing in it is worth keeping (as libzync-prep-run.sh does, round 3 LOW)
    if [ "$last" -eq 0 ]; then anchor=genesis; unset HAZYNC_PREP_CONTINUE; rm -f "$Z/out/history.bin"; rm -rf "${Z:?}/cl/"*
    else anchor="$Z/out/state_$last.json"; export HAZYNC_PREP_CONTINUE=1; fi
    # The trie stays in memory (no HAZYNC_TRIE_DIR): a disk trie saves only a one-line "HZTRIE-DISK" marker as
    # state_H.trie, which zync-serve cannot load. zync-serve holds the trie in memory anyway.
    HAZYNC_COINLOG_DIR="$Z/cl" HAZYNC_PREP_HISTORY=1 HAZYNC_PREP_CONSUME=1 \
        "$HOST" libzync-prep $((last + 1)) "$H" "$Z/blocks" "$anchor" "$Z/out" > "$Z/tmp/prep.log" 2>&1 \
        || { tail -5 "$Z/tmp/prep.log"; die "prep $((last + 1))..$H failed"; }
fi
# the facts must be about the proof's chain: prep's block at H is the spine's (review 11-12, #10). A node on another
# branch, or a reorg at H, would give history and trie for a different chain than the published proof; the page would
# refuse every fact, and nothing would say why.
PH=$(python3 -c 'import json,sys;print(bytes(json.load(open(sys.argv[1]))["tip_hash"]).hex())' "$Z/out/state_$H.json" 2>/dev/null || true)
[ "$PH" = "$BH" ] || die "prep's block $H (${PH:-none}) is not the proof's ($BH): the node is on another branch or $H was reorged; not published"
[ -s "$Z/out/state_$H.trie" ] || die "prep left no trie snapshot at $H"
[ "$(head -c 8 "$Z/out/state_$H.trie")" = HZTRIE01 ] || die "state_$H.trie is not a trie snapshot zync-serve can load"
# history.bin must run from genesis to exactly H: H+1 records (zync-serve refuses a trie at another height anyway)
n=$(python3 - "$Z/out/history.bin" <<'PY'
import sys, struct
b = open(sys.argv[1], "rb").read(); i = n = 0
while i < len(b):
    nf, na = struct.unpack_from("<II", b, i + 44)
    u = na >> 31; na &= 0x7FFFFFFF   # bit 31: the record ends with the block's undo hash (libzync/src/history.rs)
    i += 52 + 16 * nf + 32 * na + 32 * u; n += 1
print(n if i == len(b) else -1)
PY
)
[ "$n" -eq $((H + 1)) ] || die "history.bin holds $n records, not $((H + 1)) (genesis..$H)"

# 3. publish the three files together, then restart zync-serve (it reads them at start)
install -d $(own) "$SERVE"
for f in history.bin:"$Z/out/history.bin" state.trie:"$Z/out/state_$H.trie" proof.rcpt:"$Z/tmp/spine.bin"; do
    install -m 0644 $(own) "${f#*:}" "$SERVE/.${f%%:*}.new"
done
for f in history.bin state.trie proof.rcpt; do [ -e "$SERVE/$f" ] && cp -p "$SERVE/$f" "$SERVE/.$f.prev"; done
for f in history.bin state.trie proof.rcpt; do mv -f "$SERVE/.$f.new" "$SERVE/$f"; done
$RESTART
# zync-serve refuses files it cannot load by exiting: if it is not still up, put the previous set back
sleep "${ZAPP_SETTLE_S:-5}"
if ! $ACTIVE; then
    for f in history.bin state.trie proof.rcpt; do [ -e "$SERVE/.$f.prev" ] && mv -f "$SERVE/.$f.prev" "$SERVE/$f"; done
    $RESTART
    die "zync-serve did not stay up on the files for $H: the previous set is restored"
fi
# keep the newest two save points' stores; the per-block state files below them are not needed again
find "$Z/out/" -maxdepth 1 -name 'state_*.smt' -printf '%f\n' | sed -n 's/^state_\([0-9]*\)\.smt$/\1/p' | sort -n | head -n -2 |
    while read -r h; do rm -f "$Z/out/state_$h".{smt,coinlog,trie}; done
find "$Z/out/" -maxdepth 1 -name 'state_*.json' -printf '%f\n' | sed -n 's/^state_\([0-9]*\)\.json$/\1/p' |
    while read -r h; do if [ "$h" -lt "$last" ]; then rm -f "$Z/out/state_$h.json"; fi; done   # not `[ ] &&`: a false last test fails the run under set -e
log "serving facts at $H (proof: the spine of 1..$H)"
