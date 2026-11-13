#!/usr/bin/env bash
# libzync prep on the bridge box (ROADMAP H-B, mainnet prep plan): the libzync BRIDGE. Resumes by itself: from genesis
# when there is no save point yet, else CONTINUING the same chain from the newest save point (state_<h>.json with its
# .smt / .coinlog / .trie). Fed by libzync-fetch.service (fetch_block_rpc.py range ... --after-saves).
#   LZ_ROOT  (default /var/lib/libzync-prep)  blocks/ cl/ trie/ out/ leaves/ bin/
#   LZ_HI    the last height of this run (a slice, or the tip)
#   other HAZYNC_PREP_* settings pass through (SAVE_EVERY, LEAF_AT, MIN_FREE_GB, REPORT_EVERY, CHECK)
set -euo pipefail
R="${LZ_ROOT:-/var/lib/libzync-prep}"
HI="${LZ_HI:?set LZ_HI}"
mkdir -p "$R/blocks" "$R/cl" "$R/trie" "$R/out" "$R/leaves"
export HAZYNC_COINLOG_DIR="$R/cl" HAZYNC_TRIE_DIR="$R/trie" HAZYNC_PREP_CONSUME=1 HAZYNC_PREP_LEAF_BUNDLES="$R/leaves"
# the newest COMPLETE save point: a crash between its writes (state, .smt, .coinlog, .trie) leaves a newer partial one.
# The disk trie is advanced in place and its own meta says its height (bytes 17..25, little-endian): a crash after its
# flush but before the state_<h>.trie marker leaves the trie AT h, so h is the save point, not the one before it, which
# the trie can never go back to (review 11-12, #3; prep also checks the trie before it touches the coin log).
trie_h=$(python3 -c 'import sys,struct;b=open(sys.argv[1],"rb").read();print(struct.unpack_from("<Q",b,17)[0] if b[:8]==b"HZTRIED1" and len(b)>=73 else "")' "$R/trie/meta" 2>/dev/null || true)
last=""
for h in $(find "$R/out/" -maxdepth 1 -name 'state_*.smt' -printf '%f\n' | sed -n 's/^state_\([0-9]*\)\.smt$/\1/p' | sort -rn); do
    if [ -s "$R/out/state_$h.json" ] && [ -e "$R/out/state_$h.coinlog" ] && { [ -s "$R/out/state_$h.trie" ] || [ "$trie_h" = "$h" ]; }; then last=$h; break; fi
done
if [ -z "$last" ]; then
    # No save point: anything in the stores is a genesis run that died before its first save, and a run from genesis
    # refuses stores that are already there (review 11-12, #8: it restart-looped). Nothing in them is worth keeping.
    if [ -n "$(ls -A "$R/cl" "$R/trie" 2>/dev/null)" ]; then
        echo "libzync-prep: no save point, so the stores left by a run that died before its first one are cleared"
        rm -rf "${R:?}/cl/"* "${R:?}/trie/"*
    fi
    lo=1; anchor=genesis; unset HAZYNC_PREP_CONTINUE
else
    lo=$((last + 1)); anchor="$R/out/state_$last.json"; export HAZYNC_PREP_CONTINUE=1
fi
[ "$lo" -le "$HI" ] || { echo "libzync-prep: already at $last >= LZ_HI=$HI"; exit 0; }
echo "libzync-prep: $lo..$HI from ${anchor##*/}"
# only the two newest BIP30 snapshots are kept (a restart needs the newest; each is the whole tree, tens of MB on mainnet)
( while sleep 60; do
    find "$R/out/" -maxdepth 1 -name 'state_*.smt' -printf '%f\n' | sed -n 's/^state_\([0-9]*\)\.smt$/\1/p' | sort -n | head -n -2 |
        while read -r h; do rm -f "$R/out/state_$h.smt"; done
  done ) &
PRUNER=$!
trap 'kill $PRUNER 2>/dev/null || true' EXIT
"$R/bin/host_lz" libzync-prep "$lo" "$HI" "$R/blocks" "$anchor" "$R/out"
