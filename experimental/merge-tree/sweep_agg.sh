#!/bin/bash
# sweep_agg.sh: the aggregate's cycles against how many chunk receipts it merges (HAZYNC_CHUNKS = 1..32), same block,
# execute only (agg-exec-fake: the block's coin log and trie around it are MODELLED, lz::cl_synth_around, as in the
# cost survey). Prints one TSV row per (block, chunks): chunk user cycles, aggregate user + padded cycles, trie.
# Segments at 2^21 (the GPU host's default). Also writes the aggregate's pprof for chunks=16 and chunks=1.
#   bash sweep_agg.sh <host> <out dir> <block json>...
set -uo pipefail
H=${1:?host}; O=${2:?out}; shift 2; mkdir -p "$O"
export HAZYNC_PROFILE_LAYOUT=cost HAZYNC_HINT_THREADS=4 HAZYNC_SEG_PO2=21
export HAZYNC_COINLOG_TXCOUNTS=/root/merge-tree/src/experimental/libzync-bench/cost/txcounts.txt
echo -e "block\tchunks\tchunk_user\tagg_user\tagg_padded\tagg_segs\ttrie_user\trc"
for f in "$@"; do
  b=$(basename "$f" .json | cut -d_ -f2)
  for k in 1 2 4 8 16 32; do
    export HAZYNC_CHUNKS=$k
    HAZYNC_BLOCK=$f $H libzync-hints "$O/${b}_$k.lzb" > "$O/${b}_$k.hints.log" 2>&1 || { echo -e "$b\t$k\tHINTS_FAILED"; continue; }
    if [ $k = 16 ] || [ $k = 1 ]; then export RISC0_PPROF_OUT=$O/agg_${b}_$k.pprof; else unset RISC0_PPROF_OUT; fi
    HAZYNC_BLOCK=$f HAZYNC_LZ_HINTS=$O/${b}_$k.lzb $H agg-exec-fake > "$O/${b}_$k.log" 2>&1; rc=$?
    unset RISC0_PPROF_OUT
    c=$(grep -a 'chunk .*executed:' "$O/${b}_$k.log" | grep -a -oE 'executed: [0-9]+' | awk '{s+=$2} END {print s+0}')
    a=$(grep -a -oE 'block validation cycles +[0-9]+' "$O/${b}_$k.log" | awk '{print $4}')
    p=$(grep -a -oE 'block validation padded +[0-9]+ \([0-9]+' "$O/${b}_$k.log" | awk '{print $4"\t"substr($5,2)}')
    t=$(grep -a -oE 'trie update executed: [0-9]+' "$O/${b}_$k.log" | awk '{print $4}')
    echo -e "$b\t$k\t$c\t${a:-FAIL}\t${p:-FAIL}\t${t:-0}\trc=$rc"
  done
done
echo SWEEP_DONE
