#!/bin/bash
# zeroconf negatives on the metering image (the guest's own checks; env::verify accepts without proofs here)
cd /root/engine/zc1 || exit 1
Z=./zyncapp_skip; E=/root/engine-kit/engine/host/target/release/hzengine; ST=/root/engine/rtprep
run() { # <name> <tip> <chunk> <index> <paths> <net flag>
  $Z prove-zeroconf $2 $3 $4 $5 out $6 --stream neg.bin > /dev/null 2>&1 || { echo "$1: STREAM FAILED"; return; }
  $E execute --dir ../keys-z-zeroconfb --input neg.bin --journal neg.j --log neg.log > neg.out 2>&1; rc=$?
  why=$(grep -a "panicked\|Guest panicked" -A1 neg.log neg.out 2>/dev/null | grep -av "panicked at\|^--" | head -1 | sed 's/^[^:]*://' | cut -c1-110)
  [ $rc != 0 ] && echo "refused  $1: $why" || echo "ACCEPTED $1"
}
python3 - <<'PY'
import json
p=json.load(open("paths.json"))
json.dump([p[1],p[0]], open("paths_swapped.json","w"))                     # each input given the other's path
json.dump([p[0]], open("paths_short.json","w"))                            # one path for two inputs
q=json.loads(json.dumps(p)); q[0][1][0]=("0" if q[0][1][0][0]!="0" else "1")+q[0][1][0][1:]; json.dump(q, open("paths_tampered.json","w"))
import struct
j=bytearray(open("x111/chunk_00.r0journal","rb").read()); j[8]^=1; open("chunk_otherheight.j","wb").write(j)   # height word
k=bytearray(open("x111/chunk_00.r0journal","rb").read()); struct.pack_into("<I",k,4,0); open("chunk_invalid.j","wb").write(k) # all_valid 0
PY
C=x111/chunk_00.r0journal
run "control (as the fixture)"           $ST/state_110.json $C 0 paths.json --regtest
run "paths swapped between the inputs"   $ST/state_110.json $C 0 paths_swapped.json --regtest
run "a path node changed"                $ST/state_110.json $C 0 paths_tampered.json --regtest
run "one path for two inputs"            $ST/state_110.json $C 0 paths_short.json --regtest
run "the tip one block lower (109)"      $ST/state_109.json $C 0 paths.json --regtest
run "a chunk for another height"         $ST/state_110.json chunk_otherheight.j 0 paths.json --regtest
run "a chunk that is not all valid"      $ST/state_110.json chunk_invalid.j 0 paths.json --regtest
run "no such record (index 1)"           $ST/state_110.json $C 1 paths.json --regtest
run "the wrong network (signet)"         $ST/state_110.json $C 0 paths.json --signet
echo ZC_NEG_DONE
