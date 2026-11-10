#!/bin/bash
# The header-chain zyncAPP (ROADMAP N3) proved on ONE RunPod secure-cloud RTX PRO 6000: the whole mainnet header chain
# from genesis in one succinct receipt, with the PUBLISHED guest ELF (reproduce/LIBZYNC_IDS zyncapp-headers line).
# From the laptop: deploy, set up, upload headers + ELF, prove, HARVEST, terminate by the captured id. DEADLINE_MIN
# bounds the spend; the pod also removes itself 15 minutes after it.
#   HEADERS=<file of "height hex80" lines, gzipped> ELF=<zyncapp-headers.bin> bash run_headers_pod.sh
set -uo pipefail
HEADERS=${HEADERS:?gzipped headers file}; ELF=${ELF:?zyncapp-headers.bin}
OUT=${OUT:-$HOME/libzync-gpu/headers/pod-$(date +%m%d-%H%M)}; mkdir -p "$OUT"
Z=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
COMMIT=${COMMIT:-$(git -C "$Z" rev-parse HEAD)}
git -C "$Z" fetch -q origin && git -C "$Z" branch -r --contains "$COMMIT" | grep . >/dev/null || { echo "commit $COMMIT is not pushed"; exit 1; }
WANT_ELF=$(awk '$1=="zyncapp-headers" {print $6}' "$Z/reproduce/LIBZYNC_IDS"); WANT_ID=$(awk '$1=="zyncapp-headers" {print $4}' "$Z/reproduce/LIBZYNC_IDS")
[ "$(sha256sum "$ELF" | cut -c1-64)" = "$WANT_ELF" ] || { echo "$ELF is not the published headers ELF $WANT_ELF"; exit 1; }
DEADLINE=$(( $(date +%s) + ${DEADLINE_MIN:-120} * 60 ))
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
harvest_once () {
  ssh $K -p "$PORT" root@"$HOST" 'cd /workspace && tar czf harvest.tgz $(ls -d run host pod.log setup.log 2>/dev/null); ls -la harvest.tgz' >> "$OUT/driver.log" 2>&1 \
    && scp -q $K -P "$PORT" root@"$HOST":/workspace/harvest.tgz "$OUT/harvest.tgz" && (cd "$OUT" && tar xzf harvest.tgz)
}
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

# 1. deploy: SECURE only
for G in "NVIDIA RTX PRO 6000 Blackwell Server Edition" "NVIDIA RTX PRO 6000 Blackwell Workstation Edition"; do
  Q=$(python3 -c "import json,sys; print(json.dumps({'query': 'mutation { podFindAndDeployOnDemand(input: { cloudType: SECURE, gpuCount: 1, volumeInGb: 0, containerDiskInGb: 80, gpuTypeId: \"%s\", name: \"lz-headers\", imageName: \"runpod/pytorch:2.4.0-py3.11-cuda12.4.1-devel-ubuntu22.04\", ports: \"22/tcp\", env: [{key: \"PUBLIC_KEY\", value: \"%s\"}] }) { id costPerHr } }' % (sys.argv[2], sys.argv[1])}))" "$PUB" "$G")
  R=$(gql "$Q"); echo "$R" >> "$OUT/driver.log"
  ID=$(echo "$R" | python3 -c 'import json,sys; p=(json.load(sys.stdin).get("data") or {}).get("podFindAndDeployOnDemand"); print(p["id"] if p else "")' 2>/dev/null)
  [ -n "$ID" ] && { log "DEPLOYED $ID on $G ($(echo "$R" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["podFindAndDeployOnDemand"]["costPerHr"])') \$/hr)"; echo "$ID" > "$OUT/pod_id"; break; }
done
[ -n "$ID" ] || { log "NO POD: nothing deployed"; exit 1; }

# 2. ssh port (judge by runtime, not desiredStatus)
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

# 3. setup on the pod (background) while the inputs upload
ssh $K -p "$PORT" root@"$HOST" "cat > /workspace/setup.sh" <<EOF
set -ex
trap 'echo SETUP_FAILED' ERR
export DEBIAN_FRONTEND=noninteractive
( sleep $(( ${DEADLINE_MIN:-120} * 60 + 900 )); runpodctl remove pod "\$RUNPOD_POD_ID" ) > /workspace/selfterm.log 2>&1 &
apt-get update -qq && apt-get install -y -qq build-essential cmake git curl ca-certificates pkg-config libssl-dev clang lld protobuf-compiler
curl -fsSL -o /tmp/k.deb https://developer.download.nvidia.com/compute/cuda/repos/ubuntu2204/x86_64/cuda-keyring_1.1-1_all.deb && dpkg -i /tmp/k.deb && apt-get update -qq && apt-get install -y -qq cuda-toolkit-12-8
ln -sfn /usr/local/cuda-12.8 /usr/local/cuda
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
cd /workspace && git clone -q https://github.com/hazync/hazync.git && cd hazync && git checkout -q $COMMIT
[ "\$(git rev-parse HEAD)" = "\$(git rev-parse $COMMIT)" ] || false
git log --oneline -1
echo SETUP_DONE
EOF
ssh $K -p "$PORT" root@"$HOST" 'cd /workspace && (setsid nohup bash setup.sh > setup.log 2>&1 < /dev/null &)'
scp -q $K -P "$PORT" "$HEADERS" root@"$HOST":/workspace/headers.txt.gz && scp -q $K -P "$PORT" "$ELF" root@"$HOST":/workspace/zyncapp-headers.bin || { log "upload failed"; exit 1; }
ssh $K -p "$PORT" root@"$HOST" "cd /workspace && [ \"\$(sha256sum zyncapp-headers.bin | cut -c1-64)\" = $WANT_ELF ] && gunzip -f headers.txt.gz && wc -l headers.txt && echo INPUTS_OK" | tee -a "$OUT/driver.log" | grep INPUTS_OK >/dev/null || { log "input check failed"; exit 1; }
log "headers and the published ELF uploaded and checked"

# 4. the run, on the pod
ssh $K -p "$PORT" root@"$HOST" "cat > /workspace/pod.sh" <<EOF
#!/bin/bash
set -uo pipefail
export CUDA_PATH=/usr/local/cuda-12.8 PATH="/usr/local/cuda-12.8/bin:\$HOME/.cargo/bin:\$PATH" LD_LIBRARY_PATH="/usr/local/cuda-12.8/lib64:\${LD_LIBRARY_PATH:-}"
cd /workspace
until grep -q '^SETUP_DONE\|^SETUP_FAILED' setup.log; do sleep 10; done
grep -q '^SETUP_DONE' setup.log || { echo "SETUP FAILED (setup.log)"; echo POD_DONE; exit 1; }
mkdir -p host run; lscpu > host/lscpu.txt; nproc > host/nproc.txt; nvidia-smi -q > host/nvidia-smi-q.txt 2>&1
echo "HOST \$(grep -m1 'Model name' host/lscpu.txt | sed 's/.*: *//') x\$(cat host/nproc.txt); CARD \$(nvidia-smi --query-gpu=name,memory.total --format=csv,noheader | head -1)"
for v in \$(compgen -e | grep '^HAZYNC_'); do unset "\$v"; done; unset RISC0_DEV_MODE
echo "[\$(date +%T)] === build zyncapp (cuda, guests skipped: the published ELF is given) ==="
( cd hazync/prover && RISC0_SKIP_BUILD=1 cargo build --release --features cuda -p zyncapp-host ) > run/build.log 2>&1 || { echo "BUILD FAILED"; tail -20 run/build.log; echo POD_DONE; exit 1; }
ZA=hazync/prover/target/release/zyncapp
\$ZA headers-prove headers.txt /dev/null --execute --elf zyncapp-headers.bin | tee run/exec.txt
grep -q "^HEADERS_ID $WANT_ID\$" run/exec.txt || { echo "IMAGE ID is not the published $WANT_ID"; echo POD_DONE; exit 1; }
( nvidia-smi --query-gpu=utilization.gpu,memory.used,power.draw --format=csv,noheader -l 10 > run/smi.log 2>/dev/null ) & SMI=\$!
echo "[\$(date +%T)] === prove genesis to tip ==="
/usr/bin/time -v \$ZA headers-prove headers.txt run/headers_0_tip.rcpt --elf zyncapp-headers.bin > run/prove.txt 2> run/prove.time
echo "PROVE_EXIT=\$?"; cat run/prove.txt; grep -E 'Elapsed|Maximum resident' run/prove.time
kill \$SMI 2>/dev/null
ls -la run/headers_0_tip.rcpt 2>/dev/null && sha256sum run/headers_0_tip.rcpt
echo HEADERS_RUN_DONE
echo POD_DONE
EOF
ssh $K -p "$PORT" root@"$HOST" "cd /workspace && (setsid nohup bash pod.sh > pod.log 2>&1 < /dev/null &)"
log "pod.sh started"

# 5. wait (POD_DONE or the deadline), relaying progress, copying outputs early
seen=0
while :; do
  ssh $K -p "$PORT" root@"$HOST" 'cat /workspace/pod.log 2>/dev/null' > "$OUT/pod.log.live" 2>/dev/null
  n=$(wc -l < "$OUT/pod.log.live"); [ "$n" -gt "$seen" ] && { sed -n "$((seen+1)),${n}p" "$OUT/pod.log.live" | tee -a "$OUT/driver.log"; seen=$n; }
  if [ ! -f "$OUT/early.done" ] && grep -q HEADERS_RUN_DONE "$OUT/pod.log.live"; then
    harvest_once && touch "$OUT/early.done" && log "early copy of the outputs to $OUT"
  fi
  grep -q POD_DONE "$OUT/pod.log.live" && { log "POD_DONE"; break; }
  [ "$(date +%s)" -ge "$DEADLINE" ] && { log "DEADLINE reached: harvesting what exists"; break; }
  sleep 30
done
grep -q 'PROVE_EXIT=0' "$OUT/pod.log.live" || exit 2
