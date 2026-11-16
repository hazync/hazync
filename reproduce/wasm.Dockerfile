# The pinned build environment of a WASM module whose crate graph compiles C (scripts/wasm-build.sh uses it on its
# own when it finds such a crate; today zapps/statement/wasm, through bitcoin -> secp256k1 -> secp256k1-sys).
#
# Why: a pure-Rust module's bytes depend only on the Rust toolchain (pinned in scripts/wasm-build.sh) and the stage
# path, and reproduce on any machine (the core's 622ce430 on a laptop and on hazync-coord). C code is compiled by the
# machine's clang, and its version reaches the bytes: the statement module was 510c205b with clang 14.0.0 (a laptop) and
# 5a284b26 with clang 18.1.3 (hazync-coord), same source, same Rust (16.11.2026). So the C compiler is pinned here, with
# everything else that reaches the build:
#   - the base image by digest, and every Ubuntu package from one archive snapshot (snapshot.ubuntu.com), so the clang,
#     llvm-ar and host gcc (build scripts only) are the same bytes on every machine and every day;
#   - clang-18 / llvm-18 by exact version as well, so a snapshot that ever resolved differently fails the build instead
#     of giving other bytes;
#   - rustup-init by version and sha256, and the Rust toolchain by version (scripts/wasm-build.sh's pin, passed in).
#
#   docker build -t hazync-wasm-build:<tag> --build-arg RUST_TOOLCHAIN=1.99.0 -f reproduce/wasm.Dockerfile reproduce
#   (scripts/wasm-build.sh does this itself, tagged by this file's sha256 and the toolchain.)
FROM ubuntu:24.04@sha256:73b999393d07f2bf65a40373778a44dc9a6d54ad7718d29247d8ff9413c4d6a8
ARG RUST_TOOLCHAIN
ARG APT_SNAPSHOT=20261101T000000Z
ARG CLANG_VERSION=1:18.1.3-1ubuntu1
ARG RUSTUP_VERSION=1.28.2
ARG RUSTUP_SHA256=20a06e644b0d9bd2fbdbfd52d42540bdde820ea7df86e92e533c073da0cdd43c
ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/usr/local/rustup \
    PATH=/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
# The snapshot service answers over https only, and the base image has no CA certificates: they come first from the
# noble RELEASE pocket alone (frozen since April 2024, so pinned too); then every source is the one snapshot. Packages
# are verified by apt's archive keyring either way.
RUN test -n "$RUST_TOOLCHAIN" \
 && S=/etc/apt/sources.list.d/ubuntu.sources \
 && printf 'Types: deb\nURIs: http://archive.ubuntu.com/ubuntu/\nSuites: noble\nComponents: main\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n' > $S \
 && apt-get update && apt-get install -y --no-install-recommends ca-certificates \
 && printf 'Types: deb\nURIs: https://snapshot.ubuntu.com/ubuntu/%s/\nSuites: noble noble-updates noble-security\nComponents: main universe\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n' "$APT_SNAPSHOT" > $S \
 && apt-get update \
 && apt-get install -y --no-install-recommends \
        "clang-18=$CLANG_VERSION" "llvm-18=$CLANG_VERSION" "libclang-cpp18=$CLANG_VERSION" \
        gcc libc6-dev curl gzip python3 \
 && rm -rf /var/lib/apt/lists/* \
 && ln -s /usr/bin/clang-18 /usr/local/bin/clang && ln -s /usr/bin/llvm-ar-18 /usr/local/bin/llvm-ar \
 && curl -sSfo /tmp/rustup-init "https://static.rust-lang.org/rustup/archive/$RUSTUP_VERSION/x86_64-unknown-linux-gnu/rustup-init" \
 && echo "$RUSTUP_SHA256  /tmp/rustup-init" | sha256sum -c - \
 && chmod +x /tmp/rustup-init \
 && CARGO_HOME=/usr/local/cargo /tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain "$RUST_TOOLCHAIN" \
        --target wasm32-unknown-unknown \
 && rm /tmp/rustup-init \
 && chmod -R a+rwX /usr/local/rustup /usr/local/cargo
# the build runs as the caller's uid (its output lands in the caller's checkout); cargo's registry and git caches go to a
# writable CARGO_HOME of their own (its path is remapped out of the bytes like any CARGO_HOME)
ENV CARGO_HOME=/tmp/cargo-home HAZYNC_WASM_IN_CONTAINER=1
