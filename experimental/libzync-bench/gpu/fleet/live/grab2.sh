#!/bin/bash
# grab2.sh <name>...: like grab.sh, but a pod is KEPT only if it can reach pod A (first line of fleet.txt) on
# A's public ssh port -- RunPod blocks hairpin traffic inside one network (81.27.69.x today), so a pod in
# A's network cannot join. Rejected pods are terminated at once and the deploy is retried (max 10 per name).
cd ~/libzync-gpu/live; RP=$(cat ~/.hazync/runpod.key); PUB=$(cat ~/.ssh/ghost_signet_ed25519.pub)
read -r _ AIP AP _ < fleet.txt
K="-n -i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=15"
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
for n in "$@"; do
  for att in $(seq 1 10); do
    TRY=$((TRY+1)); NM="$n-$RUNTOK-$TRY"
    Q=$(python3 -c "import json,sys; print(json.dumps({'name': sys.argv[2], 'cloud': 'SECURE', 'gpu': {'id': 'NVIDIA RTX PRO 6000 Blackwell Server Edition', 'count': 1}, 'image': 'runpod/pytorch:2.4.0-py3.11-cuda12.4.1-devel-ubuntu22.04', 'disk': 80, 'ports': ['22/tcp', '9110/tcp'], 'env': {'PUBLIC_KEY': sys.argv[1]}}))" "$PUB" "$NM")
    ID=$(rp_create "$Q" "$NM"); rc=$?
    [ $rc = 2 ] && { echo "[$(date +%T)] deploy REFUSED (fatal): stopping"; exit 1; }
    [ $rc = 4 ] && { echo "[$(date +%T)] ⛔ $NM: create answer lost, unconfirmed -- A POD MAY BE BILLING (maybe_billing.txt): stopping"; echo "$NM" >> maybe_billing.txt; exit 1; }
    [ $rc = 3 ] && { sleep 60; continue; }
    [ -n "$ID" ] || { echo "[$(date +%T)] $n: no stock (attempt $att)"; sleep 20; continue; }
    L=""; i=0; e=0
    while [ $i -lt 24 ] && [ $e -lt 30 ]; do
      L=$(./pod.sh get "$ID" | python3 -c 'import ipaddress,json,sys
p=json.load(sys.stdin)
if not p.get("id"): print("ERR", p.get("status", "?")); sys.exit()
g=lambda ip: bool(ip) and ipaddress.ip_address(ip).is_global
ports={}; priv=[]
for x in ((p.get("runtime") or {}).get("ports") or []):
  if x.get("public") and g(x.get("ip")): ports.setdefault(x["private"], (x["ip"], x["public"]))
  elif x.get("ip") and not g(x["ip"]): priv.append(x["ip"])
if 22 in ports and 9110 in ports: print(sys.argv[1], ports[22][0], ports[22][1], ports[9110][0], ports[9110][1], (priv or ["-"])[0])' "$n" 2>/dev/null)
      # ⛔ A 429 / error body is NOT "no ports": it waits and does not count toward the 24 polls.
      case "$L" in
        ERR*) echo "[$(date +%T)] $n: RunPod answered ${L#ERR } for $ID; waiting"; L=""; e=$((e+1)); sleep 25 ;;
        "") i=$((i+1)); sleep 5 ;;
        *) break ;;
      esac
    done
    [ -n "$L" ] || { echo "[$(date +%T)] $n $ID: $([ $e -ge 30 ] && echo 'RunPod kept refusing to answer' || echo 'no ports'), terminating"; echo "$ID $n REJECTED" >> fleet_ids.txt; ./pod.sh term $ID || echo "[$(date +%T)] ⛔ $ID NOT CONFIRMED GONE"; continue; }
    set -- $L; ip=$2; p=$3
    ok=""
    for t in $(seq 1 12); do
      r=$(ssh $K -p $p root@$ip "timeout 5 bash -c '</dev/tcp/$AIP/$AP' && echo REACH || echo NOREACH" 2>/dev/null)
      [ -n "$r" ] && { ok=$r; break; }; sleep 5
    done
    if [ "$ok" = REACH ]; then
      echo "$ID $n" >> fleet_ids.txt; echo "[$(date +%T)] KEEP $n $ID at $ip (reaches A)"
      (setsid nohup ./podup.sh $n $ID > up_$n.log 2>&1 < /dev/null &)
      continue 2
    fi
    echo "[$(date +%T)] REJECT $n $ID at $ip (${ok:-ssh never answered}), terminating"; echo "$ID $n REJECTED" >> fleet_ids.txt; ./pod.sh term $ID || echo "[$(date +%T)] ⛔ $ID NOT CONFIRMED GONE"
  done
  echo "[$(date +%T)] GAVE UP on $n"
done
echo GRAB2_END
