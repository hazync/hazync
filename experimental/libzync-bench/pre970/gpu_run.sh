#!/bin/bash
# Signet GPU proof, unattended: one RTX PRO 6000 -> provision -> build -> prove 2501..2600 recursively ->
# verify -> fetch -> TERMINATE. A separate deadline kills the pod at 150 min or a balance under $3.
cd ~/libzync-gpu/pre970; LOG=gpu_run.log; exec >> $LOG 2>&1
RP=$(cat ~/.hazync/runpod.key); PUB=$(cat ~/.ssh/ghost_signet_ed25519.pub)
K="-n -i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=20"
KS="-i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=20"
rp(){ curl -s --max-time 20 -X "$1" -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" ${3:+-d "$3"} "https://api.runpod.io/v2$2"; }
# ⚠ RunPod REST v2 has no account balance; clientBalance is the ONE call left on GraphQL (retiring early 2027).
# Once it is gone the balance is UNREADABLE: every check says so, and only the time limit guards the spend.
gqlbal(){ curl -s --max-time 20 -X POST -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" -d "$1" https://api.runpod.io/graphql; }
say(){ echo "[$(date +%T)] $*"; }
GPU="${GPU:-NVIDIA RTX PRO 6000 Blackwell Server Edition}"
Q=$(python3 -c "import json,sys; print(json.dumps({'name': 'lz-pre970', 'cloud': 'SECURE', 'gpu': {'id': sys.argv[2], 'count': 1}, 'image': 'runpod/pytorch:2.4.0-py3.11-cuda12.4.1-devel-ubuntu22.04', 'disk': 80, 'ports': ['22/tcp'], 'env': {'PUBLIC_KEY': sys.argv[1]}}))" "$PUB" "$GPU")
R=$(rp POST /pods "$Q"); ID=$(echo "$R" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("id") or "")')
[ -n "$ID" ] || { say "deploy failed: $R"; exit 1; }
echo $ID > pod_id.txt; say "pod $ID deployed ($GPU)"
term(){ say "terminating $ID"; rp DELETE "/pods/$ID" > /dev/null; }
( END=$((SECONDS+120*60)); while [ -f pod_id.txt ]; do
    B=$(gqlbal '{"query":"query { myself { clientBalance } }"}' | python3 -c 'import json,sys; print(float(json.load(sys.stdin)["data"]["myself"]["clientBalance"]))' 2>/dev/null || echo UNREADABLE)
    [ "$B" = UNREADABLE ] && say "⚠ balance UNREADABLE (GraphQL clientBalance did not answer) -- only the time limit guards the spend"
    if [ $SECONDS -ge $END ] || { [ "$B" != UNREADABLE ] && python3 -c "import sys; sys.exit(0 if float('$B') < 3 else 1)"; }; then say "DEADLINE (balance $B)"; term; rm -f pod_id.txt; exit; fi
    sleep 60; done ) &
for i in $(seq 1 60); do
  L=$(rp GET "/pods/$ID" | python3 -c '
import ipaddress,json,sys
p=json.load(sys.stdin); g=lambda ip: bool(ip) and ipaddress.ip_address(ip).is_global
m=[x for x in ((p.get("runtime") or {}).get("ports") or []) if x.get("private")==22 and x.get("public") and g(x.get("ip"))]
if m: print(m[0]["ip"], m[0]["public"])')
  [ -n "$L" ] && break; sleep 10; done
[ -n "$L" ] || { say "no ports"; term; rm -f pod_id.txt; exit 1; }
set -- $L; IP=$1; P=$2; say "ssh $IP:$P"
for t in $(seq 1 30); do ssh $K -p $P root@$IP true && break; sleep 10; done
scp -q $KS -P $P pod/setup.sh pod/prep.sh pod/run.sh root@$IP:/workspace/ && say "scripts sent"
ssh $K -p $P root@$IP 'cd /workspace && (setsid nohup bash prep.sh > prep.out 2>&1 < /dev/null &)'
scp -q $KS -P $P sb.tgz sb.tgz.sha256 root@$IP:/workspace/ && say "bundle sent"
for i in $(seq 1 120); do
  S=$(ssh $K -p $P root@$IP 'grep -h "PREP_OK\|PREP_FAIL\|built in" /workspace/prep.out 2>/dev/null')
  echo "$S" | grep -q 'PREP_OK\|PREP_FAIL' && break; sleep 20; done
say "prep: $(echo $S | tr '\n' ' ')"
echo "$S" | grep -q PREP_OK || { ssh $K -p $P root@$IP 'tail -30 /workspace/build.log /workspace/setup.log' > prep_fail.log; term; rm -f pod_id.txt; exit 1; }
ssh $K -p $P root@$IP 'cd /workspace && (setsid nohup env HAZYNC_CHUNKS=16 bash run.sh 969996 970000 > run.out 2>&1 < /dev/null &)'
say "proving 969996..970000"
for i in $(seq 1 480); do
  ssh $K -p $P root@$IP 'grep -q RUN_DONE /workspace/out/prove.log 2>/dev/null' && break
  [ $((i % 10)) = 0 ] && say "  $(ssh $K -p $P root@$IP 'tail -1 /workspace/out/times.tsv 2>/dev/null')"
  sleep 15; done
say "fetching"
ssh $K -p $P root@$IP 'cd /workspace && tar czf out.tgz out prep.out build.log' 
scp -q $KS -P $P root@$IP:/workspace/out.tgz out.tgz && mkdir -p out && tar xzf out.tgz -C . && say "fetched: $(ls out | wc -l) files"
term; rm -f pod_id.txt; sleep 5
say "GPU_RUN_DONE"
