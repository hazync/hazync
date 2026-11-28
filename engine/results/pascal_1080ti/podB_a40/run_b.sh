#!/bin/bash
# Test B: the header chain (a) with the unfused bitrev path forced, (b) fused (control). Each: prove, verify, journal sha.
cd /workspace; source ~/.cargo/env; export PATH=/usr/local/cuda-12.9/bin:$PATH
mkdir -p out; H=engine/host/target/release/hzengine
nvidia-smi --query-gpu=name,memory.total,compute_cap --format=csv,noheader > out/gpu.txt
one(){ # <name> <force 0|1>
  ( nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader -lms 1000 > out/smi_$1.log ) & SMI=$!
  s=$(date +%s); HZ_FORCE_UNFUSED_BITREV=$2 $H prove --dir keys --input export/stream.bin --out out/$1 > out/$1.log 2>&1; rc=$?
  e=$(( $(date +%s)-s )); kill $SMI
  $H verify --dir keys --proof out/$1 > out/$1_verify.log 2>&1; vrc=$?
  j=$(sha256sum out/$1.journal 2>/dev/null | cut -c1-16)
  err=$(sed 's/\x1b\[[0-9;]*m//g' out/$1.log | grep -a -m1 "^Error" | cut -c1-200)
  echo -e "$1\tforce=$2\trc=$rc\tverify=$vrc\t${e}s\tjournal=$j\t$err" >> out/times.tsv
}
one unfused 1
one fused 0
echo done > out/RUN_DONE
