#!/bin/bash
# pod.sh status|get <id>|term <id> — RunPod REST v2 (api.runpod.io/v2).
#   status    {"balance": USD|null, "pods": [every v2 pod object, each with its live `runtime`]}   (read-only)
#             ⛔ exits 1 with RunPod's answer on stderr when the listing is not a listing (429, 5xx, a bad key):
#             an error must never read as "no pods" -- the watchdog would take it for "all pods gone".
#   get <id>  one v2 pod object (GET /v2/pods/<id>), or RunPod's problem+json (e.g. {"status":429,...})  (read-only)
#   term <id> terminate it: DELETE /v2/pods/<id>, CONFIRMED by a 404 (retried ~9 min); exit 1 = NOT confirmed
# ⚠ GET /v2/pods may leave out the live port mapping, so `status` fills `runtime` from GET /v2/pods/<id> for any
#   pod listed without one (1 + N calls on a 180/min quota: poll ONE pod with `get`, not `status`).
# ⚠ v2 has no account balance: `balance` is read from GraphQL's clientBalance, the one call left there
#   (retiring early 2027) -- null when it does not answer; the watchdog then says so and keeps its time limit.
RP=$(cat ~/.hazync/runpod.key)
# RUNPOD_API / RUNPOD_GQL override the endpoints for test_runpod_v2.py's fake server only.
rp(){ curl -s --max-time 20 -X "$1" -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" ${3:+-d "$3"} "${RUNPOD_API:-https://api.runpod.io/v2}$2"; }
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
gqlbal(){ curl -s --max-time 20 -X POST -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" -d "$1" "${RUNPOD_GQL:-https://api.runpod.io/graphql}"; }
case $1 in
 status)
  L=$(rp GET '/pods?limit=1000')
  echo "$L" | python3 -c 'import json,sys; d=json.load(sys.stdin); sys.exit(0 if isinstance(d.get("pods"), list) else 1)' 2>/dev/null \
    || { echo "pod.sh status: RunPod did not list pods: ${L:0:300}" >&2; exit 1; }
  B=$(gqlbal '{"query":"query { myself { clientBalance } }"}' | python3 -c 'import json,sys; print(float(json.load(sys.stdin)["data"]["myself"]["clientBalance"]))' 2>/dev/null || echo null)
  for id in $(echo "$L" | python3 -c 'import json,sys; [print(p["id"]) for p in json.load(sys.stdin)["pods"] if not p.get("runtime")]'); do
    echo "$(rp GET "/pods/$id")"
  done | python3 -c '
import json,sys
lst=json.loads(sys.argv[1]); full={}
for line in sys.stdin:
  try:
    d=json.loads(line)
    if d.get("id"): full[d["id"]]=d
  except Exception: pass
pods=[full.get(p["id"], p) for p in lst["pods"]]
print(json.dumps({"balance": None if sys.argv[2] in ("null", "None", "") else float(sys.argv[2]), "pods": pods}))' "$L" "$B" ;;
 get) rp GET "/pods/$2" ;;
 term) rp_term "$2" ;;
 *) echo "usage: pod.sh status|get <id>|term <id>" >&2; exit 2 ;;
esac
