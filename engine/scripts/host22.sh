#!/bin/bash
# host22.sh <hazync checkout> <out dir>: the PRODUCT host (libzync profile) built in ubuntu:22.04 (glibc 2.35, the RunPod
# image's), for a GPU pod that runs the product host with the engine as its prover. The RISC Zero guest is NOT rebuilt:
# the checkout's own build (prover/target/riscv-guest/.../method.bin) is embedded under pins (METHOD_ID, sha256,
# PROFILE) that build.rs checks, so the binary carries exactly that image.
set -euo pipefail
SRC=$(readlink -f "$1"); OUT=$(readlink -f "$2"); mkdir -p "$OUT/pins" "$OUT/target"
BIN=$SRC/prover/target/riscv-guest/methods/method/riscv32im-risc0-zkvm-elf/release/method.bin
[ -f "$BIN" ] || { echo "no built guest at $BIN (build the host from source once first)"; exit 1; }
cp "$BIN" "$OUT/pins/method.bin"
"$SRC/prover/target/release/host" method-id 2>/dev/null | awk '/^METHOD_ID/ {print $2}' > "$OUT/pins/METHOD_ID"
sha256sum "$OUT/pins/method.bin" | cut -c1-64 > "$OUT/pins/GUEST_ELF_SHA256"
echo "libzync=1 assumevalid=0 network=mainnet coinlog=1 trie=1 utxohash=1" > "$OUT/pins/PROFILE"
cat "$OUT/pins/METHOD_ID" "$OUT/pins/GUEST_ELF_SHA256"
docker run --rm -v /root/.cargo:/root/.cargo -v /root/.rustup:/root/.rustup -v "$SRC":/src -v "$OUT":/out -w /src/prover \
  ubuntu:22.04 bash -c '
    set -e; export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq && apt-get install -y -qq build-essential pkg-config libssl-dev clang protobuf-compiler git curl > /dev/null
    export PATH=/root/.cargo/bin:$PATH CARGO_TARGET_DIR=/out/target
    HAZYNC_GUEST_ELF=/out/pins/method.bin HAZYNC_GUEST_PIN_DIR=/out/pins HAZYNC_LIBZYNC=1 cargo build --release -p host
    /out/target/release/host method-id | tail -2'
ls -la "$OUT/target/release/host"
