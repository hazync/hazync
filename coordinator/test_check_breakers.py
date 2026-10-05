#!/usr/bin/env python3
"""Tests for hazync-check-breakers.sh, and the OnFailure= placement it exists because of.

WHY THIS EXISTS. Two failures on 2026-10-05, both of the same shape: an alert that could not fire.

  1. hazync-sponsor-bot latched its own breaker on 10-03 after four HTTP 503s and stayed off for
     TWO DAYS in silence. A skipped unit reports `Result=success`; `is-failed` says no; OnFailure=
     never runs. Nothing on the box watched for the breaker file.
  2. hazync-bridge-backfill.service carried `OnFailure=` inside [Service], where systemd ignores it
     outright -- `systemctl show -p OnFailure` returned EMPTY. It was 1 of 23 units to get it wrong.

⛔ check-unit-drift CANNOT CATCH EITHER. It compares Environment=, ExecStart and the guards; it does
not look at OnFailure= at all, and it has no idea which section a key sits in. It reported
"no drift: 31 unit(s) checked, all declared in this repo" over both bugs, before and after the fix.
So these assertions are the only thing standing between the repo and a silent alerting hole.

A breaker check has one dangerous way to be wrong, and it is the safe-looking direction: reporting
"all clear" when it cannot see the file. That is indistinguishable from a healthy box right up until
the day something is latched off and nobody is told -- which is the exact failure it was written
for. So the cases below pin BOTH directions: a present breaker must exit 1, and an unobservable path
must exit 2 rather than a cheerful 0.

No real services: breaker files are made in a temp directory.

  python3 test_check_breakers.py            # assertions; exit 0 on success
  python3 test_check_breakers.py --control  # the latch test is removed; MUST report the break
"""
import glob
import os
import subprocess
import sys
import tempfile
import time

CONTROL = "--control" in sys.argv
HERE = os.path.dirname(os.path.abspath(__file__))
DEPLOY = os.path.join(HERE, "deploy")
SCRIPT = os.path.join(DEPLOY, "hazync-check-breakers.sh")

fails = []


def check(ok, what):
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails.append(what)


work = tempfile.mkdtemp(prefix="ckbreak_")
script = os.path.join(work, "check.sh")
src = open(SCRIPT).read()

GUARD = 'if [ -e "$p" ]; then'
if GUARD not in src:
    print(f"CANNOT TEST: the latch test is not in {SCRIPT}.")
    print("These tests are pinned to it; if it was rewritten, update them deliberately.")
    sys.exit(2)
if CONTROL:
    # Removes ONLY the "is the breaker file there" test, so exactly the latch cases should trip.
    src = src.replace(GUARD, "if false; then")
open(script, "w").write(src)
os.chmod(script, 0o755)


def run(paths):
    """Run the check over a whitespace-separated path list; return (rc, output)."""
    env = dict(os.environ, HAZYNC_BREAKER_PATHS=paths)
    p = subprocess.run(["bash", script], env=env, capture_output=True, text=True)
    return p.returncode, p.stdout + p.stderr


print("── 1. the ordinary case: nothing is latched ──")
d = os.path.join(work, "sponsor-bot")
os.makedirs(d)
clear = os.path.join(d, ".failed")
rc, out = run(clear)
check(rc == 0, f"no breaker file exits 0 (rc={rc})")
check("ok: no breaker" in out, "and says so plainly")
check("all 1 breaker(s) clear" in out, "and reports how many it actually looked at")

print("── 2. ⛔ a latched breaker must be LOUD ──")
open(clear, "w").close()
rc, out = run(clear)
check(rc == 1, f"a present breaker exits 1 (rc={rc})")
check("LATCHED" in out and clear in out, "names the exact file a human has to delete")
check("~0h)" in out, "and how long it has been latched, so the age is in the alert itself")
check("OFF since then" in out or "is OFF" in out, "and says what that means: the service is off")

print("── 3. ⛔ an unobservable path is NOT 'all clear' ──")
missing = os.path.join(work, "not-installed", ".failed")
rc, out = run(missing)
check(rc == 2, f"a missing directory exits 2, not 0 (rc={rc})")
check("cannot check" in out, "and says it could not check")
check("says NOTHING" in out, "and refuses to imply anything about the breakers it could not see")

print("── 4. ⛔ a real finding outranks an unreadable one ──")
# ⛔ If this returned 2, the latched breaker -- a definite, actionable fact -- would be buried
# behind a path that merely could not be read. check-disk has the same precedence for the same
# reason; getting it backwards is how a monitor reports the least useful thing it knows.
rc, out = run(f"{clear} {missing}")
check(rc == 1, f"latched + unreadable exits 1, not 2 (rc={rc})")
check("LATCHED" in out and "could not be checked" in out,
      "and the alert mentions BOTH, so neither is lost")

print("── 5. several breakers are all checked ──")
d2 = os.path.join(work, "other-bot")
os.makedirs(d2)
clear2 = os.path.join(d2, ".failed")
rc, out = run(f"{clear2} {os.path.join(d, '.nothing-here')}")
check(rc == 0, f"two clear paths exit 0 (rc={rc})")
check(out.count("ok: no breaker") == 2, "both are named individually, not summarised away")

print("── 6. ⚠ a clock that moved must not crash or go quiet ──")
# A breaker with a future mtime still means the service is OFF. Reporting a negative age would be
# noise; going silent would be a missed alert. It must still exit 1.
open(clear2, "w").close()
os.utime(clear2, (time.time() + 86400, time.time() + 86400))
rc, out = run(clear2)
check(rc == 1, f"a future mtime still exits 1 (rc={rc})")
check("LATCHED" in out, "and still says the service is latched off")

print("── 7. ⛔⛔ OnFailure= must be in [Unit] in EVERY shipped unit ──")
# This is the regression guard for the 10-05 bug. systemd silently ignores OnFailure= in [Service],
# so a unit with it there can never alert -- and nothing else in this repo checks for that.
misplaced = []
withhook = 0
for path in sorted(glob.glob(os.path.join(DEPLOY, "*.service"))):
    section = None
    for line in open(path):
        s = line.strip()
        if s.startswith("[") and s.endswith("]"):
            section = s
        elif s.startswith("OnFailure="):
            withhook += 1
            if section != "[Unit]":
                misplaced.append(f"{os.path.basename(path)} has OnFailure= in {section}")
            break
check(withhook >= 20, f"{withhook} shipped units declare OnFailure= (so this test has teeth)")
check(not misplaced, "every one of them puts it in [Unit]" + (f" — BROKEN: {misplaced}" if misplaced else ""))

print("── 8. the shipped unit and the script agree ──")
unit = open(os.path.join(DEPLOY, "hazync-check-breakers.service")).read()
default = ""
for line in open(SCRIPT):
    if line.startswith("PATHS="):
        default = line.split(":-", 1)[1].rstrip('}"\n ')
        break
check("/var/lib/hazync/sponsor-bot/.failed" in default,
      f"the script defaults to the sponsor-bot breaker (got {default!r})")
check(f"HAZYNC_BREAKER_PATHS={default}" in unit,
      "and the unit passes that same path, so the two cannot drift apart")
# ⛔ systemd splits Environment= on whitespace unless it is quoted. Unquoted, a second breaker path
# would be silently dropped and the check would still report success -- exactly what happened to
# hazync-check-disk, which watched one of its two paths and said "all 1 path(s) above the floor".
check('Environment="HAZYNC_BREAKER_PATHS=' in unit,
      "the path list is QUOTED, so adding a second breaker cannot silently drop it")
check("hazync-run-check check-breakers" in unit,
      "it runs through hazync-run-check, so alerting and the re-send throttle work like the others")

EXPECTED_CONTROL_FAILURES = {
    "a present breaker exits 1 (rc=0)",
    "names the exact file a human has to delete",
    "and how long it has been latched, so the age is in the alert itself",
    "and says what that means: the service is off",
    "latched + unreadable exits 1, not 2 (rc=2)",
    "and the alert mentions BOTH, so neither is lost",
    "a future mtime still exits 1 (rc=0)",
    "and still says the service is latched off",
}

print()
if CONTROL:
    got = set(fails)
    if got == EXPECTED_CONTROL_FAILURES:
        print(f"CONTROL OK — the latch test was removed and exactly the {len(got)} assertion(s) "
              "that depend on it failed:")
        for f in sorted(got):
            print(f"  - {f}")
        sys.exit(0)
    print("CONTROL FAILED — removing the latch test did not produce the expected failures.")
    for f in sorted(EXPECTED_CONTROL_FAILURES - got):
        print(f"  should have failed and did not: {f}")
    for f in sorted(got - EXPECTED_CONTROL_FAILURES):
        print(f"  failed unexpectedly: {f}")
    sys.exit(1)

if fails:
    print(f"FAILED {len(fails)}: " + "; ".join(fails))
    sys.exit(1)
print("all good")
