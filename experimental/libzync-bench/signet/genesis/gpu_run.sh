#!/bin/bash
# gpu_run.sh [HI]: signet 1..HI from genesis on one RTX PRO 6000, on the published image: provision, build, prove,
# verify, fetch, TERMINATE. The newest receipt is copied to the laptop every 10 minutes (ckpt/); a rerun sends it to
# the new pod and the prover resumes after it, so a dead pod loses at most those 10 minutes plus bring-up.
# A watcher harvests and terminates at the deadline, or when the balance falls under $1.
# Spends money. Env: DEADLINE_MIN (default 570, 9.5 h).
HI=${1:-2600}; DEADLINE_MIN=${DEADLINE_MIN:-570}; HERE=$(cd "$(dirname "$0")" && pwd)
cd ~/libzync-gpu/signet-genesis; cp -r "$HERE/pod" .; LOG=gpu_run.log; exec >> $LOG 2>&1
# Checkpoints are per image: a receipt of another image (or design B) can never resume this run (review 11-03).
M=$(tar xzOf sb/sb.tgz ./pins/METHOD_ID 2>/dev/null | grep -v '^#' | head -1); [ -n "$M" ] || { echo "no METHOD_ID pin in sb/sb.tgz"; exit 1; }
CK=ckpt/$M; mkdir -p $CK
# A coin-log bundle resumes only from a receipt that has its log: pick the highest height with both.
CL=$(tar xzOf sb/sb.tgz ./COINLOG 2>/dev/null || echo 0)
TR=$(tar xzOf sb/sb.tgz ./TRIE 2>/dev/null || echo 0)   # a product bundle: the trie snapshot is part of a checkpoint
[ -f sb/sb.tgz ] || { echo "run mkbundle.sh first"; exit 1; }
RP=$(cat ~/.hazync/runpod.key); PUB=$(cat ~/.ssh/ghost_signet_ed25519.pub)
K="-n -i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=20"
KS="-i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=20"
rp(){ curl -s --max-time 20 -X "$1" -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" ${3:+-d "$3"} "https://api.runpod.io/v2$2"; }
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
# ⚠ RunPod REST v2 has no account balance; clientBalance is the ONE call left on GraphQL (retiring early 2027).
# Once it is gone the balance is UNREADABLE: every check says so, and only the time limit guards the spend.
gqlbal(){ curl -s --max-time 20 -X POST -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" -d "$1" https://api.runpod.io/graphql; }
say(){ echo "[$(date +%T)] $*"; }
GPU="${GPU:-NVIDIA RTX PRO 6000 Blackwell Server Edition}"
TRY=$((TRY+1)); NM="lz-signet-$RUNTOK-$TRY"
Q=$(python3 -c "import json,sys; print(json.dumps({'name': sys.argv[-1], 'cloud': 'SECURE', 'gpu': {'id': sys.argv[2], 'count': 1}, 'image': 'runpod/pytorch:2.4.0-py3.11-cuda12.4.1-devel-ubuntu22.04', 'disk': 80, 'ports': ['22/tcp'], 'env': {'PUBLIC_KEY': sys.argv[1]}}))" "$PUB" "$GPU" "$NM")
ID=$(rp_create "$Q" "$NM"); rc=$?
[ $rc = 4 ] && { say "⛔ the create of $NM got no answer and could not be confirmed: A POD NAMED $NM MAY BE BILLING (recorded in maybe_billing.txt)"; echo "$NM" >> maybe_billing.txt; exit 1; }
[ -n "$ID" ] || { say "deploy failed (RunPod's answer above)"; exit 1; }
echo $ID > pod_id.txt; touch run.alive; say "pod $ID deployed ($GPU)"
term(){ say "terminating $ID"; rp_term "$ID" && rm -f pod_id.txt; }   # the record goes only when RunPod confirms
# Fetch whatever the pod has (conn.txt = "ip port", written once ssh works). Bounded, so a dead pod cannot hang it.
# Exactly once: the deadline watcher and the main script can both get here; mkdir is atomic (review 5).
harvest(){ mkdir harvest.lock 2>/dev/null || { say "harvest already running"; return 1; }
  [ -f conn.txt ] || return 0; set -- $(cat conn.txt); say "harvesting from $1:$2"
  timeout 120 ssh $K -p $2 root@$1 'cd /workspace && N=$(ls out | sed -n "s/^block_\([0-9]*\)\.rcpt$/\1/p" | sort -n | tail -1); tar czf out.tgz prep.out build.log run.out $(ls -d out/* | grep -Ev "/(bundle_|block_[0-9]+\.rcpt)") "out/block_$N.rcpt" 2>/dev/null'
  timeout 300 scp -q $KS -P $2 root@$1:/workspace/out.tgz out.tgz && rm -rf out2 && mkdir out2 && tar xzf out.tgz -C out2 && say "fetched: $(ls out2/out | wc -l) files" || say "FETCH FAILED"; return 0; }
rm -f conn.txt; rmdir harvest.lock 2>/dev/null
( END=$((SECONDS+DEADLINE_MIN*60)); while [ -f run.alive ]; do
    B=$(gqlbal '{"query":"query { myself { clientBalance } }"}' | python3 -c 'import json,sys; print(float(json.load(sys.stdin)["data"]["myself"]["clientBalance"]))' 2>/dev/null || echo UNREADABLE)
    [ "$B" = UNREADABLE ] && say "⚠ balance UNREADABLE (GraphQL clientBalance did not answer) -- only the time limit guards the spend"
    if [ $SECONDS -ge $END ] || { [ "$B" != UNREADABLE ] && python3 -c "import sys; sys.exit(0 if float('$B') < 1 else 1)"; }; then say "DEADLINE (balance $B)"; rm -f run.alive; harvest && term; exit; fi
    sleep 60; done ) &
for i in $(seq 1 60); do
  L=$(rp GET "/pods/$ID" | python3 -c '
import ipaddress,json,sys
p=json.load(sys.stdin); g=lambda ip: bool(ip) and ipaddress.ip_address(ip).is_global
m=[x for x in ((p.get("runtime") or {}).get("ports") or []) if x.get("private")==22 and x.get("public") and g(x.get("ip"))]
if m: print(m[0]["ip"], m[0]["public"])')
  [ -n "$L" ] && break; sleep 10; done
[ -n "$L" ] || { say "no ports"; term; rm -f run.alive; exit 1; }
set -- $L; IP=$1; P=$2; say "ssh $IP:$P"; echo "$IP $P" > conn.txt
for t in $(seq 1 30); do ssh $K -p $P root@$IP true && break; sleep 10; done
scp -q $KS -P $P pod/setup.sh pod/prep.sh pod/run.sh root@$IP:/workspace/ && say "scripts sent"
ssh $K -p $P root@$IP 'cd /workspace && (setsid nohup bash prep.sh > prep.out 2>&1 < /dev/null &)'
scp -q $KS -P $P sb/sb.tgz sb/sb.tgz.sha256 root@$IP:/workspace/ && say "bundle sent"
ssh $K -p $P root@$IP 'mkdir -p /workspace/out'
C=$(for n in $(ls $CK 2>/dev/null | sed -n 's/^block_\([0-9]*\)\.rcpt$/\1/p' | sort -n); do
      { [ "$CL" != 1 ] || [ -f $CK/state_$n.coinlog ]; } && { [ "$TR" != 1 ] || [ -f $CK/state_$n.trie ]; } && echo $n; done | tail -1)
# A coin-log run resumes only with the log file of the same height (state_<C>.coinlog), and a product run also with
# the trie snapshot (state_<C>.trie), each sent with its receipt.
[ -z "$C" ] || { scp -q $KS -P $P $CK/block_$C.rcpt $(ls $CK/state_$C.coinlog $CK/state_$C.trie 2>/dev/null) root@$IP:/workspace/out/ && say "checkpoint $C sent"; }
for i in $(seq 1 120); do
  [ -f run.alive ] || exit 1   # the deadline fired (and harvested)
  S=$(ssh $K -p $P root@$IP 'grep -h "PREP_OK\|PREP_FAIL\|built in" /workspace/prep.out 2>/dev/null')
  echo "$S" | grep -q 'PREP_OK\|PREP_FAIL' && break; sleep 20; done
say "prep: $(echo $S | tr '\n' ' ')"
echo "$S" | grep -q PREP_OK || { ssh $K -p $P root@$IP 'tail -n 40 /workspace/build.log; tail -n 40 /workspace/setup.log; tail -n 20 /workspace/prep.out' > prep_fail.log 2>&1; term; rm -f run.alive; exit 1; }
ssh $K -p $P root@$IP 'cd /workspace && (setsid nohup bash run.sh $HI > run.out 2>&1 < /dev/null &)'
say "proving 1..$HI"
for i in $(seq 1 $((DEADLINE_MIN*4))); do
  [ -f run.alive ] || exit 1   # the deadline fired (and harvested)
  ssh $K -p $P root@$IP 'grep -q RUN_DONE /workspace/out/prove.log 2>/dev/null' && break
  ssh $K -p $P root@$IP true && F=0 || F=$((${F:-0}+1))
  [ "${F:-0}" -ge 40 ] && { say "POD_LOST: unreachable for 10 min; rerun to resume from $CK"; rm -f run.alive; term; exit 1; }
  [ $((i % 10)) = 0 ] && say "  $(ssh $K -p $P root@$IP 'tail -1 /workspace/out/times.tsv 2>/dev/null')"
  if [ $((i % 40)) = 0 ]; then   # write under a temp name: a cut-off copy must never look like a receipt
    N=$(ssh $K -p $P root@$IP 'ls /workspace/out' | sed -n 's/^block_\([0-9]*\)\.rcpt$/\1/p' | sort -n | tail -2 | head -1)
    # Coin log and trie first, then the receipt: a receipt in $CK is a checkpoint only with its log and trie (the run
    # keeps the last two of each, so N's may already be gone; then this round takes nothing and the next one tries again).
    if [ -n "$N" ] && { [ "$CL" != 1 ] || { timeout 300 scp -q $KS -P $P root@$IP:/workspace/out/state_$N.coinlog $CK/.tmpl \
          && mv $CK/.tmpl $CK/state_$N.coinlog; }; } \
                   && { [ "$TR" != 1 ] || { timeout 300 scp -q $KS -P $P root@$IP:/workspace/out/state_$N.trie $CK/.tmpt \
          && mv $CK/.tmpt $CK/state_$N.trie; }; }; then
      timeout 120 scp -q $KS -P $P root@$IP:/workspace/out/block_$N.rcpt $CK/.tmp && mv $CK/.tmp $CK/block_$N.rcpt && say "  checkpoint $N"
    fi
  fi
  sleep 15; done
rm -f run.alive   # stop the watcher; if it already started, the lock makes it the one that harvests
harvest && term; sleep 5
say "GPU_RUN_DONE"
