"""'Also fold while proving' runs a folder beside the prover on the same card (11-13).

Stand-in workers (real child processes, as the window runs them): `run` lands a block and exits 0; `fold` finds
nothing to fold and exits 0. Ticked: Start runs both, on the same card; the prover leads Home; the prover keeps taking
blocks while the folder, which had nothing to do, waits the idle delay instead of restarting every two seconds.
Unticked: only the prover. The choice is saved.

    python test_also_fold.py            # must PASS
    python test_also_fold.py --control  # the tick-box ignored: MUST FAIL
"""
import os
import sys
import tempfile
import time

CONTROL = "--control" in sys.argv
fails = 0

STAND_IN = r'''
import os, sys
job = sys.argv[1]
open(os.path.join(os.environ["HAZYNC_HOME"], "runs"), "a").write(job + "\n")
if job == "run":
    print("claimed block 1500 (yours for 60 min)", flush=True)
    print("✓ range 1500: the coordinator re-verified your proof and put it on the board as 'W'.", flush=True)
else:
    print("nothing to fold — every adjacent pair on the board is already folded.", flush=True)
sys.exit(0)
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
        import firstrun
    except Exception as e:                 # noqa: BLE001
        print(f"  SKIP cannot import the window: {e}")
        return 0
    hazync_gui.App.refresh = lambda self: None
    hazync_gui.App.rescan = lambda self: None
    hazync_gui.App.AGAIN_MS = {"done": 150, "idle": 8000, "transient": 8000}
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
    runs = os.path.join(tmp, "runs")

    def jobs():
        return open(runs).read().split() if os.path.exists(runs) else []

    def spin(secs, until=lambda: False):
        end = time.time() + secs
        while time.time() < end:
            app.update()
            if until():
                return True
            time.sleep(0.02)
        return until()

    spawned = []
    _orig_spawn = app._spawn

    def _spawn(i, base, job, extra, card=None, label=""):
        spawned.append((job, card, label))
        return _orig_spawn(i, base, job, extra, card=card, label=label)
    app._spawn = _spawn
    app.also_fold.set(not CONTROL)
    app._persist()
    check(firstrun.load_config().get("also_fold") is (not CONTROL), "the tick is saved")
    app._start()
    spin(1.0)
    started = jobs()
    check("run" in started and "fold" in started, f"ticked: Start runs a prover AND a folder ({started})")
    first_run = next((x for x in spawned if x[0] == "run"), None)
    first_fold = next((x for x in spawned if x[0] == "fold"), None)
    check(first_fold is not None and first_run is not None and first_fold[1:] == first_run[1:],
          f"the folder is started on the prover's card ({first_fold} beside {first_run})")
    spin(2.0)
    j = jobs()
    check(j.count("run") >= 4 and j.count("fold") == 1,
          f"the prover keeps taking blocks; the folder, with nothing to fold, waits the idle delay ({j.count('run')} runs, {j.count('fold')} folds)")
    app._home_refresh()
    check("1,500" in app.v_head.get() or "block" in app.v_head.get().lower(),
          f"the prover leads Home: {app.v_head.get()!r}")
    app._stop()
    spin(5, lambda: not app.workers and not app._respawns)

    os.remove(runs)
    app.also_fold.set(False)
    app._start()
    spin(1.0)
    check(jobs() and set(jobs()) == {"run"}, f"unticked: only the prover ({jobs()})")
    app._stop()
    spin(5, lambda: not app.workers and not app._respawns)
    app.destroy()

    if CONTROL:
        if fails >= 2:
            print(f"CONTROL OK: {fails} checks fail without the tick-box, as they must")
            return 0
        print(f"CONTROL BROKEN: only {fails} failed")
        return 1
    print("PASS" if fails == 0 else f"FAIL ({fails})")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
