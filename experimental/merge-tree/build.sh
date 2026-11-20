#!/bin/bash
# build.sh: on hazync-coord, build the EXPERIMENTAL host+guest (origin/main + merge-tree.patch: guest modes 19 k-ary join
# and 20 empty session, host `mt-join`). Never the shipped image: the patch changes the guest, so the image id differs
# from 46666b6f by construction (printed at the end). Tree in /root/merge-tree/src, Core base in /root/merge-tree/base.
set -euo pipefail
export PATH=/root/.cargo/bin:$PATH
M=/root/merge-tree; S=$M/src
[ -d $M/base/bitcoin-core ] || bash $S/scripts/libzync-base.sh $M/base > $M/base.log 2>&1
# the patch: applied to a fresh copy of origin/main in $S (idempotent: skipped when already applied)
git -C $S apply --check $S/experimental/merge-tree/merge-tree.patch 2>/dev/null && git -C $S apply $S/experimental/merge-tree/merge-tree.patch || grep -q "fn join_k" $S/prover/methods/guest/src/main.rs
cd $S/prover
RUSTUP_TOOLCHAIN=$(sed -n 's/^channel = "\(.*\)"$/\1/p' rust-toolchain.toml); export RUSTUP_TOOLCHAIN
for v in $(compgen -e | grep '^HAZYNC_'); do unset "$v"; done
rm -rf $M/target/riscv-guest
# TEST_LINEAGE: the mainnet test variant (empty lineage); its id is in no pin table.
env HAZYNC_BASE=$M/base HAZYNC_LIBZYNC=1 HAZYNC_TEST_LINEAGE=1 CARGO_TARGET_DIR=$M/target \
  cargo build --locked --profile fasthost -p host -j4 > $M/build.log 2>&1
cp $M/target/fasthost/host $M/host_mt
$M/host_mt method-id | head -2
