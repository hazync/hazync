#!/bin/bash
# Continuation: toolchains for risc0+sp1 and CUDA 12.9 already installed by pod_setup.sh.
source ~/.cargo/env
export PATH=/usr/local/cuda-12.9/bin:$HOME/.risc0/bin:$HOME/.sp1/bin:$HOME/.cargo/bin:$PATH CUDA_HOME=/usr/local/cuda-12.9 CUDA_PATH=/usr/local/cuda-12.9
export CUDA_ARCHS=$(nvidia-smi --query-gpu=compute_cap --format=csv,noheader | head -1 | tr -d .)
cd /workspace
log(){ echo "[$(date +%H:%M:%S)] $*"; }
rustup install 1.97.1 > inst_rust.log 2>&1 && log "rust 1.97.1 ok"
( rustup install nightly-2026-02-28 && rustup component add rust-src --toolchain nightly-2026-02-28 && \
  cargo +1.91.1 install --locked --git https://github.com/openvm-org/openvm.git --tag v2.1.0 --features cuda cargo-openvm ) > inst_openvm.log 2>&1 && log "openvm cli ok" || log "openvm cli FAILED"
[ -d risc0 ] || tar xzf bench.tgz
( cd risc0 && cargo +1.97.1 build --release --features cuda ) > build_risc0.log 2>&1 && log "risc0 built" || log "risc0 BUILD FAILED"
( cd sp1/script && cargo +1.91.1 build --release --features cuda ) > build_sp1.log 2>&1 && log "sp1 built" || log "sp1 BUILD FAILED"
( cd openvm && cargo openvm build && s=$(date +%s) && cargo openvm keygen --app-only && echo "openvm keygen secs=$(( $(date +%s)-s ))" ) > build_openvm.log 2>&1 && log "openvm built" || log "openvm BUILD FAILED"
grep "keygen secs" build_openvm.log
log SETUP_DONE
