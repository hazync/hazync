#!/bin/bash
# Build both libzync guests from clean and print their image ids and ELF hashes. Run inside the container
# (reproduce/libzync.Dockerfile), where every path is fixed: the image id depends on absolute build paths.
set -euo pipefail
cd /hazync-zkvm/prover
# The host toolchain is the one prover/rust-toolchain.toml pins (1.99.1), which the dependency stage installed. It
# reaches the image ids even though the guest is compiled by risc0's pinned Rust (rzup): stable 1.100.0 gave other
# ids from the same source (2026-11-12). Name it explicitly so rustup never resolves a channel inside the image.
RUSTUP_TOOLCHAIN=$(sed -n 's/^channel = "\(.*\)"$/\1/p' rust-toolchain.toml)
[ -n "$RUSTUP_TOOLCHAIN" ] || { echo "no pinned channel in prover/rust-toolchain.toml" >&2; exit 1; }
export RUSTUP_TOOLCHAIN
echo "host toolchain: $RUSTUP_TOOLCHAIN ($(rustc --version)); guest toolchain: $(ls /root/.risc0/toolchains 2>/dev/null | tr '\n' ' ')" >&2
# The product image: the libzync profile builds the coin log, the counting trie and the live-coin hash (E2c / E2;
# design B, the SwiftSync hints file, is retired).
# Only the variables set below may reach the build: an inherited HAZYNC_* lever would change the image while the
# line printed still names the plain one (review 11-03).
for v in $(compgen -e | grep '^HAZYNC_'); do unset "$v"; done
kind=libzync-product
for net in mainnet signet; do
  rm -rf target/riscv-guest   # never reuse a guest across networks (LIBZYNC.md round 8: a stale ELF)
  netenv=(); [ "$net" = signet ] && netenv=(HAZYNC_NETWORK=signet)
  env HAZYNC_BASE=/root/hazync-libzync-base HAZYNC_LIBZYNC=1 "${netenv[@]}" \
    cargo build --profile fasthost -p host -j"$(nproc)" >&2
  id=$(target/fasthost/host method-id | awk 'NR==1{print $NF}')   # awk reads it all: `head -1` closed the pipe early and the host panicked on the broken pipe (2026-11-12, a timing race; fatal under pipefail)
  elf=$(sha256sum target/riscv-guest/methods/method/riscv32im-risc0-zkvm-elf/release/method.bin | cut -c1-64)
  mkdir -p /root/libzync-elf
  cp target/riscv-guest/methods/method/riscv32im-risc0-zkvm-elf/release/method.bin /root/libzync-elf/$net.bin   # what a published proof must run
  echo "$kind $net METHOD_ID $id GUEST_ELF_SHA256 $elf"
done
# The header-chain zyncAPP (ROADMAP K1 / N3): a separate image, never the core's. Pure Rust, built by the same pinned
# guest toolchain; no HAZYNC_* variable reaches it (unset above).
cargo build --profile fasthost -p zyncapp-host -j"$(nproc)" >&2
hid=$(target/fasthost/zyncapp id | awk '/^ZYNCAPP_HEADERS_ID /{print $2}')
helf=target/riscv-guest/zyncapp-methods/zyncapp-headers/riscv32im-risc0-zkvm-elf/release/zyncapp-headers.bin
cp "$helf" /root/libzync-elf/zyncapp-headers.bin
echo "zyncapp-headers mainnet METHOD_ID $hid GUEST_ELF_SHA256 $(sha256sum "$helf" | cut -c1-64)"
