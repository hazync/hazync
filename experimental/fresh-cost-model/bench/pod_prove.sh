#!/bin/bash
# Runs ON the GPU pod after setup. Each prove is timed end-to-end incl. witness gen; proof verified.
source ~/.cargo/env
export PATH=/usr/local/cuda-12.9/bin:$PATH; export PATH="$HOME/.risc0/bin:$HOME/.sp1/bin:$HOME/.cargo/bin:$PATH"
cd /workspace
BIN=/workspace/b976379.bin
nvidia-smi --query-gpu=name --format=csv,noheader
for m in ${MODES:-1 3 2}; do
  RISC0_DEV_MODE=0 ./risc0/target/release/bench-risc0 $m $BIN prove > r0_prove_$m.log 2>&1; grep RESULT r0_prove_$m.log || echo "RESULT vm=risc0 mode=$m FAILED"
  SP1_PROVER=cuda ./sp1/script/target/release/bench-sp1 $m $BIN prove > sp1_prove_$m.log 2>&1; grep RESULT sp1_prove_$m.log || echo "RESULT vm=sp1 mode=$m FAILED"
  if [ "$m" != 2 ]; then
    ( cd openvm && s=$(date +%s.%N); cargo openvm prove app --input ../ovm_in_$m.json > ../ovm_prove_$m.log 2>&1; rc=$?; e=$(date +%s.%N)
      cargo openvm verify app > ../ovm_verify_$m.log 2>&1; vrc=$?
      echo "RESULT vm=openvm mode=$m action=prove rc=$rc verify_rc=$vrc secs=$(echo "$e - $s" | bc)" )
  fi
done
echo PROVE_DONE
