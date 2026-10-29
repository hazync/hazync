#!/bin/bash
# tiplive.sh <seconds>: follow the live tip on the pods in fleet.txt (first line = A) for <seconds>.
# Per new tip block: mkblock on hazync-coord -> pushed coord->A; A: libzync-hints, copy to peers, early aggregate,
# own lane; peers: lanes. Done when A's seg-serve exits with a verified receipt. Blocks run one at a time, in order.
cd ~/libzync-gpu/live; DUR=${1:-1800}; END=$((SECONDS+DUR)); L=$PWD/log; mkdir -p $L
K="-n -i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=15"
say(){ echo "[$(date +%T)] $*" | tee -a $L/live.log; }
mapfile -t F < fleet.txt; N=${#F[@]}; read -r _ AIP AP A9IP A9P _ <<<"${F[0]}"
sshA(){ local t rc; for t in 1 2 3 4; do ssh $K -p $AP root@$AIP "$@"; rc=$?; [ $rc -ne 255 ] && return $rc; sleep 3; done; return 255; }  # 255 = ssh itself failed: retry
now(){ date +%s.%N; }
last=$(ssh -n hazync-coord bitcoin-cli getblockcount); say "following the tip from $last for ${DUR}s on $N pods"
echo -e "height\tinputs\tdetect\tjson_on_A\thints_s\tdist_end\tserve_end\trc\tdetect_to_proof_s" > $L/blocks.tsv
# REPLAY="h1 h2": push those blocks through the same pipeline now (a dress rehearsal), then stop.
RQ=(${REPLAY:-})
while [ $SECONDS -lt $END ]; do
  if [ ${#RQ[@]} -gt 0 ]; then H=${RQ[0]}; RQ=("${RQ[@]:1}"); TD=$(now)
  elif [ -n "${REPLAY:-}" ]; then break
  else
  tip=$(ssh -n hazync-coord bitcoin-cli getblockcount 2>/dev/null) || { sleep 5; continue; }
  [ "$tip" -gt "$last" ] 2>/dev/null || { sleep 5; continue; }
  H=$((last+1)); last=$H; TD=$(now)
  [ "$tip" -gt "$H" ] && say "⚠ $((tip-H)) block(s) queued behind $H (still proving in order)"
  fi
  say "== block $H detected"
  ssh -n hazync-coord "python3 /tmp/mkblock.py $H /tmp/lz_$H.json && scp -q -i /tmp/lz_eph -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -P $AP /tmp/lz_$H.json root@$AIP:/workspace/in_$H.json" > $L/mk_$H.log 2>&1 || { say "mkblock/push FAILED for $H"; cat $L/mk_$H.log; continue; }
  TJ=$(now); say "  json on A ($(head -1 $L/mk_$H.log))"
  sshA "bash /workspace/blockA.sh $H" > $L/blockA_$H.log 2>&1
  grep -q BLOCKA_READY $L/blockA_$H.log || { say "blockA FAILED for $H"; sshA "cat /workspace/tip_$H/times; tail -5 /workspace/tip_$H/hints.log"; continue; }
  for k in $(seq 1 $((N-1))); do read -r nm ip p _ <<<"${F[$k]}"
    ssh $K -p $p root@$ip "(setsid nohup bash /workspace/lanew.sh $H $AIP:$AP $A9IP:$A9P $nm < /dev/null > /dev/null 2>&1 &)" & done; wait
  say "  hints + lanes started: $(sshA "grep -E 'batch:|ecmh:|time:' /workspace/tip_$H/hints.log" | tr '\n' ' ')"
  while ! sshA "grep -q SERVE_END /workspace/tip_$H/times" 2>/dev/null; do sleep 2; done
  TE=$(now)
  sshA "cat /workspace/tip_$H/times; grep -E 'AGG_EARLY|segment phase|assembly|TOTAL|RECEIPT VERIFIED' /workspace/tip_$H/serve.log" > $L/times_$H.txt
  python3 - $H $TD $TJ $L/times_$H.txt $L/mk_$H.log $TE >> $L/blocks.tsv <<'PY'
import sys
h, td, tj, tf, mk, te = sys.argv[1], float(sys.argv[2]), float(sys.argv[3]), sys.argv[4], sys.argv[5], float(sys.argv[6])
t = {}
for l in open(tf):
    p = l.split()
    if p and p[0] in ("HINTS_START", "HINTS_END", "DIST_END", "SERVE_END"): t[p[0]] = (float(p[1]), p[2] if len(p) > 2 else "")
inp = open(mk).read().split(" inputs")[0].split()[-1] if " inputs" in open(mk).read() else "?"
ok = "rc=0" if t.get("SERVE_END", (0, ""))[1] == "rc=0" else t.get("SERVE_END", (0, "rc=?"))[1]
g = lambda k: t[k][0] if k in t else float("nan")
print("\t".join([h, inp, f"{td:.1f}", f"{tj-td:.1f}", f"{g('HINTS_END')-g('HINTS_START'):.1f}",
                 f"{g('DIST_END')-td:.1f}", f"{g('SERVE_END')-td:.1f}", ok, f"{te-td:.1f}"]))
PY
  say "  block $H: $(tail -1 $L/blocks.tsv)"; grep -E 'VERIFIED' $L/times_$H.txt | head -1 | sed 's/^/  /' | tee -a $L/live.log
done
say "LIVE_DONE"
