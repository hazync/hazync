#!/usr/bin/env bash
# Integrity check: the utreexo follower (hazync-utreexo-follow, README.md here) keeps up, and its forest stays below
# the memory it is allowed. Installed as /usr/local/sbin/hazync-check-utreexo-follow, run through hazync-run-check.
#
# WHY. The follower is a oneshot on a timer: a run that fails pages through OnFailure=, but a follower that stops being
# RUN (a disabled timer, a run that hangs until its timeout every time, a lock never released) or that keeps "doing
# nothing" while it should step, pages nobody. This compares what was published with what the follower should have
# reached: H = min(node tip - MARGIN, the core proof's height). A published H below that, last written more than
# UTREEXO_STALL_SECS ago, is a stall. While the core proof does not move, H = T does not either and nothing is wrong.
# Second, the forest's heap at the last step ($D/forest-heap, written by the follower) above FOREST_HEAP_WARN_BYTES:
# the forest grows with the proven height and would hit the unit's MemoryMax hours into a proof (UTREEXO_ZAPP.md
# §10.5.1); this says so while there is time to raise it (a repo change: the unit carries MemoryMax).
#
# EXIT CODES (the integrity-check contract): 0 holds, 1 failing, 2 could not check (says nothing either way).
set -uo pipefail

D=${UTREEXO_DIR:-/var/lib/hazync-utreexo}
ZYNCAPP=${ZYNCAPP:-/opt/hazync-utreexo/bin/zyncapp}
CORE=${CORE:-/var/lib/hazync-lz/spine/spine.bin}
BITCOIN_CLI=${BITCOIN_CLI:-bitcoin-cli -conf=/etc/bitcoin/bitcoin.conf -datadir=/var/lib/bitcoind}
MARGIN=${MARGIN:-6}
# a CPU step of MAX_STEP blocks can take hours; at the tip a step is a few blocks
STALL_SECS=${UTREEXO_STALL_SECS:-21600}
HEAP_WARN=${FOREST_HEAP_WARN_BYTES:-12884901888}

say() { echo "[utreexo] $*"; }
J=$D/pub/latest.json

heap_bad=0
if [ -s "$D/forest-heap" ]; then
    read -r heap at < "$D/forest-heap"
    if [[ "${heap:-}" =~ ^[0-9]+$ ]] && [ "$heap" -gt "$HEAP_WARN" ]; then
        say "FOREST HEAP: $heap bytes at block ${at:-?}, above FOREST_HEAP_WARN_BYTES $HEAP_WARN: raise the unit's MemoryMax (a repo change) before the forest reaches it"
        heap_bad=1
    fi
fi

if [ ! -s "$J" ]; then
    say "NOTHING PUBLISHED: $J is missing or empty (the follower has not published a proof)"
    exit 1
fi
h=$(python3 -c 'import json,sys; print(int(json.load(open(sys.argv[1]))["height"]))' "$J" 2>/dev/null) \
    || { say "cannot check: $J does not parse"; exit 2; }
age=$(( $(date +%s) - $(stat -c %Y "$J") ))
nt=$($BITCOIN_CLI getblockcount </dev/null 2>/dev/null) || { say "cannot check: the node did not answer getblockcount"; exit 2; }
[[ "$nt" =~ ^[0-9]+$ ]] || { say "cannot check: getblockcount gave '$nt'"; exit 2; }
t=$("$ZYNCAPP" core-info "$CORE" 2>/dev/null | sed -n 's/^CORE height=\([0-9]*\) .*/\1/p')
[[ "$t" =~ ^[0-9]+$ ]] || { say "cannot check: core-info $CORE gave no height"; exit 2; }
target=$(( nt - MARGIN < t ? nt - MARGIN : t ))

if [ "$h" -lt "$target" ] && [ "$age" -gt "$STALL_SECS" ]; then
    say "STALLED: published block $h, the follower should be at $target (node tip $nt - $MARGIN, core proof $t); latest.json last written ${age} s ago (> $STALL_SECS)"
    exit 1
fi
say "ok: published block $h, target $target (node tip $nt, core proof $t), latest.json ${age} s old"
[ "$heap_bad" = 0 ] || exit 1
exit 0
