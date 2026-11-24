#!/bin/bash
# gpu_block.sh <bundle.tgz>: ⚠ SPENDS MONEY. One RunPod GPU (default RTX A6000, SECURE): build hzengine with CUDA,
# prove a whole block with in-guest recursion (pod/run.sh), harvest, terminate. Pod handling as
# scripts/zapp-regtest/gpu/gpu_run.sh: a watcher harvests then terminates at DEADLINE_MIN or a balance under $1.
# The bundle holds host/ (engine/host source), keys/ (sdk.pk agg.vk openvm.toml guest.vmexe baseline.json) and
# export/ (the product host's engine-export of the block).
DEADLINE_MIN=${DEADLINE_MIN:-90}; HERE=$(cd "$(dirname "$0")" && pwd); RPY=$HERE/../../scripts/zapp-regtest/gpu/rp.py
B=$(readlink -f "${1:?bundle.tgz}"); W=$(dirname "$B")/run_$(date +%Y%m%d_%H%M%S); mkdir -p "$W"; cd "$W" || exit 1
LOG=gpu_block.log; exec >> $LOG 2>&1
RP=$(cat ~/.hazync/runpod.key); PUB=$(cat ~/.ssh/ghost_signet_ed25519.pub)
K="-n -i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=20"
KS="-i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=20"
export RP; rp(){ python3 "$RPY" "$@"; }
gqlbal(){ curl -s -X POST -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" -d '{"query":"query { myself { clientBalance } }"}' https://api.runpod.io/graphql; }
say(){ echo "[$(date +%T)] $*"; }
# GPUS: tried in order until one deploys (A40 = the A6000's GA102 chip, same 48 GB; L40S the next class up)
GPUS="${GPUS:-NVIDIA RTX A6000|NVIDIA A40|NVIDIA L40S}"; PORT_TRIES=${PORT_TRIES:-60}
# DEPLOY_WAIT_MIN: keep retrying the list every 2 min until one deploys (default: one pass)
ID=""; IFS='|' read -ra GL <<< "$GPUS"; DEND=$((SECONDS + ${DEPLOY_WAIT_MIN:-0}*60))
while :; do
  for GPU in "${GL[@]}"; do ID=$(rp deploy hz-engine-block "$GPU" "$PUB") && [ -n "$ID" ] && break 2; say "no $GPU"; ID=""; done
  [ $SECONDS -lt $DEND ] || break; sleep 120
done
[ -n "$ID" ] || { say "deploy failed on every GPU type"; exit 1; }
[ -n "$ID" ] || { say "deploy failed: no pod id"; exit 1; }
echo "$ID" > pod_id.txt; say "pod $ID deployed ($GPU)"
term(){ say "terminating $ID"; rp terminate "$ID" || say "TERMINATE FAILED for $ID: check the RunPod console"; }
harvest(){ mkdir harvest.lock 2>/dev/null || { say "harvest already running"; return 1; }
  [ -f conn.txt ] || return 0; read -r CIP CP < conn.txt; say "harvesting from $CIP:$CP"
  for t in 1 2 3 4 5 6; do
    timeout 300 ssh $K -p $CP root@$CIP 'cd /workspace && tar czf out.tgz --exclude="*.proof.json" out setup.out build.log; tar czf proofs.tgz out/*/block.proof.json out/block 2>/dev/null 2>/dev/null; sha256sum out.tgz | cut -c1-64 > out.tgz.sha256'
    R=$(timeout 60 ssh $K -p $CP root@$CIP 'cat /workspace/out.tgz.sha256')
    timeout 900 scp -q $KS -P $CP root@$CIP:/workspace/out.tgz out.tgz && [ -n "$R" ] && [ "$(sha256sum out.tgz | cut -c1-64)" = "$R" ] \
      && rm -rf out2 && mkdir out2 && tar xzf out.tgz -C out2 && { say "fetched (try $t, sha $R)"; return 0; }
    say "FETCH FAILED (try $t)"; sleep 20; done
  return 1; }
rm -f conn.txt; rmdir harvest.lock 2>/dev/null
( END=$((SECONDS+DEADLINE_MIN*60)); while [ -f pod_id.txt ]; do
    BAL=$(gqlbal | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["myself"]["clientBalance"])' 2>/dev/null || echo 99)
    if [ $SECONDS -ge $END ] || python3 -c "import sys; sys.exit(0 if float('$BAL') < 1 else 1)"; then say "DEADLINE (balance $BAL)"; rm -f pod_id.txt; harvest; term; exit; fi
    sleep 60; done ) &
for i in $(seq 1 $PORT_TRIES); do L=$(rp ssh "$ID"); [ -n "$L" ] && break; sleep 10; done
[ -n "$L" ] || { say "no ports"; term; rm -f pod_id.txt; exit 1; }
set -- $L; IP=$1; P=$2; say "ssh $IP:$P"; echo "$IP $P" > conn.txt
for t in $(seq 1 30); do ssh $K -p $P root@$IP true && break; sleep 10; done
RUN=${RUN:-run.sh}
scp -q $KS -P $P "$HERE/pod/setup.sh" "$HERE/pod/$RUN" "$B" root@$IP:/workspace/ && say "scripts + bundle sent"
ssh $K -p $P root@$IP 'cd /workspace && mv -f *.tgz bundle.tgz 2>/dev/null; (setsid nohup bash setup.sh > setup.out 2>&1 < /dev/null &)'
for i in $(seq 1 180); do
  [ -f pod_id.txt ] || exit 1
  S=$(ssh $K -p $P root@$IP 'grep -h "SETUP_OK\|SETUP_FAIL\|built in" /workspace/setup.out 2>/dev/null')
  echo "$S" | grep -q 'SETUP_OK\|SETUP_FAIL' && break; sleep 20; done
say "setup: $(echo $S | tr '\n' ' ')"
echo "$S" | grep -q SETUP_OK || { rm -f pod_id.txt; harvest; term; exit 1; }
ssh $K -p $P root@$IP "cd /workspace && (R0_BLOCKS='$R0_BLOCKS' JOBS='$JOBS' setsid nohup bash $RUN > run.out 2>&1 < /dev/null &)"
say "proving the block"
for i in $(seq 1 $((DEADLINE_MIN*4))); do
  [ -f pod_id.txt ] || exit 1
  ssh $K -p $P root@$IP 'test -f /workspace/out/RUN_DONE' && break
  ssh $K -p $P root@$IP true && F=0 || F=$((${F:-0}+1))
  [ "${F:-0}" -ge 40 ] && { say "POD_LOST: unreachable for 10 min"; rm -f pod_id.txt; term; exit 1; }
  [ $((i % 8)) = 0 ] && say "  $(ssh $K -p $P root@$IP 'tail -n1 /workspace/out/times.tsv 2>/dev/null; grep -h "proved\|aggregate" /workspace/out/prove_block.log 2>/dev/null | tail -1')"
  sleep 15; done
say "run: $(ssh $K -p $P root@$IP 'cat /workspace/out/RUN_DONE /workspace/out/times.txt /workspace/out/times.tsv 2>/dev/null')"
rm -f pod_id.txt
if harvest; then term; sleep 5; say "GPU_BLOCK_DONE"; else say "HARVEST_FAILED: pod $ID LEFT RUNNING for a manual harvest"; fi
