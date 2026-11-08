#!/bin/bash
# Safety net while the user sleeps: terminate EVERY RunPod pod if the balance drops below $3 or 85 minutes
# pass, whichever comes first. Independent of live_hour.sh, so a hang there cannot keep billing.
cd ~/libzync-gpu/live; END=$((SECONDS + 85 * 60))
while true; do
  S=$(./pod.sh status)
  BAL=$(echo "$S" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["myself"]["clientBalance"])' 2>/dev/null || echo 99)
  N=$(echo "$S" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["data"]["myself"]["pods"]))' 2>/dev/null || echo 1)
  [ "$N" = 0 ] && grep -q LIVE_HOUR_DONE live_hour.out 2>/dev/null && { echo "[$(date +%T)] all pods gone, watchdog exits" >> watchdog.log; exit 0; }
  if [ $SECONDS -ge $END ] || python3 -c "import sys; sys.exit(0 if float('$BAL') < 3 else 1)"; then
    echo "[$(date +%T)] WATCHDOG FIRED (balance $BAL, elapsed ${SECONDS}s): terminating all pods" >> watchdog.log
    echo "$S" | python3 -c 'import json,sys; [print(p["id"]) for p in json.load(sys.stdin)["data"]["myself"]["pods"]]' | while read id; do ./pod.sh term $id > /dev/null; done
    exit 0
  fi
  sleep 120
done
