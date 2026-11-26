#!/bin/bash
# Build the WASM verifier byte-reproducibly, as ../history/verifier-wasm/build.sh did: stage the crate and its path
# dependencies at one fixed location and remap every embedded path, so any checkout gives the same .wasm. The build
# itself runs in the pinned container reproduce/wasm.Dockerfile (below), so any machine gives the same .wasm too.
set -euo pipefail
ARGS=("$@")
# this script's absolute path, symlinks resolved, taken before any cd (the container block re-runs it by its path in
# the checkout, and the checkout is derived from it)
SELF_ABS=$(realpath "${BASH_SOURCE[0]}")
cd "$(dirname "$SELF_ABS")"
# ⛔ THE BYTES DEPEND ON THE COMPILER. Same source, same paths: Rust 1.99.0 gives the published 446fdaeb…,
# 1.97.1 gives a3a85b7f… (measured 2026-11-12, on the release box). So the toolchain is pinned (the block below),
# not left to whatever the machine's default is; a new pin means a new published sha (README, the site, deploy.sh).
REPO=$(cd .. && pwd)
CRATE=swiftsync-verify-wasm
# engine/verify-core always: swiftsync-verify names it as an OPTIONAL path dependency, and cargo loads every path
# dependency of the graph, enabled or not (a default build without it fails: "no matching package")
STAGED="swiftsync-verify-wasm swiftsync-verify libzync engine/verify-core prover/methods/guest/src/handover_pin.rs"
# --engine: the module with engine (OpenVM) proofs (engine/VERIFIER.md), swiftsync_verify_wasm_engine.wasm. OpenVM and
# stark-backend v2.1.0 patched (abi-stable, nozstd, nologging): made on the HOST by engine/scripts/openvm_src.sh from the
# pinned commits (the container has no git), under this crate's target/ (never staged as source, ignored by git), and
# accepted only with the tree digest below, which the CONTAINER recomputes before staging it: a tree it did not fetch
# is used only if it is exactly this one (engine/scripts/tree_digest.sh).
ENGINE=0
for a in "${ARGS[@]}"; do [ "$a" = --engine ] && ENGINE=1; done
OVM_PATCHES="engine/wasm-verify/openvm-abi-stable.patch engine/handover/openvm-verify-nozstd.patch engine/handover/stark-sdk-nologging.patch"
OVM_DIGEST=38eccd0935910175840b4644340fb1b788b5c13a0818c57f46b2503b0547fa6e
OVM_TREE=$CRATE/target/openvm-src
if [ "$ENGINE" = 1 ]; then
    STAGED="$STAGED engine/scripts/openvm_src.sh engine/scripts/tree_digest.sh $OVM_PATCHES"
    if [ -z "${HAZYNC_WASM_IN_CONTAINER:-}" ]; then
        # shellcheck disable=SC2086 # the patch list
        (cd "$REPO" && OVM_CACHE="$REPO/$CRATE/target/openvm-cache" engine/scripts/openvm_src.sh "$OVM_TREE" $OVM_PATCHES >&2)
        got=$("$REPO/engine/scripts/tree_digest.sh" "$REPO/$OVM_TREE")
        [ "$got" = "$OVM_DIGEST" ] || { echo "$0: the patched OpenVM tree's digest is $got, not the pinned $OVM_DIGEST" >&2; exit 1; }
    fi
fi
# The stage path is FIXED: it reaches the bytes (reproduce/wasm.Dockerfile). SWIFTSYNC_WASM_STAGE is gone (a build at
# another stage gave 05754e5f…, 235 bytes larger): refuse it rather than ignore it.
STAGE=/tmp/swiftsync-wasm-build
[ -z "${SWIFTSYNC_WASM_STAGE:-}" ] || { echo "SWIFTSYNC_WASM_STAGE is no longer honoured: the stage path reaches the bytes, so it is fixed ($STAGE, inside the build container)" >&2; exit 1; }
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
    C_CRATES=$(cargo metadata --locked --format-version 1 --filter-platform wasm32-unknown-unknown --manifest-path "$REPO/$CRATE/Cargo.toml" \
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
    exec docker run --rm --platform linux/amd64 --user "$(id -u):$(id -g)" -e HOME=/tmp -e HAZYNC_WASM_TOOLCHAIN="$TOOLCHAIN" \
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
for c in swiftsync-verify-wasm swiftsync-verify libzync; do
    mkdir -p "$STAGE/$c"
    # every target/ dir at any depth (build output, never source; no tracked path has a component named target)
    (cd "$REPO/$c" && tar --exclude=target --exclude=./vectors -cf - .) | (cd "$STAGE/$c" && tar -xf -)
done
mkdir -p "$STAGE/engine/verify-core"
(cd "$REPO/engine/verify-core" && tar --exclude=target -cf - .) | (cd "$STAGE/engine/verify-core" && tar -xf -)
# swiftsync-verify include!s the guest's handover pin (one source for the pinned receipt ids)
mkdir -p "$STAGE/prover/methods/guest/src"
cp "$REPO/prover/methods/guest/src/handover_pin.rs" "$STAGE/prover/methods/guest/src/"
SYSROOT=$(rustc --print sysroot)
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$SYSROOT=/rust-sysroot --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=$STAGE=/hazync"
unset CARGO_TARGET_DIR CARGO_BUILD_TARGET_DIR
# --locked: the staged Cargo.lock is the checkout's, used as it is (never re-resolved)
if [ "$ENGINE" = 1 ]; then
    # the tree checked HERE, by the container, before anything of it is staged (the host made it)
    got=$("$REPO/engine/scripts/tree_digest.sh" "$REPO/$OVM_TREE")
    [ "$got" = "$OVM_DIGEST" ] || { echo "the patched OpenVM tree's digest is $got, not the pinned $OVM_DIGEST: refusing" >&2; exit 1; }
    mkdir -p "$STAGE/openvm-src"
    (cd "$REPO/$OVM_TREE" && tar --exclude=./.cache -cf - .) | (cd "$STAGE/openvm-src" && tar -xf -)
    # the engine graph's own lock (every default package and version kept; the patched crates added as path packages)
    cp "$REPO/$CRATE/Cargo.engine.lock" "$STAGE/swiftsync-verify-wasm/Cargo.lock"
    CFG=(--config 'patch."https://github.com/openvm-org/openvm.git".openvm-verify-stark-host.path="../openvm-src/openvm/crates/verify"')
    while read -r f; do
        n=$(sed -n '/^\[package\]/,/^\[/{s/^name = "\(.*\)"/\1/p}' "$f" | head -1)
        d=$(dirname "$f")
        [ -n "$n" ] && CFG+=(--config "patch.\"https://github.com/openvm-org/stark-backend.git\".$n.path=\"../openvm-src/stark-backend/${d#"$STAGE/openvm-src/stark-backend/"}\"")
    done < <(find "$STAGE/openvm-src/stark-backend/crates" -name Cargo.toml -not -path "*/tests/*" -not -path "*/benches/*" | LC_ALL=C sort)
    # a random source that refuses (no randomness in verification; swiftsync-verify-wasm src/lib.rs no_rng)
    export RUSTFLAGS="$RUSTFLAGS --cfg getrandom_backend=\"custom\""
    (cd "$STAGE/swiftsync-verify-wasm" && cargo build --locked --release --target wasm32-unknown-unknown --features engine "${CFG[@]}")
    W=swiftsync_verify_wasm_engine.wasm
    cp "$STAGE/swiftsync-verify-wasm/target/wasm32-unknown-unknown/release/swiftsync_verify_wasm.wasm" "$OUT/$W"
else
(cd "$STAGE/swiftsync-verify-wasm" && cargo build --locked --release --target wasm32-unknown-unknown)
W=swiftsync_verify_wasm.wasm
cp "$STAGE/swiftsync-verify-wasm/target/wasm32-unknown-unknown/release/$W" "$OUT/"
fi
printf 'built %s\n  raw     %8d bytes\n  gzipped %8d bytes\n  sha256  %s\n' "$CRATE/target/wasm32-unknown-unknown/release/$W" \
    "$(stat -c%s "$OUT/$W")" "$(gzip -9 -c "$OUT/$W" | wc -c)" "$(sha256sum "$OUT/$W" | cut -d' ' -f1)"
