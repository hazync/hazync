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
#   hazync-lz-zapp-follow            (the timer runs this)
# Env: ZAPP_ROOT (stores, blocks, states), ZAPP_SERVE_DIR (zync-serve's files), COORD_SPINE, ZAPP_HOST (a libzync host
# with HAZYNC_PREP_HISTORY: v0.24.0's host predates it), ZAPP_FETCH (fetch_block_rpc.py), HAZYNC_RPC_COOKIE.
set -euo pipefail
Z=${ZAPP_ROOT:-/srv/bulk/libzync-zapp}
SERVE=${ZAPP_SERVE_DIR:-/var/lib/hazync-lz/zapp}
SPINE=${COORD_SPINE:-/var/lib/hazync-lz/spine}
HOST=${ZAPP_HOST:-$Z/bin/host}
FETCH=${ZAPP_FETCH:-/srv/bulk/libzync-board/bin/fetch_block_rpc.py}
log() { echo "[$(date -u +%FT%TZ)] $*"; }
die() { log "ERROR: $*"; exit 1; }

mkdir -p "$Z/blocks" "$Z/cl" "$Z/out" "$Z/tmp"
exec 9>"$Z/.lock"; flock -n 9 || { log "another run holds the lock"; exit 0; }

# 1. the proof, and what it proves (asked of the host, from the copied bytes: the spine may move under us)
[ -s "$SPINE/spine.bin" ] || { log "no spine yet"; exit 0; }
cp "$SPINE/spine.bin" "$Z/tmp/spine.bin"
H=$("$HOST" verify-any "$Z/tmp/spine.bin" 1 2>/dev/null | awk '/^RANGE-OK lo=1 / && / anchored=1$/ { for (i = 1; i <= NF; i++) if ($i ~ /^hi=/) print substr($i, 4) }')
[ -n "$H" ] || die "the spine does not verify as a genesis-anchored proof under this host's image"

# 2. the last height this follower's stores are at (a save point with every file prep resumes from), else genesis
last=0
for h in $(find "$Z/out/" -maxdepth 1 -name 'state_*.smt' -printf '%f\n' | sed -n 's/^state_\([0-9]*\)\.smt$/\1/p' | sort -rn); do
    if [ -s "$Z/out/state_$h.json" ] && [ -e "$Z/out/state_$h.coinlog" ] && [ -e "$Z/out/state_$h.trie" ]; then last=$h; break; fi
done
if [ "$H" -le "$last" ]; then
    [ -s "$SERVE/proof.rcpt" ] && cmp -s "$Z/tmp/spine.bin" "$SERVE/proof.rcpt" && { log "at $H, nothing new"; exit 0; }
    [ "$H" -lt "$last" ] && die "the spine ($H) is below this follower's stores ($last): a re-based board needs a fresh ZAPP_ROOT"
fi
if [ "$H" -gt "$last" ]; then
    log "prep $((last + 1))..$H (history and trie for the spine at $H)"
    python3 "$FETCH" range $((last + 1)) "$H" "$Z/blocks" >/dev/null || die "fetching blocks $((last + 1))..$H failed"
    if [ "$last" -eq 0 ]; then anchor=genesis; unset HAZYNC_PREP_CONTINUE; rm -f "$Z/out/history.bin"
    else anchor="$Z/out/state_$last.json"; export HAZYNC_PREP_CONTINUE=1; fi
    # The trie stays in memory (no HAZYNC_TRIE_DIR): a disk trie saves only a one-line "HZTRIE-DISK" marker as
    # state_H.trie, which zync-serve cannot load. zync-serve holds the trie in memory anyway.
    HAZYNC_COINLOG_DIR="$Z/cl" HAZYNC_PREP_HISTORY=1 HAZYNC_PREP_CONSUME=1 \
        "$HOST" libzync-prep $((last + 1)) "$H" "$Z/blocks" "$anchor" "$Z/out" > "$Z/tmp/prep.log" 2>&1 \
        || { tail -5 "$Z/tmp/prep.log"; die "prep $((last + 1))..$H failed"; }
fi
[ -s "$Z/out/state_$H.trie" ] || die "prep left no trie snapshot at $H"
[ "$(head -c 8 "$Z/out/state_$H.trie")" = HZTRIE01 ] || die "state_$H.trie is not a trie snapshot zync-serve can load"
# history.bin must run from genesis to exactly H: H+1 records (zync-serve refuses a trie at another height anyway)
n=$(python3 - "$Z/out/history.bin" <<'PY'
import sys, struct
b = open(sys.argv[1], "rb").read(); i = n = 0
while i < len(b):
    nf, na = struct.unpack_from("<II", b, i + 44); i += 52 + 16 * nf + 32 * na; n += 1
print(n if i == len(b) else -1)
PY
)
[ "$n" -eq $((H + 1)) ] || die "history.bin holds $n records, not $((H + 1)) (genesis..$H)"

# 3. publish the three files together, then restart zync-serve (it reads them at start)
install -d -o hazync -g hazync "$SERVE"
for f in history.bin:"$Z/out/history.bin" state.trie:"$Z/out/state_$H.trie" proof.rcpt:"$Z/tmp/spine.bin"; do
    install -m 0644 -o hazync -g hazync "${f#*:}" "$SERVE/.${f%%:*}.new"
done
for f in history.bin state.trie proof.rcpt; do [ -e "$SERVE/$f" ] && cp -p "$SERVE/$f" "$SERVE/.$f.prev"; done
for f in history.bin state.trie proof.rcpt; do mv -f "$SERVE/.$f.new" "$SERVE/$f"; done
systemctl restart hazync-lz-zync-serve
# zync-serve refuses files it cannot load by exiting: if it is not still up, put the previous set back
sleep 5
if ! systemctl is-active --quiet hazync-lz-zync-serve; then
    for f in history.bin state.trie proof.rcpt; do [ -e "$SERVE/.$f.prev" ] && mv -f "$SERVE/.$f.prev" "$SERVE/$f"; done
    systemctl restart hazync-lz-zync-serve
    die "zync-serve did not stay up on the files for $H: the previous set is restored"
fi
# keep the newest two save points' stores; the per-block state files below them are not needed again
find "$Z/out/" -maxdepth 1 -name 'state_*.smt' -printf '%f\n' | sed -n 's/^state_\([0-9]*\)\.smt$/\1/p' | sort -n | head -n -2 |
    while read -r h; do rm -f "$Z/out/state_$h".{smt,coinlog,trie}; done
find "$Z/out/" -maxdepth 1 -name 'state_*.json' -printf '%f\n' | sed -n 's/^state_\([0-9]*\)\.json$/\1/p' |
    while read -r h; do [ "$h" -lt "$last" ] && rm -f "$Z/out/state_$h.json"; done
log "serving facts at $H (proof: the spine of 1..$H)"
