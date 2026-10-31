#!/bin/bash
# Signet GPU proof, unattended: one RTX PRO 6000 -> provision -> build -> prove 2501..2600 recursively ->
# verify -> fetch -> TERMINATE. A separate deadline kills the pod at 75 min or a balance under $1, and
# HARVESTS /workspace/out first (review 5: a deadline must never destroy the run's evidence).
cd ~/libzync-gpu/signet; LOG=gpu_run2.log; exec >> $LOG 2>&1
RP=$(cat ~/.hazync/runpod.key); PUB=$(cat ~/.ssh/ghost_signet_ed25519.pub)
K="-n -i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=20"
KS="-i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=20"
gql(){ curl -s -X POST -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" -d "$1" https://api.runpod.io/graphql; }
say(){ echo "[$(date +%T)] $*"; }
GPU="${GPU:-NVIDIA RTX PRO 6000 Blackwell Server Edition}"
Q=$(python3 -c "import json,sys; print(json.dumps({'query': 'mutation { podFindAndDeployOnDemand(input: { cloudType: SECURE, gpuCount: 1, volumeInGb: 0, containerDiskInGb: 80, gpuTypeId: \"%s\", name: \"lz-signet\", imageName: \"runpod/pytorch:2.4.0-py3.11-cuda12.4.1-devel-ubuntu22.04\", ports: \"22/tcp\", env: [{key: \"PUBLIC_KEY\", value: \"%s\"}] }) { id costPerHr } }' % (sys.argv[2], sys.argv[1])}))" "$PUB" "$GPU")
R=$(gql "$Q"); ID=$(echo "$R" | python3 -c 'import json,sys; p=(json.load(sys.stdin).get("data") or {}).get("podFindAndDeployOnDemand"); print(p["id"] if p else "")')
[ -n "$ID" ] || { say "deploy failed: $R"; exit 1; }
echo $ID > pod_id.txt; say "pod $ID deployed ($GPU)"
term(){ say "terminating $ID"; gql "{\"query\":\"mutation { podTerminate(input: {podId: \\\"$ID\\\"}) }\"}" > /dev/null; }
# Fetch whatever the pod has (conn.txt = "ip port", written once ssh works). Bounded, so a dead pod cannot hang it.
# Exactly once: the deadline watcher and the main script can both get here; mkdir is atomic (review 5).
harvest(){ mkdir harvest.lock 2>/dev/null || { say "harvest already running"; return 1; }
  [ -f conn.txt ] || return 0; set -- $(cat conn.txt); say "harvesting from $1:$2"
  timeout 120 ssh $K -p $2 root@$1 'cd /workspace && tar czf out.tgz out prep.out build.log run.out 2>/dev/null'
  timeout 300 scp -q $KS -P $2 root@$1:/workspace/out.tgz out.tgz && rm -rf out2 && mkdir out2 && tar xzf out.tgz -C out2 && say "fetched: $(ls out2/out | wc -l) files" || say "FETCH FAILED"; return 0; }
rm -f conn.txt; rmdir harvest.lock 2>/dev/null
( END=$((SECONDS+75*60)); while [ -f pod_id.txt ]; do
    B=$(gql '{"query":"query { myself { clientBalance } }"}' | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["myself"]["clientBalance"])' 2>/dev/null || echo 99)
    if [ $SECONDS -ge $END ] || python3 -c "import sys; sys.exit(0 if float('$B') < 1 else 1)"; then say "DEADLINE (balance $B)"; rm -f pod_id.txt; harvest && term; exit; fi
    sleep 60; done ) &
for i in $(seq 1 60); do
  L=$(gql '{"query":"query { myself { pods { id runtime { ports { ip isIpPublic privatePort publicPort } } } } }"}' | python3 -c '
import json,sys
for p in json.load(sys.stdin)["data"]["myself"]["pods"]:
  if p["id"]!=sys.argv[1]: continue
  for x in ((p.get("runtime") or {}).get("ports") or []):
    if x["isIpPublic"] and x["privatePort"]==22: print(x["ip"], x["publicPort"])' $ID)
  [ -n "$L" ] && break; sleep 10; done
[ -n "$L" ] || { say "no ports"; term; rm -f pod_id.txt; exit 1; }
set -- $L; IP=$1; P=$2; say "ssh $IP:$P"; echo "$IP $P" > conn.txt
for t in $(seq 1 30); do ssh $K -p $P root@$IP true && break; sleep 10; done
scp -q $KS -P $P pod/setup.sh pod/prep.sh pod/run.sh root@$IP:/workspace/ && say "scripts sent"
ssh $K -p $P root@$IP 'cd /workspace && (setsid nohup bash prep.sh > prep.out 2>&1 < /dev/null &)'
scp -q $KS -P $P sb2/sb.tgz sb2/sb.tgz.sha256 root@$IP:/workspace/ && say "bundle sent"
for i in $(seq 1 120); do
  [ -f pod_id.txt ] || exit 1   # the deadline fired (and harvested)
  S=$(ssh $K -p $P root@$IP 'grep -h "PREP_OK\|PREP_FAIL\|built in" /workspace/prep.out 2>/dev/null')
  echo "$S" | grep -q 'PREP_OK\|PREP_FAIL' && break; sleep 20; done
say "prep: $(echo $S | tr '\n' ' ')"
echo "$S" | grep -q PREP_OK || { ssh $K -p $P root@$IP 'tail -n 40 /workspace/build.log; tail -n 40 /workspace/setup.log; tail -n 20 /workspace/prep.out' > prep_fail.log 2>&1; term; rm -f pod_id.txt; exit 1; }
ssh $K -p $P root@$IP 'cd /workspace && (setsid nohup bash run.sh 2501 2600 > run.out 2>&1 < /dev/null &)'
say "proving 2501..2600"
for i in $(seq 1 480); do
  [ -f pod_id.txt ] || exit 1   # the deadline fired (and harvested)
  ssh $K -p $P root@$IP 'grep -q RUN_DONE /workspace/out/prove.log 2>/dev/null' && break
  [ $((i % 10)) = 0 ] && say "  $(ssh $K -p $P root@$IP 'tail -1 /workspace/out/times.tsv 2>/dev/null')"
  sleep 15; done
rm -f pod_id.txt   # stop the watcher; if it already started, the lock makes it the one that harvests
harvest && term; sleep 5
say "GPU_RUN_DONE"
