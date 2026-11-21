#!/bin/bash
# run_a.sh <host> <inputs dir> <out dir> [height ...]
# (A) REAL STATE: each leaf bundle (prep's leaf_<h>.bin: S(h-1) + the BIP30, coin-log and trie witnesses + block h)
# executed end to end exactly as a board prover proves it (`libzync-prove-range h h <dir> leaf <out>` with
# HAZYNC_LEAF_BUNDLE, the command behind the worker's `prove-range-bridge h`, lz_coord.rs), in DEV MODE: every guest
# run is executed (signature-hint collect per chunk, the chunks, the aggregate collect run, the trie update, the
# aggregate); nothing is proved (FakeReceipts). The shipped chunking: HAZYNC_CHUNKS unset (host default 2, cost-packed).
# Segments at po2 21 (the CUDA host's default, so the segment counts are a GPU prover's).
# Writes <out>/leaf_<h>.log (LZ_STATS lines + a TIMED line), <out>/o<h>/ and the state comparison with prep's state_<h>.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
H=$(readlink -f "${1:?host}"); IN=$(readlink -f "${2:?inputs}"); O=$(readlink -m "${3:?out}"); shift 3
HS=${*:-170 1000 100000 200000 270000 280000 290000 300000 400000}
export RISC0_DEV_MODE=1 HAZYNC_SEG_PO2=21
unset HAZYNC_CHUNKS HAZYNC_PROFILE_LAYOUT HAZYNC_LZ_HINTS HAZYNC_BLOCK HAZYNC_HINT_THREADS RISC0_PPROF_OUT
mkdir -p "$O"; cd "$O" || exit 1
for h in $HS; do
  rm -rf "o$h" "leaf_$h.log"
  HAZYNC_LEAF_BUNDLE=$IN/leaves/leaf_$h.bin python3 -I "$HERE/timed.py" "leaf_$h.log" -- \
    nice -n 10 "$H" libzync-prove-range "$h" "$h" unused leaf "o$h"
  python3 -I "$HERE/cmp_state.py" "o$h/state_$h.json" "$IN/out/state_$h.json" >> "leaf_$h.log" 2>&1
  grep -a 'LZ_STATS\|TIMED\|STATE_' "leaf_$h.log"
done
echo RUN_A_DONE
