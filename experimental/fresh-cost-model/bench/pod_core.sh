#!/bin/bash
# Run 3 ON the pod: prove the prebuilt Core-on-OpenVM guest (ovm-core, batch+liftx) for block 976,379 on this card:
# app proof (comparable to Hazync's chunk receipts) and the STARK wrap (one proof), each verified.
cd /workspace; log(){ echo "[$(date +%H:%M:%S)] $*"; }
nvidia-smi --query-gpu=name,driver_version --format=csv,noheader
apt-get update -qq && apt-get install -y -qq build-essential pkg-config libssl-dev clang curl git bc > /dev/null && log "apt ok"
wget -q https://developer.download.nvidia.com/compute/cuda/repos/ubuntu2204/x86_64/cuda-keyring_1.1-1_all.deb && dpkg -i cuda-keyring_1.1-1_all.deb > /dev/null && apt-get update -qq && apt-get install -y -qq cuda-toolkit-12-9 > /dev/null && log "cuda 12.9 ok"
export PATH=/usr/local/cuda-12.9/bin:$PATH CUDA_HOME=/usr/local/cuda-12.9 CUDA_PATH=/usr/local/cuda-12.9
export CUDA_ARCHS=$(nvidia-smi --query-gpu=compute_cap --format=csv,noheader | head -1 | tr -d .)
curl -sSf https://sh.rustup.rs | sh -s -- -y -q --default-toolchain 1.91.1 > inst_rust.log 2>&1; source ~/.cargo/env
cargo +1.91.1 install --locked --git https://github.com/openvm-org/openvm.git --tag v2.1.0 --features cuda cargo-openvm > inst_openvm.log 2>&1 && log "openvm cli ok" || { log "openvm cli FAILED"; echo CORE_DONE; exit 1; }
sha256sum ovm-core-batch.vmexe
cd /workspace/ovm-core   # the CLI resolves its manifest here; --exe skips any build
mkdir -p ovk && cargo openvm keygen --config openvm.toml --output-dir ovk > ../keygen.log 2>&1; log "keygen rc=$?"; ls ovk
cargo openvm setup > ../setup_agg.log 2>&1; log "agg setup rc=$?"
( nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader -l 2 > ../smi.log ) & SMI=$!
s=$(date +%s.%N); cargo openvm prove app --exe ../ovm-core-batch.vmexe --config openvm.toml --app-pk ovk/app.pk --input ../core_batch.json --proof core.app.proof > ../prove_app.log 2>&1; rc=$?; e=$(date +%s.%N)
cargo openvm verify app --app-vk ovk/app.vk --proof core.app.proof > ../verify_app.log 2>&1; v=$?
echo "RESULT vm=openvm-core tag=block_976379 action=app rc=$rc verify_rc=$v secs=$(echo "$e - $s" | bc)"
s=$(date +%s.%N); cargo openvm prove stark --exe ../ovm-core-batch.vmexe --config openvm.toml --app-pk ovk/app.pk --agg-prefix-pk ovk/agg_prefix.pk --input ../core_batch.json --proof core.stark.proof > ../prove_stark.log 2>&1; rc=$?; e=$(date +%s.%N)
cargo openvm verify stark --proof core.stark.proof > ../verify_stark.log 2>&1; v=$?
echo "RESULT vm=openvm-core tag=block_976379 action=stark rc=$rc verify_rc=$v secs=$(echo "$e - $s" | bc) bytes=$(stat -c %s core.stark.proof 2>/dev/null)"
kill $SMI; awk -F', ' '{gsub(/ %/,"",$1); s+=$1; n++} END {print "gpu util mean", s/n}' ../smi.log
echo CORE_DONE
