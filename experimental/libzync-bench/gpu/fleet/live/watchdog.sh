#!/bin/bash
# Safety net while the user sleeps: terminate EVERY RunPod pod if the balance drops below $3 or 85 minutes
# pass, whichever comes first. Independent of live_hour.sh, so a hang there cannot keep billing.
cd ~/libzync-gpu/live; END=$((SECONDS + 85 * 60))
while true; do
  # ⛔ pod.sh status exits 1 on an error body: then S is empty, N is UNKNOWN (never 0 = "all pods gone").
  S=$(./pod.sh status) || S=""
  BAL=$(echo "$S" | python3 -c 'import json,sys; b=json.load(sys.stdin)["balance"]; print(float(b))' 2>/dev/null || echo UNREADABLE)
  N=$(echo "$S" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["pods"]))' 2>/dev/null || echo UNKNOWN)
  [ "$BAL" = UNREADABLE ] && echo "[$(date +%T)] ⚠ balance UNREADABLE -- only the ${SECONDS}s/85-min time limit guards the spend" >> watchdog.log
  [ "$N" = UNKNOWN ] && echo "[$(date +%T)] ⚠ RunPod did not list pods -- pod count UNKNOWN" >> watchdog.log
  [ "$N" = 0 ] && grep -q LIVE_HOUR_DONE live_hour.out 2>/dev/null && { echo "[$(date +%T)] all pods gone, watchdog exits" >> watchdog.log; exit 0; }
  if [ $SECONDS -ge $END ] || { [ "$BAL" != UNREADABLE ] && python3 -c "import sys; sys.exit(0 if float('$BAL') < 3 else 1)"; }; then
    echo "[$(date +%T)] WATCHDOG FIRED (balance $BAL, elapsed ${SECONDS}s): terminating all pods" >> watchdog.log
    # ⛔ The listing may be what failed: terminate what it shows AND every pod this run rented (fleet_ids.txt).
    { echo "$S" | python3 -c 'import json,sys; [print(p["id"]) for p in json.load(sys.stdin)["pods"]]' 2>/dev/null
      awk '{print $1}' fleet_ids.txt 2>/dev/null; } | sort -u > watchdog_ids.txt
    bad=0
    while read -r id; do
      [ -n "$id" ] || continue
      if ./pod.sh term "$id" 2>> watchdog.log; then echo "[$(date +%T)] $id gone" >> watchdog.log
      else echo "[$(date +%T)] ⛔ $id NOT CONFIRMED GONE -- TERMINATE IT BY HAND" >> watchdog.log; bad=1; fi
    done < watchdog_ids.txt
    exit $bad
  fi
  sleep 120
done
