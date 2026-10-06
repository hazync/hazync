#!/usr/bin/env bash
# Integrity check: is the bridge still ADVANCING? (hazync#467)
# Installed as /usr/local/sbin/hazync-check-bridge-progress, invoked through hazync-run-check.
#
#   hazync-check-bridge-progress                      # check with the measured defaults
#   HAZYNC_BRIDGE_STALL_S=5400 hazync-check-bridge-progress
#
# ⛔ WHY THIS EXISTS. During catch-up the bridge is SILENT BY CONSTRUCTION and a stall is invisible.
#
#   * a crash pages, via hazync-bridge.service.d/alert.conf -- that path works and is not the gap
#   * hazync-check-bundle-gap detects a STALL, but it judges by BUNDLES ARRIVING at the coordinator.
#     Below HAZYNC_BRIDGE_EMIT_FROM the bridge advances accumulator state and writes NO bundles, so a
#     stalled bridge and a healthy one produce identical evidence: none.
#
# Measured 2026-09-22: a restart pushed the reload peak over MemoryHigh and the process sat in D state
# (mem_cgroup_handle_over_high) at 0% CPU for ~20 minutes, memory.pressure 97.3%, advancing nothing.
# It did not crash, so alert.conf stayed quiet; it emits no bundles, so bundle-gap had nothing to
# compare. The only evidence was `checkpoint @ N` lines silently not appearing in a journal nobody
# reads. A human happened to be watching. On an unattended night it would have burned until someone
# noticed the frontier had not moved.
#
# EXIT CODES, matching the other integrity checks so hazync-run-check can alert on it:
#   0  the bridge advanced within the window
#   1  it did NOT advance -- a stall, or a start WEDGED in `activating` (which can never fail)
#   2  could not check (stopped, no journal, no checkpoint lines yet, still legitimately starting)
set -uo pipefail

UNIT="${HAZYNC_BRIDGE_UNIT:-hazync-bridge}"

# ⚠ THE WINDOW IS DERIVED FROM THE MEASURED RATE, NOT GUESSED, AND IT IS GENEROUS.
# The bridge checkpoints every HAZYNC_BRIDGE_CKPT blocks (2000 live). At the rate measured on
# 2026-09-22 -- 0.638 / 0.655 / 0.699 s per block over three consecutive intervals -- that is roughly
# 22 minutes per checkpoint. Three missed intervals is the threshold, because:
#
#   * the walk rate is NOT flat (project_hazync_bridge_walk_rates: no flat tail, it degrades with
#     height), so a window sized to today's rate would cry wolf next week
#   * a reload after a restart legitimately takes minutes with no checkpoint written
#   * a check that cries wolf gets muted, and a muted check is worse than none
STALL_S="${HAZYNC_BRIDGE_STALL_S:-4200}"        # 70 min ~= 3 checkpoint intervals at the measured rate

say() { echo "$*"; }

# ⛔⛔ "NOT ACTIVE" HIDES TWO VERY DIFFERENT THINGS AND ONE OF THEM IS A SILENT OUTAGE. A deliberate
# stop is a human decision the alerter should not second-guess, and a crash is already covered by
# alert.conf. A unit WEDGED IN `activating` is neither. hazync-bridge has TimeoutStartUSec=infinity
# and an ExecStartPre that loops `until bitcoin-cli getblockcount`, so while the node is down the
# bridge waits FOR EVER: it never becomes `failed`, so OnFailure= cannot fire, and it is not
# `active`, so this check used to stand down with exit 2 -- which hazync-run-check turns into 0.
# Measured 2026-10-06: the node came back from a reboot DISABLED, the bridge sat in start-pre, bundle
# production was stopped, and every signal in the system still said success.
ACTIVATING_S="${HAZYNC_BRIDGE_ACTIVATING_S:-1800}"   # 30 min; a healthy start is seconds to minutes

act="$(systemctl is-active "$UNIT" 2>/dev/null)"

if [ "$act" = "activating" ]; then
    # ⚠ MONOTONIC, NOT WALL CLOCK. A box wedged in start-pre has usually just rebooted, which is
    # exactly when the clock is most likely to step, and a backwards step must not make a start that
    # has hung for an hour look fresh.
    since_us="$(systemctl show "$UNIT" -p InactiveExitTimestampMonotonic --value 2>/dev/null | tr -dc '0-9')"
    now_us="$(awk '{printf "%d", $1 * 1000000}' /proc/uptime 2>/dev/null | tr -dc '0-9')"
    if [ -n "$since_us" ] && [ -n "$now_us" ] && [ "$since_us" -gt 0 ] && [ "$now_us" -ge "$since_us" ]; then
        act_age=$(( (now_us - since_us) / 1000000 ))
        # ⚠ HAZYNC_BRIDGE_NO_ACTIVATING_GUARD exists ONLY for test-bridge-progress.sh --control.
        if [ "${HAZYNC_BRIDGE_NO_ACTIVATING_GUARD:-0}" != "1" ] && [ "$act_age" -gt "$ACTIVATING_S" ]; then
            say "WEDGED: $UNIT has been 'activating' for $((act_age / 60)) min (limit $((ACTIVATING_S / 60)) min).
It is NOT running, it emits nothing, and it will never fail by itself — so nothing else will page.
Its ExecStartPre is almost always waiting on something that is down; check the node FIRST:
  systemctl is-active bitcoind; systemctl is-enabled bitcoind
  systemctl show $UNIT -p ExecStartPre --value
  journalctl -u $UNIT -n 20 --no-pager"
            exit 1
        fi
        say "cannot check: $UNIT is still starting (${act_age}s so far, wedged at $((ACTIVATING_S / 60))m)"
        exit 2
    fi
    say "cannot check: $UNIT is $act and its start timestamp could not be read"
    exit 2
fi

if [ "$act" != "active" ]; then
    say "cannot check: $UNIT is not active ($act)"
    exit 2
fi

# ⛔ TWO REGIMES, AND THIS CHECK ONLY KNEW ONE (hazync#487). While walking, the bridge writes
# `checkpoint @ N` every HAZYNC_BRIDGE_CKPT (2000) blocks. Once it CATCHES UP it advances one block
# at a time as each finalises and writes `caught up to N` — the next checkpoint is 2000 blocks away,
# which at the tip is about a fortnight of chain. So from the moment the bridge started doing its
# actual job, this check saw no progress line and paged hourly. Measured 2026-09-23: seven
# consecutive FAILEDs against a bridge that was advancing normally, which is precisely the
# cry-wolf-gets-muted failure the original comment warned about.
#
# `caught up to N` counts as progress. Both are read, and the newest wins.
last="$(journalctl -u "$UNIT" -o short-unix --no-pager -n 4000 2>/dev/null \
        | grep -oE '^[0-9]+\.[0-9]+ .*(checkpoint @|caught up to) [0-9]+' | tail -1)"

if [ -z "$last" ]; then
    # ⚠ A bridge that has only just started, or one whose journal has rotated past its last progress
    # line, is unknown -- not failing. Resuming from a 19 GB state file takes minutes before the
    # first line appears, and calling that a stall would alert on every restart.
    say "cannot check: no 'checkpoint @' or 'caught up to' line in the last 4000 entries for $UNIT"
    exit 2
fi

# ⛔ AT THE TIP, "HOW LONG AGO" IS THE WRONG QUESTION. A caught-up bridge is idle by construction
# between blocks, and Bitcoin's inter-block gaps are exponential — 70-minute quiet spells are normal
# and would page every time. The honest question there is POSITIONAL: is the bridge where it is
# supposed to be, i.e. at (node tip - finality)? If it is, it is not stalled however long it has sat
# there. Only if it is BEHIND that does elapsed time mean anything.
FINALITY="$(systemctl show "$UNIT" -p Environment --value 2>/dev/null \
            | tr ' ' '\n' | sed -n 's/^HAZYNC_BRIDGE_FINALITY=//p' | head -1)"
FINALITY="${FINALITY:-100}"
NODE_TIP=""
if command -v bitcoin-cli >/dev/null 2>&1; then
    DD="$(systemctl show "$UNIT" -p Environment --value 2>/dev/null \
          | tr ' ' '\n' | sed -n 's/^HAZYNC_BITCOIN_DATADIR=//p' | head -1)"
    # ⚠ Failure here is not an answer. If the node cannot be asked, fall through to the time-based
    # test rather than assuming the bridge is fine.
    NODE_TIP="$(bitcoin-cli ${DD:+-datadir="$DD"} getblockcount 2>/dev/null | tr -dc '0-9')"
fi

ts="${last%%.*}"
height="$(printf '%s' "$last" | grep -oE '(checkpoint @|caught up to) [0-9]+' | grep -oE '[0-9]+')"
now="$(date +%s)"
age=$(( now - ts ))

# ⚠ A NEGATIVE AGE MEANS THE CLOCK MOVED, NOT THAT THE BRIDGE IS FINE. Treat it as unknown rather
# than as a pass: a wall clock that stepped backwards would otherwise make every stall look fresh.
if [ "$age" -lt 0 ]; then
    say "cannot check: last checkpoint is ${age}s in the FUTURE (clock stepped?) — height $height"
    exit 2
fi

# The positional test, where it can be made: at the tip and nothing to do is HEALTHY.
if [ -n "$NODE_TIP" ] && [ -n "$height" ]; then
    want=$(( NODE_TIP - FINALITY ))
    if [ "$height" -ge "$want" ]; then
        say "ok: $UNIT is AT THE TIP — height $height, node tip $NODE_TIP, finality $FINALITY \
(last progress $((age / 60))m ago; idle between blocks is how a caught-up bridge looks)"
        exit 0
    fi
fi

# ⚠ HAZYNC_BRIDGE_NO_STALL_GUARD exists ONLY for test-bridge-progress.sh --control, which
# must be able to remove the guard and prove the test notices.
if [ "${HAZYNC_BRIDGE_NO_STALL_GUARD:-0}" != "1" ] && [ "$age" -gt "$STALL_S" ]; then
    say "STALLED: $UNIT last checkpointed at height $height, $((age / 60)) min ago (limit $((STALL_S / 60)) min).
The process may be alive but making no progress — check memory.pressure and process state:
  systemctl status $UNIT
  ps -o pid,stat,wchan:24 -p \$(pgrep -f hazync-host-bridge | head -1)
  cat /sys/fs/cgroup/system.slice/$UNIT.service/memory.pressure"
    exit 1
fi

say "ok: $UNIT checkpointed height $height $((age / 60))m ago (limit $((STALL_S / 60))m)"
exit 0
