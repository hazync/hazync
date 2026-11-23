#!/bin/bash
# Runs ON the GPU pod. Installs toolchains, builds all three provers with CUDA, proves modes 1-3, logs RESULT lines.
set -u
cd /workspace
exec > >(tee -a setup.log) 2>&1
nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader
nvidia-smi | grep -o "CUDA Version: [0-9.]*"
apt-get update -qq && apt-get install -y -qq build-essential pkg-config libssl-dev clang curl git bc > /dev/null
# CUDA 12.9 toolkit (OpenVM's recommended); used for ALL three builds so the comparison is like-for-like
wget -q https://developer.download.nvidia.com/compute/cuda/repos/ubuntu2204/x86_64/cuda-keyring_1.1-1_all.deb && dpkg -i cuda-keyring_1.1-1_all.deb > /dev/null   && apt-get update -qq && apt-get install -y -qq cuda-toolkit-12-9 > /dev/null && echo "cuda 12.9 toolkit installed"
export PATH=/usr/local/cuda-12.9/bin:$PATH CUDA_HOME=/usr/local/cuda-12.9 CUDA_PATH=/usr/local/cuda-12.9
nvcc --version | tail -1
GPU_CC=$(nvidia-smi --query-gpu=compute_cap --format=csv,noheader | head -1 | tr -d .)
export CUDA_ARCHS=$GPU_CC
curl -sSf https://sh.rustup.rs | sh -s -- -y -q --default-toolchain 1.91.1 && source ~/.cargo/env
rustup install -q 1.97.0
export PATH="$HOME/.risc0/bin:$HOME/.sp1/bin:$HOME/.cargo/bin:$PATH"
# toolchains (same versions as the CPU run)
( curl -sL https://risczero.com/install | bash && $HOME/.risc0/bin/rzup install cargo-risczero 3.1.0 && $HOME/.risc0/bin/rzup install r0vm 3.1.0 && $HOME/.risc0/bin/rzup install rust ) > inst_risc0.log 2>&1 &
( curl -sL https://sp1up.succinct.xyz | bash && $HOME/.sp1/bin/sp1up --version v6.10.0 ) > inst_sp1.log 2>&1 &
( rustup install -q nightly-2026-02-28 && rustup component add rust-src --toolchain nightly-2026-02-28 && \
  cargo +1.91.1 install -q --locked --git https://github.com/openvm-org/openvm.git --tag v2.1.0 --features cuda cargo-openvm ) > inst_openvm.log 2>&1 &
wait
echo "toolchains installed"
tar xzf bench.tgz
# CUDA builds, one at a time (nvcc is memory hungry)
( cd risc0 && cargo +1.97.0 build --release --features cuda ) > build_risc0.log 2>&1 && echo "risc0 built" || echo "risc0 BUILD FAILED"
( cd sp1/script && cargo +1.91.1 build --release --features cuda ) > build_sp1.log 2>&1 && echo "sp1 built" || echo "sp1 BUILD FAILED"
( cd openvm && cargo openvm build && s=$(date +%s) && cargo openvm keygen --app-only && echo "openvm keygen secs=$(( $(date +%s)-s ))" ) > build_openvm.log 2>&1 && echo "openvm built" || echo "openvm BUILD FAILED"
grep "keygen secs" build_openvm.log
echo SETUP_DONE
