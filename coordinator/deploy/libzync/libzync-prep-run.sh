#!/usr/bin/env bash
# libzync prep on the bridge box (ROADMAP H-B, mainnet prep plan): the libzync BRIDGE. Resumes by itself: from genesis
# when there is no save point yet, else CONTINUING the same chain from the newest save point (state_<h>.json with its
# .smt / .coinlog / .trie). Fed by libzync-fetch.service (fetch_block_rpc.py range ... --after-saves).
#   LZ_ROOT  (default /var/lib/libzync-prep)  blocks/ cl/ trie/ out/ leaves/ bin/
#   LZ_HI    the last height of this run (a slice), or `follow`: no last height, the run follows the node's tip (the tip
#            run, docs/libzync/REORG.md §19 D1; its fetcher runs `range 1 follow`)
#   other HAZYNC_PREP_* settings pass through (SAVE_EVERY, LEAF_AT, MIN_FREE_GB, REPORT_EVERY, CHECK)
#   LZ_RUN_DRYRUN=1        print the save point a start would continue from (or that there is none) and change NOTHING
#                          (no directory made, no rewind, no store cleared, prep not started); exit 0, or 3 when a start
#                          would refuse (below). The RUNBOOK runs it before a restart.
#   LZ_PREP_CLEAR_STORES=1 the one way to let a start with NO complete save point clear stores that out/ has states for,
#                          a `follow` run's, or a trie that was ever flushed (its meta has a height: a save point existed):
#                          without it that start refuses with exit 3 (review of D1, HIGH 1; round 2: the board pass too).
#                          A run that died before its first save point is the legitimate case; a save point this runner
#                          cannot see (an older host's files, a moved out/) is the dangerous one, weeks of prep.
# At every start it writes out/RUN.json (LZ_HI and the leaf settings it runs with): the checks compare the handover
# height and the leaf window with the coordinator's from there (review round 2, MEDIUM 1 and 2).
set -euo pipefail
R="${LZ_ROOT:-/var/lib/libzync-prep}"
HI="${LZ_HI:?set LZ_HI}"
case "$HI" in follow) ;; ''|*[!0-9]*) echo "libzync-prep: LZ_HI=$HI: a height or follow" >&2; exit 2;; esac
DRY=${LZ_RUN_DRYRUN:-0}
# the newest COMPLETE save point: a crash between its writes (state, .smt, .coinlog, .trie) leaves a newer partial one.
# The disk trie is advanced in place and its own meta says its height (bytes 17..25, little-endian): a crash after its
# flush but before the state_<h>.trie marker leaves the trie AT h, so h is the save point, not the one before it, which
# the trie can never go back to (review 11-12, #3; prep also checks the trie before it touches the coin log).
pick_save_point() {
    trie_h=$(python3 -c 'import sys,struct;b=open(sys.argv[1],"rb").read(73);print(struct.unpack_from("<Q",b,17)[0] if b[:8]==b"HZTRIED1" and len(b)>=73 else "")' "$R/trie/meta" 2>/dev/null || true)
    last=""
    for h in $(find "$R/out/" -maxdepth 1 -name 'state_*.smt' -printf '%f\n' 2>/dev/null | sed -n 's/^state_\([0-9]*\)\.smt$/\1/p' | sort -rn); do
        if [ -s "$R/out/state_$h.json" ] && [ -e "$R/out/state_$h.coinlog" ] && { [ -s "$R/out/state_$h.trie" ] || [ "$trie_h" = "$h" ]; }; then last=$h; break; fi
    done
}
# stores to clear (anything in cl/ or trie/), and whether anything says a longer run was here: a state_<h>.json in out/,
# or a trie flushed at a height (it is flushed only at save points)
stores_used() { [ -n "$(find "$R/cl" "$R/trie" -mindepth 1 -print -quit 2>/dev/null)" ]; }
out_has_states() { [ -n "$(find "$R/out/" -maxdepth 1 -name 'state_*.json' -print -quit 2>/dev/null)" ] || [ "${trie_h:-0}" -gt 0 ] 2>/dev/null; }
if [ "$DRY" = 1 ]; then
    if [ -e "$R/out/REWIND" ] || { [ "${HAZYNC_REORG_AUTO:-0}" = 1 ] && [ -e "$R/blocks/REORG" ]; }; then
        echo "DRYRUN libzync-prep: a start would first run the rewind (out/REWIND or an auto marker), which moves the save point"
    fi
    pick_save_point
    if [ -n "$last" ]; then echo "DRYRUN libzync-prep: would continue from state_$last.json ($((last + 1))..$HI); trie meta at ${trie_h:-?}"; exit 0; fi
    if stores_used && { out_has_states || [ "$HI" = follow ]; } && [ "${LZ_PREP_CLEAR_STORES:-0}" != 1 ]; then
        echo "DRYRUN libzync-prep: NO complete save point; a start would REFUSE (exit 3) rather than clear the stores"; exit 3
    fi
    echo "DRYRUN libzync-prep: no complete save point; a start would begin at genesis$(stores_used && echo ', clearing cl/ and trie/')"; exit 0
fi
mkdir -p "$R/blocks" "$R/cl" "$R/trie" "$R/out" "$R/leaves"
export HAZYNC_COINLOG_DIR="$R/cl" HAZYNC_TRIE_DIR="$R/trie" HAZYNC_PREP_CONSUME=1 HAZYNC_PREP_LEAF_BUNDLES="$R/leaves"
# A reorg rewind first (docs/libzync/REORG.md §5-§6, P3): an interrupted one (out/REWIND), or the fetcher's REORG marker
# when automatic rewinds are on (HAZYNC_REORG_AUTO=1; prep exits 5 for it). It leaves a complete save point at the fork,
# which the rule below then takes. A refusal (exit 4: beyond W, records missing) stops here: set -e passes the 4 on.
if [ -e "$R/out/REWIND" ] || { [ "${HAZYNC_REORG_AUTO:-0}" = 1 ] && [ -e "$R/blocks/REORG" ]; }; then
    "$R/bin/host_lz" libzync-prep-rewind "$R/out" "$R/blocks"
fi
pick_save_point
if [ -z "$last" ]; then
    # No save point: stores left by a genesis run that died before its first save, which a run from genesis refuses
    # (review 11-12, #8: it restart-looped), are cleared. But only when nothing says a longer run was here: with state
    # files in out/ (or a `follow` run) and no save point this runner can see, clearing could throw away weeks of prep
    # (review of D1, HIGH 1): refuse (exit 3: the unit fails and alerts) unless a person says LZ_PREP_CLEAR_STORES=1.
    if stores_used; then
        if { out_has_states || [ "$HI" = follow ]; } && [ "${LZ_PREP_CLEAR_STORES:-0}" != 1 ]; then
            echo "libzync-prep: REFUSED: no complete save point in $R/out/, but cl/ or trie/ hold stores and $( [ "$HI" = follow ] && echo "this is a follow run" || echo "out/ holds state files or the trie was flushed at ${trie_h:-?}"): not clearing them. Look (LZ_RUN_DRYRUN=1 shows what a start sees); LZ_PREP_CLEAR_STORES=1 clears them and starts at genesis" >&2
            exit 3
        fi
        echo "libzync-prep: no save point, so the stores left by a run that died before its first one are cleared"
        rm -rf "${R:?}/cl/"* "${R:?}/trie/"*
    fi
    lo=1; anchor=genesis; unset HAZYNC_PREP_CONTINUE
else
    lo=$((last + 1)); anchor="$R/out/state_$last.json"; export HAZYNC_PREP_CONTINUE=1
fi
if [ "$HI" != follow ] && [ "$lo" -gt "$HI" ]; then echo "libzync-prep: already at $last >= LZ_HI=$HI"; exit 0; fi
python3 -c 'import json,os,sys,time
p = os.path.join(sys.argv[1], "RUN.json")
json.dump({"lz_hi": sys.argv[2], "leaf_from": int(sys.argv[3]) if sys.argv[3] else None,
           "leaf_window": int(sys.argv[4]) if sys.argv[4] else None, "at": int(time.time())}, open(p + ".tmp", "w"))
os.replace(p + ".tmp", p)' "$R/out" "$HI" "${HAZYNC_PREP_LEAF_FROM:-}" "${HAZYNC_PREP_LEAF_WINDOW:-}"
echo "libzync-prep: $lo..$HI from ${anchor##*/}"
# only the two newest BIP30 snapshots are kept (a restart needs the newest; each is the whole tree, tens of MB on mainnet)
( while sleep 60; do
    find "$R/out/" -maxdepth 1 -name 'state_*.smt' -printf '%f\n' | sed -n 's/^state_\([0-9]*\)\.smt$/\1/p' | sort -n | head -n -2 |
        while read -r h; do rm -f "$R/out/state_$h.smt"; done
  done ) &
PRUNER=$!
trap 'kill $PRUNER 2>/dev/null || true' EXIT
"$R/bin/host_lz" libzync-prep "$lo" "$HI" "$R/blocks" "$anchor" "$R/out"
