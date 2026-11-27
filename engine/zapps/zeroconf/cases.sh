#!/bin/bash
# zeroconf refusal cases that need their OWN transaction and chunk (make_fixture.py's 4th argument), on the metering
# image (the guest's own checks; env::verify accepts the journals without proofs). Build box, CPU.
#   spent   T spends a coin the chain spent by 110 (a double-spend): must be refused
#   parent  the block holds T and T2 spending T:0; T2 (record 1) must be refused, T (record 0) still accepted
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
cd /root/engine || exit 1
K=/root/engine-kit; ZR=$K/scripts/zapp-regtest; TF=/root/engine/rigx2/core/test/functional; MF=$K/engine/zapps/zeroconf/make_fixture.py
H=/root/engine-src-rt/prover/target/release/host; E=$K/engine/host/target/release/hzengine; ZP=/root/engine/zync-path
Z=/root/engine/zc1/zyncapp_skip; ST=/root/engine/rtprep
# 0. the default fixture is unchanged by the new cases
rm -rf zc_base && python3 $MF $ZR $TF zc_base > /dev/null && cmp zc_base/T.json zc1/T.json && cmp zc_base/template_111.json zc1/template_111.json && echo "0. default fixture unchanged (T.json, template_111.json byte-identical)" || { echo "0. DEFAULT FIXTURE CHANGED"; exit 1; }
chunk() { # <dir>: hints + export of its template's speculative chunk, executed on the regtest core image
  local d=$1
  ( export HAZYNC_NETWORK=regtest HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1 HAZYNC_SPECULATIVE=1 HAZYNC_BLOCK=$d/template_111.json
    # both stop at the aggregate, as they must for a speculative block (never aggregated); the chunk is written first
    $H libzync-hints $d/t111.lzb > $d/hints.log 2>&1; HAZYNC_LZ_HINTS=$d/t111.lzb $H engine-export $d/x111 > $d/export.log 2>&1 )
  [ -f $d/x111/chunk_00.in ] || { echo "   no chunk exported: $(sed 's/\x1b\[[0-9;]*m//g' $d/hints.log $d/export.log | grep -a 'panicked' -A1 | tail -1 | cut -c1-120)"; return 1; }
  $E execute --dir keys-rtch --input $d/x111/chunk_00.in --journal $d/chunk.j > $d/exec.log 2>&1 || { echo "   chunk execute FAILED"; return 1; }
  python3 - $d/chunk.j <<'PY'
import struct,sys
w=struct.unpack("<%dI"%(len(open(sys.argv[1],"rb").read())//4), open(sys.argv[1],"rb").read())
print(f"   chunk: all_valid {w[1]}, height {w[2]}")
PY
}
paths() { # <dir> <record index> -> <dir>/paths_<i>.json: each input's leaf from the chunk record, found in the log at 110
  python3 - $1 $2 $ZP $ST/history.bin <<'PY'
import struct, sys, json, subprocess
d, want, Z, hist = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4]
j=open(f"{d}/chunk.j","rb").read(); w=list(struct.unpack("<%dI"%(len(j)//4), j))
hx=lambda ws: b"".join(struct.pack("<I",x) for x in ws).hex()
i=13; n_rec=w[i]; i+=1
for r in range(n_rec):
    i+=24; start,count,first=w[i:i+3]; i+=3; i+=2; i+=6; i+=2; nout=w[i]; i+=1+19*nout
    leaves=[]
    for _ in range(count):
        leaf=hx(w[i:i+8]); f=w[i+8]; i+=9
        if f&1: i+=8
        i+=16+8
        leaves.append((leaf,f))
    if r==want: mine=leaves
assert i==len(w), (i,len(w))
out=[]
for leaf,f in mine:
    o=subprocess.run([Z,"find",hist,"110",leaf],capture_output=True,text=True)
    if o.returncode!=0:
        print(f"   input leaf {leaf[:16]}.. flags {f}: not in the log ({(o.stdout+o.stderr).strip()[:60]}); given a dummy path"); out.append([0,[]]); continue
    pos=int(o.stdout.split()[-1])
    c=json.loads(subprocess.run([Z,"coin",hist,"110","110",str(pos)],capture_output=True,text=True).stdout)
    print(f"   input leaf {leaf[:16]}.. flags {f}: log position {pos}, spent_by {c['spent_by']}")
    out.append([pos,c["path"]])
json.dump(out, open(f"{d}/paths_{want}.json","w"))
PY
}
run() { # <name> <dir> <record index>
  $Z prove-zeroconf $ST/state_110.json $2/chunk.j $3 $2/paths_$3.json $2/out --regtest --stream $2/zc_$3.bin > $2/stream_$3.log 2>&1 || { echo "   $1: STREAM FAILED: $(tail -1 $2/stream_$3.log | cut -c1-120)"; return; }
  $E execute --dir keys-z-zeroconfb --input $2/zc_$3.bin --journal $2/zc_$3.j --log $2/zc_$3.log > $2/zc_$3.out 2>&1; rc=$?
  why=$(grep -a "panicked\|Guest panicked" -A1 $2/zc_$3.log $2/zc_$3.out 2>/dev/null | grep -av "panicked at\|^--" | head -1 | sed 's/^[^:]*://' | cut -c1-110)
  [ $rc != 0 ] && echo "   refused  $1: $why" || echo "   ACCEPTED $1"
}
for c in spent parent; do
  d=/root/engine/zc_$c; rm -rf "${d:?}"
  echo "== $c: $(python3 $MF $ZR $TF $d $c | tr '\n' ' ' | cut -c1-200)"
  TJ=$d/T.json; [ $c = parent ] && TJ=$d/T.json,$d/T2.json
  echo "   Core v31.1: $(python3 zc_core_accept.py /root/engine/rigx2/bin/bitcoind $ZR/chain/raw.txt 110 $TJ 18977 | grep testmempool | cut -c1-260)"
  chunk $d || continue
  if [ $c = spent ]; then paths $d 0; run "T spending a coin spent by 110" $d 0
  else paths $d 0; run "T (record 0, its inputs unspent): the control" $d 0
       paths $d 1; run "T2 (record 1) spending T:0, an unconfirmed parent" $d 1; fi
done
echo ZC_CASES_DONE
