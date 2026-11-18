#!/bin/bash
# pod.sh status|get <id>|term <id> — RunPod REST v2 (api.runpod.io/v2).
#   status    {"balance": USD|null, "pods": [every v2 pod object, each with its live `runtime`]}   (read-only)
#             ⛔ exits 1 with RunPod's answer on stderr when the listing is not a listing (429, 5xx, a bad key):
#             an error must never read as "no pods" -- the watchdog would take it for "all pods gone".
#   ours      the ids of THIS run's pods (fleet_ids.txt + names in maybe_billing.txt)
#   get <id>  one v2 pod object (GET /v2/pods/<id>), or RunPod's problem+json (e.g. {"status":429,...})  (read-only)
#   term <id>...  terminate them: every DELETE first, then each CONFIRMED by a 404 (retried ~9 min); exit 1 = NOT confirmed
# ⚠ GET /v2/pods may leave out the live port mapping, so `status` fills `runtime` from GET /v2/pods/<id> for any
#   pod listed without one (1 + N calls on a 180/min quota: poll ONE pod with `get`, not `status`).
# ⚠ v2 has no account balance: `balance` is read from GraphQL's clientBalance, the one call left there
#   (retiring early 2027) -- null when it does not answer; the watchdog then says so and keeps its time limit.
RP=$(cat ~/.hazync/runpod.key)
# RUNPOD_API / RUNPOD_GQL override the endpoints for test_runpod_v2.py's fake server only.
rp(){ curl -s --max-time 20 -X "$1" -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" ${3:+-d "$3"} "${RUNPOD_API:-https://api.runpod.io/v2}$2"; }
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
 ours)  # this run's pods: every id in fleet_ids.txt (column 1), plus any pod named in maybe_billing.txt (a create
        # whose answer was lost). Exit 1 when such a name cannot be looked up: those pods MAY BE BILLING.
  awk '$1 ~ /^[a-z0-9]+$/ {print $1}' fleet_ids.txt 2>/dev/null
  if [ -s maybe_billing.txt ]; then
    rp GET '/pods?limit=1000' | python3 -c 'import json,sys
names={l.strip() for l in open("maybe_billing.txt") if l.strip()}
for p in json.load(sys.stdin)["pods"]:
    if p.get("name") in names and p.get("status")!="TERMINATED": print(p["id"])' \
      || { echo "pod.sh ours: could not look up $(tr '\n' ' ' < maybe_billing.txt)-- they MAY BE BILLING" >&2; exit 1; }
  fi ;;
 term) shift; rp_term "$@" ;;
 *) echo "usage: pod.sh status|ours|get <id>|term <id>..." >&2; exit 2 ;;
esac
