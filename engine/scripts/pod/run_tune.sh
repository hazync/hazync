#!/bin/bash
# On the pod: prove-block over (export, workers) pairs to measure chunk sizing and GPU sharing. JOBS="name:workers ...".
cd /workspace; source ~/.cargo/env; export PATH=/usr/local/cuda-12.9/bin:$PATH
mkdir -p out; H=host/target/release/hzengine; T=out/times.tsv
nvidia-smi --query-gpu=name,memory.total --format=csv,noheader > out/gpu.txt
for j in ${JOBS:-}; do
  x=${j%:*}; w=${j#*:}; tag=${x}_w$w
  ( nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader -l 1 > out/smi_$tag.log ) & SMI=$!
  s=$(date +%s); $H prove-block --dir keys --export export/$x --out out/$tag --workers $w > out/$tag.log 2>&1; rc=$?
  e=$(( $(date +%s)-s )); kill $SMI
  u=$(awk -F', ' '{gsub(/ %/,"",$1); s+=$1; n++} END {printf "%.0f", s/n}' out/smi_$tag.log)
  m=$(awk -F', ' '{gsub(/ MiB/,"",$2); if ($2>m) m=$2} END {print m}' out/smi_$tag.log)
  $H verify --dir keys --proof out/$tag/block > out/${tag}_verify.log 2>&1; v=$?
  echo -e "$tag\trc=$rc\tverify=$v\t${e}s\tgpu_util=${u}%\tpeak_mem=${m}MiB" >> $T
  rm -f out/$tag/*.proof.json
done
echo "rc=0" > out/RUN_DONE
