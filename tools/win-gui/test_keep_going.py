"""Start keeps proving until Stop (Xesktop, 11-13: the app proved block 1,339, then the session ended).

A stand-in worker (a real child process, as the window runs it) proves one "block" per run and exits as told: 0 (a
block landed), 75 (nothing free) or 78 (misconfigured). The window must start it again after 0 and 75, never after
78, and Stop must also cancel a start that is only scheduled.

    python test_keep_going.py            # must PASS
    python test_keep_going.py --control  # the restart taken out: MUST FAIL
"""
import os
import sys
import tempfile
import time

CONTROL = "--control" in sys.argv
fails = 0

STAND_IN = r'''
import os, sys
runs = os.path.join(os.environ["HAZYNC_HOME"], "runs")
n = len(open(runs).read().split()) if os.path.exists(runs) else 0
open(runs, "a").write("x\n")
print(f"claimed block {1339 + n} (yours for 60 min)", flush=True)
sys.exit(int(open(os.path.join(os.environ["HAZYNC_HOME"], "rc")).read()))
'''


def check(ok, what):
    global fails
    print(("  ok   " if ok else "  FAIL ") + what)
    fails += 0 if ok else 1


def main():
    tmp = tempfile.mkdtemp()
    os.environ["HAZYNC_HOME"] = tmp
    try:
        import hazync_gui
        import supervisor
    except Exception as e:                 # noqa: BLE001
        print(f"  SKIP cannot import the window: {e}")
        return 0
    hazync_gui.App.refresh = lambda self: None
    hazync_gui.App.rescan = lambda self: None
    hazync_gui.App.AGAIN_MS = {"done": 200, "idle": 400, "transient": 400}
    if CONTROL:
        hazync_gui.App._again = lambda self, w, kind: None
        print("CONTROL: no restart -- the checks below MUST fail")
    try:
        app = hazync_gui.App()
    except Exception as e:                 # noqa: BLE001
        if "display" in str(e).lower():
            print(f"  SKIP no display: {e}")
            return 0
        raise
    app.withdraw()
    stand_in = os.path.join(tmp, "stand_in_worker.py")
    open(stand_in, "w").write(STAND_IN)
    supervisor.gpu_cards = lambda: []
    supervisor.preflight = lambda *a, **kw: ([], [])
    app.worker_var.set(stand_in)
    app.ident_var.set(tmp)
    app.mode.set("auto")
    app.nworkers.set(1)
    app._setup_ok, app._steps = True, [object()]
    runs, rcf = os.path.join(tmp, "runs"), os.path.join(tmp, "rc")

    def count():
        return len(open(runs).read().split()) if os.path.exists(runs) else 0

    def spin(secs, until):
        end = time.time() + secs
        while time.time() < end:
            app.update()
            if until():
                return True
            time.sleep(0.02)
        return until()

    open(rcf, "w").write("0")
    app._start()
    check(spin(15, lambda: count() >= 3), f"a block landed (exit 0): it takes the next, and the next ({count()} runs)")
    check(str(app.stop_btn["state"]) == "normal", "Stop stays live between blocks")
    app._stop()
    spin(5, lambda: not app.workers)
    c = count()
    spin(1.5, lambda: False)
    check(count() == c and not app.workers and not getattr(app, "_respawns", {}),
          f"Stop: nothing more is started, not even a start already scheduled ({c} then {count()})")

    os.remove(runs)
    open(rcf, "w").write("75")
    app._start()
    check(spin(15, lambda: count() >= 2), f"nothing free (exit 75): it asks again ({count()} runs)")
    app._stop()
    spin(5, lambda: not app.workers and not app._respawns)

    os.remove(runs)
    open(rcf, "w").write("78")
    app._start()
    spin(3, lambda: False)
    check(count() == 1, f"misconfigured (exit 78): never restarted ({count()} run)")
    app._stop()
    app.destroy()

    if CONTROL:
        if fails >= 2:
            print(f"CONTROL OK: {fails} checks fail without the restart, as they must")
            return 0
        print(f"CONTROL BROKEN: only {fails} failed")
        return 1
    print("PASS" if fails == 0 else f"FAIL ({fails})")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
