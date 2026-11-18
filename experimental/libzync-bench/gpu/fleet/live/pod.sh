#!/bin/bash
# pod.sh status|get <id>|term <id> — RunPod REST v2 (api.runpod.io/v2).
#   status    {"balance": USD|null, "pods": [every v2 pod object, each with its live `runtime`]}   (read-only)
#   get <id>  one v2 pod object (GET /v2/pods/<id>)                                                (read-only)
#   term <id> terminate it (DELETE /v2/pods/<id>)
# ⚠ GET /v2/pods may leave out the live port mapping, so `status` fills `runtime` from GET /v2/pods/<id> for any
#   pod listed without one. ⚠ v2 has no account balance: `balance` is read from GraphQL's clientBalance, the one
#   call left there (retiring early 2027) -- null when it does not answer, and the watchdog then reads 99.
RP=$(cat ~/.hazync/runpod.key)
rp(){ curl -s -X "$1" -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" ${3:+-d "$3"} "https://api.runpod.io/v2$2"; }
gqlbal(){ curl -s -X POST -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" -d "$1" https://api.runpod.io/graphql; }
case $1 in
 status)
  B=$(gqlbal '{"query":"query { myself { clientBalance } }"}' | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["myself"]["clientBalance"])' 2>/dev/null || echo null)
  L=$(rp GET '/pods?limit=1000')
  for id in $(echo "$L" | python3 -c 'import json,sys; [print(p["id"]) for p in json.load(sys.stdin)["pods"] if not p.get("runtime")]' 2>/dev/null); do
    echo "$(rp GET "/pods/$id")"
  done | python3 -c '
import json,sys
lst=json.loads(sys.argv[1]); full={}
for line in sys.stdin:
  try: d=json.loads(line); full[d["id"]]=d
  except Exception: pass
pods=[full.get(p["id"], p) for p in lst.get("pods") or []]
print(json.dumps({"balance": None if sys.argv[2] in ("null", "None", "") else float(sys.argv[2]), "pods": pods}))' "$L" "$B" ;;
 get) rp GET "/pods/$2" ;;
 term) rp DELETE "/pods/$2" ;;
esac
