#!/bin/bash
# On the pod: prove-block over (export, workers, segment GiB) to measure chunk sizing, GPU sharing and segment size.
# JOBS="name:workers:segGiB ..." (segGiB = HZ_SEG_MEM_GB; OpenVM's default is 15). A bare "name": no flags, the
# engine's own defaults (ENGINE.md, Defaults). "name@n": on the first n GPUs only. A trailing "~" (name~, name@n~):
# HZ_SEGPAR=0, the aggregate's segments on one GPU as before (no segment workers).
cd /workspace || exit 1
# shellcheck source=/dev/null
source ~/.cargo/env; export PATH=/usr/local/cuda-12.9/bin:$PATH
mkdir -p out; H=host/target/release/hzengine; T=out/times.tsv
nvidia-smi --query-gpu=name,memory.total --format=csv,noheader > out/gpu.txt
for j in ${JOBS:-}; do
  IFS=: read -r x w g lc ic <<< "$j"; ng=""; sp=""
  # optional 5th field: HZ_INTERNAL_CHILDREN (proofs per internal proof; prover-side)
  if [ -n "$ic" ]; then export HZ_INTERNAL_CHILDREN=$ic; else unset HZ_INTERNAL_CHILDREN; fi
  # optional 4th field: HZ_LEAF_CHILDREN (app segment proofs per leaf proof; prover-side)
  if [ -n "$lc" ]; then export HZ_LEAF_CHILDREN=$lc; else unset HZ_LEAF_CHILDREN; fi
  case $x in *\~) sp=_noseg; x=${x%\~};; esac
  if [ -n "$sp" ]; then export HZ_SEGPAR=0; else unset HZ_SEGPAR; fi
  case $x in *@*) ng=${x#*@}; x=${x%@*};; esac
  if [ -n "$ng" ]; then CUDA_VISIBLE_DEVICES=$(seq -s, 0 $((ng-1))); export CUDA_VISIBLE_DEVICES; else unset CUDA_VISIBLE_DEVICES; fi
  if [ -z "$w" ]; then tag=${x}_default${ng:+_g$ng}$sp; else g=${g:-15}; tag=${x}_w${w}_s$g${lc:+_l$lc}${ic:+_i$ic}${ng:+_g$ng}$sp; fi
  ( nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader -lms 200 > out/smi_$tag.log ) & SMI=$!
  s=$(date +%s)
  if [ -z "$w" ]; then $H prove-block --dir keys --export export/$x --out out/$tag > out/$tag.log 2>&1; rc=$?
  else env "$(case $g in *m) echo HZ_SEG_MEM_MB=${g%m};; *) echo HZ_SEG_MEM_GB=$g;; esac)" $H prove-block --dir keys --export export/$x --out out/$tag --workers $w > out/$tag.log 2>&1; rc=$?; fi
  e=$(( $(date +%s)-s )); kill $SMI
  u=$(awk -F', ' '{gsub(/ %/,"",$1); s+=$1; n++} END {printf "%.0f", s/n}' out/smi_$tag.log)
  m=$(awk -F', ' '{gsub(/ MiB/,"",$2); if ($2+0>m+0) m=$2+0} END {print m}' out/smi_$tag.log)   # numeric (as text "673" > "6643")
  $H verify --dir keys --proof out/$tag/block > out/${tag}_verify.log 2>&1; v=$?
  echo -e "$tag\trc=$rc\tverify=$v\t${e}s\tgpu_util=${u}%\tpeak_mem=${m}MiB" >> $T
  rm -f out/$tag/*.proof.json
done
echo "rc=0" > out/RUN_DONE
