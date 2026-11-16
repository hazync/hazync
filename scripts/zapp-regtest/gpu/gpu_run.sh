#!/bin/bash
# gpu_run.sh <bundle dir>: ⚠ SPENDS MONEY. The zAPP regtest batch on one RunPod RTX PRO 6000: deploy, provision, build
# the CUDA host around the regtest guest, prove A (1..112) and B (111..113 from A's 110), verify, HARVEST, TERMINATE.
# Adapted from experimental/libzync-bench/signet/genesis/gpu_run.sh (same pod image, keys, watcher), without its
# mid-run checkpoints: the whole batch is ESTIMATED at about an hour (README.md "GPU batch").
# A watcher harvests and then terminates at the deadline, or when the balance falls under $1 (harvest BEFORE release).
# Env: DEADLINE_MIN (default 150), GPU (default the RTX PRO 6000 Blackwell Server Edition), CLOUD (SECURE default;
# COMMUNITY is cheaper but its listings often never start: no port within PORT_TRIES x 10 s, default 60, ends the run
# after a terminate). The harvest retries 6x and checks the tarball sha256 against the pod's; if it still fails on the normal
# path the pod is LEFT RUNNING for a manual harvest (the deadline path terminates regardless: the cost cap wins).
DEADLINE_MIN=${DEADLINE_MIN:-150}; HERE=$(cd "$(dirname "$0")" && pwd)
W=$(readlink -f "${1:?bundle dir (the out dir of mkbundle.sh)}"); [ -f "$W/sb/sb.tgz" ] || { echo "run mkbundle.sh first"; exit 1; }
cd "$W"; cp -r "$HERE/pod" .; LOG=gpu_run.log; exec >> $LOG 2>&1
RP=$(cat ~/.hazync/runpod.key); PUB=$(cat ~/.ssh/ghost_signet_ed25519.pub)
K="-n -i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=20"
KS="-i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=20"
gql(){ curl -s -X POST -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" -d "$1" https://api.runpod.io/graphql; }
say(){ echo "[$(date +%T)] $*"; }
GPU="${GPU:-NVIDIA RTX PRO 6000 Blackwell Server Edition}"; CLOUD=${CLOUD:-SECURE}; PORT_TRIES=${PORT_TRIES:-60}
Q=$(python3 -c "import json,sys; print(json.dumps({'query': 'mutation { podFindAndDeployOnDemand(input: { cloudType: %s, gpuCount: 1, volumeInGb: 0, containerDiskInGb: 80, gpuTypeId: \"%s\", name: \"lz-zapp-regtest\", imageName: \"runpod/pytorch:2.4.0-py3.11-cuda12.4.1-devel-ubuntu22.04\", ports: \"22/tcp\", env: [{key: \"PUBLIC_KEY\", value: \"%s\"}] }) { id costPerHr } }' % (sys.argv[3], sys.argv[2], sys.argv[1])}))" "$PUB" "$GPU" "$CLOUD")
R=$(gql "$Q"); ID=$(echo "$R" | python3 -c 'import json,sys; p=(json.load(sys.stdin).get("data") or {}).get("podFindAndDeployOnDemand"); print(p["id"] if p else "")')
[ -n "$ID" ] || { say "deploy failed: $R"; exit 1; }
echo $ID > pod_id.txt; say "pod $ID deployed ($GPU): $R"
term(){ say "terminating $ID"; gql "{\"query\":\"mutation { podTerminate(input: {podId: \\\"$ID\\\"}) }\"}" > /dev/null; }
# Everything the run wrote except the leaf bundles; bounded, exactly once (mkdir is atomic).
harvest(){ mkdir harvest.lock 2>/dev/null || { say "harvest already running"; return 1; }
  [ -f conn.txt ] || return 0; set -- $(cat conn.txt); say "harvesting from $1:$2"
  for t in 1 2 3 4 5 6; do
    timeout 300 ssh $K -p $2 root@$1 'cd /workspace && tar czf out.tgz --exclude="bundle_*.lzb" out prep.out build.log run.out setup.log toolchain.txt 2>/dev/null; sha256sum out.tgz | cut -c1-64 > out.tgz.sha256'
    R=$(timeout 60 ssh $K -p $2 root@$1 'cat /workspace/out.tgz.sha256')
    timeout 900 scp -q $KS -P $2 root@$1:/workspace/out.tgz out.tgz && [ -n "$R" ] && [ "$(sha256sum out.tgz | cut -c1-64)" = "$R" ] \
      && rm -rf out2 && mkdir out2 && tar xzf out.tgz -C out2 && echo "$R  out.tgz" > out.tgz.sha256 \
      && { say "fetched (try $t, sha $R): $(ls out2/out/a 2>/dev/null | grep -c rcpt) A receipts, $(ls out2/out/b 2>/dev/null | grep -c rcpt) B receipts"; return 0; }
    say "FETCH FAILED (try $t)"; sleep 20; done
  return 1; }
rm -f conn.txt; rmdir harvest.lock 2>/dev/null
( END=$((SECONDS+DEADLINE_MIN*60)); while [ -f pod_id.txt ]; do
    B=$(gql '{"query":"query { myself { clientBalance } }"}' | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["myself"]["clientBalance"])' 2>/dev/null || echo 99)
    if [ $SECONDS -ge $END ] || python3 -c "import sys; sys.exit(0 if float('$B') < 1 else 1)"; then say "DEADLINE (balance $B)"; rm -f pod_id.txt; harvest; term; exit; fi
    sleep 60; done ) &
for i in $(seq 1 $PORT_TRIES); do
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
scp -q $KS -P $P sb/sb.tgz sb/sb.tgz.sha256 root@$IP:/workspace/ && say "bundle sent"
for i in $(seq 1 120); do
  [ -f pod_id.txt ] || exit 1   # the deadline fired (and harvested)
  S=$(ssh $K -p $P root@$IP 'grep -h "PREP_OK\|PREP_FAIL\|built in" /workspace/prep.out 2>/dev/null')
  echo "$S" | grep -q 'PREP_OK\|PREP_FAIL' && break; sleep 20; done
say "prep: $(echo $S | tr '\n' ' ')"
echo "$S" | grep -q PREP_OK || { ssh $K -p $P root@$IP 'tail -n 40 /workspace/build.log; tail -n 40 /workspace/setup.log; tail -n 20 /workspace/prep.out' > prep_fail.log 2>&1; rm -f pod_id.txt; harvest; term; exit 1; }
ssh $K -p $P root@$IP 'cd /workspace && (setsid nohup bash run.sh > run.out 2>&1 < /dev/null &)'
say "proving A 1..112, then B 111..113"
for i in $(seq 1 $((DEADLINE_MIN*4))); do
  [ -f pod_id.txt ] || exit 1   # the deadline fired (and harvested)
  ssh $K -p $P root@$IP 'test -f /workspace/out/RUN_DONE' && break
  ssh $K -p $P root@$IP true && F=0 || F=$((${F:-0}+1))
  [ "${F:-0}" -ge 40 ] && { say "POD_LOST: unreachable for 10 min"; rm -f pod_id.txt; term; exit 1; }
  [ $((i % 10)) = 0 ] && say "  $(ssh $K -p $P root@$IP 'tail -qn1 /workspace/out/a/times.tsv /workspace/out/b/times.tsv 2>/dev/null | tail -1')"
  sleep 15; done
say "run: $(ssh $K -p $P root@$IP 'cat /workspace/out/RUN_DONE')"
rm -f pod_id.txt   # stop the watcher; if it already started, the lock makes it the one that harvests
if harvest; then term; sleep 5; say "GPU_RUN_DONE"; else say "HARVEST_FAILED: pod $ID LEFT RUNNING for a manual harvest"; fi
