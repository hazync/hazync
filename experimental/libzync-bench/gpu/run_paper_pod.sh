#!/bin/bash
# The paper's same-card run (user 11-07), ONE RunPod secure-cloud RTX PRO 6000: gpu-paper.sh proves each block by
# libzync (a leaf from prep's REAL-state leaf_<h>.bin) and by the original CORE guest (v0.22.1 release, bundle_<h>.json).
# From the laptop: deploy, set up, upload, run, HARVEST, then terminate (by the captured id); DEADLINE_MIN bounds spend.
#   HEIGHTS="200000 300000" bash run_paper_pod.sh      (bundle dir ~/libzync-gpu/paper: method.bin pins/ leaf/ core/)
set -uo pipefail
# RUN=range: gpu-range.sh instead (a real chain proof LO..HI from genesis; bundle: method.bin pins/ blocks/)
RUN=${RUN:-paper}; LO=${LO:-1}; HI=${HI:-}
L=~/libzync-gpu/paper; OUT=${OUT:-$L/pod-$RUN-$(date +%m%d-%H%M)}; HEIGHTS=${HEIGHTS:-}; MODELLED=${MODELLED:-}; mkdir -p "$OUT"
# the commit the pod runs: by default this checkout's HEAD, which must be on the remote (the pod clones it)
Z=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
COMMIT=${COMMIT:-$(git -C "$Z" rev-parse HEAD)}
git -C "$Z" fetch -q origin && git -C "$Z" branch -r --contains "$COMMIT" | grep . >/dev/null || { echo "commit $COMMIT is not pushed"; exit 1; }
DEADLINE=$(( $(date +%s) + ${DEADLINE_MIN:-150} * 60 ))
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
# one attempt at copying the run's outputs off the pod (tar of whatever exists, then scp, then unpack here)
harvest_once () {
  ssh $K -p "$PORT" root@"$HOST" 'cd /workspace && tar czf harvest.tgz $(ls -d paper range/keep range/*.log host pod.log setup.log 2>/dev/null); ls -la harvest.tgz' >> "$OUT/driver.log" 2>&1 \
    && scp -q $K -P "$PORT" root@"$HOST":/workspace/harvest.tgz "$OUT/harvest.tgz" && (cd "$OUT" && tar xzf harvest.tgz)
}
# the 11-07 range run lost its proof: ONE failed attempt (an ssh timeout at the end) and the pod was terminated. Now:
# retry with back-off for up to 10 minutes, and say clearly if it never worked (the caller still terminates: spend)
harvest () {
  [ -n "$HOST" ] || return 0
  log "harvest"
  for i in 1 2 3 4 5 6 7 8; do
    harvest_once && { log "harvested to $OUT (attempt $i)"; return 0; }
    log "harvest attempt $i failed; retrying in $((i * 15)) s"; sleep $((i * 15))
  done
  log "HARVEST FAILED after 8 attempts (~9 min)"
}
trap 'harvest; term' EXIT

# 1. bundles: one tarball (the pinned libzync guest + pins, leaf bundles, CORE bundles)
log "packing bundles"
rm -rf "$OUT/up"; mkdir -p "$OUT/up/bundles"
if [ "$RUN" = range ]; then
  [ -n "$HI" ] || { echo "RUN=range needs HI"; exit 1; }
  cp -r $L/method.bin $L/pins "$OUT/up/bundles/"; mkdir -p "$OUT/up/bundles/blocks"
  for h in $(seq "$LO" "$HI"); do cp ~/libzync-gpu/ssweep/block_$h.json "$OUT/up/bundles/blocks/" || exit 1; done
else
  cp -r $L/method.bin $L/pins $L/leaf $L/core $L/modelled $L/txcounts.txt "$OUT/up/bundles/"
fi
(cd "$OUT/up" && tar czhf ../bundles.tgz bundles) && sha256sum "$OUT/bundles.tgz" | cut -c1-64 > "$OUT/bundles.sha"
log "bundles.tgz $(du -h "$OUT/bundles.tgz" | cut -f1)"

# 2. deploy: SECURE only (community hosts mostly never start)
for G in "NVIDIA RTX PRO 6000 Blackwell Server Edition" "NVIDIA RTX PRO 6000 Blackwell Workstation Edition"; do
  Q=$(python3 -c "import json,sys; print(json.dumps({'query': 'mutation { podFindAndDeployOnDemand(input: { cloudType: SECURE, gpuCount: 1, volumeInGb: 0, containerDiskInGb: 120, gpuTypeId: \"%s\", name: \"lz-paper\", imageName: \"runpod/pytorch:2.4.0-py3.11-cuda12.4.1-devel-ubuntu22.04\", ports: \"22/tcp\", env: [{key: \"PUBLIC_KEY\", value: \"%s\"}] }) { id costPerHr } }' % (sys.argv[2], sys.argv[1])}))" "$PUB" "$G")
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
set -ex
trap 'echo SETUP_FAILED' ERR
export DEBIAN_FRONTEND=noninteractive
# spend guard on the pod itself, in case the laptop driver dies: remove this pod after the deadline (best effort:
# needs runpodctl and the pod's own RUNPOD_* environment; the laptop's deadline is the primary guard)
( sleep $(( ${DEADLINE_MIN:-150} * 60 + 900 )); runpodctl remove pod "\$RUNPOD_POD_ID" ) > /workspace/selfterm.log 2>&1 &
apt-get update -qq && apt-get install -y -qq build-essential cmake git curl ca-certificates pkg-config libssl-dev clang lld protobuf-compiler
curl -fsSL -o /tmp/k.deb https://developer.download.nvidia.com/compute/cuda/repos/ubuntu2204/x86_64/cuda-keyring_1.1-1_all.deb && dpkg -i /tmp/k.deb && apt-get update -qq && apt-get install -y -qq cuda-toolkit-12-8
ln -sfn /usr/local/cuda-12.8 /usr/local/cuda
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
cd /workspace && git clone -q -b libzync https://github.com/hazync/hazync.git && cd hazync && git checkout -q $COMMIT
[ "\$(git rev-parse HEAD)" = "\$(git rev-parse $COMMIT)" ] || false
git log --oneline -1
echo SETUP_DONE
EOF
ssh $K -p "$PORT" root@"$HOST" 'cd /workspace && (setsid nohup bash setup.sh > setup.log 2>&1 < /dev/null &)'
scp -q $K -P "$PORT" "$OUT/bundles.tgz" root@"$HOST":/workspace/ || { log "bundle upload failed"; exit 1; }
ssh $K -p "$PORT" root@"$HOST" "cd /workspace && [ \"\$(sha256sum bundles.tgz | cut -c1-64)\" = $(cat "$OUT/bundles.sha") ] && tar xzf bundles.tgz && echo BUNDLES_OK" | tee -a "$OUT/driver.log" | grep BUNDLES_OK >/dev/null || { log "bundle check failed"; exit 1; }
log "bundles uploaded and checked"

# 5. the run, on the pod
ssh $K -p "$PORT" root@"$HOST" "cat > /workspace/pod.sh" <<'EOF'
#!/bin/bash
set -uo pipefail
export CUDA_PATH=/usr/local/cuda-12.8 PATH="/usr/local/cuda-12.8/bin:$HOME/.cargo/bin:$PATH" LD_LIBRARY_PATH="/usr/local/cuda-12.8/lib64:${LD_LIBRARY_PATH:-}"
cd /workspace
until grep -q '^SETUP_DONE\|^SETUP_FAILED' setup.log; do sleep 10; done
grep -q '^SETUP_DONE' setup.log || { echo "SETUP FAILED (setup.log)"; echo POD_DONE; exit 1; }
echo "SETUP $(tail -2 setup.log | head -1)"
H=/workspace/hazync/experimental/libzync-bench/gpu
# the host, before and during: is anything else using the CPUs this card's witness generation needs?
mkdir -p host; lscpu > host/lscpu.txt; nproc > host/nproc.txt; nvidia-smi -q > host/nvidia-smi-q.txt 2>&1
( while :; do echo "== $(date +%T) $(cut -d' ' -f1-3 /proc/loadavg)"; top -bn1 -w 200 | sed -n '7,22p'; sleep 20; done ) > host/top.log 2>&1 & TOP=$!
echo "HOST $(grep -m1 'Model name' host/lscpu.txt | sed 's/.*: *//') x$(cat host/nproc.txt), load $(cut -d' ' -f1-3 /proc/loadavg)"
if [ "$RUN" = range ]; then BUNDLE=/workspace/bundles LO="$LO" HI="$HI" OUT=/workspace/range bash $H/gpu-range.sh; echo "PAPER_EXIT=$?"
else BUNDLE=/workspace/bundles HEIGHTS="$HEIGHTS" MODELLED="$MODELLED" OUT=/workspace/paper bash $H/gpu-paper.sh; echo "PAPER_EXIT=$?"; fi
kill $TOP 2>/dev/null
echo POD_DONE
EOF
ssh $K -p "$PORT" root@"$HOST" "cd /workspace && (RUN='$RUN' LO='$LO' HI='$HI' HEIGHTS='$HEIGHTS' MODELLED='$MODELLED' setsid nohup bash pod.sh > pod.log 2>&1 < /dev/null &)"
log "pod.sh started (heights: $HEIGHTS)"

# 6. wait (POD_DONE or the deadline), relaying progress lines
seen=0
while :; do
  ssh $K -p "$PORT" root@"$HOST" 'cat /workspace/pod.log 2>/dev/null' > "$OUT/pod.log.live" 2>/dev/null
  n=$(wc -l < "$OUT/pod.log.live"); [ "$n" -gt "$seen" ] && { sed -n "$((seen+1)),${n}p" "$OUT/pod.log.live" | tee -a "$OUT/driver.log"; seen=$n; }
  # push each leaf bundle to the pod as prep writes it (on the coordinator box: the main run's or the check run's leaves)
  for h in $HEIGHTS; do
    [ -f "$OUT/pushed_$h" ] && continue
    for d in /srv/bulk/libzync-prep/leaves /srv/bulk/libzync-check/leaves; do
      if ssh -o ConnectTimeout=15 hazync-proof "test -f $d/leaf_$h.bin" 2>/dev/null; then
        mkdir -p "$L/leaf"; scp -q hazync-proof:$d/leaf_$h.bin "$L/leaf/leaf_$h.bin.part" && mv "$L/leaf/leaf_$h.bin.part" "$L/leaf/leaf_$h.bin" \
          && scp -q $K -P "$PORT" "$L/leaf/leaf_$h.bin" root@"$HOST":/workspace/bundles/leaf/leaf_$h.bin.part \
          && ssh $K -p "$PORT" root@"$HOST" "mv /workspace/bundles/leaf/leaf_$h.bin.part /workspace/bundles/leaf/leaf_$h.bin" \
          && touch "$OUT/pushed_$h" && log "pushed leaf_$h.bin"
        break
      fi
    done
  done
  # copy the key outputs as soon as they exist, not only at the end (a pod can vanish before the final harvest)
  if [ ! -f "$OUT/early.done" ] && grep -q "RANGE_RUN_DONE\|PAPER_RUN_DONE" "$OUT/pod.log.live"; then
    harvest_once && touch "$OUT/early.done" && log "early copy of the outputs to $OUT"
  fi
  grep -q POD_DONE "$OUT/pod.log.live" && { log "POD_DONE"; break; }
  [ "$(date +%s)" -ge "$DEADLINE" ] && { log "DEADLINE reached: harvesting what exists"; break; }
  sleep 30
done
# the EXIT trap harvests, then terminates; the exit code says whether every run passed
grep -q 'PAPER_EXIT=[1-9]\|SETUP FAILED' "$OUT/pod.log.live" && exit 2
grep -q POD_DONE "$OUT/pod.log.live" || exit 3
exit 0
