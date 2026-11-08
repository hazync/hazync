#!/bin/bash
# Prove each 09-29 tip block on the 4-pod fleet with run4.sh (fresh receipts each time).
cd ~/libzync-gpu/run; mapfile -t F < fleet.txt; K="-n -i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR"
for H in "$@"; do
  for l in "${F[@]}"; do read -r nm ip p _ <<<"$l"
    ssh $K -p $p root@$ip "printf 'export HAZYNC_BLOCK=/workspace/block_$H.json HAZYNC_ECMH_HINTS=/workspace/tip_$H/ecmh.bin HG=/workspace/tip_$H/hg\n' > /workspace/blockenv.sh; rm -rf /workspace/lane /workspace/agg"; done
  RUN=t$H ./run4.sh > t$H.out 2>&1
  echo "== $H"; grep -E 'chunks:|receipts on A|segment phase|assembly|TOTAL|VERIFIED|spread wall|RUN4_DONE|FAIL|need' t$H.out
done
for l in "${F[@]}"; do read -r nm ip p _ <<<"$l"; ssh $K -p $p root@$ip 'rm -f /workspace/blockenv.sh'; done
echo TIPRUN_DONE
