#!/bin/bash
# podup.sh <name> [<pod id>]: wait for the pod's ports, record them in fleet.txt, push files, start prep, upload bundle.
cd ~/libzync-gpu/run; N=$1; ID=${2:-}
# ⛔ ONE pod per poll (`pod.sh get <id>`, 1 call), never `pod.sh status` (1 + N calls each, for every podup at
# once: N(N+1) on a 180/min quota). A 429 or any non-pod answer is NOT "no ports": it waits longer and does not
# count toward the 40 polls.
[ -n "$ID" ] || ID=$(./pod.sh status | python3 -c 'import json,sys; [print(p["id"]) for p in json.load(sys.stdin)["pods"] if p["name"]==sys.argv[1]]' "$N" | head -1)
[ -n "$ID" ] || { echo "$N: no such pod"; exit 1; }
i=0; e=0; L=""
while [ $i -lt 40 ] && [ $e -lt 30 ]; do
  L=$(./pod.sh get "$ID" | python3 -c 'import ipaddress,json,sys
p=json.load(sys.stdin)
if not p.get("id"): print("ERR", p.get("status", "?")); sys.exit()
g=lambda ip: bool(ip) and ipaddress.ip_address(ip).is_global
ports={}; priv=[]
for x in ((p.get("runtime") or {}).get("ports") or []):
  if x.get("public") and g(x.get("ip")): ports.setdefault(x["private"], (x["ip"], x["public"]))
  elif x.get("ip") and not g(x["ip"]): priv.append(x["ip"])
if 22 in ports and 9110 in ports: print(sys.argv[1], ports[22][0], ports[22][1], ports[9110][0], ports[9110][1], (priv or ["-"])[0])' "$N" 2>/dev/null)
  case "$L" in
    "ERR 404") echo "$N: RunPod no longer has pod $ID"; exit 1 ;;
    ERR*) echo "[$(date +%T)] $N: RunPod answered ${L#ERR } for $ID; waiting"; L=""; e=$((e+1)); sleep 30 ;;
    "") i=$((i+1)); sleep 10 ;;
    *) break ;;
  esac
done
[ -n "$L" ] || { [ $e -ge 30 ] && echo "$N: RunPod kept refusing to answer for $ID (state UNKNOWN, not 'no ports')" || echo "$N NO PORTS"; exit 1; }
grep -v "^$N " fleet.txt 2>/dev/null > fleet.tmp; echo "$L" >> fleet.tmp; mv fleet.tmp fleet.txt
set -- $L; K="-i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=15"
for t in $(seq 1 12); do ssh $K -p $3 root@$2 true && break; sleep 10; done
scp -q $K -P $3 setup.sh prep.sh lzenv.sh root@$2:/workspace/ || { echo "$N scp failed"; exit 1; }
ssh $K -p $3 root@$2 'cd /workspace && (setsid nohup bash prep.sh > prep.out 2>&1 < /dev/null &)'
scp -q $K -P $3 ~/libzync-gpu/libzync-bundle.tgz ~/libzync-gpu/libzync-bundle.tgz.sha256 root@$2:/workspace/ && echo "$N up: $L, bundle sent"
