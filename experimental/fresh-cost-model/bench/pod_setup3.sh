#!/bin/bash
# Run 2 setup ON the GPU pod: SP1 + OpenVM only (risc0 ruled out by run 1). No tee/process substitution (run 1 deadlocked on it).
cd /workspace
log(){ echo "[$(date +%H:%M:%S)] $*"; }
nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader
apt-get update -qq && apt-get install -y -qq build-essential pkg-config libssl-dev clang curl git bc protobuf-compiler zstd > /dev/null && log "apt ok"
wget -q https://developer.download.nvidia.com/compute/cuda/repos/ubuntu2204/x86_64/cuda-keyring_1.1-1_all.deb && dpkg -i cuda-keyring_1.1-1_all.deb > /dev/null && apt-get update -qq && apt-get install -y -qq cuda-toolkit-12-9 > /dev/null && log "cuda 12.9 ok"
export PATH=/usr/local/cuda-12.9/bin:$PATH CUDA_HOME=/usr/local/cuda-12.9 CUDA_PATH=/usr/local/cuda-12.9
export CUDA_ARCHS=$(nvidia-smi --query-gpu=compute_cap --format=csv,noheader | head -1 | tr -d .)
curl -sSf https://sh.rustup.rs | sh -s -- -y -q --default-toolchain 1.91.1 > inst_rust.log 2>&1; source ~/.cargo/env
export PATH="$HOME/.sp1/bin:$HOME/.cargo/bin:$PATH"
tar xzf bench.tgz
( curl -sL https://sp1up.succinct.xyz | bash && $HOME/.sp1/bin/sp1up --version v6.10.0 && \
  cd sp1/script && cargo +1.91.1 build --release --features cuda ) > build_sp1.log 2>&1 && log "sp1 built" || log "sp1 BUILD FAILED" &
( rustup install nightly-2026-02-28 && rustup component add rust-src --toolchain nightly-2026-02-28 && \
  cargo +1.91.1 install --locked --git https://github.com/openvm-org/openvm.git --tag v2.1.0 --features cuda cargo-openvm && \
  cd openvm && cargo openvm build && cargo openvm setup && cargo openvm keygen ) > build_openvm.log 2>&1 && log "openvm built + setup + keygen" || log "openvm FAILED" &
wait
log SETUP_DONE
