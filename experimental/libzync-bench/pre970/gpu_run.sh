#!/bin/bash
# Signet GPU proof, unattended: one RTX PRO 6000 -> provision -> build -> prove 2501..2600 recursively ->
# verify -> fetch -> TERMINATE. A separate deadline kills the pod at 150 min or a balance under $3.
cd ~/libzync-gpu/pre970; LOG=gpu_run.log; exec >> $LOG 2>&1
RP=$(cat ~/.hazync/runpod.key); PUB=$(cat ~/.ssh/ghost_signet_ed25519.pub)
K="-n -i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=20"
KS="-i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=20"
rp(){ curl -s --max-time 20 -X "$1" -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" ${3:+-d "$3"} "https://api.runpod.io/v2$2"; }
# rp_create <body> <name>: POST /v2/pods; prints the new pod's id. Exit 0 = a pod; 1 = no capacity (400) or no
# pod found; 2 = FATAL for every type (401 key, 402 balance, 422 request, a Cloudflare 403); 3 = rate-limited (429,
# wait and retry). RunPod's refusal goes to stderr. ⛔ With NO answer or a 5xx the pod may exist anyway: it is looked
# up by NAME before anyone retries or tries another type -- otherwise a lost answer rents two pods.
rp_create(){ local R ID
  R=$(rp POST /pods "$1")
  ID=$(printf '%s' "$R" | python3 -c 'import json,sys
try: d=json.load(sys.stdin)
except Exception: print("?"); sys.exit()
s=d.get("status") if isinstance(d, dict) else None
if isinstance(d, dict) and d.get("id") and not isinstance(s, int): print(d["id"])
elif isinstance(s, int) and s < 500: print("E%d" % s)
else: print("?")' 2>/dev/null)
  case "$ID" in
    "?"|"") ID=$(rp GET '/pods?limit=1000' | python3 -c 'import json,sys; print(next((p["id"] for p in json.load(sys.stdin)["pods"] if p.get("name")==sys.argv[1]), ""))' "$2" 2>/dev/null)
            if [ -n "$ID" ]; then echo "create answer lost (${R:0:120}); found $2 by name: $ID" >&2; printf '%s\n' "$ID"; return 0; fi
            echo "create: no answer and no pod named $2 (${R:0:200})" >&2; return 1 ;;
    E400) echo "create: no capacity: ${R:0:200}" >&2; return 1 ;;
    E401|E402|E422) echo "create REFUSED (fatal): ${R:0:300}" >&2; return 2 ;;
    E403) case "$R" in *'"status"'*) echo "create: no access to this type: ${R:0:200}" >&2; return 1 ;;
                       *) echo "create REFUSED (fatal): ${R:0:300}" >&2; return 2 ;; esac ;;
    E429) echo "create: rate-limited: ${R:0:200}" >&2; return 3 ;;
    E*) echo "create refused: ${R:0:300}" >&2; return 1 ;;
  esac
  printf '%s\n' "$ID"; }
# rp_term <id>: DELETE, then confirm (GET answers 404, or the pod reads TERMINATED); retried with growing waits for
# ~10 min, a 429 included. ⛔ Loud when it cannot confirm: a pod that is not confirmed gone keeps billing.
rp_term(){ local t
  for t in $(seq 1 20); do
    rp DELETE "/pods/$1" > /dev/null
    if rp GET "/pods/$1" | python3 -c 'import json,sys; d=json.load(sys.stdin); sys.exit(0 if d.get("status") in (404, "TERMINATED") else 1)' 2>/dev/null; then
      echo "[$(date +%T)] terminated $1 (confirmed gone)" >&2; return 0; fi
    sleep $([ "$t" -le 3 ] && echo 5 || echo 30)
  done
  echo "[$(date +%T)] ⛔ $1 NOT CONFIRMED GONE after ~9 min -- TERMINATE IT BY HAND (it may still be billing)" >&2
  return 1; }
# ⚠ RunPod REST v2 has no account balance; clientBalance is the ONE call left on GraphQL (retiring early 2027).
# Once it is gone the balance is UNREADABLE: every check says so, and only the time limit guards the spend.
gqlbal(){ curl -s --max-time 20 -X POST -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" -d "$1" https://api.runpod.io/graphql; }
say(){ echo "[$(date +%T)] $*"; }
GPU="${GPU:-NVIDIA RTX PRO 6000 Blackwell Server Edition}"
Q=$(python3 -c "import json,sys; print(json.dumps({'name': 'lz-pre970', 'cloud': 'SECURE', 'gpu': {'id': sys.argv[2], 'count': 1}, 'image': 'runpod/pytorch:2.4.0-py3.11-cuda12.4.1-devel-ubuntu22.04', 'disk': 80, 'ports': ['22/tcp'], 'env': {'PUBLIC_KEY': sys.argv[1]}}))" "$PUB" "$GPU")
ID=$(rp_create "$Q" lz-pre970)
[ -n "$ID" ] || { say "deploy failed (RunPod's answer above)"; exit 1; }
echo $ID > pod_id.txt; say "pod $ID deployed ($GPU)"
term(){ say "terminating $ID"; rp_term "$ID"; }
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
