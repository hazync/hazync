#!/bin/bash
# HANDOVER step 2 rehearsal (engine/HANDOVER.md): block 200 proved by the test image keys-xv as a handover from keys-ch's
# chain (its proof of block 199) through the ovm-xver path, on CPU.
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$HOME/.risc0/bin:$PATH
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
E=/root/engine-kit/engine/host/target/release/hzengine; cd /root/engine || exit 1
cd /root/engine-kit && git fetch -q origin engine/openvm && git merge -q --ff-only FETCH_HEAD && git log --oneline -1 | cut -c1-50; cd /root/engine
# 1. the host of the test variant with the rehearsal lineage (a copy of the checkout; the checkout is not modified)
X=/root/engine-src-xv; rm -rf "${X:?}"; cp -a /root/engine-src "$X"
(cd "$X" && git fetch -q origin engine/openvm && git checkout -q -B engine/openvm FETCH_HEAD)
python3 - "$X/prover/methods/guest/src/lineage/test.rs" /root/engine/xv_lineage.rs <<'P'
import sys
p, new = sys.argv[1], open(sys.argv[2]).read()
new = "\n".join(l for l in new.splitlines() if not l.startswith("//"))
s = open(p).read(); a = "pub const LINEAGE: &[crate::handover_pin::Handover] = &[];"
assert s.count(a) == 1; open(p, "w").write(s.replace(a, new))
P
s=$(date +%s); (cd $X/prover && RISC0_SKIP_BUILD=1 HAZYNC_BASE=/root/product-base6 HAZYNC_LIBZYNC=1 HAZYNC_TEST_LINEAGE=1 nice cargo build --release -p host > /root/engine/hv_host.log 2>&1) || { echo HOST_FAIL; grep -a "^error" -A6 /root/engine/hv_host.log | head; exit 1; }
HX=$X/prover/target/release/host; echo "1. test-lineage host built $(( $(date +%s)-s )) s: $($HX libzync-lineage 2>&1 | tail -2 | tr '\n' ' ' | cut -c1-200)"
# 2. the handover image with the final edits
rm -rf /root/engine-guestxv && HZ_OVM_XVER_KEYS=/root/engine/xvkeys HZ_TEST_LINEAGE=/root/engine/xv_lineage.rs /root/engine-kit/engine/guest/assemble.sh /root/engine-guestxv > asmxv.log 2>&1 || { echo ASM_FAIL; exit 1; }
rm -rf keys-xv && mkdir keys-xv && cp keys-rt29/{sdk.pk,agg.vk,openvm.toml} keys-xv/
HZ_GUEST_FEATURES=test-lineage,ovm-xver HZ_BASE=/root/engine-base6 nice $E build --dir keys-xv --guest /root/engine-guestxv/prover/methods/guest > keys-xv_build.log 2>&1 || { echo XV_BUILD_FAIL; exit 1; }
echo "2. keys-xv: $(grep -a '^build:' keys-xv_build.log | cut -c1-100)"
# 3. the predecessor: keys-ch's proof of 199 (+ its plain encoding), prep's coin log and trie at 199
P=/root/engine/xvpred; rm -rf "${P:?}"; mkdir -p $P
cp core200/block_199.ovm.proof.json core200/block_199.ovm.journal $P/
$E wasm-inputs --dir keys-ch --proof $P/block_199.ovm --out $P/wi > /dev/null 2>&1 && cp $P/wi/proof.bin $P/block_199.ovm.bin
HAZYNC_COINLOG_TXCOUNTS=/root/engine-kit/experimental/libzync-bench/cost/txcounts.txt /root/engine-src/prover/target/release/host libzync-prep 1 199 /root/engine/xchain/blocks genesis $P/prep > $P/prep.log 2>&1
cp $P/prep/state_199.coinlog $P/prep/state_199.trie $P/
echo "3. predecessor files: $(ls $P | tr '\n' ' ')"
# 4. block 200 as the handover block
s=$(date +%s); HAZYNC_ENGINE=$E HAZYNC_ENGINE_KEYS=/root/engine/keys-xv HAZYNC_HANDOVER_PROOF=$P/block_199.ovm \
  HAZYNC_COINLOG_TXCOUNTS=/root/engine-kit/experimental/libzync-bench/cost/txcounts.txt \
  nice $HX libzync-prove-range 200 200 /root/engine/xchain/blocks handover /root/engine/xvout > /root/engine/hv_run.log 2>&1; rc=$?
echo "4. handover block 200 rc=$rc $(( $(date +%s)-s )) s :: $(sed 's/\x1b\[[0-9;]*m//g' /root/engine/hv_run.log | grep -a 'block 200:\|panicked' | tail -2 | tr '\n' ' ' | cut -c1-220)"
echo HV_DONE
