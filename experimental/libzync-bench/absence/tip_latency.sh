#!/bin/bash
# E1: wall-clock of the aggregate's execution with and without both trees (laptop executor, execute mode). Every host
# output line is timestamped; aggregate time = from the last chunk's "executed:" line to "block validation cycles".
# usage: tip_latency.sh <out dir>
R=~/libzync-gpu/r18; Z=~/dev/hazync-libzync; O=${1:?out dir}; C=~/libzync-gpu/e1/cost
export HAZYNC_CHUNKS=16 HAZYNC_HINT_THREADS=2 HAZYNC_COINLOG_TXCOUNTS=$Z/experimental/libzync-bench/cost/txcounts.txt
unset RISC0_DEV_MODE
for b in $Z/prover/block_962000.json $HOME/libzync-gpu/sample/block_750000.json; do
  n=$(basename $b .json)
  for img in c10b c10os; do
    HAZYNC_BLOCK=$b HAZYNC_LZ_HINTS=$C/${n}_$img.lzb $R/host_$img agg-exec-fake 2>&1 | while IFS= read -r l; do printf '%s %s\n' "$(date +%s.%N)" "$l"; done > $O/${n}_$img.log
    awk -v n=$n -v i=$img '/chunk .*executed:/{t0=$1} /coinlog:|itree/{if(!ts)ts=$1} /block validation cycles/{t1=$1; c=$NF} END{printf "%-14s %-6s aggregate %11d cycles, %.1f s after the last chunk\n", n, i, c, t1-t0}' $O/${n}_$img.log
  done
done
echo TIP_LATENCY_DONE
