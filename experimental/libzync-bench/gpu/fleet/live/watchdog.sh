#!/bin/bash
# Safety net while the user sleeps: terminate THIS RUN's pods (pod.sh ours: fleet_ids.txt + maybe_billing.txt --
# never the rest of the account) if the balance drops below $3 or 85 minutes pass, whichever comes first.
# Independent of live_hour.sh, so a hang there cannot keep billing.
cd ~/libzync-gpu/live; END=$((SECONDS + 85 * 60))
while true; do
  # ⛔ pod.sh status exits 1 on an error body: then S is empty, N is UNKNOWN (never 0 = "all pods gone").
  S=$(./pod.sh status) || S=""
  OURS=$(./pod.sh ours 2>> watchdog.log) || OURS="UNKNOWN"
  BAL=$(echo "$S" | python3 -c 'import json,sys; b=json.load(sys.stdin)["balance"]; print(float(b))' 2>/dev/null || echo UNREADABLE)
  N=$(echo "$S" | python3 -c 'import json,sys
ours=set(sys.argv[1].split()); print(sum(1 for p in json.load(sys.stdin)["pods"] if p["id"] in ours))' "$OURS" 2>/dev/null || echo UNKNOWN)
  [ "$OURS" = UNKNOWN ] && N=UNKNOWN
  [ "$BAL" = UNREADABLE ] && echo "[$(date +%T)] ⚠ balance UNREADABLE -- only the 85-min time limit guards the spend" >> watchdog.log
  [ "$N" = UNKNOWN ] && echo "[$(date +%T)] ⚠ this run's pods could not be counted -- count UNKNOWN" >> watchdog.log
  [ "$N" = 0 ] && grep -q LIVE_HOUR_DONE live_hour.out 2>/dev/null && { echo "[$(date +%T)] all of this run's pods gone, watchdog exits" >> watchdog.log; exit 0; }
  if [ $SECONDS -ge $END ] || { [ "$BAL" != UNREADABLE ] && python3 -c "import sys; sys.exit(0 if float('$BAL') < 3 else 1)"; }; then
    echo "[$(date +%T)] WATCHDOG FIRED (balance $BAL, elapsed ${SECONDS}s): terminating this run's pods" >> watchdog.log
    IDS=$(./pod.sh ours 2>> watchdog.log; awk '$1 ~ /^[a-z0-9]+$/ {print $1}' fleet_ids.txt 2>/dev/null)
    IDS=$(echo $IDS | tr ' ' '\n' | sort -u | tr '\n' ' ')
    # every DELETE first, then each confirmed (pod.sh term = rp_term); loud per pod it cannot confirm
    if [ -z "${IDS// /}" ] || ./pod.sh term $IDS 2>> watchdog.log; then echo "[$(date +%T)] all gone" >> watchdog.log; exit 0; fi
    echo "[$(date +%T)] ⛔ NOT ALL CONFIRMED GONE -- see above, TERMINATE BY HAND" >> watchdog.log; exit 1
  fi
  sleep 120
done
