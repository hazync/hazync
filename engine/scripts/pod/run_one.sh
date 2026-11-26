#!/bin/bash
# On the pod: ONE proof of export/stream.bin with the bundle's keys (`hzengine prove`), timed, with nvidia-smi sampled,
# then `hzengine verify` on it. For a zyncAPP's stream (e.g. the header chain, FEASIBILITY Q20), not a block export.
cd /workspace; source ~/.cargo/env; export PATH=/usr/local/cuda-12.9/bin:$PATH
mkdir -p out; H=host/target/release/hzengine
nvidia-smi --query-gpu=name,memory.total --format=csv,noheader > out/gpu.txt
( nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader -lms 1000 > out/smi_one.log ) & SMI=$!
s=$(date +%s)
$H prove --dir keys --input export/stream.bin --out out/one > out/one.log 2>&1; rc=$?
e=$(( $(date +%s)-s )); kill $SMI
$H verify --dir keys --proof out/one > out/one_verify.log 2>&1; vrc=$?
u=$(awk -F, '{gsub(/ %/,"",$1); s+=$1; n++} END {if(n) printf "%.0f", s/n}' out/smi_one.log)
m=$(awk -F, '{gsub(/ MiB/,"",$2); if ($2+0>m+0) m=$2+0} END {print m}' out/smi_one.log)
echo -e "one\trc=$rc\tverify=$vrc\t${e}s\tgpu_util=${u}%\tpeak_mem=${m}MiB" > out/times.tsv
echo done > out/RUN_DONE
