# The pinned build environment of every WASM module: the core verifier's (swiftsync-verify-wasm/build.sh) and every
# zAPP's (scripts/wasm-build.sh). Both scripts re-run themselves in it, so the bytes depend on the source alone.
#
# Why a container (measured 16.11.2026):
#   - the STAGE PATH reached the bytes despite --remap-path-prefix: the core built f16b5b98 staged at
#     /tmp/swiftsync-wasm-build and 7d79f68c staged elsewhere (54 bytes and one function apart). Cargo hashes a path
#     dependency that lies outside the building workspace by its ABSOLUTE path into the crate's -C metadata
#     (swiftsync-verify and libzync are siblings of swiftsync-verify-wasm, so outside its workspace): another path,
#     other symbol hashes, other inlining. Remapping cannot reach that hash; one fixed path, the same everywhere, can.
#     In here the stage is always the script's fixed path, whatever the host's paths, users or concurrent builds.
#   - the C COMPILER reached the bytes: zapps/statement/wasm compiles secp256k1-sys with clang: 510c205b with clang
#     14.0.0, 5a284b26 with clang 18.1.3, same source and Rust. Here clang is pinned (only for a graph that compiles C).
# Pinned: the base image by digest; every Ubuntu package from one snapshot.ubuntu.com snapshot; clang-18 / llvm-18 by
# exact version; rustup-init by version and sha256; the Rust toolchain (the scripts' pin, passed in).
#
#   docker build -t <tag> --build-arg RUST_TOOLCHAIN=1.99.0 [--build-arg WITH_C=1] -f reproduce/wasm.Dockerfile reproduce
#   (the scripts do this themselves, tagged by this file's sha256, the toolchain and WITH_C)
FROM ubuntu:24.04@sha256:73b999393d07f2bf65a40373778a44dc9a6d54ad7718d29247d8ff9413c4d6a8
ARG RUST_TOOLCHAIN
ARG WITH_C=0
ARG APT_SNAPSHOT=20261101T000000Z
ARG CLANG_VERSION=1:18.1.3-1ubuntu1
ARG RUSTUP_VERSION=1.28.2
ARG RUSTUP_SHA256=20a06e644b0d9bd2fbdbfd52d42540bdde820ea7df86e92e533c073da0cdd43c
ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/usr/local/rustup \
    PATH=/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
# The snapshot service answers over https only and the base image has no CA certificates: they come first from the
# noble RELEASE pocket alone (frozen since April 2024, so pinned too); then every source is the one snapshot. apt
# verifies every package against the archive keyring either way.
RUN test -n "$RUST_TOOLCHAIN" \
 && S=/etc/apt/sources.list.d/ubuntu.sources \
 && printf 'Types: deb\nURIs: http://archive.ubuntu.com/ubuntu/\nSuites: noble\nComponents: main\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n' > $S \
 && apt-get update && apt-get install -y --no-install-recommends ca-certificates \
 && printf 'Types: deb\nURIs: https://snapshot.ubuntu.com/ubuntu/%s/\nSuites: noble noble-updates noble-security\nComponents: main universe\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n' "$APT_SNAPSHOT" > $S \
 && apt-get update \
 && apt-get install -y --no-install-recommends gcc libc6-dev curl gzip \
 && if [ "$WITH_C" = 1 ]; then \
        apt-get install -y --no-install-recommends "clang-18=$CLANG_VERSION" "llvm-18=$CLANG_VERSION" "libclang-cpp18=$CLANG_VERSION" \
        && ln -s /usr/bin/clang-18 /usr/local/bin/clang && ln -s /usr/bin/llvm-ar-18 /usr/local/bin/llvm-ar; fi \
 && rm -rf /var/lib/apt/lists/* \
 && curl -sSfo /tmp/rustup-init "https://static.rust-lang.org/rustup/archive/$RUSTUP_VERSION/x86_64-unknown-linux-gnu/rustup-init" \
 && echo "$RUSTUP_SHA256  /tmp/rustup-init" | sha256sum -c - \
 && chmod +x /tmp/rustup-init \
 && CARGO_HOME=/usr/local/cargo /tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain "$RUST_TOOLCHAIN" \
        --target wasm32-unknown-unknown \
 && rm /tmp/rustup-init \
 && chmod -R a+rwX /usr/local/rustup /usr/local/cargo
# The build runs as the caller's uid (its output lands in the caller's checkout); cargo's registry cache goes to a
# writable CARGO_HOME of its own (remapped out of the bytes like any CARGO_HOME).
ENV CARGO_HOME=/tmp/cargo-home HAZYNC_WASM_IN_CONTAINER=1 HAZYNC_WASM_WITH_C=${WITH_C}
