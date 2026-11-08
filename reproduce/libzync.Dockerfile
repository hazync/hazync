# Reproducible build of the libzync guest image ids (mainnet and signet), the M a SwiftSync-hints proof is
# verified against (docs/libzync/SWIFTSYNC_PROOFS.md §5 steps 1 and 9).
#
#   docker build -t hazync-libzync-repro -f reproduce/libzync.Dockerfile .
#   docker run --rm hazync-libzync-repro      # prints both ids and ELF hashes
#
# The image id depends on absolute build paths (reproduce/Dockerfile explains), so this fixes them all:
# HOME=/root, the repo at /hazync-zkvm, the patched Core/secp tree at /root/hazync-libzync-base, built by
# scripts/libzync-base.sh from pinned upstream commits and this repo's patches. An id built anywhere else
# (a developer laptop) differs by design and is not the published one.
#
# Stage 1 is the canonical container's dependency stage, unchanged (same pinned base image, toolchain,
# provisioning script). DEPS_IMAGE may name an already-built copy of it to skip the network fetches;
# it must give the same ids, and a from-scratch build is the reference.
ARG DEPS_IMAGE=deps-cpu

FROM ubuntu:22.04@sha256:0e0a0fc6d18feda9db1590da249ac93e8d5abfea8f4c3c0c849ce512b5ef8982 AS deps-cpu
ENV DEBIAN_FRONTEND=noninteractive \
    HOME=/root \
    REPO_DIR=/hazync-zkvm \
    WORK=/root/hazync-build \
    HAZYNC_BASE=/root/hazync-build \
    RISC0_HOME=/root/.risc0 \
    PATH=/root/.risc0/bin:/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
WORKDIR /hazync-zkvm
COPY provision-vps.sh /hazync-zkvm/provision-vps.sh
COPY patches/ /hazync-zkvm/patches/
COPY coreshim/ /hazync-zkvm/coreshim/
RUN --mount=type=secret,id=gh_token,required=false \
    if [ -s /run/secrets/gh_token ]; then export GITHUB_TOKEN="$(cat /run/secrets/gh_token)"; fi && \
    SKIP_GROTH16=1 HAZYNC_PROVISION=deps ./provision-vps.sh

FROM ${DEPS_IMAGE} AS build
COPY . /hazync-zkvm
ENV RECURSION_SRC_PATH=/hazync-zkvm/reproduce/vendor/recursion_zkr.zip
# prover/rust-toolchain.toml asks for channel "stable" plus components the dependency image lacks. Current rustup
# then updates the whole toolchain to today's stable to add them, and that fails on overlayfs (EXDEV), and would
# make the host toolchain depend on the build date. Drop the components line in this copy: the host toolchain stays
# the one the dependency stage installed. The guest, and so the image id, is compiled by risc0's pinned Rust (rzup),
# not by this toolchain. (The canonical reproduce/Dockerfile has the same exposure once its image is not current.)
RUN sed -i '/^components/d' prover/rust-toolchain.toml
RUN bash scripts/libzync-base.sh /root/hazync-libzync-base
RUN bash reproduce/libzync-ids.sh > /root/LIBZYNC_IDS && cat /root/LIBZYNC_IDS
CMD ["cat", "/root/LIBZYNC_IDS"]
