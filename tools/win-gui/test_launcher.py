#!/usr/bin/env python3
"""Tests for Hazync.bat — the double-click entry point, which is the part nobody can see fail.

⛔⛔ WHY THIS EXISTS. Hazync.bat shipped invoking `py -3 -w hazync_gui.py`. There is no -w option on
the py launcher and none on python either, so `py` passed it through, python.exe answered
`Unknown option: -w`, printed its usage and exited. Measured on a real Windows machine 2026-10-05:
"a window popped up and then closed" — a console with a usage error, not the GUI.

⚠ AND IT FAILED FOR EVERYBODY. It was the FIRST branch in the file and `where py` succeeds on every
standard python.org install, so none of the fallbacks were ever reached. The windowless interpreter
is a separate executable, pyw.exe, not a flag.

⛔ A .BAT CANNOT BE RUN ON LINUX, WHICH IS EXACTLY HOW THIS SHIPPED. Every other file here has a
self-test that runs anywhere; the launcher had none, because "it needs Windows" felt like the end of
the thought. It is not: the file is a text file making claims about interpreters, and those claims
are checkable. This test reads it as text.

⭐ IT PROVES ITS OWN PREMISE. Rather than asserting "-w is invalid" from memory, it ASKS the
interpreter it is running on, so the test cannot quietly outlive the fact it depends on.

  python3 test_launcher.py            # assertions; exit 0 on success
  python3 test_launcher.py --control  # -w is put back; the -w assertions MUST fail
"""
import os as _os, sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
import winconsole as _wc  # noqa: E402
_wc.fix()   # ⛔ BEFORE anything prints: a ✅ on a cp1252 console raises, not degrades

import re
import subprocess
import sys

CONTROL = "--control" in sys.argv
HERE = _os.path.dirname(_os.path.abspath(__file__))
BAT = _os.path.join(HERE, "Hazync.bat")

fails = []


def check(ok, what):
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails.append(what)


src = open(BAT).read()
if CONTROL:
    # Put the shipped bug back, exactly as it was.
    src = src.replace('start "" pyw -3 hazync_gui.py', 'start "" py -3 -w hazync_gui.py')

# Lines that actually launch something: `start "" <interp> ... hazync_gui.py`
LAUNCH = re.compile(r'^\s*start\s+""\s+(\S+)((?:\s+\S+)*?)\s+hazync_gui\.py', re.M)
launches = [(m.group(1), m.group(2).split()) for m in LAUNCH.finditer(src)]

print("── 1. the file launches the GUI at all ──")
check(bool(launches), f"{len(launches)} launch line(s) found — a launcher that launches nothing is the bug")
check(all(i in ("py", "pyw", "python", "pythonw") for i, _ in launches),
      f"every launch uses a known interpreter: {[i for i, _ in launches]}")

print("── 2. ⭐ PROVE THE PREMISE: ask this interpreter whether -w is valid ──")
r = subprocess.run([sys.executable, "-w", "-c", "pass"], capture_output=True, text=True)
rejected = r.returncode != 0 and "nknown option" in (r.stdout + r.stderr)
check(rejected, f"python rejects -w (rc={r.returncode}, said {(r.stdout + r.stderr).strip().splitlines()[0][:40]!r})")
if not rejected:
    print("       ⚠ if a future python ACCEPTS -w, the assertions below are no longer load-bearing")

print("── 3. ⛔ no launch may pass -w to an interpreter ──")
with_w = [(i, a) for i, a in launches if "-w" in a]
check(not with_w,
      "no launch line passes -w" + (f" — BROKEN: {with_w}" if with_w else ""))

print("── 4. the first interpreter tried must be WINDOWLESS ──")
# The whole point of a double-click launcher is no console behind the app. If the first branch that
# succeeds is a console interpreter, every user gets a console — which is a regression, not a crash,
# so nothing would report it.
first = launches[0][0] if launches else None
check(first in ("pyw", "pythonw"),
      f"first launch uses a windowless interpreter (got {first!r})")

print("── 5. every branch guards on the interpreter it then runs ──")
# ⛔ `where py` followed by `start "" pyw` would test one binary and run another — present on a
# machine with py.exe but no pyw.exe, which is how a 'fixed' launcher still fails to start.
guards = re.findall(r'^\s*where\s+(\S+)\s', src, re.M)
pairs = list(zip(guards, [i for i, _ in launches]))
mismatched = [(g, l) for g, l in pairs if g != l]
check(len(guards) == len(launches),
      f"{len(guards)} `where` guard(s) for {len(launches)} launch line(s)")
check(not mismatched,
      "each `where X` is followed by a launch of X" + (f" — MISMATCH: {mismatched}" if mismatched else ""))

print("── 6. it still says what to do when there is no python ──")
# ⚠ The failure path is the one a newcomer is most likely to hit and the least likely to be tested.
check("python.org/downloads" in src, "points at the real installer")
check("Add python.exe to PATH" in src, "names the checkbox that actually matters")
check("pause" in src, "pauses, so the message is readable after a double-click")
check("Store" in src, "explains the Microsoft Store stub, which looks identical to success")

print("── 7. it runs from its own directory ──")
# A double-click from Explorer starts in the user's home, not the script's folder, so an unqualified
# `hazync_gui.py` would not be found.
check('cd /d "%~dp0"' in src, "cd /d %~dp0 — a double-click does not start in the script's folder")

EXPECTED_CONTROL_FAILURES = {
    "no launch line passes -w — BROKEN: [('py', ['-3', '-w'])]",
    "first launch uses a windowless interpreter (got 'py')",
    "each `where X` is followed by a launch of X — MISMATCH: [('pyw', 'py')]",
}

print()
if CONTROL:
    got = set(fails)
    if got == EXPECTED_CONTROL_FAILURES:
        print(f"CONTROL OK — the shipped `py -3 -w` was restored and exactly the {len(got)} "
              "assertion(s) that depend on it failed:")
        for f in sorted(got):
            print(f"  - {f}")
        sys.exit(0)
    print("CONTROL FAILED — restoring `py -3 -w` did not produce the expected failures.")
    for f in sorted(EXPECTED_CONTROL_FAILURES - got):
        print(f"  should have failed and did not: {f}")
    for f in sorted(got - EXPECTED_CONTROL_FAILURES):
        print(f"  failed unexpectedly: {f}")
    sys.exit(1)

if fails:
    print(f"FAILED {len(fails)}: " + "; ".join(fails))
    sys.exit(1)
print("all good")
