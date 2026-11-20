#!/bin/bash
# control.sh <stage dir> <work dir>: ⚠ SPENDS MONEY. The GPU confirmation run, driven from the LAPTOP (the RunPod key
# never leaves it): ONE RTX PRO 6000 SECURE pod named hz-measure-*, pod_run.sh on it, harvest as it goes, terminate.
# Caps: no SSH within 15 min -> terminate and try ONE more pod, then stop; pod time MAX_MIN (default 110) or spend
# MAX_USD (default 9.5) -> harvest, terminate. Every harvest checks the tarball's sha256 against the pod's.
# Env: RUNPOD_API_KEY_FILE (the key file), HZ_COORD_DIR (a checkout's coordinator/ with sponsor_bot.py).
HERE=$(cd "$(dirname "$0")" && pwd); ST=$(readlink -f "${1:?stage}"); W=$(readlink -m "${2:?work}"); mkdir -p "$W"; cd "$W" || exit 1
exec >> control.log 2>&1
MAX_MIN=${MAX_MIN:-110}; MAX_USD=${MAX_USD:-9.5}
rp(){ python3 "$HERE/rpm.py" "$@"; }
say(){ echo "[$(date +%T)] $*"; }
KEY=${HZ_POD_SSH_KEY:-$HOME/.ssh/id_ed25519}; PUB=$(cat $KEY.pub)   # an ssh key the pod will accept (HZ_POD_SSH_KEY)
K="-i $KEY -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=20 -o ServerAliveInterval=30"
B0=$(rp balance); say "balance before $B0"; say "pods before: $(rp list | tr '\n' ';')"
ID=""; for attempt in 1 2; do
  ID=$(rp deploy "hz-measure-costcurve-$attempt" "$PUB") || { say "deploy failed"; ID=""; continue; }
  echo "$ID" > pod_id.txt; T_DEPLOY=$SECONDS; say "pod $ID deployed (attempt $attempt)"
  L=""; while [ $((SECONDS - T_DEPLOY)) -lt 900 ]; do L=$(rp ssh "$ID"); [ -n "$L" ] && ssh $K -n -p ${L#* } root@${L% *} true 2>/dev/null && break; L=""; sleep 15; done
  [ -n "$L" ] && break
  say "no SSH in 15 min: terminating $ID"; say "$(rp terminate "$ID")"; ID=""
done
[ -n "$ID" ] || { say "NO_POD: two attempts, none booted"; say "pods after: $(rp list | tr '\n' ';')"; exit 1; }
IP=${L% *}; P=${L#* }; echo "$IP $P" > conn.txt; say "ssh $IP:$P (boot $((SECONDS - T_DEPLOY)) s)"
S(){ ssh $K -n -p $P root@$IP "$@"; }
sha256sum "$ST/../rel/SHA256SUMS.txt" | cut -c1-64 > want_sums.txt
tar czf stage.tgz -C "$ST" . && scp -q $K -P $P stage.tgz want_sums.txt "$HERE/pod_run.sh" root@$IP:/workspace/ && say "inputs sent"
S 'cd /workspace && mkdir -p in && tar xzf stage.tgz -C in && mv in/inputs/leaves in/leaves && mkdir -p in/blocks && mv in/blocks/*.json in/blocks/ 2>/dev/null; cp in/src/cost/txcounts.txt in/txcounts.txt; ls in in/leaves | head -20; (setsid nohup bash pod_run.sh > run.out 2>&1 < /dev/null &)'
harvest(){ for t in 1 2 3 4; do
    S 'cd /workspace && tar czf out.tgz out run.out 2>/dev/null; sha256sum out.tgz | cut -c1-64' > h.sha 2>/dev/null
    timeout 600 scp -q $K -P $P root@$IP:/workspace/out.tgz out.tgz.new && [ -s h.sha ] && [ "$(sha256sum out.tgz.new | cut -c1-64)" = "$(cat h.sha)" ] \
      && mv out.tgz.new out.tgz && rm -rf out && tar xzf out.tgz && { say "harvest ok (try $t): $(ls out | tr '\n' ' ' | cut -c1-200)"; return 0; }
    say "harvest failed (try $t)"; sleep 15; done; return 1; }
fin(){ say "$1"; harvest || harvest; say "terminate: $(rp terminate "$ID")"; rm -f pod_id.txt; B1=$(rp balance); say "balance after $B1 (spent $(python3 -c "print(round($B0-$B1,2))"))"; say "pods after: $(rp list | tr '\n' ';')"; exit 0; }
n=0; while true; do
  sleep 60; n=$((n+1))
  st=$(S 'cat /workspace/out/RUN_DONE 2>/dev/null; cat /workspace/out/phase 2>/dev/null; tail -1 /workspace/out/steps.tsv 2>/dev/null' | tr '\n' '|')
  say "t=$(( (SECONDS - T_DEPLOY)/60 )) min: $st"
  echo "$st" | grep -q '^OK\|^SUMS_BAD\|^SHA_BAD\|^MID_BAD' && fin "run ended: $st"
  [ $((n % 5)) = 0 ] && harvest
  B=$(rp balance 2>/dev/null || echo "$B0")
  python3 -c "import sys; sys.exit(0 if $B0 - $B >= $MAX_USD else 1)" && fin "SPEND CAP (balance $B)"
  [ $(( (SECONDS - T_DEPLOY)/60 )) -ge $MAX_MIN ] && fin "TIME CAP"
done
