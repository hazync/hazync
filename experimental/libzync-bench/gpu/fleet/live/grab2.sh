#!/bin/bash
# grab2.sh <name>...: like grab.sh, but a pod is KEPT only if it can reach pod A (first line of fleet.txt) on
# A's public ssh port -- RunPod blocks hairpin traffic inside one network (81.27.69.x today), so a pod in
# A's network cannot join. Rejected pods are terminated at once and the deploy is retried (max 10 per name).
cd ~/libzync-gpu/live; RP=$(cat ~/.hazync/runpod.key); PUB=$(cat ~/.ssh/ghost_signet_ed25519.pub)
read -r _ AIP AP _ < fleet.txt
K="-n -i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=15"
gql(){ curl -s -X POST -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" -d "$1" https://api.runpod.io/graphql; }
for n in "$@"; do
  for att in $(seq 1 10); do
    Q=$(python3 -c "import json,sys; print(json.dumps({'query': 'mutation { podFindAndDeployOnDemand(input: { cloudType: SECURE, gpuCount: 1, volumeInGb: 0, containerDiskInGb: 80, gpuTypeId: \"NVIDIA RTX PRO 6000 Blackwell Server Edition\", name: \"%s\", imageName: \"runpod/pytorch:2.4.0-py3.11-cuda12.4.1-devel-ubuntu22.04\", ports: \"22/tcp,9110/tcp\", env: [{key: \"PUBLIC_KEY\", value: \"%s\"}] }) { id } }' % (sys.argv[2], sys.argv[1])}))" "$PUB" "$n")
    ID=$(gql "$Q" | python3 -c 'import json,sys; p=(json.load(sys.stdin).get("data") or {}).get("podFindAndDeployOnDemand"); print(p["id"] if p else "")' 2>/dev/null)
    [ -n "$ID" ] || { echo "[$(date +%T)] $n: no stock (attempt $att)"; sleep 20; continue; }
    L=""
    for i in $(seq 1 24); do
      L=$(./pod.sh status | python3 -c '
import json,sys
for p in json.load(sys.stdin)["data"]["myself"]["pods"]:
  if p["id"]!=sys.argv[1]: continue
  r=p.get("runtime") or {}; ports={x["privatePort"]:(x["ip"],x["publicPort"]) for x in (r.get("ports") or []) if x["isIpPublic"]}
  priv=[x["ip"] for x in (r.get("ports") or []) if not x["isIpPublic"]]
  if 22 in ports and 9110 in ports: print(sys.argv[2], ports[22][0], ports[22][1], ports[9110][0], ports[9110][1], (priv or ["-"])[0])' "$ID" "$n")
      [ -n "$L" ] && break; sleep 5
    done
    [ -n "$L" ] || { echo "[$(date +%T)] $n $ID: no ports, terminating"; ./pod.sh term $ID > /dev/null; continue; }
    set -- $L; ip=$2; p=$3
    ok=""
    for t in $(seq 1 12); do
      r=$(ssh $K -p $p root@$ip "timeout 5 bash -c '</dev/tcp/$AIP/$AP' && echo REACH || echo NOREACH" 2>/dev/null)
      [ -n "$r" ] && { ok=$r; break; }; sleep 5
    done
    if [ "$ok" = REACH ]; then
      echo "$ID $n" >> fleet_ids.txt; echo "[$(date +%T)] KEEP $n $ID at $ip (reaches A)"
      (setsid nohup ./podup.sh $n > up_$n.log 2>&1 < /dev/null &)
      continue 2
    fi
    echo "[$(date +%T)] REJECT $n $ID at $ip (${ok:-ssh never answered}), terminating"; ./pod.sh term $ID > /dev/null
  done
  echo "[$(date +%T)] GAVE UP on $n"
done
echo GRAB2_END
