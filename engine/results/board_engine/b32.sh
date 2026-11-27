#!/bin/bash
# engine/STAGE3.md 3.2 on the build box's CPU: the board's host commands on the engine for mainnet blocks 1..4, on
# keys-ch (the image of the engine chain 1..200, c7fe0eb8). Leaves (prove-range-bridge), folds (fold-range, HZE1 in
# and out), verify-any on each (--check-end against prep's states), the board's chain check (out_tip == next in_tip),
# refusals, method-id and seg-po2. Usage: b32.sh <commit>
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$PATH
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
C=${1:?commit}; R=/root/b32; S=$R/src; W=$R/run; K=/root/engine/keys-ch
t(){ local s=$(date +%s); "$@"; local rc=$?; echo "STEP rc=$rc secs=$(( $(date +%s)-s )) :: $*"; return $rc; }
mkdir -p $R
if [ ! -d $S ]; then cp -a /root/engine-src $S; mkdir -p $S/engine/host && cp -a /root/engine-kit/engine/host/target $S/engine/host/ ; fi
(cd $S && git fetch -q origin engine/openvm && git checkout -q -f $C && git log --oneline -1 | cut -c1-70)
echo "== build"
t bash -c "cd $S/engine/host && nice cargo build --release -q > $R/hz_build.log 2>&1" || { grep -a '^error' -A6 $R/hz_build.log | head; exit 1; }
t bash -c "cd $S/prover && RISC0_SKIP_BUILD=1 HAZYNC_BASE=/root/product-base6 HAZYNC_LIBZYNC=1 nice cargo build --release -q -p host > $R/host_build.log 2>&1" || { grep -a '^error' -A6 $R/host_build.log | head; exit 1; }
E=$S/engine/host/target/release/hzengine; H=$S/prover/target/release/host
rm -rf $W && mkdir -p $W/bundles $W/prep && cd $W
echo "== the board's key: pack block 200 of the 1..200 chain (also the key file and its pin)"
t $E pack --dir $K --proof /root/engine/core200/block_200.ovm --out $W/b200.hze --key $W/key.bin | tee $W/pack.out
PIN=$(awk '/^key_pin/{print $2}' $W/pack.out); IMG=$(awk '/^image/{print $2}' $W/pack.out); echo "image $IMG pin $PIN"
echo "== prep 1..4 with leaf bundles"
t env HAZYNC_PREP_LEAF_BUNDLES=$W/bundles HAZYNC_ENGINE_KEYS=$K $H libzync-prep 1 4 /root/engine/blocks genesis $W/prep > $W/prep.log 2>&1 || { tail -20 $W/prep.log; exit 1; }
ls $W/bundles
BOARD="HAZYNC_BRIDGE_OUT=$W/bundles HAZYNC_LZ_STATE_DIR=$W/prep HAZYNC_ENGINE_VK=$W/key.bin HAZYNC_ENGINE_VK_PIN=$PIN"
PROVE="HAZYNC_BRIDGE_OUT=$W/bundles HAZYNC_ENGINE=$E HAZYNC_ENGINE_KEYS=$K RAYON_NUM_THREADS=8"
echo "== method-id / seg-po2"
env $BOARD $H method-id; env $PROVE $H method-id; env $PROVE $H seg-po2
echo "== leaves"
for h in 1 2 3 4; do
  mkdir -p $W/w$h; (cd $W/w$h && t env $PROVE nice $H prove-range-bridge $h > $W/leaf_$h.log 2>&1) || { tail -30 $W/leaf_$h.log; exit 1; }
  grep -aE "engine: .*proved|leaf engine proof|packed" $W/leaf_$h.log | tail -4
  cp $W/w$h/range_$h.bin $W/; ls -la $W/range_$h.bin | awk '{print "  " $5 " bytes"}'
done
echo "== verify-any on each leaf (--check-end)"
for h in 1 2 3 4; do env $BOARD $H verify-any $W/range_$h.bin $h --check-end; echo "  rc=$?"; done > $W/va_leaves.out 2>&1; cat $W/va_leaves.out
echo "== the board's chain check: out_tip(h) == in_tip(h+1)"
python3 - $W/va_leaves.out <<'P'
import sys, re
L = [dict(re.findall(r'(\w+)=(\S+)', l)) for l in open(sys.argv[1]) if l.startswith('RANGE-OK')]
ok = len(L) == 4 and all(L[i]['out_tip'] == L[i+1]['in_tip'] for i in range(3)) and L[0]['anchored'] == '1'
print("CHAIN", "OK" if ok else "BROKEN", [(d['lo'], d['hi'], d['in_tip'][:8], d['out_tip'][:8]) for d in L])
P
echo "== folds (HZE1 in, HZE1 out)"
cd $W && t env $PROVE nice $H fold-range range_1.bin range_2.bin fold_1_2.bin > $W/fold12.log 2>&1 || { tail -20 $W/fold12.log; exit 1; }
t env $PROVE nice $H fold-range range_3.bin range_4.bin fold_3_4.bin > $W/fold34.log 2>&1 || { tail -20 $W/fold34.log; exit 1; }
t env $PROVE nice $H fold-range fold_1_2.bin fold_3_4.bin range_1_4.bin > $W/fold14.log 2>&1 || { tail -20 $W/fold14.log; exit 1; }
grep -ah "JOINED\|packed" $W/fold12.log $W/fold34.log $W/fold14.log
ls -la fold_1_2.bin fold_3_4.bin range_1_4.bin | awk '{print "  " $9 " " $5 " bytes"}'; ls $W | grep -c 'proof.json' | sed 's/^/  pairs left in the work dir: /'
env $BOARD $H verify-any fold_1_2.bin 1 --check-end; echo "  rc=$?"
env $BOARD $H verify-any fold_3_4.bin 3 --check-end; echo "  rc=$?"
env $BOARD $H verify-any range_1_4.bin 1 --check-end; echo "  rc=$?"
echo "== refusals (each must REJECT, rc 1)"
python3 -c "import sys;b=bytearray(open('range_2.bin','rb').read());b[-5]^=1;open('bad_journal.bin','wb').write(b)"
python3 -c "import sys;b=bytearray(open('range_2.bin','rb').read());b[2000]^=1;open('bad_proof.bin','wb').write(b)"
env $BOARD $H verify-any bad_journal.bin 2 --check-end; echo "  tampered journal rc=$?"
env $BOARD $H verify-any bad_proof.bin 2 --check-end; echo "  tampered proof rc=$?"
env $BOARD $H verify-any range_3.bin 2 --check-end; echo "  range 3 claimed from 2 rc=$?"
env $BOARD $H verify-any fold_3_4.bin 1 --check-end; echo "  fold 3..4 claimed from genesis rc=$?"
env ${BOARD/$PIN/$(printf '0%.0s' {1..64})} $H verify-any range_1.bin 1 --check-end; echo "  wrong pin rc=$?"
env HAZYNC_BRIDGE_OUT=$W/bundles HAZYNC_LZ_STATE_DIR=$W/prep HAZYNC_ENGINE_VK=$S/zapps/headers/tests/data/engine4031/vk.bin HAZYNC_ENGINE_VK_PIN=$(cut -d' ' -f1 $S/zapps/headers/tests/data/engine4031/vk.sha256) $H verify-any range_1.bin 1 --check-end; echo "  another image's key rc=$?"
env HAZYNC_BRIDGE_OUT=$W/bundles HAZYNC_LZ_STATE_DIR=$W/prep $H verify-any range_1.bin 1 --check-end; echo "  a RISC Zero board rc=$?"
echo B32_DONE
