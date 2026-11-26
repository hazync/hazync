#!/bin/bash
# Stage 2.3: utreexo-tip on the engine (children of two images in one run, hzengine HZ_CHILD_KEYS list).
source ~/.cargo/env; for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done; unset RISC0_DEV_MODE
cd /root/engine-kit && git fetch -q origin engine/openvm && git merge -q --ff-only FETCH_HEAD && git log --oneline -1 | cut -c1-50
(cd engine/host && nice cargo build -q --release 2>&1 | grep -a "^error" -A5)
E=/root/engine-kit/engine/host/target/release/hzengine; C=/root/engine/core200; B=/root/engine/xchain/blocks
T=/root/engine/zt; N=$T/zyncapp_new; cd /root/engine || exit 1
eid() { python3 -c "import json,hashlib;j=json.load(open('$1/image.json'));print(hashlib.sha256(bytes.fromhex(j['exe'])+bytes.fromhex(j['vm'])).hexdigest())"; }
UID_=$(eid keys-z-utreexo); echo "engine utreexo image id $UID_ (journal's self_id: $(python3 -c "print(open('$T/u200.j','rb').read()[4:36].hex())"))"
# 1. single-image regression: the same execute as zt.sh
HZ_CHILD_KEYS=keys-ch $E execute --dir keys-z-utreexo --input $T/u200.bin --self-id-at 0 --child $C/block_200.ovm --journal $T/u200b.j > $T/u200b.log 2>&1
cmp -s $T/u200.j $T/u200b.j && echo "1. base to 200 re-executed: journal IDENTICAL to before the change" || echo "1. base to 200: DIFFERENT ($(tail -2 $T/u200b.log))"
# 2. the base to 100, proved
$N utreexo-prove $B 100 $C/state_200.json $T/u_eng100 --stream $T/u100.bin > $T/u100_stream.log 2>&1
s=$(date +%s); HZ_CHILD_KEYS=keys-ch nice $E prove --dir keys-z-utreexo --input $T/u100.bin --self-id-at 0 --child $C/block_200.ovm --out $T/base100 > $T/base100.log 2>&1; rc=$?
echo "2. base to 100 PROVED rc=$rc $(( $(date +%s)-s )) s"; [ $rc = 0 ] || { tail -5 $T/base100.log; exit 1; }
$E verify --dir keys-z-utreexo --proof $T/base100 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -a "^verify" | cut -c1-120
# 3. the tip step 100 -> 200 on that proof and the core proof
$N utreexo-tip-prove $B $T/base100.journal 200 $C/state_200.json $T/tip_eng --stream $T/tip.bin --base-id $UID_ > $T/tip_stream.log 2>&1; echo "tip stream rc=$?: $(grep -a 'UTREEXO_TIP_STREAM\|die\|zyncapp:' $T/tip_stream.log | cut -c1-160)"
HZ_CHILD_KEYS=keys-z-utreexo,keys-ch $E execute --dir keys-z-utreexo-tip --input $T/tip.bin --self-id-at 0 --child $T/base100 --child $C/block_200.ovm --journal $T/tip.j --log $T/tip.guest.log > $T/tip.exec.log 2>&1; rc=$?
echo "3. tip execute rc=$rc :: $(sed 's/\x1b\[[0-9;]*m//g' $T/tip.exec.log | grep -a '^execute\|rror\|EXECUTE' | head -2 | tr '\n' ' ' | cut -c1-200)"
python3 - $T/tip.j $T/eb_new.journal <<'PY'
import sys
a, b = open(sys.argv[1], "rb").read(), open(sys.argv[2], "rb").read()
d = [i for i in range(min(len(a), len(b))) if a[i] != b[i]]
print(f"   engine tip journal {len(a)} bytes vs RISC Zero {len(b)}; differing bytes {d[0] if d else '-'}..{d[-1] if d else '-'} ({len(d)}); "
      + ("ONLY self_id + base_id (bytes 4..68)" if len(a) == len(b) and d and d[0] >= 4 and d[-1] < 68 else "CHECK"))
PY
# 4. negative controls: the children's images swapped; one image for both
HZ_CHILD_KEYS=keys-ch,keys-z-utreexo $E execute --dir keys-z-utreexo-tip --input $T/tip.bin --self-id-at 0 --child $T/base100 --child $C/block_200.ovm > $T/neg1.log 2>&1; r1=$?
HZ_CHILD_KEYS=keys-ch $E execute --dir keys-z-utreexo-tip --input $T/tip.bin --self-id-at 0 --child $T/base100 --child $C/block_200.ovm > $T/neg2.log 2>&1; r2=$?
echo "4. swapped images rc=$r1 ($(sed 's/\x1b\[[0-9;]*m//g' $T/neg1.log | grep -a 'Error\|rror:' | head -1 | cut -c1-120)); one image for both rc=$r2 ($(sed 's/\x1b\[[0-9;]*m//g' $T/neg2.log | grep -a 'Error\|rror:' | head -1 | cut -c1-120))"
echo ZT2_DONE
