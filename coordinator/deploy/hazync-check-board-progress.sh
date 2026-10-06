#!/usr/bin/env bash
# Nobody is proving, while there is work to prove (hazync#1003).
#
#   hazync-check-board-progress
#   BOARD_IDLE_ALERT_SECS=7200 hazync-check-board-progress
#
# ⛔⛔ WHY THIS EXISTS. On 2026-10-06 the board sat idle for EIGHT HOURS and nothing said so. Three
# contributors stopped between 00:20 and 02:00; it was noticed at 08:00 by a person looking at a
# chart. Every existing check was green the whole time, correctly: the coordinator was healthy, the
# API served, the proofs on disk verified, the spine's genesis proof was intact. Not one of them
# asks the question that mattered — IS ANY WORK ACTUALLY LANDING?
#
# ⚠ THE OBVIOUS VERSION OF THIS CHECK IS WRONG, which is why it is worth writing down. "Alert when
# a contributor goes quiet" sounds right and is unusable: contributors are volunteers on rented
# cards, they stop whenever they like, and a check that pages about one person stopping would fire
# every day and be muted within a week. What matters is not who stopped but whether the BOARD is
# still moving.
#
# ⚠ AND IT MUST ONLY FIRE WHEN THERE IS WORK. An idle board with nothing claimable is finished, not
# broken. The check asks the coordinator for the next pick; no pick means no alarm.
#
# EXIT CODES, matching the other integrity checks so hazync-run-check can alert on it:
#   0  work is landing, or there is none to land
#   1  nothing has been proved for BOARD_IDLE_ALERT_SECS while work is available
#   2  could not check (the coordinator did not answer, or answered something unusable)
set -uo pipefail

COORD="${COORD_URL:-http://127.0.0.1:8899}"
IDLE="${BOARD_IDLE_ALERT_SECS:-7200}"          # 2 h: long enough that a quiet patch is not an alarm
STATE="${STATE_DIRECTORY:-/var/lib/hazync-checks}/board-progress"

say() { echo "[board] $*"; }

body=$(curl -s --max-time 20 "$COORD/api/state" 2>/dev/null) || body=""
if [ -z "$body" ]; then
    say "cannot check: $COORD/api/state did not answer"
    exit 2
fi

# ⚠ ONE SUBSTITUTION, SINGLE-QUOTED. The first version put this python in a heredoc inside a
# command substitution with escaped quotes; the shell mangled it and the check reported "no usable
# progress.proven" for EVERY input — including good ones. A check that cannot parse a healthy
# answer fails safe to exit 2, which is quiet, which is the failure mode this whole file is about.
parsed=$(printf '%s' "$body" | python3 -c '
import json, sys
try:
    d = json.load(sys.stdin)
except Exception:
    print("x 0"); raise SystemExit
p = (d.get("progress") or {}).get("proven")
# ⚠ "is there work" is asked of the BOARD, not derived here. A block the coordinator will not hand
# out is not work, however open it looks from outside.
blocked = d.get("blocked") or {}
has = 1 if (blocked.get("block") is not None or (d.get("board") or [])) else 0
print(p if isinstance(p, int) else "x", has)
' 2>/dev/null)
proven=${parsed%% *}
pick_present=${parsed##* }

if [ "${proven:-x}" = "x" ] || ! [ "$proven" -eq "$proven" ] 2>/dev/null; then
    say "cannot check: /api/state had no usable progress.proven"   # ⚠ no backticks inside " ": bash runs them
    exit 2
fi

now=$(date +%s)
mkdir -p "$(dirname "$STATE")" 2>/dev/null || true

prev_proven=""; prev_ts=""
if [ -r "$STATE" ]; then
    read -r prev_proven prev_ts < "$STATE" 2>/dev/null || true
fi

# ⛔ THE STAMP MOVES ONLY WHEN THE COUNT RISES. Rewriting it every run would reset the clock on
# every tick and the check could never fire — the same shape as a heartbeat on a timer rather than
# on progress, which this project has already been bitten by (#256).
if [ -z "${prev_proven:-}" ] || [ "$proven" -gt "${prev_proven:-0}" ]; then
    printf '%s %s\n' "$proven" "$now" > "$STATE"
    say "ok: $proven proven (advanced; clock reset)"
    exit 0
fi

idle_for=$(( now - ${prev_ts:-$now} ))
if [ "$pick_present" != "1" ]; then
    say "ok: $proven proven, unchanged for ${idle_for}s — but the board is offering no work"
    exit 0
fi

if [ "$idle_for" -ge "$IDLE" ]; then
    say "NOBODY IS PROVING: $proven proven, unchanged for $(( idle_for / 3600 ))h $(( (idle_for % 3600) / 60 ))m, and the board still has work"
    say "the coordinator is probably fine — check whether any worker is actually running"
    exit 1
fi
say "ok: $proven proven, last advanced ${idle_for}s ago (alert at ${IDLE}s)"
exit 0
