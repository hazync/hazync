#!/bin/bash
# GPU benchmarks on the freeze candidate (c14, bbf82800), ONE RunPod secure-cloud RTX PRO 6000:
#   run B   gpu-libzync-multi.sh: block 962,000, chunks + aggregate (EXPECT_CYCLES from the laptop cost table)
#   join    gpu-join.sh: blocks 1..8 as independent leaves on prep states, a real join tree, root == linear block 8
# From the laptop: deploy, set up, upload the bundles, run both, HARVEST, then terminate (by the captured id).
# A hard deadline (DEADLINE_MIN, default 150) harvests what exists and terminates, so spend is bounded.
#   bash run_c14_pod.sh            (bundles in ~/libzync-gpu/gpu14: runb/, join/; output in ~/libzync-gpu/gpu14/pod)
set -uo pipefail
L=~/libzync-gpu/gpu14; OUT=$L/pod; mkdir -p "$OUT"
COMMIT=${COMMIT:-e24f7d3}; DEADLINE=$(( $(date +%s) + ${DEADLINE_MIN:-150} * 60 ))
RP=$(cat ~/.hazync/runpod.key); PUB=$(cat ~/.ssh/ghost_signet_ed25519.pub)
K="-i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=15 -o ServerAliveInterval=30"
log () { echo "[$(date +%H:%M:%S)] $*" | tee -a "$OUT/driver.log"; }
gql () { curl -s -X POST -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" -d "$1" https://api.runpod.io/graphql; }
ID=""; HOST=""; PORT=""
term () {
  [ -n "$ID" ] || return 0
  log "terminating pod $ID"; gql "{\"query\":\"mutation { podTerminate(input: {podId: \\\"$ID\\\"}) }\"}" >> "$OUT/driver.log"; echo >> "$OUT/driver.log"
  ID=""
}
harvest () {
  [ -n "$HOST" ] || return 0
  log "harvest"
  ssh $K -p "$PORT" root@"$HOST" 'cd /workspace && tar czf harvest.tgz runb join pod.log setup.log 2>/dev/null; ls -la harvest.tgz' >> "$OUT/driver.log" 2>&1
  scp -q $K -P "$PORT" root@"$HOST":/workspace/harvest.tgz "$OUT/harvest.tgz" && (cd "$OUT" && tar xzf harvest.tgz) && log "harvested to $OUT" || log "HARVEST FAILED"
}
trap 'harvest; term' EXIT

# 1. bundles: one tarball (method.bin once; the join bundle links to it)
log "packing bundles"
rm -rf "$OUT/up"; mkdir -p "$OUT/up/bundles"; cp -r $L/runb $L/join "$OUT/up/bundles/"
rm "$OUT/up/bundles/join/method.bin"; ln -s ../runb/method.bin "$OUT/up/bundles/join/method.bin"
(cd "$OUT/up" && tar czhf ../bundles.tgz bundles) && sha256sum "$OUT/bundles.tgz" | cut -c1-64 > "$OUT/bundles.sha"
log "bundles.tgz $(du -h "$OUT/bundles.tgz" | cut -f1)"

# 2. deploy: SECURE only (community hosts mostly never start)
for G in "NVIDIA RTX PRO 6000 Blackwell Server Edition" "NVIDIA RTX PRO 6000 Blackwell Workstation Edition"; do
  Q=$(python3 -c "import json,sys; print(json.dumps({'query': 'mutation { podFindAndDeployOnDemand(input: { cloudType: SECURE, gpuCount: 1, volumeInGb: 0, containerDiskInGb: 120, gpuTypeId: \"%s\", name: \"lz-c14-bench\", imageName: \"runpod/pytorch:2.4.0-py3.11-cuda12.4.1-devel-ubuntu22.04\", ports: \"22/tcp\", env: [{key: \"PUBLIC_KEY\", value: \"%s\"}] }) { id costPerHr } }' % (sys.argv[2], sys.argv[1])}))" "$PUB" "$G")
  R=$(gql "$Q"); echo "$R" >> "$OUT/driver.log"
  ID=$(echo "$R" | python3 -c 'import json,sys; p=(json.load(sys.stdin).get("data") or {}).get("podFindAndDeployOnDemand"); print(p["id"] if p else "")' 2>/dev/null)
  [ -n "$ID" ] && { log "DEPLOYED $ID on $G"; echo "$ID" > "$OUT/pod_id"; break; }
done
[ -n "$ID" ] || { log "NO POD: nothing deployed"; exit 1; }

# 3. ssh port (judge by runtime, not desiredStatus)
for i in $(seq 1 60); do
  read HOST PORT < <(gql '{"query":"query { myself { pods { id runtime { uptimeInSeconds ports { ip isIpPublic privatePort publicPort } } } } }"}' | python3 -c '
import json,sys
for p in json.load(sys.stdin)["data"]["myself"]["pods"]:
  if p["id"]!=sys.argv[1]: continue
  for x in ((p.get("runtime") or {}).get("ports") or []):
    if x["isIpPublic"] and x["privatePort"]==22: print(x["ip"], x["publicPort"])' "$ID")
  [ -n "${HOST:-}" ] && ssh $K -p "$PORT" root@"$HOST" true 2>/dev/null && break
  HOST=""; sleep 15
done
[ -n "$HOST" ] || { log "pod never reachable over ssh"; exit 1; }
log "ssh root@$HOST -p $PORT"

# 4. setup (background on the pod) while the bundles upload
ssh $K -p "$PORT" root@"$HOST" "cat > /workspace/setup.sh" <<EOF
set -x
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq && apt-get install -y -qq build-essential cmake git curl ca-certificates pkg-config libssl-dev clang lld protobuf-compiler
curl -fsSL -o /tmp/k.deb https://developer.download.nvidia.com/compute/cuda/repos/ubuntu2204/x86_64/cuda-keyring_1.1-1_all.deb && dpkg -i /tmp/k.deb && apt-get update -qq && apt-get install -y -qq cuda-toolkit-12-8
ln -sfn /usr/local/cuda-12.8 /usr/local/cuda
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
cd /workspace && git clone -q -b libzync https://github.com/hazync/hazync.git && cd hazync && git checkout -q $COMMIT && git log --oneline -1
echo SETUP_DONE
EOF
ssh $K -p "$PORT" root@"$HOST" 'cd /workspace && (setsid nohup bash setup.sh > setup.log 2>&1 < /dev/null &)'
scp -q $K -P "$PORT" "$OUT/bundles.tgz" root@"$HOST":/workspace/ || { log "bundle upload failed"; exit 1; }
ssh $K -p "$PORT" root@"$HOST" "cd /workspace && [ \"\$(sha256sum bundles.tgz | cut -c1-64)\" = $(cat "$OUT/bundles.sha") ] && tar xzf bundles.tgz && echo BUNDLES_OK" | tee -a "$OUT/driver.log" | grep -q BUNDLES_OK || { log "bundle check failed"; exit 1; }
log "bundles uploaded and checked"

# 5. the run, on the pod
ssh $K -p "$PORT" root@"$HOST" "cat > /workspace/pod.sh" <<'EOF'
#!/bin/bash
set -uo pipefail
export CUDA_PATH=/usr/local/cuda-12.8 PATH="/usr/local/cuda-12.8/bin:$HOME/.cargo/bin:$PATH" LD_LIBRARY_PATH="/usr/local/cuda-12.8/lib64:${LD_LIBRARY_PATH:-}"
cd /workspace
until grep -q SETUP_DONE setup.log; do sleep 10; done
echo "SETUP $(tail -2 setup.log | head -1)"
H=/workspace/hazync/experimental/libzync-bench/gpu
EXPECT_CYCLES=$(cat bundles/runb/EXPECT_CYCLES) BUNDLE=/workspace/bundles/runb OUT=/workspace/runb bash $H/gpu-libzync-multi.sh; echo "RUNB_EXIT=$?"
SKIP_BUILD=1 BUNDLE=/workspace/bundles/join N=8 OUT=/workspace/join bash $H/gpu-join.sh; echo "JOIN_EXIT=$?"
echo POD_DONE
EOF
ssh $K -p "$PORT" root@"$HOST" 'cd /workspace && (setsid nohup bash pod.sh > pod.log 2>&1 < /dev/null &)'
log "pod.sh started"

# 6. wait (POD_DONE or the deadline), relaying progress lines
seen=0
while :; do
  ssh $K -p "$PORT" root@"$HOST" 'cat /workspace/pod.log 2>/dev/null' > "$OUT/pod.log.live" 2>/dev/null
  n=$(wc -l < "$OUT/pod.log.live"); [ "$n" -gt "$seen" ] && { sed -n "$((seen+1)),${n}p" "$OUT/pod.log.live" | tee -a "$OUT/driver.log"; seen=$n; }
  grep -q POD_DONE "$OUT/pod.log.live" && { log "POD_DONE"; break; }
  [ "$(date +%s)" -ge "$DEADLINE" ] && { log "DEADLINE reached: harvesting what exists"; break; }
  sleep 30
done
exit 0   # the EXIT trap harvests, then terminates
