#!/bin/bash
# ab_962000.sh <host> <tag> <out dir>: block 962,000 on a mainnet libzync host, 16 chunks, signature hints then
# agg-exec-fake (execute only, nothing proved). Run once for each arm with the same block and env; the cost of the
# block forest is the difference. Results of 2026-11-02 are in ab_962000/ (HANDOVER.md §8).
H=$(realpath "$1"); T=$2; D=$3; mkdir -p "$D"
export HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=16 HAZYNC_HINT_THREADS=2
export HAZYNC_BLOCK=$(realpath "$(dirname "$0")/../../../prover/block_962000.json")
"$H" libzync-hints "$D/hints_$T.lzb" > "$D/hints_$T.log" 2>&1 || { echo "HINTS_FAIL $T"; exit 1; }
HAZYNC_LZ_HINTS=$D/hints_$T.lzb "$H" agg-exec-fake > "$D/agg_$T.log" 2>&1; echo "AGG_EXIT=$?" >> "$D/agg_$T.log"
