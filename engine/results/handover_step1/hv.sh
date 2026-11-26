#!/bin/bash
# HANDOVER.md step 1: OpenVM's verifier inside an engine guest, on a real engine proof (block 130,000's aggregate, keys-hz24;
# wasmin/ from hzengine wasm-inputs) and a flipped-byte control. Cost and verdict from `hzengine execute`.
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$PATH
cd /root/engine-kit && git fetch -q origin engine/openvm && git merge -q --ff-only FETCH_HEAD && git log --oneline -1 | cut -c1-50
E=/root/engine-kit/engine/host/target/release/hzengine; T=/root/engine/hv; rm -rf $T; mkdir -p $T; cd /root/engine || exit 1
/root/engine-kit/engine/handover/assemble.sh $T/w || exit 1
rm -rf keys-hv && mkdir keys-hv && cp keys-rt29/{sdk.pk,agg.vk,openvm.toml} keys-hv/
s=$(date +%s); nice $E build --dir keys-hv --guest $T/w/guest > $T/build.log 2>&1 || { echo BUILD_FAIL; sed "s/\x1b\[[0-9;]*m//g" $T/build.log | grep -a "^error\|error:" -A6 | head -40; exit 1; }
echo "build $(( $(date +%s)-s )) s: $(grep -a '^build:' $T/build.log | cut -c1-120)"
zstd -q -d -f /root/engine/wasmin/proof.zst -o $T/proof.bin; ls -la $T/proof.bin | awk '{print "proof", $5, "B"}'
P=/root/engine-kit/engine/handover/hvstream.py
python3 -I $P /root/engine/wasmin/vk.bin $T/proof.bin $T/ok.bin
python3 -I $P /root/engine/wasmin/vk.bin $T/proof.bin $T/bad.bin 100000
for c in ok bad; do
  s=$(date +%s); $E execute --dir keys-hv --input $T/$c.bin --journal $T/$c.j > $T/$c.log 2>&1; rc=$?
  echo "$c rc=$rc $(( $(date +%s)-s )) s :: $(sed 's/\x1b\[[0-9;]*m//g' $T/$c.log | grep -a '^execute\|rror\|panicked\|Maximum' | head -1 | cut -c1-150)"
  [ -f $T/$c.j ] && python3 -c "
import struct; b=open('$T/$c.j','rb').read(); w=struct.unpack('<%dI'%(len(b)//4), b)
ok=w[0]; img=bytes(w[1:33]); pvs=bytes(w[33:65]); print('  verified', ok, 'image', img.hex()[:16], 'pvs', pvs.hex()[:16])"
done
python3 -c "
import hashlib, json; i=json.load(open('/root/engine/keys-hz24/image.json')); print('  keys-hz24 image id', hashlib.sha256(bytes.fromhex(i['exe'])+bytes.fromhex(i['vm'])).hexdigest()[:16])
print('  sha256(block journal)', hashlib.sha256(open('/root/engine/sp130/block.journal','rb').read()).hexdigest()[:16])"
echo HV_DONE
