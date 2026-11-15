#!/bin/bash
# Build the balance statement zyncAPP's WASM module byte-reproducibly, as ../../../swiftsync-verify-wasm/build.sh does:
# the same pinned Rust, the sources staged at one fixed path, every embedded path remapped.
set -euo pipefail
cd "$(dirname "$0")"
TOOLCHAIN=${HAZYNC_WASM_TOOLCHAIN:-1.99.0}
rustup toolchain install "$TOOLCHAIN" --profile minimal --target wasm32-unknown-unknown >/dev/null 2>&1 \
    || rustup target add --toolchain "$TOOLCHAIN" wasm32-unknown-unknown >/dev/null
export RUSTUP_TOOLCHAIN="$TOOLCHAIN"
REPO=$(cd ../../.. && pwd)
STAGE=${ZAPP_STATEMENT_WASM_STAGE:-/tmp/zapp-statement-wasm-build}
rm -rf "$STAGE" && mkdir -p "$STAGE/zapps"
(cd "$REPO/zapps" && tar --exclude=target --exclude=./statement/vectors -cf - statement) | (cd "$STAGE/zapps" && tar -xf -)
for c in swiftsync-verify libzync; do
    mkdir -p "$STAGE/$c"
    (cd "$REPO/$c" && tar --exclude=./target --exclude=./vectors -cf - .) | (cd "$STAGE/$c" && tar -xf -)
done
# swiftsync-verify include!s the guest's handover pin (one source for the pinned receipt ids)
mkdir -p "$STAGE/prover/methods/guest/src"
cp "$REPO/prover/methods/guest/src/handover_pin.rs" "$STAGE/prover/methods/guest/src/"
SYSROOT=$(rustc --print sysroot)
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$SYSROOT=/rust-sysroot --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=$STAGE=/hazync"
(cd "$STAGE/zapps/statement/wasm" && cargo build --release --target wasm32-unknown-unknown)
mkdir -p target/wasm32-unknown-unknown/release
W=target/wasm32-unknown-unknown/release/zapp_statement_wasm.wasm
cp "$STAGE/zapps/statement/wasm/$W" "$W"
printf 'built %s\n  raw     %8d bytes\n  gzipped %8d bytes\n  sha256  %s\n' "$W" "$(stat -c%s "$W")" "$(gzip -9 -c "$W" | wc -c)" "$(sha256sum "$W" | cut -d' ' -f1)"
