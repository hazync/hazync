#!/bin/bash
# Build the WASM verifier byte-reproducibly, as ../history/verifier-wasm/build.sh did: stage the crate and its path
# dependencies at one fixed location and remap every embedded path, so any checkout gives the same .wasm.
set -euo pipefail
cd "$(dirname "$0")"
# ⛔ THE BYTES DEPEND ON THE COMPILER. Same source, same paths: Rust 1.99.0 gives the published 446fdaeb…,
# 1.97.1 gives a3a85b7f… (measured 2026-11-12, on the release box). So the toolchain is pinned here, not left
# to whatever the machine's default is; a new pin means a new published sha (README, the site, deploy.sh).
TOOLCHAIN=${HAZYNC_WASM_TOOLCHAIN:-1.99.0}
rustup toolchain install "$TOOLCHAIN" --profile minimal --target wasm32-unknown-unknown >/dev/null 2>&1 \
    || rustup target add --toolchain "$TOOLCHAIN" wasm32-unknown-unknown >/dev/null
export RUSTUP_TOOLCHAIN="$TOOLCHAIN"
REPO=$(cd .. && pwd)
STAGE=${SWIFTSYNC_WASM_STAGE:-/tmp/swiftsync-wasm-build}
rm -rf "$STAGE" && mkdir -p "$STAGE"
for c in swiftsync-verify-wasm swiftsync-verify libzync; do
    mkdir -p "$STAGE/$c"
    (cd "$REPO/$c" && tar --exclude=./target --exclude=./vectors -cf - .) | (cd "$STAGE/$c" && tar -xf -)
done
# swiftsync-verify include!s the guest's handover pin (one source for the pinned receipt ids)
mkdir -p "$STAGE/prover/methods/guest/src"
cp "$REPO/prover/methods/guest/src/handover_pin.rs" "$STAGE/prover/methods/guest/src/"
# ...and the header-chain guest's MMR (one source for the guest, the verifier and the host)
mkdir -p "$STAGE/prover/zyncapps/methods/headers/src"
cp "$REPO/prover/zyncapps/methods/headers/src/mmr.rs" "$STAGE/prover/zyncapps/methods/headers/src/"
SYSROOT=$(rustc --print sysroot)
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$SYSROOT=/rust-sysroot --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=$STAGE=/hazync"
(cd "$STAGE/swiftsync-verify-wasm" && cargo build --release --target wasm32-unknown-unknown)
mkdir -p target/wasm32-unknown-unknown/release
cp "$STAGE/swiftsync-verify-wasm/target/wasm32-unknown-unknown/release/swiftsync_verify_wasm.wasm" target/wasm32-unknown-unknown/release/
W=target/wasm32-unknown-unknown/release/swiftsync_verify_wasm.wasm
printf 'built %s\n  raw     %8d bytes\n  gzipped %8d bytes\n  sha256  %s\n' "$W" "$(stat -c%s "$W")" "$(gzip -9 -c "$W" | wc -c)" "$(sha256sum "$W" | cut -d' ' -f1)"
