#!/bin/bash
# run_prof.sh <host> <inputs dir> <blocks dir> <out dir> <A heights> -- <B heights>
# Execute-mode pprof profiles (RISC0_PPROF_OUT, capture_pprof.py keeps every execution's file) of the same runs as
# run_a.sh / run_b.sh, for the per-category split (prof_split.py). Profiling only adds the sampler; the cycle counts
# are the same executions (each file's non-paging total is checked against the host's printed count in RESULT.md).
# A (leaf, dev mode), HAZYNC_HINT_THREADS=1 so executions are serial; with c chunks the files are, in order:
#   0..c-1 signature-hint collect runs, c..2c-1 the chunks, 2c the aggregate collect run, 2c+1 the trie update,
#   2c+2 the aggregate.  B (agg-exec-fake): 0..c-1 the chunks, c the coin-log collect run, c+1 trie, c+2 aggregate.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
H=$(readlink -f "$1"); IN=$(readlink -f "$2"); BL=$(readlink -f "$3"); O=$(readlink -m "$4"); shift 4
AH=(); while [ $# -gt 0 ] && [ "$1" != "--" ]; do AH+=("$1"); shift; done; shift; BH=("$@")
mkdir -p "$O/cap"; cd "$O" || exit 1
export HAZYNC_SEG_PO2=21 HAZYNC_HINT_THREADS=1 HAZYNC_COINLOG_TXCOUNTS=$HERE/../cost/txcounts.txt
unset HAZYNC_CHUNKS HAZYNC_PROFILE_LAYOUT HAZYNC_LZ_HINTS
for h in "${AH[@]}"; do
  rm -rf "o$h"
  RISC0_DEV_MODE=1 HAZYNC_LEAF_BUNDLE=$IN/leaves/leaf_$h.bin RISC0_PPROF_OUT=$O/cap/live.pb python3 -I "$HERE/timed.py" "profA_$h.log" -- \
    python3 -I "$HERE/capture_pprof.py" "$O/cap/live.pb" "$O/cap/A$h" -- nice -n 10 "$H" libzync-prove-range "$h" "$h" unused leaf "o$h"
done
for h in "${BH[@]}"; do
  HAZYNC_BLOCK=$BL/block_$h.json nice -n 10 "$H" libzync-hints "b$h.lzb" > "profB_hints_$h.log" 2>&1
  HAZYNC_BLOCK=$BL/block_$h.json HAZYNC_LZ_HINTS=$O/b$h.lzb RISC0_PPROF_OUT=$O/cap/live.pb python3 -I "$HERE/timed.py" "profB_$h.log" -- \
    python3 -I "$HERE/capture_pprof.py" "$O/cap/live.pb" "$O/cap/B$h" -- nice -n 10 "$H" agg-exec-fake
done
echo RUN_PROF_DONE
