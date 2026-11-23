#!/bin/bash
# Block 976,379 through the Hazync host built with the input recorder (CPU, RISC Zero EXECUTION, product image
# 46666b6f): hints, then agg-exec-fake. Prints the aggregate's risc0 cycles and journal sha256; writes agg_in.bin.
cd /root/fresh-bench
until grep -q HOST_DONE hzhost_build.out; do sleep 30; done
H=/root/fresh-bench/hzhost/prover/target/release/host
$H method-id 2>&1 | tail -1
export HAZYNC_BLOCK=/root/fresh-bench/block_976379.json HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=16 \
       HAZYNC_COINLOG_TXCOUNTS=/root/fresh-bench/hz_txcounts.txt HAZYNC_HINT_THREADS=16
cd /root/fresh-bench/hzhost/prover
s=$(date +%s); $H libzync-hints /root/fresh-bench/agg_b.lzb > /root/fresh-bench/agg_hints.log 2>&1; echo "HINTS rc=$? secs=$(( $(date +%s)-s ))"
export HAZYNC_LZ_HINTS=/root/fresh-bench/agg_b.lzb HAZYNC_AGG_INPUT_DUMP=/root/fresh-bench/agg_in.bin
s=$(date +%s); $H agg-exec-fake > /root/fresh-bench/agg_exec.log 2>&1; echo "EXEC rc=$? secs=$(( $(date +%s)-s ))"
grep -a -E "aggregate input recorded|block validation cycles|block validation padded|journal sha256|REJECT" /root/fresh-bench/agg_exec.log
ls -la /root/fresh-bench/agg_in.bin
echo REC_DONE
