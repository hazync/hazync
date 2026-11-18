#!/bin/bash
# grab2.sh <name>...: like grab.sh, but a pod is KEPT only if it can reach pod A (first line of fleet.txt) on
# A's public ssh port -- RunPod blocks hairpin traffic inside one network (81.27.69.x today), so a pod in
# A's network cannot join. Rejected pods are terminated at once and the deploy is retried (max 10 per name).
cd ~/libzync-gpu/live; RP=$(cat ~/.hazync/runpod.key); PUB=$(cat ~/.ssh/ghost_signet_ed25519.pub)
read -r _ AIP AP _ < fleet.txt
K="-n -i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=15"
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
for n in "$@"; do
  for att in $(seq 1 10); do
    Q=$(python3 -c "import json,sys; print(json.dumps({'name': sys.argv[2], 'cloud': 'SECURE', 'gpu': {'id': 'NVIDIA RTX PRO 6000 Blackwell Server Edition', 'count': 1}, 'image': 'runpod/pytorch:2.4.0-py3.11-cuda12.4.1-devel-ubuntu22.04', 'disk': 80, 'ports': ['22/tcp', '9110/tcp'], 'env': {'PUBLIC_KEY': sys.argv[1]}}))" "$PUB" "$n")
    ID=$(rp_create "$Q" "$n"); rc=$?
    [ $rc = 2 ] && { echo "[$(date +%T)] deploy REFUSED (fatal): stopping"; exit 1; }
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
