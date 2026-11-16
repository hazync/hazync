#!/bin/bash
# Build one WASM crate byte-reproducibly: every zAPP's (zapps/<name>/wasm/build.sh) goes through here, with the core
# verifier's flags (swiftsync-verify-wasm/build.sh holds the same container block). As ../history/verifier-wasm/build.sh
# did: stage the crate and its path dependencies at one fixed location and remap every embedded path; and since
# 16.11.2026 inside the pinned container reproduce/wasm.Dockerfile (below), so any machine gives the same .wasm.
#
#   scripts/wasm-build.sh <crate dir> <stage dir> <wasm file> <path dependency dir or included file>...   (relative to the repo)
#
# The .wasm is copied to <crate dir>/target/wasm32-unknown-unknown/release/<wasm file>.
# ⚠ The bytes depend on the STAGE PATH (cargo's -C metadata of path dependencies: reproduce/wasm.Dockerfile): each
# zAPP's build.sh passes its own FIXED path (no override), used inside the container, and a published sha is for it.
set -euo pipefail
[ $# -ge 3 ] || { echo "usage: $0 <crate dir> <stage dir> <wasm file> <path dependency dir or included file>..." >&2; exit 1; }
ARGS=("$@")
REPO=$(cd "$(dirname "$0")/.." && pwd)
CRATE=$1 STAGE=$2 WASM=$3
shift 3
# ⛔ THE BYTES DEPEND ON THE COMPILER. Same source, same paths: Rust 1.99.0 gives the published 446fdaeb…,
# 1.97.1 gives a3a85b7f… (measured 2026-11-12, on the release box). So the toolchain is pinned (the block below);
# a new pin means a new published sha (README, the site, deploy.sh).
# The tests-only regtest image (zapps/corecheck's and zapps/common's `regtest-test` feature, scripts/zapp-regtest) is never
# in a WASM module: refused here, on the host, before the container (the feature's compile_error! refuses any build
# without debug assertions as well). zapps/common/test.sh holds the control.
if [ -z "${HAZYNC_WASM_IN_CONTAINER:-}" ]; then
    RT=$(cargo metadata --format-version 1 --filter-platform wasm32-unknown-unknown --manifest-path "$REPO/$CRATE/Cargo.toml" \
        | python3 -c 'import json, sys
m = json.load(sys.stdin)
print(" ".join(sorted(n["id"] for n in m["resolve"]["nodes"] if "regtest-test" in n.get("features", []))))')
    [ -z "$RT" ] || { echo "$0: $CRATE's WASM graph enables the tests-only feature regtest-test ($RT): never in a WASM module" >&2; exit 1; }
fi
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
# The target dir is the staged crate's own: a build script's OUT_DIR path is the target dir's.
unset CARGO_TARGET_DIR CARGO_BUILD_TARGET_DIR
(cd "$STAGE/$CRATE" && cargo build --release --target wasm32-unknown-unknown)
OUT="$REPO/$CRATE/target/wasm32-unknown-unknown/release"
mkdir -p "$OUT"
cp "$STAGE/$CRATE/target/wasm32-unknown-unknown/release/$WASM" "$OUT/"
W="$CRATE/target/wasm32-unknown-unknown/release/$WASM"
cd "$REPO"
printf 'built %s\n  raw     %8d bytes\n  gzipped %8d bytes\n  sha256  %s\n' "$W" "$(stat -c%s "$W")" "$(gzip -9 -c "$W" | wc -c)" "$(sha256sum "$W" | cut -d' ' -f1)"
