#!/bin/bash
# Feed the recorded aggregate input (agg_in.bin: the exact stream RISC Zero's guest read) to the OpenVM port and meter
# it. The port reveals SHA-256 of its journal; compare with "journal sha256" in agg_exec.log (RISC Zero, same input).
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$PATH
cd /root/fresh-bench
python3 -c "d=open('agg_in.bin','rb').read(); open('agg_in.json','w').write('{\"input\":[\"0x01'+d.hex()+'\"]}')"
cd /root/fresh-bench/hzhost/prover/methods/ovmagg
[ -f openvm/app.pk ] || cargo openvm keygen --app-only > /root/fresh-bench/agg_keygen.log 2>&1
s=$(date +%s); cargo openvm run --mode meter --input /root/fresh-bench/agg_in.json > /root/fresh-bench/agg_ovm.log 2>&1
echo "OVM rc=$? secs=$(( $(date +%s)-s ))"
sed "s/\x1b\[[0-9;]*m//g" /root/fresh-bench/agg_ovm.log | grep -a -E "Execution output|instructions executed|Total cost|panicked|journal|REJECT|batch:|swiftsync" | head -12
python3 - <<'P'
import re
o = open('/root/fresh-bench/agg_ovm.log', errors='replace').read()
m = re.search(r'Execution output: \[([0-9, ]+)\]', o)
r = re.search(r'journal sha256\s+([0-9a-f]{64})', open('/root/fresh-bench/agg_exec.log', errors='replace').read())
if m and r:
    ov = bytes(int(x) for x in m.group(1).split(',')).hex()
    print("RISC0  journal sha256", r.group(1)); print("OpenVM journal sha256", ov); print("MATCH" if ov == r.group(1) else "MISMATCH")
P
echo OVM_DONE
