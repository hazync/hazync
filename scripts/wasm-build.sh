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
# this script's absolute path, symlinks resolved, taken before any cd (the container block re-runs it by its path in
# the checkout, and the checkout is derived from it)
SELF_ABS=$(realpath "${BASH_SOURCE[0]}")
REPO=$(cd "$(dirname "$SELF_ABS")/.." && pwd)
CRATE=$1 STAGE=$2 WASM=$3
shift 3
# Engine proofs only (engine/CUTOVER.md C5): every module links engine/verify-core through swiftsync-verify, so OpenVM's
# patched tree is made and staged for every build (engine/scripts/wasm_ovm.sh), and engine/verify-core and engine/journal
# are staged whether or not the caller names them. HAZYNC_WASM_WRITE_LOCK=1 writes the graph's resolved Cargo.lock to
# the output dir instead of building.
# shellcheck source=../engine/scripts/wasm_ovm.sh
. "$REPO/engine/scripts/wasm_ovm.sh"
EXTRA=""
for c in engine/verify-core engine/journal; do case " $* " in *" $c "*) ;; *) EXTRA="$EXTRA $c" ;; esac; done
set -- "$@" $EXTRA
STAGED="$CRATE $* prover/methods/guest/src/handover_pin.rs $OVM_FILES"
HOST_META=(--locked); [ -z "${HAZYNC_WASM_WRITE_LOCK:-}" ] || HOST_META=()
if [ -z "${HAZYNC_WASM_IN_CONTAINER:-}" ]; then
    [ -n "${HAZYNC_WASM_DRY_RUN:-}" ] || ovm_make_tree
    ovm_cfg "$REPO/$OVM_TREE"
fi
# ⛔ THE BYTES DEPEND ON THE COMPILER. Same source, same paths: Rust 1.99.0 gives the published 446fdaeb…,
# 1.97.1 gives a3a85b7f… (measured 2026-11-12, on the release box). So the toolchain is pinned (the block below);
# a new pin means a new published sha (README, the site, deploy.sh).
# The tests-only regtest image (zapps/corecheck's and zapps/common's `regtest-test` feature, scripts/zapp-regtest) is never
# in a WASM module: refused here, on the host, before the container (the feature's compile_error! refuses any build
# without debug assertions as well). zapps/common/test.sh holds the control.
if [ -z "${HAZYNC_WASM_IN_CONTAINER:-}" ]; then
    # --locked: never rewrite the crate's Cargo.lock; a crate without one is refused here, with cargo's own message
    META=$(cargo metadata "${HOST_META[@]}" --format-version 1 --filter-platform wasm32-unknown-unknown "${OVM_CFG[@]}" --manifest-path "$REPO/$CRATE/Cargo.toml") \
        || { echo "$0: cargo metadata --locked failed for $CRATE (above): its Cargo.lock is missing or not up to date" >&2; exit 1; }
    RT=$(printf '%s' "$META" | python3 -c 'import json, sys
m = json.load(sys.stdin)
print(" ".join(sorted(n["id"] for n in m["resolve"]["nodes"] if "regtest-test" in n.get("features", []))))')
    [ -z "$RT" ] || { echo "$0: $CRATE's WASM graph enables the tests-only feature regtest-test ($RT): never in a WASM module" >&2; exit 1; }
fi
# ---- the pinned container: this block is identical in swiftsync-verify-wasm/build.sh and scripts/wasm-build.sh ----
# (scripts/wasm-build.sh exists where the zAPP modules are.) Before it the script sets REPO, CRATE, STAGE, STAGED (the
# repo paths it stages), SELF_ABS (its own path, symlinks resolved, taken before any cd) and ARGS; after it, OUT is
# where the .wasm goes.
# ⛔ Outside reproduce/wasm.Dockerfile the bytes depended on the machine (that file says how, with the measurements):
# the stage path reached them through cargo's -C metadata hash of path dependencies, and a host clang reached a C
# graph's. So the build always runs in that container, at this script's FIXED stage path, as the caller's uid: the
# checkout mounted READ-ONLY (a build script or proc-macro cannot write into it), the output directory writable.
# The host needs docker, cargo >= 1.78 (it reads the v4 Cargo.lock to see whether the graph compiles C), python3 and
# the network (the image's first build; cargo's registry in the container); git, in a git checkout, compares the staged
# files with the commit. Not a wasm32 Rust toolchain: the container has its own. x86_64 only (linux/amd64).
TOOLCHAIN=${HAZYNC_WASM_TOOLCHAIN:-1.99.0}
OUT=$REPO/$CRATE/target/wasm32-unknown-unknown/release
if [ -n "${HAZYNC_WASM_IN_CONTAINER:-}" ] && [ ! -f /etc/hazync-wasm-build-container ]; then
    # the variable alone never skips the container: only the image's own marker file does
    echo "$0: HAZYNC_WASM_IN_CONTAINER is set, but this is not the build container (no /etc/hazync-wasm-build-container)" >&2
    exit 1
fi
if [ -z "${HAZYNC_WASM_IN_CONTAINER:-}" ]; then
    # this script, relative to the checkout (mounted at /hazync-src in the container); SELF_ABS was taken on the
    # script's first lines (CI calls ./swiftsync-verify-wasm/build.sh from the repo root: 16.11.2026)
    [ -n "${SELF_ABS:-}" ] && [ -f "$SELF_ABS" ] || { echo "$0: SELF_ABS (this script's absolute path) is not set" >&2; exit 1; }
    SELF=$(realpath --relative-to="$REPO" "$SELF_ABS")
    [ -f "$REPO/$SELF" ] && [ "${SELF#../}" = "$SELF" ] || { echo "$0: $SELF_ABS is not inside the checkout $REPO" >&2; exit 1; }
    if [ -n "${HAZYNC_WASM_DRY_RUN:-}" ]; then
        # scripts/test-wasm-build-cwd.sh: how the script finds itself and its checkout, from any cwd; nothing is built
        echo "dry run: $CRATE in $REPO, re-run in the container as bash $SELF"; exit 0
    fi
    for t in docker cargo python3; do
        command -v "$t" >/dev/null || { echo "$0: the WASM build needs $t on this machine (docker runs the pinned container; cargo >= 1.78 and python3 read the crate graph), and it is not installed" >&2; exit 1; }
    done
    # The bytes are those of the files staged: a README pins "this commit", so say loudly when they are not the commit's
    # (HAZYNC_WASM_REQUIRE_CLEAN=1 refuses instead).
    if ! git -C "$REPO" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        # a source tarball: nothing to compare the staged files with
        [ -z "${HAZYNC_WASM_REQUIRE_CLEAN:-}" ] || { echo "$0: $REPO is not a git checkout (HAZYNC_WASM_REQUIRE_CLEAN): cannot check the sources" >&2; exit 1; }
        echo "⚠ $CRATE: $REPO is not a git checkout; the .wasm is of these files, whatever commit they came from" >&2
        DIRTY=
    else
        # shellcheck disable=SC2086 # STAGED is a list of repo paths
        DIRTY=$(git -C "$REPO" status --porcelain --untracked-files=all -- $STAGED)
    fi
    if [ -n "$DIRTY" ]; then
        [ -z "${HAZYNC_WASM_REQUIRE_CLEAN:-}" ] || { echo "$0: the staged sources differ from the commit (HAZYNC_WASM_REQUIRE_CLEAN):" >&2; echo "$DIRTY" >&2; exit 1; }
        echo "⚠ $CRATE: the staged sources differ from commit $(git -C "$REPO" rev-parse --short HEAD); the .wasm is of THESE files, not of the commit a README pins:" >&2
        echo "$DIRTY" | sed 's/^/    /' >&2
    fi
    # a crate in the wasm32 graph that declares `links` (a native library) or has cc / cmake as a dependency compiles C:
    # then the image carries the pinned clang. --locked: never rewrite the checkout's Cargo.lock.
    C_CRATES=$(cargo metadata "${HOST_META[@]}" --format-version 1 --filter-platform wasm32-unknown-unknown "${OVM_CFG[@]}" --manifest-path "$REPO/$CRATE/Cargo.toml" \
        | python3 -c '
import json, sys
m = json.load(sys.stdin)
pk = {p["id"]: p for p in m["packages"]}
print(" ".join(sorted({pk[n["id"]]["name"] for n in m["resolve"]["nodes"]
                       if pk[n["id"]].get("links") or any(pk[d["pkg"]]["name"] in ("cc", "cmake") for d in n["deps"])})))') \
        || { echo "$0: cargo metadata --locked failed for $CRATE (Cargo.lock not up to date, or cargo older than 1.78)" >&2; exit 1; }
    WITH_C=0; [ -z "$C_CRATES" ] || WITH_C=1
    # The image: the PUBLISHED one by digest when reproduce/wasm-image.digests has a line for its tag (pulled once, then
    # used as long as the local tag still carries that digest); else reproduce/wasm.Dockerfile built here, said loudly.
    # A release refuses a local build (HAZYNC_WASM_REQUIRE_PUBLISHED=1, package-release.sh) unless
    # HAZYNC_WASM_ALLOW_LOCAL_IMAGE=1 says so explicitly.
    TAG=$("$REPO/reproduce/wasm-image-tag.sh" "$TOOLCHAIN" "$WITH_C")
    IMG=hazync-wasm-build:$TAG REMOTE=ghcr.io/hazync/hazync-wasm-build
    DIGEST=$(awk -v t="$TAG" '$1 == t && $2 ~ /^sha256:[0-9a-f]{64}$/ { print $2 }' "$REPO/reproduce/wasm-image.digests" 2>/dev/null | head -1)
    if [ -n "$DIGEST" ]; then
        if ! docker image inspect --format '{{join .RepoDigests " "}}' "$IMG" 2>/dev/null | tr ' ' '\n' | grep -qxF "$REMOTE@$DIGEST"; then
            docker pull --platform linux/amd64 "$REMOTE@$DIGEST" >&2 && docker tag "$REMOTE@$DIGEST" "$IMG" \
                || { echo "$0: could not pull the published build image $REMOTE@$DIGEST" >&2; exit 1; }
        fi
        ORIGIN="the published image $REMOTE@$DIGEST"
    else
        if [ -n "${HAZYNC_WASM_REQUIRE_PUBLISHED:-}" ] && [ -z "${HAZYNC_WASM_ALLOW_LOCAL_IMAGE:-}" ]; then
            echo "$0: no published build image for tag $TAG in reproduce/wasm-image.digests, and a release takes no local build (HAZYNC_WASM_REQUIRE_PUBLISHED; HAZYNC_WASM_ALLOW_LOCAL_IMAGE=1 overrides)" >&2
            exit 1
        fi
        echo "⚠ no published build image for tag $TAG (reproduce/wasm-image.digests): building reproduce/wasm.Dockerfile LOCALLY" >&2
        docker image inspect "$IMG" >/dev/null 2>&1 || docker build --platform linux/amd64 --progress=plain -t "$IMG" \
            --build-arg RUST_TOOLCHAIN="$TOOLCHAIN" --build-arg WITH_C="$WITH_C" -f "$REPO/reproduce/wasm.Dockerfile" "$REPO/reproduce" >&2
        ORIGIN="a LOCAL build of reproduce/wasm.Dockerfile, $IMG"
    fi
    echo "building $CRATE in $ORIGIN${C_CRATES:+ (it compiles C: $C_CRATES)}" >&2
    mkdir -p "$OUT"
    exec docker run --rm --platform linux/amd64 --user "$(id -u):$(id -g)" -e HOME=/tmp -e HAZYNC_WASM_TOOLCHAIN="$TOOLCHAIN" -e HAZYNC_WASM_WRITE_LOCK="${HAZYNC_WASM_WRITE_LOCK:-}" \
        --mount "type=bind,src=$REPO,dst=/hazync-src,readonly" --mount "type=bind,src=$OUT,dst=/hazync-out" \
        -w /hazync-src "$IMG" bash "$SELF" "${ARGS[@]}"
fi
OUT=/hazync-out
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
# --locked: the staged Cargo.lock is the checkout's, used as it is (never re-resolved)
ovm_stage
ovm_build_or_lock "$STAGE/$CRATE"
cp "$STAGE/$CRATE/target/wasm32-unknown-unknown/release/$WASM" "$OUT/"
printf 'built %s\n  raw     %8d bytes\n  gzipped %8d bytes\n  sha256  %s\n' "$CRATE/target/wasm32-unknown-unknown/release/$WASM" \
    "$(stat -c%s "$OUT/$WASM")" "$(gzip -9 -c "$OUT/$WASM" | wc -c)" "$(sha256sum "$OUT/$WASM" | cut -d' ' -f1)"
