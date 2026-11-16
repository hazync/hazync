#!/bin/bash
# Build one WASM crate byte-reproducibly: the core verifier's (swiftsync-verify-wasm/build.sh) and every zAPP's
# (zapps/<name>/wasm/build.sh) go through here, so they all get the same flags. As ../history/verifier-wasm/build.sh
# did: stage the crate and its path dependencies at one fixed location and remap every embedded path.
#
#   scripts/wasm-build.sh <crate dir> <stage dir> <wasm file> <path dependency dir or included file>...   (relative to the repo)
#
# The .wasm is copied to <crate dir>/target/wasm32-unknown-unknown/release/<wasm file>.
# ⚠ The bytes depend on the STAGE PATH (a SWIFTSYNC_WASM_STAGE override gave other bytes, +235, with no path in the
# strings): each caller passes its own fixed default, and a published sha is for that default only.
set -euo pipefail
[ $# -ge 3 ] || { echo "usage: $0 <crate dir> <stage dir> <wasm file> <path dependency dir or included file>..." >&2; exit 1; }
REPO=$(cd "$(dirname "$0")/.." && pwd)
CRATE=$1 STAGE=$2 WASM=$3
shift 3
# ⛔ THE BYTES DEPEND ON THE COMPILER. Same source, same paths: Rust 1.99.0 gives the published 446fdaeb…,
# 1.97.1 gives a3a85b7f… (measured 2026-11-12, on the release box). So the toolchain is pinned here, not left
# to whatever the machine's default is; a new pin means a new published sha (README, the site, deploy.sh).
TOOLCHAIN=${HAZYNC_WASM_TOOLCHAIN:-1.99.0}
rustup toolchain install "$TOOLCHAIN" --profile minimal --target wasm32-unknown-unknown >/dev/null 2>&1 \
    || rustup target add --toolchain "$TOOLCHAIN" wasm32-unknown-unknown >/dev/null
export RUSTUP_TOOLCHAIN="$TOOLCHAIN"
rm -rf "$STAGE" && mkdir -p "$STAGE"
for c in "$CRATE" "$@"; do
    # a file: one a crate include!s from outside itself (as swiftsync-verify the guest's handover pin, below)
    if [ -f "$REPO/$c" ]; then mkdir -p "$STAGE/$(dirname "$c")" && cp "$REPO/$c" "$STAGE/$c"; continue; fi
    mkdir -p "$STAGE/$c"
    # every target/ dir at any depth (a crate's own, a zAPP's wasm/, ffi/ and cli/ build dirs): build output, never
    # source (no tracked path has a component named target), and a CLI's can be gigabytes
    (cd "$REPO/$c" && tar --exclude=target --exclude=./vectors -cf - .) \
        | (cd "$STAGE/$c" && tar -xf -)
done
# swiftsync-verify include!s the guest's handover pin (one source for the pinned receipt ids)
mkdir -p "$STAGE/prover/methods/guest/src"
cp "$REPO/prover/methods/guest/src/handover_pin.rs" "$STAGE/prover/methods/guest/src/"
SYSROOT=$(rustc --print sysroot)
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$SYSROOT=/rust-sysroot --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=$STAGE=/hazync"
# The target dir is the staged crate's own: a CARGO_TARGET_DIR from the caller (CI's zapps job sets one for the native
# builds) would put the build elsewhere, and a build script's OUT_DIR path is the target dir's.
unset CARGO_TARGET_DIR CARGO_BUILD_TARGET_DIR
(cd "$STAGE/$CRATE" && cargo build --release --target wasm32-unknown-unknown)
OUT="$REPO/$CRATE/target/wasm32-unknown-unknown/release"
mkdir -p "$OUT"
cp "$STAGE/$CRATE/target/wasm32-unknown-unknown/release/$WASM" "$OUT/"
W="$CRATE/target/wasm32-unknown-unknown/release/$WASM"
cd "$REPO"
printf 'built %s\n  raw     %8d bytes\n  gzipped %8d bytes\n  sha256  %s\n' "$W" "$(stat -c%s "$W")" "$(gzip -9 -c "$W" | wc -c)" "$(sha256sum "$W" | cut -d' ' -f1)"
