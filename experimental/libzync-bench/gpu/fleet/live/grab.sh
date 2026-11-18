#!/bin/bash
# Retry deploying PRO 6000 pods for the names given, every 30 s for 12 min; podup each one that lands.
cd ~/libzync-gpu/live; RP=$(cat ~/.hazync/runpod.key); PUB=$(cat ~/.ssh/ghost_signet_ed25519.pub)
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
want=("$@"); end=$((SECONDS+720))
while [ ${#want[@]} -gt 0 ] && [ $SECONDS -lt $end ]; do
  left=()
  for n in "${want[@]}"; do
    got=""
    for G in "NVIDIA RTX PRO 6000 Blackwell Server Edition" "NVIDIA RTX PRO 6000 Blackwell Workstation Edition"; do
      Q=$(python3 -c "import json,sys; print(json.dumps({'name': sys.argv[2], 'cloud': 'SECURE', 'gpu': {'id': sys.argv[3], 'count': 1}, 'image': 'runpod/pytorch:2.4.0-py3.11-cuda12.4.1-devel-ubuntu22.04', 'disk': 80, 'ports': ['22/tcp', '9110/tcp'], 'env': {'PUBLIC_KEY': sys.argv[1]}}))" "$PUB" "$n" "$G")
      ID=$(rp_create "$Q" "$n"); rc=$?
      [ $rc = 2 ] && { echo "[$(date +%T)] deploy REFUSED (fatal): stopping"; exit 1; }
      [ $rc = 3 ] && { sleep 60; continue; }
      [ -n "$ID" ] && { got=$ID; echo "$ID $n" >> fleet_ids.txt; echo "[$(date +%T)] GOT $n $ID on $G"; (setsid nohup ./podup.sh $n $ID > up_$n.log 2>&1 < /dev/null &); break; }
    done
    [ -z "$got" ] && left+=("$n")
  done
  want=("${left[@]}"); [ ${#want[@]} -gt 0 ] && sleep 30
done
echo "[$(date +%T)] GRAB_END still wanted: ${want[*]:-none}"
