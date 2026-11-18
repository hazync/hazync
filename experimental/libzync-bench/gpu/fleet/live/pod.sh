#!/bin/bash
# pod.sh status|get <id>|term <id> — RunPod REST v2 (api.runpod.io/v2).
#   status    {"balance": USD|null, "pods": [every v2 pod object, each with its live `runtime`]}   (read-only)
#             ⛔ exits 1 with RunPod's answer on stderr when the listing is not a listing (429, 5xx, a bad key):
#             an error must never read as "no pods" -- the watchdog would take it for "all pods gone".
#   get <id>  one v2 pod object (GET /v2/pods/<id>), or RunPod's problem+json (e.g. {"status":429,...})  (read-only)
#   term <id> terminate it (DELETE /v2/pods/<id>)
# ⚠ GET /v2/pods may leave out the live port mapping, so `status` fills `runtime` from GET /v2/pods/<id> for any
#   pod listed without one (1 + N calls on a 180/min quota: poll ONE pod with `get`, not `status`).
# ⚠ v2 has no account balance: `balance` is read from GraphQL's clientBalance, the one call left there
#   (retiring early 2027) -- null when it does not answer; the watchdog then says so and keeps its time limit.
RP=$(cat ~/.hazync/runpod.key)
# RUNPOD_API / RUNPOD_GQL override the endpoints for test_runpod_v2.py's fake server only.
rp(){ curl -s --max-time 20 -X "$1" -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" ${3:+-d "$3"} "${RUNPOD_API:-https://api.runpod.io/v2}$2"; }
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
 term) rp DELETE "/pods/$2" ;;
 *) echo "usage: pod.sh status|get <id>|term <id>" >&2; exit 2 ;;
esac
