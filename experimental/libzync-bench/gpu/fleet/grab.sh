#!/bin/bash
# Retry deploying PRO 6000 pods for the names given, every 30 s for 12 min; podup each one that lands.
cd ~/libzync-gpu/run; RP=$(cat ~/.hazync/runpod.key); PUB=$(cat ~/.ssh/ghost_signet_ed25519.pub)
rp(){ curl -s --max-time 20 -X "$1" -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" ${3:+-d "$3"} "https://api.runpod.io/v2$2"; }
want=("$@"); end=$((SECONDS+720))
while [ ${#want[@]} -gt 0 ] && [ $SECONDS -lt $end ]; do
  left=()
  for n in "${want[@]}"; do
    got=""
    for G in "NVIDIA RTX PRO 6000 Blackwell Server Edition" "NVIDIA RTX PRO 6000 Blackwell Workstation Edition"; do
      Q=$(python3 -c "import json,sys; print(json.dumps({'name': sys.argv[2], 'cloud': 'SECURE', 'gpu': {'id': sys.argv[3], 'count': 1}, 'image': 'runpod/pytorch:2.4.0-py3.11-cuda12.4.1-devel-ubuntu22.04', 'disk': 80, 'ports': ['22/tcp', '9110/tcp'], 'env': {'PUBLIC_KEY': sys.argv[1]}}))" "$PUB" "$n" "$G")
      R=$(rp POST /pods "$Q")
      ID=$(echo "$R" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("id") or "")' 2>/dev/null)
      [ -n "$ID" ] && { got=$ID; echo "$ID $n" >> fleet_ids.txt; echo "[$(date +%T)] GOT $n $ID on $G"; (setsid nohup ./podup.sh $n $ID > up_$n.log 2>&1 < /dev/null &); break; }
    done
    [ -z "$got" ] && left+=("$n")
  done
  want=("${left[@]}"); [ ${#want[@]} -gt 0 ] && sleep 30
done
echo "[$(date +%T)] GRAB_END still wanted: ${want[*]:-none}"
