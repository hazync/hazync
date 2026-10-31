#!/bin/bash
# Build both libzync guests from clean and print their image ids and ELF hashes. Run inside the container
# (reproduce/libzync.Dockerfile), where every path is fixed: the image id depends on absolute build paths.
set -euo pipefail
cd /hazync-zkvm/prover
# prover/rust-toolchain.toml says only "stable", which makes rustup try to update inside the image (and fail on
# overlayfs). Use the toolchain the dependency stage installed. The guest, and so the image id, is compiled by
# risc0's pinned Rust (rzup), not by this host toolchain.
export RUSTUP_TOOLCHAIN=$(rustup toolchain list | awk '/^stable/{print $1; exit}')
echo "host toolchain: $RUSTUP_TOOLCHAIN ($(rustc --version)); guest toolchain: $(ls /root/.risc0/toolchains 2>/dev/null | tr '\n' ' ')" >&2
for net in mainnet signet; do
  rm -rf target/riscv-guest   # never reuse a guest across networks (LIBZYNC.md round 8: a stale ELF)
  env HAZYNC_BASE=/root/hazync-libzync-base HAZYNC_LIBZYNC=1 $([ $net = signet ] && echo HAZYNC_NETWORK=signet) \
    cargo build --profile fasthost -p host -j"$(nproc)" >&2
  id=$(target/fasthost/host method-id | head -1 | awk '{print $NF}')
  elf=$(sha256sum target/riscv-guest/methods/method/riscv32im-risc0-zkvm-elf/release/method.bin | cut -c1-64)
  echo "libzync $net METHOD_ID $id GUEST_ELF_SHA256 $elf"
done
