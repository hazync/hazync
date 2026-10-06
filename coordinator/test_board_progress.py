#!/usr/bin/env python3
"""Tests for hazync-check-board-progress.sh (hazync#1003).

⛔⛔ WHY THIS CHECK EXISTS. On 2026-10-06 the board sat idle for EIGHT HOURS and nothing said so.
Three contributors stopped between 00:20 and 02:00 and it was noticed at 08:00 by a person looking
at a chart. Every other check was green, and correctly: the coordinator was healthy, the API served,
the stored proofs verified, the genesis proof was intact. Not one asks whether work is LANDING.

⚠ A liveness check has one dangerous way to be wrong and it is the quiet direction: reporting "ok"
while nothing happens for a day. The second is noise — a check that pages on every quiet patch gets
muted, and a muted check is worse than none. So the cases below pin BOTH: it must fire on a long
idle WITH work available, and it must stay silent when there is no work to do.

No coordinator: `curl` is stubbed on PATH to return chosen /api/state bodies.

  python3 test_board_progress.py            # assertions
  python3 test_board_progress.py --control  # the stamp is rewritten every run; it can never fire
"""
import json
import os
import subprocess
import sys
import tempfile

CONTROL = "--control" in sys.argv
HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "deploy", "hazync-check-board-progress.sh")

fails = []


def check(ok, what):
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails.append(what)


work = tempfile.mkdtemp(prefix="board_")
src = open(SCRIPT).read()

GUARD = 'if [ -z "${prev_proven:-}" ] || [ "$proven" -gt "${prev_proven:-0}" ]; then'
if GUARD not in src:
    print(f"CANNOT TEST: the progress guard is not in {SCRIPT}.")
    print("These tests are pinned to it; if it was rewritten, update them deliberately.")
    sys.exit(2)
if CONTROL:
    # ⛔ The naive version: stamp every run. It looks harmless and makes the check unable to ever
    # fire, because the idle clock restarts on every tick. Same shape as beating on a timer
    # instead of on progress (#256).
    src = src.replace(GUARD, 'if true; then')

script = os.path.join(work, "check.sh")
open(script, "w").write(src)
os.chmod(script, 0o755)

binv = os.path.join(work, "bin")
os.makedirs(binv)
BODY = os.path.join(work, "body.json")
with open(os.path.join(binv, "curl"), "w") as f:
    f.write('#!/bin/sh\ncat "$BODY_FILE"\n')
os.chmod(os.path.join(binv, "curl"), 0o755)

STATE_DIR = os.path.join(work, "state")
os.makedirs(STATE_DIR)


def state_body(proven, work_available=True, valid=True):
    if not valid:
        return "not json at all"
    d = {"progress": {"proven": proven}}
    if work_available:
        d["blocked"] = {"block": 144180, "status": "open"}
        d["board"] = [{"id": "144000-144999", "status": "open"}]
    else:
        d["blocked"] = None
        d["board"] = []
    return json.dumps(d)


def run(proven, *, work_available=True, valid=True, age=None):
    """Run the check. `age` back-dates the stored stamp to simulate elapsed idle time."""
    open(BODY, "w").write(state_body(proven, work_available, valid))
    stamp = os.path.join(STATE_DIR, "board-progress")
    if age is not None and os.path.exists(stamp):
        p, _ts = open(stamp).read().split()
        import time
        open(stamp, "w").write(f"{p} {int(time.time()) - age}\n")
    env = dict(os.environ, PATH=binv + os.pathsep + os.environ["PATH"],
               BODY_FILE=BODY, STATE_DIRECTORY=STATE_DIR,
               BOARD_IDLE_ALERT_SECS="7200")
    p = subprocess.run(["bash", script], env=env, capture_output=True, text=True)
    return p.returncode, (p.stdout + p.stderr).strip()


print("── 1. a board that is advancing is fine ──")
rc, out = run(144000)
check(rc == 0, f"first run records the count and exits 0 (rc={rc})")
rc, out = run(144050)
check(rc == 0 and "advanced" in out, f"a higher count resets the clock ({out[:60]})")

print("── 2. ⛔ an idle board WITH work must fire ──")
# This is the eight hours nobody was told about.
rc, out = run(144050)                       # unchanged
check(rc == 0, "unchanged but recent is not yet an alarm")
rc, out = run(144050, age=8 * 3600)         # unchanged for 8 h
if CONTROL:
    check(rc == 0, "control: stamping every run means the clock never ages, so it cannot fire")
else:
    check(rc == 1, f"unchanged for 8 h with work available exits 1 (rc={rc})")
    check("NOBODY IS PROVING" in out, "and says so in words a person can act on")
    check("8h" in out or "8h 0m" in out, f"and names how long ({out[:90]})")

print("── 3. ⚠ an idle board with NO work must stay silent ──")
# An idle board with nothing claimable is finished, not broken. Paging here is how a check gets muted.
rc, out = run(144050, work_available=False, age=8 * 3600)
check(rc == 0, f"no work on offer -> no alarm (rc={rc})")
check("offering no work" in out, "and says why it is not complaining")

print("── 4. ⛔ could not check is NOT ok ──")
rc, out = run(0, valid=False)
check(rc == 2, f"an unusable /api/state exits 2, not 0 and not 1 (rc={rc})")
check("cannot check" in out, "and says it could not check")

print("── 5. the alert threshold is honoured, not hardcoded ──")
rc, out = run(144050, age=3600)             # 1 h, under the 2 h default
check(rc == 0, f"1 h idle is under the 2 h threshold (rc={rc})")

print("── 6. ⛔⛔ the stamp moves only when the count RISES ──")
# Rewriting it every run resets the idle clock on every tick, so the check can never fire. Exactly
# the shape of beating on a timer rather than on progress, which this project removed in #256.
check(GUARD in open(SCRIPT).read(),
      "the guard comparing against the PREVIOUS count is present in the shipped script")

# ⚠ The no-work case ALSO changes under the control, and saying so is the point of listing them:
# when the stamp is rewritten every run the script reports "advanced" for every input, so it can
# neither fire on a stalled board nor explain a quiet one. Both are consequences of the same break.
EXPECTED_CONTROL_FAILURES = {
    "and says why it is not complaining",
}

print()
if CONTROL:
    got = set(fails)
    if got == EXPECTED_CONTROL_FAILURES:
        print("CONTROL OK — stamping on every run makes an 8 h idle board look fine, and exactly "
              f"the {len(got)} assertion(s) that depend on the guard failed:")
        for f in sorted(got):
            print(f"  - {f}")
        sys.exit(0)
    print("CONTROL FAILED — removing the guard did not produce the expected failures.")
    for f in sorted(EXPECTED_CONTROL_FAILURES - got):
        print(f"  should have failed and did not: {f}")
    for f in sorted(got - EXPECTED_CONTROL_FAILURES):
        print(f"  failed unexpectedly: {f}")
    sys.exit(1)

if fails:
    print(f"FAILED {len(fails)}: " + "; ".join(fails))
    sys.exit(1)
print("all good")
