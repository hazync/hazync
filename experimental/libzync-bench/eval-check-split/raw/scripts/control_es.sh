#!/bin/bash
# control_es.sh <work dir>: SPENDS MONEY (user approved 23.11.2026: "yes do all that", total pod spend < ~$15).
# Adapted from ../../paper/d8/control_d8.sh: ONE RTX PRO 6000 pod named hz-measure-evalsplit-*, pod_run_es.sh on it,
# harvest as it goes (sha-checked, retried), harvest again before terminating, terminate, confirm gone.
# Listings tried cheapest first (LISTINGS: "<gpu id>|<cloud>" ...); no SSH in 12 min -> terminate, next listing.
# Caps: pod time MAX_MIN or spend MAX_USD -> harvest, terminate. Env: RUNPOD_API_KEY_FILE, HZ_COORD_DIR.
HERE=$(cd "$(dirname "$0")" && pwd); ST=$HERE/stage; W=$(readlink -m "${1:?work}"); mkdir -p "$W"; cd "$W" || exit 1
exec >> control.log 2>&1
MAX_MIN=${MAX_MIN:-210}; MAX_USD=${MAX_USD:-11}
rp(){ python3 "$HERE/rpm_es.py" "$@"; }
say(){ echo "[$(date +%T)] $*"; }
KEY=${HZ_POD_SSH_KEY:-$HOME/.ssh/ghost_signet_ed25519}; PUB=$(cat $KEY.pub)
K="-i $KEY -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=20 -o ServerAliveInterval=30"
B0=$(rp balance); say "balance before $B0"; say "pods before: $(rp list | tr '\n' ';')"
ID=""; n=0
LIST=("NVIDIA RTX PRO 6000 Blackwell Max-Q Workstation Edition|SECURE" "NVIDIA RTX PRO 6000 Blackwell Server Edition|COMMUNITY" "NVIDIA RTX PRO 6000 Blackwell Server Edition|SECURE")
for li in "${LIST[@]}"; do
  n=$((n+1)); gpu=${li%|*}; cloud=${li#*|}
  out=$(rp deploy "hz-measure-evalsplit-$n" "$PUB" "$gpu" "$cloud") || { say "deploy failed: $gpu $cloud"; continue; }
  ID=${out%% *}; COST=${out#* }; echo "$ID" > pod_id.txt; T_DEPLOY=$SECONDS; say "pod $ID deployed: $gpu $cloud \$$COST/h"
  L=""; while [ $((SECONDS - T_DEPLOY)) -lt 720 ]; do L=$(rp ssh "$ID"); [ -n "$L" ] && ssh $K -n -p ${L#* } root@${L% *} true 2>/dev/null && break; L=""; sleep 15; done
  [ -n "$L" ] && break
  say "no SSH in 12 min: terminating $ID"; say "$(rp terminate "$ID")"; ID=""
done
[ -n "$ID" ] || { say "NO_POD: no listing booted"; say "pods after: $(rp list | tr '\n' ';')"; exit 1; }
IP=${L% *}; P=${L#* }; echo "$IP $P" > conn.txt; say "ssh $IP:$P (boot $((SECONDS - T_DEPLOY)) s)"
S(){ ssh $K -n -p $P root@$IP "$@"; }
tar czf stage.tgz -C "$ST" . && scp -q $K -P $P stage.tgz "$HERE/pod_run_es.sh" root@$IP:/workspace/ && say "inputs sent"
S 'cd /workspace && mkdir -p in && tar xzf stage.tgz -C in && ls in/leaves && (setsid nohup bash pod_run_es.sh > run.out 2>&1 < /dev/null &)'
harvest(){ for t in 1 2 3 4 5 6; do
    S 'cd /workspace && find out run.out -type f -size -300M ! -name "*.sqlite" 2>/dev/null > hlist && tar czf out.tgz -T hlist && sha256sum out.tgz | cut -c1-64' > h.sha 2>/dev/null
    timeout 1200 scp -q $K -P $P root@$IP:/workspace/out.tgz out.tgz.new && [ -s h.sha ] && [ "$(sha256sum out.tgz.new | cut -c1-64)" = "$(cat h.sha)" ] \
      && mv out.tgz.new out.tgz && rm -rf out && tar xzf out.tgz && { say "harvest ok (try $t): $(du -sh out | cut -f1), $(wc -l < out/steps.tsv 2>/dev/null) steps"; return 0; }
    say "harvest failed (try $t)"; sleep $((15 * t)); done; return 1; }
fin(){ say "$1"; harvest || { sleep 60; harvest; } || say "HARVEST FAILED TWICE"
  cp -a out "out.final.$(date +%H%M%S)" 2>/dev/null
  say "terminate: $(rp terminate "$ID")"; rm -f pod_id.txt; sleep 20; B1=$(rp balance); say "balance after $B1 (spent $(python3 -c "print(round($B0-$B1,2))"))"; say "pods after: $(rp list | tr '\n' ';')"; exit 0; }
n=0; while true; do
  sleep 60; n=$((n+1))
  st=$(S 'cat /workspace/out/RUN_DONE 2>/dev/null; cat /workspace/out/phase 2>/dev/null; tail -1 /workspace/out/steps.tsv 2>/dev/null' | tr '\n' '|')
  say "t=$(( (SECONDS - T_DEPLOY)/60 )) min: $st"
  echo "$st" | grep -q '^OK\|^SUMS_BAD\|^SHA_BAD\|^MID_BAD\|^ELF_BAD' && fin "run ended: $st"
  [ $((n % 10)) = 0 ] && harvest
  B=$(rp balance 2>/dev/null || echo "$B0")
  python3 -c "import sys; sys.exit(0 if $B0 - $B >= $MAX_USD else 1)" && fin "SPEND CAP (balance $B)"
  [ $(( (SECONDS - T_DEPLOY)/60 )) -ge $MAX_MIN ] && fin "TIME CAP"
done
