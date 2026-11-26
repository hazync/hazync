#!/bin/bash
# FEASIBILITY Q20: the header zyncAPP on the engine. Assemble, build (its own image, into a copy of keys-m26),
# execute headers 0..40,319 from genesis (20 retarget epochs), check the end state against the node; then PROVE a
# shorter run (0..4,031: two epochs) on CPU and verify it.
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$PATH
cd /root/engine-kit || exit 1
git fetch -q origin engine/openvm && git merge -q --ff-only FETCH_HEAD && git log --oneline -1 | cut -c1-50
E=/root/engine-kit/engine/host/target/release/hzengine; T=/root/engine/hdr; mkdir -p $T; cd /root/engine || exit 1
/root/engine-kit/engine/zapps/headers/assemble.sh $T/guest || exit 1
rm -rf keys-hdr && cp -r keys-m26 keys-hdr && rm -f keys-hdr/guest.vmexe keys-hdr/image.json keys-hdr/baseline.json
s=$(date +%s); nice $E build --dir keys-hdr --guest $T/guest > $T/build.log 2>&1 || { echo BUILD_FAIL; grep -a "^error\|panicked" -A8 $T/build.log | head -30; exit 1; }
echo "build $(( $(date +%s)-s )) s: $(grep -a '^build:' $T/build.log | cut -c1-140)"
P=/root/engine-kit/engine/zapps/headers/hdrtool.py; export BITCOIN_DATADIR=/root/.bitcoin
for n in 40319 4031; do
  [ -f $T/h0_$n.txt ] || python3 -I $P fetch 0 $n $T/h0_$n.txt
  python3 -I $P stream $T/h0_$n.txt $T/s0_$n.bin
done
s=$(date +%s); nice $E execute --dir keys-hdr --input $T/s0_40319.bin --journal $T/j0_40319.bin > $T/exec40319.log 2>&1; rc=$?
echo "execute 0..40319 rc=$rc $(( $(date +%s)-s )) s: $(sed 's/\x1b\[[0-9;]*m//g' $T/exec40319.log | grep -a '^execute\|rror\|panicked' | head -2 | cut -c1-140)"
[ $rc = 0 ] && python3 -I $P check $T/j0_40319.bin $T/h0_40319.txt
s=$(date +%s); nice $E prove --dir keys-hdr --input $T/s0_4031.bin --out $T/p0_4031 > $T/prove4031.log 2>&1; rc=$?
echo "prove 0..4031 rc=$rc $(( $(date +%s)-s )) s: $(sed 's/\x1b\[[0-9;]*m//g' $T/prove4031.log | grep -a '^prove\|journal\|rror\|panicked' | tail -3 | tr '\n' ' ' | cut -c1-200)"
[ $rc = 0 ] && { nice $E verify --dir keys-hdr --proof $T/p0_4031 > $T/verify4031.log 2>&1; echo "verify rc=$? $(sed 's/\x1b\[[0-9;]*m//g' $T/verify4031.log | grep -a '^verify' | cut -c1-140)"; }
echo HDR_DONE
