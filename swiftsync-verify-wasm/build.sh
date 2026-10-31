#!/bin/bash
# Build the WASM verifier byte-reproducibly, as ../verifier-wasm/build.sh does: stage the crate and its path
# dependencies at one fixed location and remap every embedded path, so any checkout gives the same .wasm.
set -euo pipefail
cd "$(dirname "$0")"
rustup target add wasm32-unknown-unknown 2>/dev/null || true
REPO=$(cd .. && pwd)
STAGE=${SWIFTSYNC_WASM_STAGE:-/tmp/swiftsync-wasm-build}
rm -rf "$STAGE" && mkdir -p "$STAGE"
for c in swiftsync-verify-wasm swiftsync-verify libzync; do
    mkdir -p "$STAGE/$c"
    (cd "$REPO/$c" && tar --exclude=./target --exclude=./vectors -cf - .) | (cd "$STAGE/$c" && tar -xf -)
done
SYSROOT=$(rustc --print sysroot)
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$SYSROOT=/rust-sysroot --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=$STAGE=/hazync"
(cd "$STAGE/swiftsync-verify-wasm" && cargo build --release --target wasm32-unknown-unknown)
mkdir -p target/wasm32-unknown-unknown/release
cp "$STAGE/swiftsync-verify-wasm/target/wasm32-unknown-unknown/release/swiftsync_verify_wasm.wasm" target/wasm32-unknown-unknown/release/
W=target/wasm32-unknown-unknown/release/swiftsync_verify_wasm.wasm
printf 'built %s\n  raw     %8d bytes\n  gzipped %8d bytes\n  sha256  %s\n' "$W" "$(stat -c%s "$W")" "$(gzip -9 -c "$W" | wc -c)" "$(sha256sum "$W" | cut -d' ' -f1)"
