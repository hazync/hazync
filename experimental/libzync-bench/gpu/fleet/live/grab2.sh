#!/bin/bash
# grab2.sh <name>...: like grab.sh, but a pod is KEPT only if it can reach pod A (first line of fleet.txt) on
# A's public ssh port -- RunPod blocks hairpin traffic inside one network (81.27.69.x today), so a pod in
# A's network cannot join. Rejected pods are terminated at once and the deploy is retried (max 10 per name).
cd ~/libzync-gpu/live; RP=$(cat ~/.hazync/runpod.key); PUB=$(cat ~/.ssh/ghost_signet_ed25519.pub)
read -r _ AIP AP _ < fleet.txt
K="-n -i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=15"
rp(){ curl -s --max-time 20 -X "$1" -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" ${3:+-d "$3"} "https://api.runpod.io/v2$2"; }
for n in "$@"; do
  for att in $(seq 1 10); do
    Q=$(python3 -c "import json,sys; print(json.dumps({'name': sys.argv[2], 'cloud': 'SECURE', 'gpu': {'id': 'NVIDIA RTX PRO 6000 Blackwell Server Edition', 'count': 1}, 'image': 'runpod/pytorch:2.4.0-py3.11-cuda12.4.1-devel-ubuntu22.04', 'disk': 80, 'ports': ['22/tcp', '9110/tcp'], 'env': {'PUBLIC_KEY': sys.argv[1]}}))" "$PUB" "$n")
    ID=$(rp POST /pods "$Q" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("id") or "")' 2>/dev/null)
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
    [ -n "$L" ] || { echo "[$(date +%T)] $n $ID: $([ $e -ge 30 ] && echo 'RunPod kept refusing to answer' || echo 'no ports'), terminating"; ./pod.sh term $ID > /dev/null; continue; }
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
    echo "[$(date +%T)] REJECT $n $ID at $ip (${ok:-ssh never answered}), terminating"; ./pod.sh term $ID > /dev/null
  done
  echo "[$(date +%T)] GAVE UP on $n"
done
echo GRAB2_END
