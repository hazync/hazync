#!/bin/bash
# One hour of live tip following, then harvest, then terminate EVERY pod. Unattended (user asleep).
cd ~/libzync-gpu/live
./tiplive.sh 3600 > live_hour.out 2>&1
echo "[$(date +%T)] tiplive ended, harvesting" >> live_hour.out
K="-n -i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=30"
mkdir -p harvest
while read -r nm ip p _; do
  for t in 1 2 3; do
    ssh $K -p $p root@$ip 'cd /workspace && tar czf h.tgz tip_* 2>/dev/null; true' \
      && scp -q -i ~/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=30 -P $p root@$ip:/workspace/h.tgz harvest/$nm.tgz && break
    sleep 5
  done
done < fleet.txt
ls -la harvest >> live_hour.out
for id in $(awk '{print $1}' fleet_ids.txt); do ./pod.sh term $id > /dev/null; done
sleep 8
echo "[$(date +%T)] pods left: $(./pod.sh status | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["data"]["myself"]["pods"]))')" >> live_hour.out
ssh hazync-coord 'shred -u /tmp/lz_eph 2>/dev/null; rm -f /tmp/lz_*.json'
echo LIVE_HOUR_DONE >> live_hour.out
