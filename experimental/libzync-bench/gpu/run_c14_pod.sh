#!/bin/bash
# GPU benchmarks on the freeze candidate (c14, bbf82800), ONE RunPod secure-cloud RTX PRO 6000:
#   run B   gpu-libzync-multi.sh: block 962,000, chunks + aggregate (EXPECT_CYCLES from the laptop cost table)
#   join    gpu-join.sh: blocks 1..8 as independent leaves on prep states, a real join tree, root == linear block 8
# From the laptop: deploy, set up, upload the bundles, run both, HARVEST, then terminate (by the captured id).
# A hard deadline (DEADLINE_MIN, default 150) harvests what exists and terminates, so spend is bounded.
#   bash run_c14_pod.sh            (bundles in ~/libzync-gpu/gpu14: runb/, join/; output in ~/libzync-gpu/gpu14/pod)
#   RUNS=runb OUT=~/libzync-gpu/gpu14/pod2 bash run_c14_pod.sh    (run B only, e.g. a rerun on another host)
# Every run records the host: CPU model, cores, load and the busiest processes before and during proving.
set -uo pipefail
L=~/libzync-gpu/gpu14; OUT=${OUT:-$L/pod}; RUNS=${RUNS:-runb join}; mkdir -p "$OUT"
# the commit the pod runs: by default this checkout's HEAD, which must be on the remote (the pod clones it)
Z=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
COMMIT=${COMMIT:-$(git -C "$Z" rev-parse HEAD)}
git -C "$Z" fetch -q origin && git -C "$Z" branch -r --contains "$COMMIT" | grep . >/dev/null || { echo "commit $COMMIT is not pushed"; exit 1; }
DEADLINE=$(( $(date +%s) + ${DEADLINE_MIN:-150} * 60 ))
RP=$(cat ~/.hazync/runpod.key); PUB=$(cat ~/.ssh/ghost_signet_ed25519.pub)
K="-i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=15 -o ServerAliveInterval=30"
log () { echo "[$(date +%H:%M:%S)] $*" | tee -a "$OUT/driver.log"; }
rp (){ curl -s --max-time 20 -X "$1" -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" ${3:+-d "$3"} "https://api.runpod.io/v2$2"; }
RUNTOK=${RUNTOK:-$(date +%s)}; TRY=0   # every create attempt gets its own pod name (rp_create)
# rp_create <body> <name>: POST /v2/pods; prints the new pod's id. Exit 0 = a pod; 1 = no capacity (400) or
# RunPod's own 403 for this type; 2 = FATAL for every type (401 key, 402 balance, 422 request, a Cloudflare page);
# 3 = rate-limited (429: wait, retry); 4 = THE ANSWER WAS LOST and no pod could be confirmed either way -- a pod
# named <name> MAY BE BILLING: stop renting under that name and record it for the teardown. (Exit 4 with an id
# printed: that pod is ours, but a duplicate of the same name would not release.)
# ⛔ Give every ATTEMPT its own <name> (a run token and a try number): with no answer or a 5xx the pod may exist
# anyway, and it is looked up by that exact name -- a reused name could find another run's pod.
rp_create(){ local R ID t N="$2"
  R=$(rp POST /pods "$1")
  ID=$(printf '%s' "$R" | python3 -c 'import json,sys
try: d=json.load(sys.stdin)
except Exception: print("?"); sys.exit()
s=d.get("status") if isinstance(d, dict) else None
if isinstance(d, dict) and d.get("id") and not isinstance(s, int): print(d["id"])
elif isinstance(s, int) and s < 500: print("E%d" % s)
else: print("?")' 2>/dev/null)
  case "$R" in *"error code: 10"*) echo "create REFUSED (fatal, Cloudflare): ${R:0:120}" >&2; return 2 ;; esac
  case "$ID" in
    "?"|"")
      for t in 1 2 3; do
        ID=$(rp GET '/pods?limit=1000' | python3 -c 'import json,sys
hits=[p["id"] for p in json.load(sys.stdin)["pods"] if p.get("name")==sys.argv[1] and p.get("status")!="TERMINATED"]
print(" ".join(hits) if hits else "-")' "$2" 2>/dev/null)
        [ -n "$ID" ] && break; sleep 5
      done
      case "$ID" in
        "") echo "⛔ create of $2: no answer (${R:0:120}) and RunPod could not be listed -- A POD NAMED $2 MAY BE BILLING" >&2; return 4 ;;
        -) echo "create of $2: no answer (${R:0:120}); RunPod lists no pod of that name" >&2; return 1 ;;
        *" "*) echo "⛔ create of $2: no answer, and SEVERAL pods carry that name: $ID -- keeping the first, releasing the rest" >&2
               set -- $ID; ID=$1; shift
               rp_term "$@" || { echo "⛔ a duplicate pod of $N could not be released" >&2; printf '%s\n' "$ID"; return 4; } ;;
        *) echo "create of $2: answer lost (${R:0:120}); found it by name: $ID" >&2 ;;
      esac
      printf '%s\n' "$ID"; return 0 ;;
    E400) echo "create: no capacity: ${R:0:200}" >&2; return 1 ;;
    E401|E402|E422) echo "create REFUSED (fatal): ${R:0:300}" >&2; return 2 ;;
    E403) echo "create: no access to this type: ${R:0:200}" >&2; return 1 ;;
    E429) echo "create: rate-limited: ${R:0:200}" >&2; return 3 ;;
    E*) echo "create refused: ${R:0:300}" >&2; return 1 ;;
  esac
  printf '%s\n' "$ID"; }
# rp_term <id>...: EVERY DELETE first, then confirm each (GET answers 404, or the pod reads TERMINATED); the ones
# still there are deleted and checked again, with growing waits, for ~10 min (a 429 included). ⛔ Loud for each pod
# it cannot confirm gone (it may still be billing); exit 1 then, 2 when given no id at all.
rp_term(){ local t id left="$*" still
  left=$(echo $left); [ -n "$left" ] || { echo "rp_term: no pod id given" >&2; return 2; }
  for t in $(seq 1 20); do
    for id in $left; do rp DELETE "/pods/$id" > /dev/null; done
    still=""
    for id in $left; do
      if rp GET "/pods/$id" | python3 -c 'import json,sys; d=json.load(sys.stdin); sys.exit(0 if d.get("status") in (404, "TERMINATED") else 1)' 2>/dev/null; then
        echo "[$(date +%T)] terminated $id (confirmed gone)" >&2
      else still="$still $id"; fi
    done
    left=$(echo $still); [ -n "$left" ] || return 0
    sleep $([ "$t" -le 3 ] && echo 5 || echo 30)
  done
  for id in $left; do echo "[$(date +%T)] ⛔ $id NOT CONFIRMED GONE after ~9 min -- TERMINATE IT BY HAND (it may still be billing)" >&2; done
  return 1; }
ID=""; HOST=""; PORT=""
term () {
  [ -n "$ID" ] || return 0
  log "terminating pod $ID"; rp_term "$ID" 2>>"$OUT/driver.log"; local rc=$?
  # ⛔ The pod is forgotten only once RunPod confirms it gone.
  if [ $rc = 0 ]; then log "pod $ID confirmed gone"; ID=""; else log "⛔ pod $ID NOT CONFIRMED GONE -- TERMINATE IT BY HAND"; fi
  return $rc
}
harvest () {
  [ -n "$HOST" ] || return 0
  log "harvest"
  ssh $K -p "$PORT" root@"$HOST" 'cd /workspace && tar czf harvest.tgz runb join host pod.log setup.log 2>/dev/null; ls -la harvest.tgz' >> "$OUT/driver.log" 2>&1
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
  TRY=$((TRY+1)); NM="lz-c14-bench-$RUNTOK-$TRY"
  Q=$(python3 -c "import json,sys; print(json.dumps({'name': sys.argv[-1], 'cloud': 'SECURE', 'gpu': {'id': sys.argv[2], 'count': 1}, 'image': 'runpod/pytorch:2.4.0-py3.11-cuda12.4.1-devel-ubuntu22.04', 'disk': 120, 'ports': ['22/tcp'], 'env': {'PUBLIC_KEY': sys.argv[1]}}))" "$PUB" "$G" "$NM")
  ID=$(rp_create "$Q" "$NM" 2>>"$OUT/driver.log"); rc=$?
  [ $rc = 2 ] && { log "deploy REFUSED (fatal, see driver.log): nothing deployed"; exit 1; }
  [ $rc = 4 ] && { log "⛔ the create of $NM got no answer and could not be confirmed: A POD NAMED $NM MAY BE BILLING (recorded in maybe_billing.txt)"; echo "$NM" >> maybe_billing.txt; exit 1; }
  [ -n "$ID" ] && { log "DEPLOYED $ID on $G"; echo "$ID" > "$OUT/pod_id"; break; }
done
[ -n "$ID" ] || { log "NO POD: nothing deployed"; exit 1; }

# 3. ssh port (judge by runtime, not desiredStatus)
for i in $(seq 1 60); do
  read HOST PORT < <(rp GET "/pods/$ID" | python3 -c '
import ipaddress,json,sys
p=json.load(sys.stdin); g=lambda ip: bool(ip) and ipaddress.ip_address(ip).is_global
m=[x for x in ((p.get("runtime") or {}).get("ports") or []) if x.get("private")==22 and x.get("public") and g(x.get("ip"))]
if m: print(m[0]["ip"], m[0]["public"])')
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
case " $RUNS " in *" runb "*)
EXPECT_CYCLES=$(cat bundles/runb/EXPECT_CYCLES) BUNDLE=/workspace/bundles/runb OUT=/workspace/runb bash $H/gpu-libzync-multi.sh; echo "RUNB_EXIT=$?";; esac
case " $RUNS " in *" join "*)
SKIP_BUILD=1 BUNDLE=/workspace/bundles/join N=8 OUT=/workspace/join bash $H/gpu-join.sh; echo "JOIN_EXIT=$?";; esac
kill $TOP 2>/dev/null
echo POD_DONE
EOF
ssh $K -p "$PORT" root@"$HOST" "cd /workspace && (RUNS='$RUNS' setsid nohup bash pod.sh > pod.log 2>&1 < /dev/null &)"
log "pod.sh started (runs: $RUNS)"

# 6. wait (POD_DONE or the deadline), relaying progress lines
seen=0
while :; do
  ssh $K -p "$PORT" root@"$HOST" 'cat /workspace/pod.log 2>/dev/null' > "$OUT/pod.log.live" 2>/dev/null
  n=$(wc -l < "$OUT/pod.log.live"); [ "$n" -gt "$seen" ] && { sed -n "$((seen+1)),${n}p" "$OUT/pod.log.live" | tee -a "$OUT/driver.log"; seen=$n; }
  grep -q POD_DONE "$OUT/pod.log.live" && { log "POD_DONE"; break; }
  [ "$(date +%s)" -ge "$DEADLINE" ] && { log "DEADLINE reached: harvesting what exists"; break; }
  sleep 30
done
# the EXIT trap harvests, then terminates; the exit code says whether every run passed
grep -q 'RUNB_EXIT=[1-9]\|JOIN_EXIT=[1-9]\|SETUP FAILED' "$OUT/pod.log.live" && exit 2
grep -q POD_DONE "$OUT/pod.log.live" || exit 3
exit 0
