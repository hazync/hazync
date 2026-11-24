#!/bin/bash
# On the pod: prove-block over (export, workers, segment GiB) to measure chunk sizing, GPU sharing and segment size.
# JOBS="name:workers:segGiB ..." (segGiB = HZ_SEG_MEM_GB; OpenVM's default is 15). A bare "name": no flags, the
# engine's own defaults (ENGINE.md, Defaults).
cd /workspace; source ~/.cargo/env; export PATH=/usr/local/cuda-12.9/bin:$PATH
mkdir -p out; H=host/target/release/hzengine; T=out/times.tsv
nvidia-smi --query-gpu=name,memory.total --format=csv,noheader > out/gpu.txt
for j in ${JOBS:-}; do
  IFS=: read -r x w g <<< "$j"
  if [ -z "$w" ]; then tag=${x}_default; else g=${g:-15}; tag=${x}_w${w}_s$g; fi
  ( nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader -l 1 > out/smi_$tag.log ) & SMI=$!
  s=$(date +%s)
  if [ -z "$w" ]; then $H prove-block --dir keys --export export/$x --out out/$tag > out/$tag.log 2>&1; rc=$?
  else HZ_SEG_MEM_GB=$g $H prove-block --dir keys --export export/$x --out out/$tag --workers $w > out/$tag.log 2>&1; rc=$?; fi
  e=$(( $(date +%s)-s )); kill $SMI
  u=$(awk -F', ' '{gsub(/ %/,"",$1); s+=$1; n++} END {printf "%.0f", s/n}' out/smi_$tag.log)
  m=$(awk -F', ' '{gsub(/ MiB/,"",$2); if ($2>m) m=$2} END {print m}' out/smi_$tag.log)
  $H verify --dir keys --proof out/$tag/block > out/${tag}_verify.log 2>&1; v=$?
  echo -e "$tag\trc=$rc\tverify=$v\t${e}s\tgpu_util=${u}%\tpeak_mem=${m}MiB" >> $T
  rm -f out/$tag/*.proof.json
done
echo "rc=0" > out/RUN_DONE
