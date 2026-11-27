#!/bin/bash
# HANDOVER STEP 3 (engine/HANDOVER.md): a REAL cross-version handover. Predecessor: the engine on OpenVM v2.0.2
# (keys-v202), its proof of block 199. Successor: a v2.1.0 test image carrying v2.0.2's verifier (ovm-xver-v2_0_2)
# and that key, pinning the v2.0.2 image at 200; block 200 proved as the handover block. CPU.
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$HOME/.risc0/bin:$PATH
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
cd /root/engine-kit && git fetch -q origin engine/openvm && git merge -q --ff-only FETCH_HEAD && git log --oneline -1 | cut -c1-50
E=/root/engine-kit/engine/host/target/release/hzengine; E2=/root/engine/v202/engine/host/target/release/hzengine
cd /root/engine || exit 1
# 1. the predecessor: its key (v2.0.2's own struct), its image id, its proof's plain encoding, the files at 199
P=/root/engine/xv3pred; rm -rf "${P:?}"; mkdir -p $P
cp v202out/block_199.ovm.proof.json v202out/block_199.ovm.journal $P/
$E2 wasm-inputs --dir keys-v202 --proof $P/block_199.ovm --out $P/wi > $P/wi.log 2>&1 || { echo WI_FAIL; tail -3 $P/wi.log; exit 1; }
cp $P/wi/proof.bin $P/block_199.ovm.bin; cp xvpred/state_199.coinlog xvpred/state_199.trie $P/
PIN=$(sha256sum $P/wi/vk.bin | cut -d' ' -f1)
ID=$(python3 -c "import json,hashlib;j=json.load(open('keys-v202/image.json'));print(hashlib.sha256(bytes.fromhex(j['exe'])+bytes.fromhex(j['vm'])).hexdigest())")
echo "1. predecessor (OpenVM v2.0.2): image $ID, key pin $PIN, proof $(stat -c %s $P/block_199.ovm.bin) bytes"
K=/root/engine/xv3keys; rm -rf "${K:?}"; mkdir -p $K/v2.0.2; cp $P/wi/vk.bin $K/v2.0.2/$PIN.vk
python3 - "$ID" "$PIN" > xv3_lineage.rs <<'P2'
import sys, struct
idb = bytes.fromhex(sys.argv[1]); pin = bytes.fromhex(sys.argv[2])
w = struct.unpack("<8I", idb)
print("// handover rehearsal, step 3 (engine/HANDOVER.md): the engine on OpenVM v2.0.2, handed over at 200 to v2.1.0")
print("pub const LINEAGE: &[crate::handover_pin::Handover] = &[crate::handover_pin::Handover {")
print(f"    image: [{', '.join(str(x) for x in w)}],")
print("    first_height: 200,")
print(f"    verifier: [{', '.join(f'0x{b:02x}' for b in pin)}],")
print("}];")
P2
# 2. the v2.1.0 successor image with v2.0.2's verifier
rm -rf /root/engine-guestxv3 && HZ_OVM_XVER_KEYS=$K HZ_TEST_LINEAGE=/root/engine/xv3_lineage.rs /root/engine-kit/engine/guest/assemble.sh /root/engine-guestxv3 > asmxv3.log 2>&1 || { echo ASM_FAIL; tail -8 asmxv3.log; exit 1; }
grep -a "ovm-xver\|lineage" asmxv3.log
rm -rf keys-xv3 && mkdir keys-xv3 && cp keys-rt29/{sdk.pk,agg.vk,openvm.toml} keys-xv3/
s=$(date +%s); HZ_GUEST_FEATURES=test-lineage,ovm-xver,ovm-xver-v2_0_2 HZ_BASE=/root/engine-base6 nice $E build --dir keys-xv3 --guest /root/engine-guestxv3/prover/methods/guest > keys-xv3_build.log 2>&1 \
  || { echo XV3_BUILD_FAIL; sed 's/\x1b\[[0-9;]*m//g' keys-xv3_build.log | grep -a "error\[\|error:" -A8 | head -40; exit 1; }
echo "2. keys-xv3 (v2.1.0 + v2.0.2's verifier) in $(( $(date +%s)-s )) s: $(grep -a '^build:' keys-xv3_build.log | cut -c1-110)"
# 3. the host of the same test lineage
X=/root/engine-src-xv3; rm -rf "${X:?}"; cp -a /root/engine-src "$X"; (cd "$X" && git fetch -q origin engine/openvm && git checkout -q -B engine/openvm FETCH_HEAD)
python3 - "$X/prover/methods/guest/src/lineage/test.rs" /root/engine/xv3_lineage.rs <<'P3'
import sys
p, new = sys.argv[1], open(sys.argv[2]).read()
new = "\n".join(l for l in new.splitlines() if not l.startswith("//"))
s = open(p).read(); a = "pub const LINEAGE: &[crate::handover_pin::Handover] = &[];"
assert s.count(a) == 1; open(p, "w").write(s.replace(a, new))
P3
s=$(date +%s); (cd $X/prover && RISC0_SKIP_BUILD=1 HAZYNC_BASE=/root/product-base6 HAZYNC_LIBZYNC=1 HAZYNC_TEST_LINEAGE=1 nice cargo build --release -p host > /root/engine/xv3_host.log 2>&1) || { echo HOST_FAIL; exit 1; }
HX=$X/prover/target/release/host; echo "3. test-lineage host in $(( $(date +%s)-s )) s"
# 4. block 200, the handover from v2.0.2
rm -rf "${PWD:?}/xv3out"; s=$(date +%s)
HAZYNC_ENGINE=$E HAZYNC_ENGINE_KEYS=/root/engine/keys-xv3 HAZYNC_HANDOVER_PROOF=$P/block_199.ovm \
  HAZYNC_COINLOG_TXCOUNTS=/root/engine-kit/experimental/libzync-bench/cost/txcounts.txt \
  nice $HX libzync-prove-range 200 200 /root/engine/xchain/blocks handover /root/engine/xv3out > xv3_run.log 2>&1; rc=$?
echo "4. handover block 200 (v2.0.2 -> v2.1.0) rc=$rc $(( $(date +%s)-s )) s :: $(sed 's/\x1b\[[0-9;]*m//g' xv3_run.log | grep -a 'block 200:\|panicked' | tail -2 | tr '\n' ' ' | cut -c1-200)"
[ $rc = 0 ] && $E verify --dir keys-xv3 --proof xv3out/block_200.ovm 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -a "^verify" | cut -c1-120
echo XV3_DONE
