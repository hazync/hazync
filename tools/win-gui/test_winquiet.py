"""Every child the app or its worker starts runs without a console window (Xesktop, 11-13: one opened per block).

    python test_winquiet.py            # must PASS
    python test_winquiet.py --control  # hide_child_consoles a no-op: MUST FAIL
"""
import subprocess
import sys

import winquiet

CONTROL = "--control" in sys.argv
if CONTROL:
    winquiet.hide_child_consoles = lambda popen_cls=None, force=False: False
    print("CONTROL: consoles not hidden -- the checks below MUST fail")
fails = 0


def check(ok, what):
    global fails
    print(("  ok   " if ok else "  FAIL ") + what)
    fails += 0 if ok else 1


class FakePopen:
    def __init__(self, *args, **kwargs):
        self.kwargs = kwargs


winquiet.hide_child_consoles(FakePopen, force=True)
p = FakePopen(["host.exe", "prove"])
check(p.kwargs.get("creationflags", 0) & winquiet.CREATE_NO_WINDOW, "a child gets CREATE_NO_WINDOW")
p = FakePopen(["hazync-worker"], creationflags=0x200)
check(p.kwargs.get("creationflags") == 0x200 | winquiet.CREATE_NO_WINDOW,
      "flags already asked for (CREATE_NEW_PROCESS_GROUP) are kept")
winquiet.hide_child_consoles(FakePopen, force=True)
p = FakePopen(["x"])
check(p.kwargs.get("creationflags") == winquiet.CREATE_NO_WINDOW, "applying it twice changes nothing")

if sys.platform == "win32":
    # the real thing: a console program, its output piped back, with no window
    winquiet.hide_child_consoles()
    r = subprocess.run(["cmd", "/c", "echo", "quiet"], capture_output=True, text=True)
    check(r.returncode == 0 and "quiet" in r.stdout, f"on Windows a console child still runs and pipes its output ({r.stdout.strip()!r})")
    check(getattr(subprocess.Popen.__init__, "_hz_quiet", False), "on Windows every Popen in the process is quiet")
elif not CONTROL:
    check(winquiet.hide_child_consoles() is False, "not on Windows: nothing is changed")

if CONTROL:
    if fails >= 2:
        print(f"CONTROL OK: {fails} checks fail without it, as they must")
        sys.exit(0)
    print(f"CONTROL BROKEN: only {fails} failed")
    sys.exit(1)
print("PASS" if fails == 0 else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
