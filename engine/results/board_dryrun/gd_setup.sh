#!/bin/bash
# Gate 3 board dry run, part 1 (engine/STAGE3.md): build hzengine and the host at <commit>, prep mainnet 1..8 with leaf
# bundles, and the board's key (`hzengine pack --key` of the engine chain's block 200, keys-ch c7fe0eb8). On hazync-coord,
# CPU only, everything under /root/gd (the live coordinators on hazync-proof are not touched). Usage: gd_setup.sh <commit>
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$PATH
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
C=${1:?commit}; R=/root/gd; S=$R/src; W=$R/run; K=/root/engine/keys-ch
t(){ local s=$(date +%s); "$@"; local rc=$?; echo "STEP rc=$rc secs=$(( $(date +%s)-s )) :: $*"; return $rc; }
mkdir -p $R
if [ ! -d $S ]; then cp -a /root/engine-src $S; mkdir -p $S/engine/host && cp -a /root/engine-kit/engine/host/target $S/engine/host/ ; fi
(cd $S && git fetch -q origin engine/openvm && git checkout -q -f $C && git log --oneline -1 | cut -c1-70)
echo "== build"
t bash -c "cd $S/engine/host && nice cargo build --release -q > $R/hz_build.log 2>&1" || { grep -a '^error' -A6 $R/hz_build.log | head; exit 1; }
t bash -c "cd $S/prover && RISC0_SKIP_BUILD=1 HAZYNC_BASE=/root/product-base6 HAZYNC_LIBZYNC=1 nice cargo build --release -q -p host > $R/host_build.log 2>&1" || { grep -a '^error' -A6 $R/host_build.log | head; exit 1; }
E=$S/engine/host/target/release/hzengine; H=$S/prover/target/release/host
rm -rf $W && mkdir -p $W/bundles $W/prep && cd $W
echo "== the board's key"
t $E pack --dir $K --proof /root/engine/core200/block_200.ovm --out $W/b200.hze --key $W/key.bin | tee $W/pack.out
echo "== prep 1..8 with leaf bundles"
t env HAZYNC_PREP_LEAF_BUNDLES=$W/bundles HAZYNC_ENGINE_KEYS=$K $H libzync-prep 1 8 /root/engine/blocks genesis $W/prep > $W/prep.log 2>&1 || { tail -20 $W/prep.log; exit 1; }
ls $W/bundles | tr '\n' ' '; echo
echo "== genesis tip, method-id (board env)"
PIN=$(awk '/^key_pin/{print $2}' $W/pack.out)
$H libzync-genesis-tip
env HAZYNC_ENGINE_VK=$W/key.bin HAZYNC_ENGINE_VK_PIN=$PIN $H method-id
echo GD_SETUP_DONE
