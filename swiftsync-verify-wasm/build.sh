#!/bin/bash
# Build the WASM verifier byte-reproducibly, as ../history/verifier-wasm/build.sh did: stage the crate and its path
# dependencies at one fixed location and remap every embedded path, so any checkout gives the same .wasm. The build
# itself runs in the pinned container reproduce/wasm.Dockerfile (below), so any machine gives the same .wasm too.
set -euo pipefail
ARGS=("$@")
cd "$(dirname "$0")"
# ⛔ THE BYTES DEPEND ON THE COMPILER. Same source, same paths: Rust 1.99.0 gives the published 446fdaeb…,
# 1.97.1 gives a3a85b7f… (measured 2026-11-12, on the release box). So the toolchain is pinned (the block below),
# not left to whatever the machine's default is; a new pin means a new published sha (README, the site, deploy.sh).
REPO=$(cd .. && pwd)
CRATE=swiftsync-verify-wasm
# The stage path is FIXED: it reaches the bytes (reproduce/wasm.Dockerfile). SWIFTSYNC_WASM_STAGE is gone (a build at
# another stage gave 05754e5f…, 235 bytes larger): refuse it rather than ignore it.
STAGE=/tmp/swiftsync-wasm-build
[ -z "${SWIFTSYNC_WASM_STAGE:-}" ] || { echo "SWIFTSYNC_WASM_STAGE is no longer honoured: the stage path reaches the bytes, so it is fixed ($STAGE, inside the build container)" >&2; exit 1; }
# ---- the pinned container: this block is identical in swiftsync-verify-wasm/build.sh and scripts/wasm-build.sh ----
# ⛔ Outside reproduce/wasm.Dockerfile the bytes depended on the machine (that file says how, with the measurements):
# the stage path reached them through cargo's -C metadata hash of path dependencies, and a host clang reached a C
# graph's. So the build always runs in that container, at this script's FIXED stage path, as the caller's uid with the
# checkout mounted; the host needs docker (and cargo, to see whether the graph compiles C), not a Rust toolchain.
TOOLCHAIN=${HAZYNC_WASM_TOOLCHAIN:-1.99.0}
if [ -z "${HAZYNC_WASM_IN_CONTAINER:-}" ]; then
    command -v docker >/dev/null || { echo "$0: the WASM build runs in reproduce/wasm.Dockerfile, and docker is not installed" >&2; exit 1; }
    # a crate in the wasm32 graph that declares `links` (a native library) or has cc / cmake as a dependency compiles C:
    # then the image carries the pinned clang
    C_CRATES=$(cargo metadata --format-version 1 --filter-platform wasm32-unknown-unknown --manifest-path "$REPO/$CRATE/Cargo.toml" \
        | python3 -c '
import json, sys
m = json.load(sys.stdin)
pk = {p["id"]: p for p in m["packages"]}
print(" ".join(sorted({pk[n["id"]]["name"] for n in m["resolve"]["nodes"]
                       if pk[n["id"]].get("links") or any(pk[d["pkg"]]["name"] in ("cc", "cmake") for d in n["deps"])})))')
    WITH_C=0; [ -z "$C_CRATES" ] || WITH_C=1
    IMG=hazync-wasm-build:$(cat "$REPO/reproduce/wasm.Dockerfile" <(echo "$TOOLCHAIN $WITH_C") | sha256sum | cut -c1-16)
    docker image inspect "$IMG" >/dev/null 2>&1 || docker build -q -t "$IMG" --build-arg RUST_TOOLCHAIN="$TOOLCHAIN" \
        --build-arg WITH_C="$WITH_C" -f "$REPO/reproduce/wasm.Dockerfile" "$REPO/reproduce" >&2
    echo "building $CRATE in $IMG (reproduce/wasm.Dockerfile${C_CRATES:+; it compiles C: $C_CRATES})" >&2
    SELF=$(realpath --relative-to="$REPO" "${BASH_SOURCE[0]}")
    exec docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp -e HAZYNC_WASM_TOOLCHAIN="$TOOLCHAIN" \
        -v "$REPO:/hazync-src" -w /hazync-src "$IMG" bash "$SELF" "${ARGS[@]}"
fi
export RUSTUP_TOOLCHAIN="$TOOLCHAIN"
rustc --version | grep -q " $TOOLCHAIN " || { echo "the container's Rust is not $TOOLCHAIN: $(rustc --version)" >&2; exit 1; }
if [ "${HAZYNC_WASM_WITH_C:-0}" = 1 ]; then
    # the image's clang and llvm-ar, checked, and the C paths remapped as the Rust ones are
    clang --version | head -1 | grep -q ' 18\.1\.3 ' || { echo "the container's clang is not 18.1.3: $(clang --version | head -1)" >&2; exit 1; }
    export CC_wasm32_unknown_unknown=clang AR_wasm32_unknown_unknown=llvm-ar
    export CFLAGS_wasm32_unknown_unknown="-ffile-prefix-map=$CARGO_HOME=/cargo -ffile-prefix-map=$STAGE=/hazync"
fi
# ---- end of the identical block ----
rm -rf "$STAGE" && mkdir -p "$STAGE"
for c in swiftsync-verify-wasm swiftsync-verify libzync; do
    mkdir -p "$STAGE/$c"
    # every target/ dir at any depth (build output, never source; no tracked path has a component named target)
    (cd "$REPO/$c" && tar --exclude=target --exclude=./vectors -cf - .) | (cd "$STAGE/$c" && tar -xf -)
done
# swiftsync-verify include!s the guest's handover pin (one source for the pinned receipt ids)
mkdir -p "$STAGE/prover/methods/guest/src"
cp "$REPO/prover/methods/guest/src/handover_pin.rs" "$STAGE/prover/methods/guest/src/"
SYSROOT=$(rustc --print sysroot)
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$SYSROOT=/rust-sysroot --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=$STAGE=/hazync"
unset CARGO_TARGET_DIR CARGO_BUILD_TARGET_DIR
(cd "$STAGE/swiftsync-verify-wasm" && cargo build --release --target wasm32-unknown-unknown)
mkdir -p target/wasm32-unknown-unknown/release
cp "$STAGE/swiftsync-verify-wasm/target/wasm32-unknown-unknown/release/swiftsync_verify_wasm.wasm" target/wasm32-unknown-unknown/release/
W=target/wasm32-unknown-unknown/release/swiftsync_verify_wasm.wasm
printf 'built %s\n  raw     %8d bytes\n  gzipped %8d bytes\n  sha256  %s\n' "$W" "$(stat -c%s "$W")" "$(gzip -9 -c "$W" | wc -c)" "$(sha256sum "$W" | cut -d' ' -f1)"
