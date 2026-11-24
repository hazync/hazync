#!/bin/bash
# On the pod: the whole block (chunks, trie, aggregate verifying all of them in-guest), timed; then verify.
cd /workspace; source ~/.cargo/env; export PATH=/usr/local/cuda-12.9/bin:$PATH
mkdir -p out; H=host/target/release/hzengine
( nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader -l 2 > out/smi.log ) & SMI=$!
s=$(date +%s); $H prove-block --dir keys --export export --out out/block > out/prove_block.log 2>&1; rc=$?
echo "prove-block rc=$rc secs=$(( $(date +%s)-s ))" >> out/times.txt
$H verify --dir keys --proof out/block/block > out/verify.log 2>&1; echo "verify rc=$?" >> out/times.txt
kill $SMI; awk -F', ' '{gsub(/ %/,"",$1); s+=$1; n++} END {print "gpu util mean", s/n}' out/smi.log >> out/times.txt
cp keys/image.json out/ 2>/dev/null
echo "rc=$rc" > out/RUN_DONE
