#!/bin/bash
# Build the Utreexo zAPP checker as WASM, byte-reproducibly, as ../../../swiftsync-verify-wasm/build.sh does: stage the
# crate and its path dependencies at one fixed location, remap every embedded path, pin the toolchain.
set -euo pipefail
cd "$(dirname "$0")"
# ⛔ THE BYTES DEPEND ON THE COMPILER (swiftsync-verify-wasm/build.sh): the toolchain is pinned, a new pin is a new sha.
TOOLCHAIN=${HAZYNC_WASM_TOOLCHAIN:-1.99.0}
rustup toolchain install "$TOOLCHAIN" --profile minimal --target wasm32-unknown-unknown >/dev/null 2>&1 \
    || rustup target add --toolchain "$TOOLCHAIN" wasm32-unknown-unknown >/dev/null
export RUSTUP_TOOLCHAIN="$TOOLCHAIN"
REPO=$(cd ../../.. && pwd)
STAGE=${ZAPP_UTREEXO_WASM_STAGE:-/tmp/zapp-utreexo-wasm-build}
rm -rf "$STAGE" && mkdir -p "$STAGE"
for c in zapps/utreexo swiftsync-verify libzync; do
    mkdir -p "$STAGE/$c"
    (cd "$REPO/$c" && tar --exclude=./target --exclude=./wasm/target --exclude=./vectors -cf - .) | (cd "$STAGE/$c" && tar -xf -)
done
# swiftsync-verify include!s the guest's handover pin (one source for the pinned receipt ids)
mkdir -p "$STAGE/prover/methods/guest/src"
cp "$REPO/prover/methods/guest/src/handover_pin.rs" "$STAGE/prover/methods/guest/src/"
SYSROOT=$(rustc --print sysroot)
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$SYSROOT=/rust-sysroot --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=$STAGE=/hazync"
(cd "$STAGE/zapps/utreexo/wasm" && cargo build --release --target wasm32-unknown-unknown)
mkdir -p target/wasm32-unknown-unknown/release
cp "$STAGE/zapps/utreexo/wasm/target/wasm32-unknown-unknown/release/zapp_utreexo_wasm.wasm" target/wasm32-unknown-unknown/release/
W=target/wasm32-unknown-unknown/release/zapp_utreexo_wasm.wasm
printf 'built %s\n  raw     %8d bytes\n  gzipped %8d bytes\n  sha256  %s\n' "$W" "$(stat -c%s "$W")" "$(gzip -9 -c "$W" | wc -c)" "$(sha256sum "$W" | cut -d' ' -f1)"
