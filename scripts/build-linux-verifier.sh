#!/usr/bin/env bash
# Build the libzync verifier CLI (swiftsync-verify) for a release, on the hosts' glibc floor.
#
#   scripts/build-linux-verifier.sh x86_64  <out file>    # hazync-verify-x86_64-linux-gnu (scripts/release.sh)
#   scripts/build-linux-verifier.sh aarch64 <out file>    # hazync-verify-aarch64 (release-sign.yml), cross-compiled
#
# WHY. Every verifier from v0.23.0 to v0.27.0 was built by the build machine's OWN toolchain (the release box's
# cargo, Ubuntu 24.04; an ubuntu-latest runner for aarch64) and so needs GLIBC_2.39, while the hosts are built in
# ubuntu:22.04 and need 2.34. On Ubuntu 22.04 the v0.26.1 and v0.27.0 verifier does not start at all. Here the
# build runs in the SAME pinned ubuntu:22.04 image as prover/build-release.sh, with the toolchain the wasm build
# pins (1.99.0), and the result is refused unless scripts/check-glibc-floor.sh passes it.
#
# Needs docker only (no Rust on the machine). The toolchain and the crate cache live in docker volumes
# (hazync-verifier-rustup, hazync-verifier-cargo), so a re-run downloads nothing. Env: IMAGE, HAZYNC_VERIFIER_TOOLCHAIN.
set -euo pipefail
ARCH="${1:-}"; OUT="${2:-}"
case "$ARCH" in
    x86_64)  TRIPLE=x86_64-unknown-linux-gnu;  CROSS="" ;;
    aarch64) TRIPLE=aarch64-unknown-linux-gnu; CROSS="gcc-aarch64-linux-gnu libc6-dev-arm64-cross" ;;
    *) echo "usage: $0 {x86_64|aarch64} <out file>" >&2; exit 2 ;;
esac
[ -n "$OUT" ] || { echo "usage: $0 {x86_64|aarch64} <out file>" >&2; exit 2; }
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# The same image, by digest, as prover/build-release.sh: one glibc floor for every Linux asset.
IMAGE="${IMAGE:-ubuntu:22.04@sha256:0e0a0fc6d18feda9db1590da249ac93e8d5abfea8f4c3c0c849ce512b5ef8982}"
TOOLCHAIN="${HAZYNC_VERIFIER_TOOLCHAIN:-1.99.0}"
command -v docker >/dev/null || { echo "$0: docker is required" >&2; exit 1; }
# Its own target directory: a target built on the host (another glibc) must never be linked in here.
TGT=swiftsync-verify/target/glibc234
docker run --rm -v "$REPO:/src" -w /src \
    -v hazync-verifier-rustup:/root/.rustup -v hazync-verifier-cargo:/root/.cargo-cache \
    -e "TOOLCHAIN=$TOOLCHAIN" -e "TRIPLE=$TRIPLE" -e "CROSS=$CROSS" -e "TGT=$TGT" -e "HOST_UID=$(id -u)" -e "HOST_GID=$(id -g)" \
    -e DEBIAN_FRONTEND=noninteractive "$IMAGE" bash -euc '
    apt-get update -qq >/dev/null
    apt-get install -y -qq --no-install-recommends ca-certificates curl gcc libc6-dev $CROSS >/dev/null
    export CARGO_HOME=/root/.cargo-cache PATH=/root/.cargo-cache/bin:$PATH
    command -v rustup >/dev/null || curl -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal --default-toolchain none --no-modify-path
    rustup toolchain install "$TOOLCHAIN" --profile minimal
    rustup target add --toolchain "$TOOLCHAIN" "$TRIPLE"
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
    cargo "+$TOOLCHAIN" build --release --locked --manifest-path swiftsync-verify/Cargo.toml \
        --bin swiftsync-verify --target "$TRIPLE" --target-dir "$TGT"
    chown -R "$HOST_UID:$HOST_GID" "$TGT"
'
BIN="$REPO/$TGT/$TRIPLE/release/swiftsync-verify"
"$REPO/scripts/check-glibc-floor.sh" "$BIN" || { echo "$0: the verifier needs a newer glibc than the hosts -- not staged" >&2; exit 1; }
install -m 0755 "$BIN" "$OUT.tmp.$$" && mv -f "$OUT.tmp.$$" "$OUT"
echo "staged $OUT ($ARCH, $(sha256sum "$OUT" | cut -c1-16)…)"
