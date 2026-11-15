"""The proving window, in the real window (#725): drain at the end, skip what cannot finish, hand claims back.

A stand-in worker (a real child process, as the app runs it) claims a block, "proves" it for as long as it is told,
puts it on the board and exits 0; asked to `release <n>`, it writes the block down. The proving window is a stand-in
too, so its end can be seconds away rather than a real 07:00:

- the window ends DURING a proof: that block finishes (drain, don't kill) and no other block is taken;
- too little of the window is left for this card's measured block time: Start takes no block at all;
- Stop while a block is claimed: the app hands that block back.

    python test_schedule_flow.py            # must PASS
    python test_schedule_flow.py --control  # no schedule gate and no release: MUST FAIL
"""
import os
import sys
import tempfile
import time

CONTROL = "--control" in sys.argv
fails = 0

STAND_IN = r'''
import os, sys, time
home = os.environ["HAZYNC_HOME"]
if sys.argv[1:2] == ["release"]:
    open(os.path.join(home, "released"), "a").write(sys.argv[2] + "\n")
    print(f"released block {sys.argv[2]}: it is open to any prover again", flush=True)
    sys.exit(0)
runs = os.path.join(home, "runs")
n = len(open(runs).read().split()) if os.path.exists(runs) else 0
open(runs, "a").write("x\n")
b = 1807 + n
print(f"claimed block {b} (yours for 60 min; if you stop, it reopens by itself)", flush=True)
time.sleep(float(open(os.path.join(home, "secs")).read()))
print(f"proved range [{b}..{b}] from bridge bundle in 1.0s -> range_{b}.hzk", flush=True)
print(f"✓ range {b}: the coordinator re-verified your proof and put it on the board as 'x'.", flush=True)
open(os.path.join(home, "finished"), "a").write(f"{b}\n")
rcf = os.path.join(home, "rc")
sys.exit(int(open(rcf).read()) if os.path.exists(rcf) else 0)
'''


def check(ok, what):
    global fails
    print(("  ok   " if ok else "  FAIL ") + what)
    fails += 0 if ok else 1


class FakeWindow:
    """Open until `end` (a timestamp); the shape schedule.Window has."""

    def __init__(self, end):
        self.end = end

    def is_open(self, ts, *a):
        return ts < self.end

    def ends_at(self, ts, *a):
        return self.end if ts < self.end else None

    def seconds_left(self, ts, *a):
        return max(0.0, self.end - ts) if ts < self.end else None

    def next_start(self, ts, *a):
        return ts + 3600

    def describe(self):
        return "a test window"

    start = __import__("datetime").time(23, 0)


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
    hazync_gui.SCHEDULE_TICK_MS = 150
    if CONTROL:
        hazync_gui.App._may_take_block = lambda self, card: (True, "control")
        hazync_gui.App._release = lambda self, w: None
        hazync_gui.App._window = lambda self: FakeWindow(time.time() + 3600)
        print("CONTROL: no schedule gate and no release -- the checks below MUST fail")
    try:
        app = hazync_gui.App()
    except Exception as e:                 # noqa: BLE001
        if "display" in str(e).lower():
            print(f"  SKIP no display: {e}")
            return 0
        raise
    app.withdraw()
    stand_in = os.path.join(tmp, "stand_in_worker.py")
    open(stand_in, "w", encoding="utf-8").write(STAND_IN)
    supervisor.gpu_cards = lambda: []
    supervisor.preflight = lambda *a, **kw: ([], [])
    app.worker_var.set(stand_in)
    app.ident_var.set(tmp)
    app.mode.set("auto")
    app.nworkers.set(1)
    app._setup_ok, app._steps = True, [object()]
    app.sched = dict(hazync_gui.SCHEDULE_DEFAULT, on=True, keep_awake=True)
    files = {k: os.path.join(tmp, k) for k in ("runs", "secs", "finished", "released")}

    def lines(k):
        return open(files[k]).read().split() if os.path.exists(files[k]) else []

    def clear():
        for k in ("runs", "finished", "released"):
            if os.path.exists(files[k]):
                os.remove(files[k])

    def spin(secs, until):
        end = time.time() + secs
        while time.time() < end:
            app.update()
            if until():
                return True
            time.sleep(0.02)
        return until()

    def set_window(end):
        if not CONTROL:
            app._window = lambda: FakeWindow(end)

    print("== the window ends during a proof ==")
    clear()
    open(files["secs"], "w").write("3")
    set_window(time.time() + 1.5)                   # the proof takes 3 s; the window closes 1.5 s in
    app.block_times = {}
    hazync_gui.schedule.UNKNOWN_MIN_S = 1           # nothing measured yet: any time left will do, here
    app._start()
    check(spin(5, lambda: len(lines("runs")) >= 1), f"Start inside the window takes a block ({lines('runs')})")
    spin(1.0, lambda: False)
    check(bool(app.workers), "the window has closed and the proof in hand is still running (drain, don't kill)")
    check(spin(10, lambda: len(lines("finished")) >= 1 and not app.workers),
          f"it finishes the block ({lines('finished')})")
    spin(1.5, lambda: False)
    check(len(lines("runs")) == 1, f"and takes no other block after the window closed ({len(lines('runs'))} runs)")
    check(app._armed and str(app.stop_btn["state"]) == "normal",
          "the schedule stays armed for the next window, and Stop is still the button")
    app._stop()
    spin(2, lambda: not app.workers)

    print("== too little of the window left for this card ==")
    clear()
    open(files["secs"], "w").write("0.2")
    set_window(time.time() + 60)                    # one minute left
    app.block_times = {"this-pc": [600, 620, 610]}  # and a block takes this card ten minutes
    app._start()
    spin(2, lambda: False)
    check(not lines("runs"), f"no block is claimed ({len(lines('runs'))} runs)")
    check(app._armed, "the app waits for the next window instead")
    app._stop()
    spin(1, lambda: not app.workers)

    print("== between blocks the PC is still kept awake ==")
    clear()
    open(files["secs"], "w").write("0.1")
    set_window(time.time() + 3600)
    app.block_times = {}
    hazync_gui.App.AGAIN_MS = {"done": 1500, "idle": 1500, "transient": 1500}
    app._start()
    gap = spin(5, lambda: bool(app._respawns) and not app.workers)
    spin(0.4, lambda: False)                         # at least two schedule ticks inside the gap
    check(gap and app._awake, f"a restart is waiting and the keep-awake request stands (gap {gap}, awake {app._awake})")
    app._stop()
    spin(3, lambda: not app.workers)
    check(not app._awake, "after Stop it is dropped")
    hazync_gui.App.AGAIN_MS = {"done": 200, "idle": 400, "transient": 400}

    print("== a box that cannot prove is not started again by the schedule ==")
    clear()
    open(os.path.join(tmp, "rc"), "w").write("78")
    app._start()
    spin(4, lambda: False)
    check(len(lines("runs")) == 1, f"exit 78 inside the window: no restart, from the schedule either ({len(lines('runs'))} runs)")
    check(not app._armed, "and the schedule switches itself off, saying why")
    os.remove(os.path.join(tmp, "rc"))
    app._stop()
    spin(2, lambda: not app.workers)

    print("== Stop hands the claimed block back ==")
    clear()
    open(files["secs"], "w").write("30")
    set_window(time.time() + 3600)
    app.block_times = {}
    app._start()
    check(spin(5, lambda: len(lines("runs")) >= 1 and app.workers and app.workers[0].activity.holding),
          "a block is claimed and being proved")
    held = app.workers[0].activity.holding if app.workers else None
    app._stop()
    check(spin(10, lambda: lines("released") == [str(held)] and not app.workers),
          f"Stop hands block {held} back ({lines('released')})")
    check(not lines("finished"), "and that block was not finished, so it is not on the board")
    app.destroy()

    if CONTROL:
        if fails >= 2:
            print(f"CONTROL OK: {fails} checks fail without the schedule gate and the release, as they must")
            return 0
        print(f"CONTROL BROKEN: only {fails} failed")
        return 1
    print("PASS" if fails == 0 else f"FAIL ({fails})")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
