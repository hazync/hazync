# The build image for the ENGINE (OpenVM) image ids (engine/ROADMAP.md 4.1): reproduce/engine-ids.sh runs the build in
# it, the checkout mounted read-only, and prints each image's exe commitment, vm commitment and image id
# (SHA-256(exe || vm), what every verifier pins).
#
#   reproduce/engine-ids.sh            # builds this image if it is missing, then the ids (results in reproduce/out-engine)
#
# Everything that reaches an id is pinned here or in the checkout:
#   - the base image (digest) and apt packages from a dated snapshot;
#   - rustup-init (version + SHA-256); the HOST toolchain that builds hzengine (1.97.1: the build box's, which built
#     the reference images) and the GUEST toolchain OpenVM v2.1.0's build calls (nightly-2026-02-28 + rust-src,
#     openvm-build DEFAULT_RUSTUP_TOOLCHAIN_NAME);
#   - the riscv32 gcc/g++ 13.2 the guest's C/C++ consensus code compiles with: RISC Zero's toolchain release
#     2024.01.05 (rzup's `cpp 2024.1.5`), fetched by SHA-256 and unpacked where engine/guest/build.rs looks
#     ($HOME/.risc0/cpp). Its gcc and cc1 are byte-identical to the laptop's and the build box's rzup installs;
#   - HOME, CARGO_HOME and RUSTUP_HOME, fixed (they reach the program as paths; engine.rs remap_flags maps them too).
# The checkout pins the rest: Core v28.0 and libsecp v0.5.1 by commit (scripts/libzync-base.sh, engine/guest/base.sh),
# OpenVM v2.1.0 by tag (the guest manifests), hzengine's Cargo.lock, and the guests' own locks (reproduce/engine-locks).
FROM ubuntu:24.04@sha256:73b999393d07f2bf65a40373778a44dc9a6d54ad7718d29247d8ff9413c4d6a8
ARG APT_SNAPSHOT=20261101T000000Z
ARG RUSTUP_VERSION=1.28.2
ARG RUSTUP_SHA256=20a06e644b0d9bd2fbdbfd52d42540bdde820ea7df86e92e533c073da0cdd43c
ARG HOST_TOOLCHAIN=1.97.1
ARG GUEST_TOOLCHAIN=nightly-2026-02-28
ARG CPP_URL=https://github.com/risc0/toolchain/releases/download/2024.01.05/riscv32im-linux-x86_64.tar.xz
ARG CPP_SHA256=cc19497db5fd1ccd92fa3d315a33cacd4ba480f8d21b3c84dfb5493cfd68da0d
ENV DEBIAN_FRONTEND=noninteractive \
    HOME=/root \
    CARGO_HOME=/root/.cargo \
    RUSTUP_HOME=/root/.rustup \
    PATH=/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
# A transient mirror error (snapshot.ubuntu.com once gave a 503 on the build box, 27.11.2026) retries rather than
# failing the build: apt and every curl download retry 5 times. Retries change no fetched byte (each file is pinned by
# the snapshot date or a SHA-256 check).
RUN printf 'Acquire::Retries "5";\n' > /etc/apt/apt.conf.d/80-retries \
 && S=/etc/apt/sources.list.d/ubuntu.sources \
 && printf 'Types: deb\nURIs: http://archive.ubuntu.com/ubuntu/\nSuites: noble\nComponents: main\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n' > $S \
 && apt-get update && apt-get install -y --no-install-recommends ca-certificates \
 && printf 'Types: deb\nURIs: https://snapshot.ubuntu.com/ubuntu/%s/\nSuites: noble noble-updates noble-security\nComponents: main universe\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n' "$APT_SNAPSHOT" > $S \
 && apt-get update \
 && apt-get install -y --no-install-recommends gcc g++ libc6-dev make pkg-config git python3 curl xz-utils patch \
 && rm -rf /var/lib/apt/lists/* \
 && curl --retry 5 --retry-all-errors -sSfo /tmp/rustup-init "https://static.rust-lang.org/rustup/archive/$RUSTUP_VERSION/x86_64-unknown-linux-gnu/rustup-init" \
 && echo "$RUSTUP_SHA256  /tmp/rustup-init" | sha256sum -c - \
 && chmod +x /tmp/rustup-init \
 && /tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain "$HOST_TOOLCHAIN" \
 && rm /tmp/rustup-init \
 && rustup toolchain install --profile minimal --component rust-src "$GUEST_TOOLCHAIN" \
 && curl --retry 5 --retry-all-errors -sSfLo /tmp/cpp.tar.xz "$CPP_URL" \
 && echo "$CPP_SHA256  /tmp/cpp.tar.xz" | sha256sum -c - \
 && mkdir -p /root/.risc0 && tar -xJf /tmp/cpp.tar.xz -C /root/.risc0 && mv /root/.risc0/riscv32im-linux-x86_64 /root/.risc0/cpp \
 && rm /tmp/cpp.tar.xz
RUN echo 'reproduce/engine.Dockerfile' > /etc/hazync-engine-build-container
