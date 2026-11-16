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
ARGS=("$@")
CRATE=$1 STAGE=$2 WASM=$3
shift 3
# ⛔ THE BYTES DEPEND ON THE COMPILER. Same source, same paths: Rust 1.99.0 gives the published 446fdaeb…,
# 1.97.1 gives a3a85b7f… (measured 2026-11-12, on the release box). So the toolchain is pinned here, not left
# to whatever the machine's default is; a new pin means a new published sha (README, the site, deploy.sh).
TOOLCHAIN=${HAZYNC_WASM_TOOLCHAIN:-1.99.0}
rustup toolchain install "$TOOLCHAIN" --profile minimal --target wasm32-unknown-unknown >/dev/null 2>&1 \
    || rustup target add --toolchain "$TOOLCHAIN" wasm32-unknown-unknown >/dev/null
export RUSTUP_TOOLCHAIN="$TOOLCHAIN"
# ⛔ C CODE IN THE GRAPH: the machine's C compiler reaches the bytes. zapps/statement/wasm (bitcoin -> secp256k1-sys,
# compiled by clang) was 510c205b with clang 14.0.0 and 5a284b26 with clang 18.1.3, same source and Rust (16.11.2026);
# a pure-Rust module (the core's, every other zAPP's) reproduced on both machines. So a crate graph that compiles C is
# built inside the pinned container reproduce/wasm.Dockerfile (base image by digest, Ubuntu archive snapshot, clang
# 18.1.3 by exact version, this Rust), never with the host's clang. "Compiles C": a crate in the wasm32 graph that
# declares `links` (a native library) or has `cc` / `cmake` as a build dependency.
compiles_c() {
    cargo metadata --format-version 1 --filter-platform wasm32-unknown-unknown --manifest-path "$REPO/$CRATE/Cargo.toml" \
        | python3 -c '
import json, sys
m = json.load(sys.stdin)
pk = {p["id"]: p for p in m["packages"]}
c = sorted({pk[n["id"]]["name"] for n in m["resolve"]["nodes"]
            if pk[n["id"]].get("links") or any(pk[d["pkg"]]["name"] in ("cc", "cmake") for d in n["deps"])})
print(" ".join(c))'
}
if [ -z "${HAZYNC_WASM_IN_CONTAINER:-}" ]; then
    C_CRATES=$(compiles_c)
    if [ -n "$C_CRATES" ]; then
        command -v docker >/dev/null || { echo "$CRATE compiles C ($C_CRATES): it is built in reproduce/wasm.Dockerfile, and docker is not installed" >&2; exit 1; }
        IMG=hazync-wasm-build:$(cat "$REPO/reproduce/wasm.Dockerfile" <(echo "$TOOLCHAIN") | sha256sum | cut -c1-16)
        docker image inspect "$IMG" >/dev/null 2>&1 \
            || docker build -q -t "$IMG" --build-arg RUST_TOOLCHAIN="$TOOLCHAIN" -f "$REPO/reproduce/wasm.Dockerfile" "$REPO/reproduce" >&2
        echo "$CRATE compiles C ($C_CRATES): building in $IMG (reproduce/wasm.Dockerfile)" >&2
        # the same script and arguments inside; the stage is the container's own, at the same path
        exec docker run --rm --network=bridge --user "$(id -u):$(id -g)" -e HOME=/tmp -e HAZYNC_WASM_TOOLCHAIN="$TOOLCHAIN" \
            -v "$REPO:/hazync-src" -w /hazync-src "$IMG" bash scripts/wasm-build.sh "${ARGS[@]}"
    fi
fi
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
if [ -n "${HAZYNC_WASM_IN_CONTAINER:-}" ]; then
    # inside reproduce/wasm.Dockerfile: its clang and llvm-ar, checked, and the C paths remapped as the Rust ones are
    clang --version | head -1 | grep -q ' 18\.1\.3 ' || { echo "the container's clang is not 18.1.3: $(clang --version | head -1)" >&2; exit 1; }
    export CC_wasm32_unknown_unknown=clang AR_wasm32_unknown_unknown=llvm-ar
    export CFLAGS_wasm32_unknown_unknown="-ffile-prefix-map=${CARGO_HOME:-$HOME/.cargo}=/cargo -ffile-prefix-map=$STAGE=/hazync"
fi
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
