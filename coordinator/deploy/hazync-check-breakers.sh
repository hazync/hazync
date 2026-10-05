#!/usr/bin/env bash
# A self-latching breaker is holding a service off, and nothing else on this box would say so.
#
#   hazync-check-breakers
#   HAZYNC_BREAKER_PATHS="/var/lib/hazync/sponsor-bot/.failed" hazync-check-breakers
#
# ⛔⛔ WHY THIS EXISTS: A LATCHED BREAKER IS INVISIBLE TO systemd'S OWN ALERTING. Measured on
# hazync-proof 2026-10-03 16:20:49 -- hazync-sponsor-bot hit `HTTP Error 503` four times fetching
# SHA256SUMS.txt, did exactly the right thing (started nothing, spent nothing, alerted once), and
# latched its own breaker: a 0-byte /var/lib/hazync/sponsor-bot/.failed plus
# `ConditionPathExists=!/var/lib/hazync/sponsor-bot/.failed`.
#
# From that moment the timer kept firing every ~5 minutes and systemd logged
#
#     hazync-sponsor-bot.service ... skipped, unmet condition check ConditionPathExists=!...
#
# ⚠ A SKIPPED UNIT IS NOT A FAILED UNIT. The job reports success, `Result=success`, `is-failed` says
# no, and OnFailure= never runs. The bot did nothing for TWO DAYS and sent ZERO alerts. The breaker
# is good design -- it stops a broken bot burning money on repeat. Having nothing watch the breaker
# is the bug.
#
# ⛔ AND THE CAUSE WAS ALREADY GONE. Both release assets returned HTTP 200 when checked on 10-05, so
# the only thing still holding the bot down was the file. That is the shape of this failure: it
# outlives its cause and waits to be noticed by a human.
#
# ⚠ THIS CHECK DELIBERATELY DOES NOT CLEAR ANYTHING. Clearing hazync-sponsor-bot's breaker resumes
# real spending (--live --max-usd 50 --max-usd-per-hour 10 --max-pods 4). A monitor that un-latched
# a breaker by itself would defeat the point of having one. It reports; a human decides.
#
# EXIT CODES, matching the other integrity checks so hazync-run-check can alert on it:
#   0  no breaker is latched
#   1  a breaker IS latched -- whatever it guards has been off since then
#   2  could not check (a breaker's directory is missing) -- says NOTHING about the others
set -uo pipefail

PATHS="${HAZYNC_BREAKER_PATHS:-/var/lib/hazync/sponsor-bot/.failed}"

say() { echo "[breaker] $*"; }
latched=0
cannot=0
checked=0

now=$(date +%s)

for p in $PATHS; do
    d=$(dirname "$p")
    if [ ! -d "$d" ]; then
        # ⚠ NOT "no breaker". An absent directory means the service is not installed here, or the
        # path moved -- either way this check is not watching what it was asked to watch, and
        # reporting that as "ok" is how a monitor quietly stops monitoring.
        say "cannot check: $d does not exist, so $p cannot be observed"
        cannot=1
        continue
    fi
    checked=$((checked + 1))
    if [ -e "$p" ]; then
        latched=1
        since=$(stat -c %Y "$p" 2>/dev/null || echo "")
        when=$(stat -c %y "$p" 2>/dev/null | cut -d. -f1)
        if [ -n "$since" ] && [ "$now" -ge "$since" ]; then
            hrs=$(( (now - since) / 3600 ))
            say "LATCHED: $p since ${when:-unknown} (~${hrs}h) — whatever it guards has been OFF since then"
        else
            # A timestamp in the future says the clock moved, not that the breaker is fresh.
            say "LATCHED: $p (mtime ${when:-unknown} is not usable for an age) — whatever it guards is OFF"
        fi
    else
        say "ok: no breaker at $p"
    fi
done

# A latched breaker is definite. Exiting 2 because some OTHER path was unreadable would bury it.
if [ "$latched" != 0 ]; then
    say "a service is latched off and will stay off until a human clears the file — check why it tripped first"
    [ "$cannot" != 0 ] && say "(and at least one path could not be checked — see above)"
    exit 1
fi
if [ "$cannot" != 0 ] || [ "$checked" = 0 ]; then
    say "COULD NOT CHECK — this says NOTHING about whether a service is latched off"
    exit 2
fi
say "all $checked breaker(s) clear"
exit 0
