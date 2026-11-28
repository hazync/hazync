#!/bin/bash
# The new production zeroconf image (977439f0, 132969d8..., container build) given the REAL children of the old fixture:
# block 110 proved by keys-rtch as a base step on prep's state at 109 (anchored at 109, not genesis) and the speculative
# chunk for 111. Expected: refused IN THE GUEST ("not anchored at the network's genesis").
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
cd /root/zc4 || exit 1
H=/root/engine-src-rt/prover/target/release/host; E=/root/engine-kit/engine/host/target/release/hzengine   # the core proof, as real_proofs.sh
E2=/root/zc4/repo/engine/host/target/release/hzengine   # the zeroconf prove: hzengine at 977439f0
B=/root/engine-kit/scripts/zapp-regtest/blocks/main; ST=/root/engine/rtprep109
rm -rf keys-p && mkdir keys-p && cp /root/engine/keys-z-zeroconf3/{sdk.pk,agg.vk,openvm.toml} keys-p/ && cp out/zeroconf/{guest.vmexe,image.json,baseline.json} keys-p/
rm -rf real4 && mkdir real4
s=$(date +%s); HAZYNC_NETWORK=regtest HAZYNC_ENGINE=$E HAZYNC_ENGINE_KEYS=/root/engine/keys-rtch nice taskset -c 0-3 $H libzync-prove-range 110 110 $B $ST/state_109.json real4/core > real4/core.log 2>&1; rc=$?
echo "1. block 110 proved (base step on 109) rc=$rc $(( $(date +%s)-s )) s"
[ $rc = 0 ] || { sed 's/\x1b\[[0-9;]*m//g' real4/core.log | grep -a "panicked\|rror" | tail -3; exit 1; }
CP=$(ls real4/core/*110*.ovm.proof.json | head -1); CP=${CP%.proof.json}; echo "   core proof $CP"
CH=/root/engine/zc1/real3/chunk111
/root/engine/zc1/zyncapp_skip prove-zeroconf $CP.journal $CH.journal 0 /root/engine/zc1/paths.json out --regtest --stream real4/zc.bin | cut -c1-120
s=$(date +%s); HZ_CHILD_KEYS=/root/engine/keys-rtch nice taskset -c 0-3 $E2 prove --dir keys-p --input real4/zc.bin --child $CP --child $CH --out real4/zc > real4/zc.log 2>&1; rc=$?
echo "2. zeroconf (new image) rc=$rc $(( $(date +%s)-s )) s"
sed 's/\x1b\[[0-9;]*m//g' real4/zc.log | grep -a "panicked" -A2 | head -4
echo ZC_REAL4_DONE
