#!/bin/bash
# build_hosts.sh <src tarball of origin/main> <out dir> <net: mainnet|regtest> [jobs]
# Builds the libzync product host from origin/main INSIDE the published-image build container (hazync-libzync-repro,
# reproduce/libzync.Dockerfile: the fixed paths /hazync-zkvm and /root/hazync-libzync-base, pinned host toolchain), so
# the mainnet build must print the published 46666b6f id, and the regtest build differs from it ONLY by
# HAZYNC_NETWORK=regtest. Commands as reproduce/libzync-ids.sh (fasthost profile, --locked, every HAZYNC_* cleared).
set -euo pipefail
SRC=$(readlink -f "$1"); OUT=$(readlink -m "$2"); NET=$3; J=${4:-8}; mkdir -p "$OUT"
netenv=""; [ "$NET" = regtest ] && netenv="HAZYNC_NETWORK=regtest"
docker run --rm -v "$SRC":/src.tgz:ro -v "$OUT":/out hazync-libzync-repro bash -c "
  set -euo pipefail
  rm -rf /hazync-zkvm && mkdir /hazync-zkvm && tar xzf /src.tgz -C /hazync-zkvm
  cd /hazync-zkvm && sed -i '/^components/d' prover/rust-toolchain.toml && cd prover
  export RUSTUP_TOOLCHAIN=\$(sed -n 's/^channel = \"\(.*\)\"\$/\1/p' rust-toolchain.toml)
  for v in \$(compgen -e | grep '^HAZYNC_'); do unset \"\$v\"; done
  env HAZYNC_BASE=/root/hazync-libzync-base HAZYNC_LIBZYNC=1 $netenv cargo build --locked --profile fasthost -p host -j$J
  cp target/fasthost/host /out/host_$NET
  sha256sum target/riscv-guest/methods/method/riscv32im-risc0-zkvm-elf/release/method.bin > /out/elf_$NET.sha256
  /out/host_$NET method-id > /out/method_id_$NET.txt
" > "$OUT/build_$NET.log" 2>&1
echo "BUILD_DONE $NET rc=$?" >> "$OUT/build_$NET.log"
