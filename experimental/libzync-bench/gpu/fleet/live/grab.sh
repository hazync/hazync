#!/bin/bash
# Retry deploying PRO 6000 pods for the names given, every 30 s for 12 min; podup each one that lands.
cd ~/libzync-gpu/live; RP=$(cat ~/.hazync/runpod.key); PUB=$(cat ~/.ssh/ghost_signet_ed25519.pub)
want=("$@"); end=$((SECONDS+720))
while [ ${#want[@]} -gt 0 ] && [ $SECONDS -lt $end ]; do
  left=()
  for n in "${want[@]}"; do
    got=""
    for G in "NVIDIA RTX PRO 6000 Blackwell Server Edition" "NVIDIA RTX PRO 6000 Blackwell Workstation Edition"; do
      Q=$(python3 -c "import json,sys; print(json.dumps({'query': 'mutation { podFindAndDeployOnDemand(input: { cloudType: SECURE, gpuCount: 1, volumeInGb: 0, containerDiskInGb: 80, gpuTypeId: \"%s\", name: \"%s\", imageName: \"runpod/pytorch:2.4.0-py3.11-cuda12.4.1-devel-ubuntu22.04\", ports: \"22/tcp,9110/tcp\", env: [{key: \"PUBLIC_KEY\", value: \"%s\"}] }) { id costPerHr } }' % (sys.argv[3], sys.argv[2], sys.argv[1])}))" "$PUB" "$n" "$G")
      R=$(curl -s -X POST -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" -d "$Q" https://api.runpod.io/graphql)
      ID=$(echo "$R" | python3 -c 'import json,sys; p=(json.load(sys.stdin).get("data") or {}).get("podFindAndDeployOnDemand"); print(p["id"] if p else "")' 2>/dev/null)
      [ -n "$ID" ] && { got=$ID; echo "$ID $n" >> fleet_ids.txt; echo "[$(date +%T)] GOT $n $ID on $G"; (setsid nohup ./podup.sh $n > up_$n.log 2>&1 < /dev/null &); break; }
    done
    [ -z "$got" ] && left+=("$n")
  done
  want=("${left[@]}"); [ ${#want[@]} -gt 0 ] && sleep 30
done
echo "[$(date +%T)] GRAB_END still wanted: ${want[*]:-none}"
