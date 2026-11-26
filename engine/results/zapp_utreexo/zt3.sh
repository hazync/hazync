#!/bin/bash
# Stage 2.3: utreexo tip steps PROVED on the engine, and a step on a step. Engine: base100 (zt2.sh) -> step 150 PROVED
# (children: base100 + the core) -> step 200 on step 150 (children: step 150, an image of ITSELF, + the core), executed.
# RISC Zero: the same chain (base 100 -> 150 -> 200, execute, same ELFs). Journals must differ only in the two ids.
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done; unset RISC0_DEV_MODE
E=/root/engine-kit/engine/host/target/release/hzengine; C=/root/engine/core200; B=/root/engine/xchain/blocks
T=/root/engine/zt; N=$T/zyncapp_new; cd /root/engine || exit 1
G=/root/engine-src/prover/target/riscv-guest/zyncapp-methods; TE=$G/zyncapp-utreexo-tip/riscv32im-risc0-zkvm-elf/release/zyncapp-utreexo-tip.bin
RBID=cd8ea3698fe79148e9e5186dcd0c8e55dba8ed88641b5776d013a66cc020dbe4
eid() { python3 -c "import json,hashlib;j=json.load(open('$1/image.json'));print(hashlib.sha256(bytes.fromhex(j['exe'])+bytes.fromhex(j['vm'])).hexdigest())"; }
EBID=$(eid keys-z-utreexo)
cmpj() { python3 - "$1" "$2" "$3" <<'PY'
import sys
a, b = open(sys.argv[1], "rb").read(), open(sys.argv[2], "rb").read()
d = [i for i in range(min(len(a), len(b))) if a[i] != b[i]]
print(f"   {sys.argv[3]}: engine {len(a)} bytes vs RISC Zero {len(b)}; differing bytes {d[0] if d else '-'}..{d[-1] if d else '-'} ({len(d)}): "
      + ("ONLY self_id + base_id (4..68)" if len(a) == len(b) and d and d[0] >= 4 and d[-1] < 68 else "CHECK"))
PY
}
# RISC Zero chain: base 100 (zt.sh: e_new_100) -> 150 -> 200
$N utreexo-tip-prove $B $T/e_new_100/mainnet-100.journal 150 $C/state_200.json $T/r150 --execute --elf $TE --base-id $RBID > $T/r150.log 2>&1; echo "RISC Zero step 100->150 rc=$?"
$N utreexo-tip-prove $B $T/r150.journal 200 $C/state_200.json $T/r200 --execute --elf $TE --base-id $RBID > $T/r200.log 2>&1; echo "RISC Zero step 150->200 (on a step) rc=$? $(grep -a 'UTREEXO_TIP_JOURNAL' $T/r200.log | cut -c1-120)"
# engine: step 150, proved
$N utreexo-tip-prove $B $T/base100.journal 150 $C/state_200.json $T/x150 --stream $T/tip150.bin --base-id $EBID > $T/tip150_stream.log 2>&1 || { tail -3 $T/tip150_stream.log; exit 1; }
s=$(date +%s); HZ_CHILD_KEYS=keys-z-utreexo,keys-ch nice $E prove --dir keys-z-utreexo-tip --input $T/tip150.bin --self-id-at 0 --child $T/base100 --child $C/block_200.ovm --out $T/step150 > $T/step150.log 2>&1; rc=$?
echo "engine step 100->150 PROVED rc=$rc $(( $(date +%s)-s )) s"; [ $rc = 0 ] || { sed 's/\x1b\[[0-9;]*m//g' $T/step150.log | grep -a "rror" | tail -3; exit 1; }
$E verify --dir keys-z-utreexo-tip --proof $T/step150 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -a "^verify" | cut -c1-110
cmpj $T/step150.journal $T/r150.journal "step 150"
# engine: step 200 on step 150 (the previous proof is of the tip image itself)
$N utreexo-tip-prove $B $T/step150.journal 200 $C/state_200.json $T/x200 --stream $T/tip200s.bin --base-id $EBID > $T/tip200s_stream.log 2>&1 || { tail -3 $T/tip200s_stream.log; exit 1; }
HZ_CHILD_KEYS=keys-z-utreexo-tip,keys-ch $E execute --dir keys-z-utreexo-tip --input $T/tip200s.bin --self-id-at 0 --child $T/step150 --child $C/block_200.ovm --journal $T/tip200s.j > $T/tip200s.exec.log 2>&1; rc=$?
echo "engine step 150->200 on a step rc=$rc :: $(sed 's/\x1b\[[0-9;]*m//g' $T/tip200s.exec.log | grep -a '^execute\|rror' | head -1 | cut -c1-120)"
cmpj $T/tip200s.j $T/r200.journal "step 200 on a step"
echo ZT3_DONE
