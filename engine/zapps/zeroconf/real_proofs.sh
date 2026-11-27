#!/bin/bash
# zeroconf with REAL child proofs, on CPU (engine/zapps/zeroconf/fixture): (1) block 110 proved by the regtest core
# image (keys-rtch) as a base step on native prep's state at 109; (2) the speculative chunk for 111 proved by the same
# image; (3) zeroconf (production image keys-z-zeroconf3) proved with both as children; (4) verified.
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
cd /root/engine/zc1 || exit 1
H=/root/engine-src-rt/prover/target/release/host; E=/root/engine-kit/engine/host/target/release/hzengine
B=/root/engine-kit/scripts/zapp-regtest/blocks/main; ST=/root/engine/rtprep109
rm -rf "${PWD:?}/real3" && mkdir real3
s=$(date +%s); HAZYNC_NETWORK=regtest HAZYNC_ENGINE=$E HAZYNC_ENGINE_KEYS=/root/engine/keys-rtch nice $H libzync-prove-range 110 110 $B $ST/state_109.json real3/core > real3/core.log 2>&1; rc=$?
echo "1. block 110 proved rc=$rc $(( $(date +%s)-s )) s :: $(ls real3/core | grep -a 'ovm' | head -3 | tr '\n' ' ')"
[ $rc = 0 ] || { sed 's/\x1b\[[0-9;]*m//g' real3/core.log | grep -a "panicked\|rror" | tail -3; exit 1; }
CP=$(ls real3/core/*110*.ovm.proof.json | head -1); CP=${CP%.proof.json}; echo "   core proof $CP"
s=$(date +%s); nice $E prove --dir /root/engine/keys-rtch --input x111/chunk_00.in --out real3/chunk111 > real3/chunk.log 2>&1; rc=$?
echo "2. chunk proved rc=$rc $(( $(date +%s)-s )) s"; [ $rc = 0 ] || exit 1
cmp -s real3/chunk111.journal x111/chunk_00.r0journal && echo "   chunk proof's journal = the fixture's"
./zyncapp_skip prove-zeroconf $CP.journal real3/chunk111.journal 0 paths.json out --regtest --stream real3/zc.bin | cut -c1-120
s=$(date +%s); HZ_CHILD_KEYS=/root/engine/keys-rtch nice $E prove --dir /root/engine/keys-z-zeroconf3 --input real3/zc.bin --child $CP --child real3/chunk111 --out real3/zc > real3/zc.log 2>&1; rc=$?
echo "3. zeroconf proved rc=$rc $(( $(date +%s)-s )) s"; [ $rc = 0 ] || { sed 's/\x1b\[[0-9;]*m//g' real3/zc.log | grep -a "panicked\|rror" | tail -3; exit 1; }
$E verify --dir /root/engine/keys-z-zeroconf3 --proof real3/zc 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -a "^verify" | cut -c1-130
echo ZC_REAL_DONE
