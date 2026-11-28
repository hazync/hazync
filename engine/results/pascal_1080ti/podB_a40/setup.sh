#!/bin/bash
# Test B (28.11.2026): the engine host against stark-backend v2.1.0 + engine/ci/stark-backend-{msvc,pascal}.patch
cd /workspace; log(){ echo "[$(date +%H:%M:%S)] $*"; }
nvidia-smi --query-gpu=name,driver_version,memory.total,compute_cap --format=csv,noheader
apt-get update -qq && apt-get install -y -qq build-essential pkg-config libssl-dev clang curl git bc > /dev/null && log "apt ok"
wget -q https://developer.download.nvidia.com/compute/cuda/repos/ubuntu2204/x86_64/cuda-keyring_1.1-1_all.deb && dpkg -i cuda-keyring_1.1-1_all.deb > /dev/null && apt-get update -qq && apt-get install -y -qq cuda-toolkit-12-9 > /dev/null && log "cuda 12.9 ok"
export PATH=/usr/local/cuda-12.9/bin:$PATH CUDA_HOME=/usr/local/cuda-12.9 CUDA_PATH=/usr/local/cuda-12.9
export CUDA_ARCHS=$(nvidia-smi --query-gpu=compute_cap --format=csv,noheader | head -1 | tr -d .)
curl -sSf https://sh.rustup.rs | sh -s -- -y -q --default-toolchain 1.91.1 > inst_rust.log 2>&1; source ~/.cargo/env
tar xzf bundle.tgz && log "bundle unpacked"
grep -c HZ_FORCE_UNFUSED_BITREV sb/crates/cuda-backend/src/cuda/ntt.rs
s=$(date +%s); (cd engine/host && cargo build --release --features cuda > /workspace/build.log 2>&1) && log "hzengine cuda built in $(( $(date +%s)-s )) s" && echo SETUP_OK || { grep -n "^error" build.log | head; tail -30 build.log; echo SETUP_FAIL; }
