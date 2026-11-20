#!/bin/bash
# sweep_join.sh <host> <out dir>: execute-only cycles of one join session against its width k, on synthetic chained
# ChainStates (host `mt-join`). Sizes: "small" = 2 coin-log roots + 2 block-forest roots (early chain); "tip" = 32 + 20,
# every slot Some (the largest a mainnet journal gets near 970,000: <= 32 coin-log levels, 20 forest levels).
# Mode 20 = an empty session of the same image (startup only). Segments at 2^21 like the GPU host (a join fits in one).
set -uo pipefail
H=${1:?host}; O=${2:?out}; mkdir -p "$O"; export HAZYNC_SEG_PO2=21
$H mt-join 20 1 0 0
for sz in "2 2" "16 10" "32 20"; do
  set -- $sz
  RISC0_PPROF_OUT=$O/join18_$1_$2.pprof $H mt-join 18 2 $1 $2
  for k in 2 3 4 8 16 32 64; do
    $H mt-join 19 $k $1 $2; $H mt-join 21 $k $1 $2
  done
done
RISC0_PPROF_OUT=$O/null.pprof $H mt-join 20 1 0 0
echo JOIN_SWEEP_DONE
