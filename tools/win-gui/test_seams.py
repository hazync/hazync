#!/usr/bin/env python3
"""Where the Windows-app branch meets #737 (scheduled proving, block release): the seams, through real child processes.

    python3 test_seams.py             # must PASS
    python3 test_seams.py --control   # the seams as the first merge left them: MUST FAIL

Review of 59575b40 (M1-M3, L1, L2, L4):
  * a proving window that opens after Start fell back to the CPU prover runs the CPU prover, and its checks run OFF the
    window's thread (M2);
  * closing waits for a block hand-back that outlasts the stop budget, and names a block it could not hand back (M1);
  * one card stopping on an error leaves the schedule armed while another card still proves (M3);
  * a PC known to have several cards, where nvidia-smi does not answer once, starts no unpinned worker (M2);
  * Stop pressed while Start's checks run cancels the start (L4);
  * a block another worker of ours holds again is never released (L1);
  * a worker that prints "claimed block N" and exits at once still has N handed back (L2: output applied before reap).

The children are stand-ins that print the worker's lines and record what they were given; no GPU, network or key.
"""
import os
import sys
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
CONTROL = "--control" in sys.argv
fails = []

STAND_IN = r'''
import os, sys, time
home = os.environ["HAZYNC_HOME"]
if sys.argv[1:2] == ["release"]:
    if sys.argv[3:4] != ["c1a1000000001807"]:     # #753: a release must name the claim the run printed
        print("usage: hazync release <block> <claim>", flush=True)
        sys.exit(2)
    time.sleep(float(os.environ.get("REL_SLEEP", "0")))
    open(os.path.join(home, "released"), "a").write(sys.argv[2] + "\n")
    print(f"released block {sys.argv[2]}", flush=True)
    sys.exit(0)
open(os.path.join(home, "hosts"), "a").write(os.environ.get("HAZYNC_HOST", "?") + " "
                                             + (os.environ.get("CUDA_VISIBLE_DEVICES") or "-") + "\n")
die_on = os.environ.get("STAND_IN_DIE_ON")
if die_on and os.environ.get("CUDA_VISIBLE_DEVICES") == die_on:
    print("FATAL: guest id mismatch", flush=True)
    sys.exit(78)
print("claimed block 1807 (claim c1a1000000001807; yours for 60 min; if you stop, it reopens by itself)", flush=True)
if os.environ.get("EXIT_AFTER_CLAIM"):
    sys.exit(3)
time.sleep(60)
'''


class Win:
    """A proving window that opens at `opens` and stays open an hour."""
    def __init__(self, opens):
        self.opens, self.end = opens, opens + 3600
        self.start = __import__("datetime").time(23, 0)

    def is_open(self, ts, *a):
        return self.opens <= ts < self.end

    def ends_at(self, ts, *a):
        return self.end if self.is_open(ts) else None

    def seconds_left(self, ts, *a):
        return max(0.0, self.end - ts) if self.is_open(ts) else None

    def next_start(self, ts, *a):
        return self.opens

    def describe(self):
        return "a test window"


def check(ok, what):
    print(("  ok   " if ok else "  FAIL ") + what)
    if not ok:
        fails.append(what)


def main():
    tmp = tempfile.mkdtemp()
    os.environ["HAZYNC_HOME"] = tmp
    try:
        import hazync_gui
        import supervisor
    except Exception as e:             # noqa: BLE001
        print(f"  SKIP cannot import the window: {e}")
        return 0
    hazync_gui.App.refresh = lambda self: None
    hazync_gui.App.rescan = lambda self: None
    hazync_gui.SCHEDULE_TICK_MS = 150
    hazync_gui.schedule.UNKNOWN_MIN_S = 1
    try:
        app = hazync_gui.App()
    except Exception as e:             # noqa: BLE001
        if "display" in str(e).lower():
            print(f"  SKIP no display: {e}")
            return 0
        raise
    app.withdraw()

    def spin(secs, until=lambda: False):
        end = time.time() + secs
        while time.time() < end:
            app.update()
            if until():
                return True
            time.sleep(0.02)
        return until()

    def rd(k):
        p = os.path.join(tmp, k)
        return open(p).read().split("\n")[:-1] if os.path.exists(p) else []

    def reset(*names):
        for n in names:
            try:
                os.remove(os.path.join(tmp, n))
            except OSError:
                pass

    def quiet():
        for w in list(app.workers):
            w.stopping = True
            w.popen.kill()
        spin(10, lambda: not app.workers)
        for h in app._respawns.values():
            app.after_cancel(h)
        app._respawns.clear()
        app._armed = False
        app._problem = ""

    si = os.path.join(tmp, "si.py")
    open(si, "w").write(STAND_IN)
    threads = []
    TWO = [{"index": 0, "uuid": "GPU-a", "name": "NVIDIA GeForce RTX 4090", "vram_mb": 24564, "driver": "1",
            "cc_major": 8, "cc_minor": 9},
           {"index": 1, "uuid": "GPU-b", "name": "NVIDIA GeForce RTX 3080", "vram_mb": 10240, "driver": "1",
            "cc_major": 8, "cc_minor": 6}]
    machine = {"cards": []}

    def gpu_cards():
        threads.append(threading.current_thread() is threading.main_thread())
        return list(machine["cards"])

    def classify(p):
        threads.append(threading.current_thread() is threading.main_thread())
        kind = "cpu" if "cpu" in os.path.basename(str(p)) else "cuda"
        return {"kind": kind, "needs_driver": kind == "cuda", "driver_present": True, "dlls": ["x.dll"],
                "size": 1, "startable": True}
    supervisor.gpu_cards = gpu_cards
    supervisor.classify_host = classify
    supervisor.preflight = lambda *a, **kw: ([], [])
    cuda = os.path.join(tmp, "host_cuda.exe")
    cpu = os.path.join(tmp, "host_cpu.exe")
    for p in (cuda, cpu):
        open(p, "w").write("x")
    app.worker_var.set(si)
    app.ident_var.set(tmp)
    app.mode.set("auto")
    app.nworkers.set(1)
    app._setup_ok, app._steps = True, [object()]
    app.host_var.set(cuda)
    hazync_gui.messagebox.askokcancel = lambda *a, **k: True
    warned = []
    hazync_gui.messagebox.showwarning = lambda *a, **k: warned.append(a)

    try:
        print("── a window that opens after the CPU fallback runs the CPU prover, checked off the Tk thread (M2) ──")
        app.sched = dict(hazync_gui.SCHEDULE_DEFAULT, on=True, keep_awake=False)
        app._window = lambda: Win(time.time() + 1.5)
        win = Win(time.time() + 1.5)
        app._window = lambda: win
        app._start(host=cpu)
        spin(5, lambda: not app._starting)
        check(app._armed and not app.workers, "armed in a closed window, nothing started yet")
        threads.clear()
        if CONTROL:
            app._start = lambda host=None, scheduled=False: app._launch()      # the first merge: _launch on the Tk thread
        got = spin(8, lambda: bool(rd("hosts")))
        check(got and rd("hosts")[0].startswith(cpu), f"the window opened on the CPU prover ({rd('hosts')})")
        check(threads and not any(threads), f"⛔ its checks ran OFF the window's thread ({threads})")
        quiet()
        if CONTROL:
            del app._start
        reset("hosts")

        print("── closing waits for a hand-back that outlasts the stop budget, and names one it could not do (M1) ──")
        app.sched = dict(hazync_gui.SCHEDULE_DEFAULT, on=False)
        app.CLOSE_WAIT_S = 1
        os.environ["REL_SLEEP"] = "2.5"
        app._start()
        spin(10, lambda: app.workers and app.workers[0].activity.holding)
        check(bool(app.workers and app.workers[0].activity.holding), "a worker holds block 1807")
        # (the close itself is the LAST case: it destroys the window)

        print("── L1: a block another of our workers holds again is not released ──")
        w0 = app.workers[0]

        class Twin:
            activity = type("A", (), {"holding": "1807"})()
        twin = Twin()
        app.workers.append(twin)
        sent_before = getattr(app, "_releases", 0)
        if not CONTROL:
            app._release(w0)
        # (control: the first merge had no such guard; not calling it leaves the check to fail on holding)
        check(getattr(app, "_releases", 0) == sent_before and w0.activity.holding is None,
              "⛔ not sent while another worker here holds that block")
        app.workers.remove(twin)
        w0.activity.holding, w0.activity.holding_claim = "1807", "c1a1000000001807"

        print("── #753: a release whose claim id is unknown is not sent, and closing never waits on it ──")
        w0.activity.holding_claim = None
        before = dict(getattr(app, "_releasing", {}))
        app._release(w0)
        check(getattr(app, "_releasing", {}) == before and getattr(app, "_releases", 0) == sent_before,
              "⛔ not counted among the releases Close waits for")
        check(any("1,807" in t or "1807" in t for _, t, _tag in app._events[-2:]), "and the feed says so")
        w0.activity.holding, w0.activity.holding_claim = "1807", "c1a1000000001807"

        print("── Stop while Start's checks run cancels the start (L4) ──")
        quiet()
        real_pf = supervisor.preflight
        supervisor.preflight = lambda *a, **kw: (time.sleep(1.0) or ([], []))
        app._start()
        spin(0.2)
        check(str(app.stop_btn["state"]) == "normal", "Stop is live while the checks run")
        app._stop()
        spin(3)
        supervisor.preflight = real_pf
        check(not app.workers, f"⛔ and nothing starts when they come back ({len(app.workers)} workers)")

        print("── one card stops on an error: the schedule stays armed for the card still proving (M3) ──")
        machine["cards"] = TWO
        app.host_var.set(cuda)
        app.sched = dict(hazync_gui.SCHEDULE_DEFAULT, on=True, keep_awake=False)
        win2 = Win(time.time() - 10)
        app._window = lambda: win2
        os.environ["STAND_IN_DIE_ON"] = "GPU-b"
        try:
            app._start()
            spin(10, lambda: len(app.workers) == 1 and "RTX 3080" in str(app._card_notes))
        finally:
            os.environ.pop("STAND_IN_DIE_ON", None)
        check(len(app.workers) == 1 and app._armed,
              f"⛔ the 4090 proves on and the window still gates it ({len(app.workers)} worker, armed={app._armed})")
        quiet()

        print("── several cards known, nvidia-smi silent once: no unpinned worker (M2) ──")
        app.sched = dict(hazync_gui.SCHEDULE_DEFAULT, on=False)
        app._last_cards = list(TWO)
        machine["cards"] = []
        reset("hosts")
        app._start()
        spin(3)
        check(not app.workers, f"⛔ nothing started unpinned ({rd('hosts')})")
        check(any("did not answer" in t for _, t, _tag in app._events), "and the feed says to try again")
        machine["cards"] = TWO
        quiet()

        print("── L2: 'claimed block N' then an exit at once: N is still handed back ──")
        machine["cards"] = []
        app._last_cards = []
        reset("released")
        os.environ["EXIT_AFTER_CLAIM"] = "1"
        os.environ["REL_SLEEP"] = "0"
        try:
            app._start()
            spin(3, lambda: app.workers)
            for w in app.workers:
                w.stopping = True       # as Stop would: an exit while stopping hands the block back
            spin(8, lambda: "1807" in rd("released"))
        finally:
            os.environ.pop("EXIT_AFTER_CLAIM", None)
        check("1807" in rd("released"), f"⛔ the block was handed back ({rd('released')})")
        quiet()

        print("── closing: waits for the slow hand-back, then names nothing (M1) ──")
        reset("released")
        os.environ["REL_SLEEP"] = "2.5"
        app._start()
        spin(10, lambda: app.workers and app.workers[0].activity.holding)
        if CONTROL:
            app.RELEASE_WAIT_S = 0
        at_close = {}
        real_destroy = app.destroy
        app.destroy = lambda: (at_close.update(released=rd("released")), real_destroy())
        t0 = time.time()
        app._on_close()
        closed = False
        while time.time() - t0 < 20:
            try:
                app.update()
                app.winfo_exists()
            except Exception:          # noqa: BLE001
                closed = True
                break
            time.sleep(0.02)
        check(closed and "1807" in at_close.get("released", []) and not warned,
              f"⛔ the window closed only after block 1807 was handed back ({time.time() - t0:.1f} s, "
              f"released when it closed {at_close.get('released')}, warnings {warned})")

        print("── closing: a hand-back that does not finish in its budget is NAMED (M1) ──")
        app = hazync_gui.App()
        app.withdraw()
        app.worker_var.set(si)
        app.ident_var.set(tmp)
        app.mode.set("auto")
        app._setup_ok, app._steps = True, [object()]
        app.host_var.set(cuda)
        app.sched = dict(hazync_gui.SCHEDULE_DEFAULT, on=False)
        app.CLOSE_WAIT_S, app.RELEASE_WAIT_S = 0.5, 1.0
        reset("released")
        os.environ["REL_SLEEP"] = "6"
        app._start()
        spin(10, lambda: app.workers and app.workers[0].activity.holding)
        warned.clear()
        t0 = time.time()
        app._on_close()
        closed = False
        while time.time() - t0 < 20:
            try:
                app.update()
                app.winfo_exists()
            except Exception:          # noqa: BLE001
                closed = True
                break
            time.sleep(0.02)
        said = " ".join(str(a) for a in warned)
        check(closed and time.time() - t0 < 5 and ("1,807" in said or "1807" in said),
              f"⛔ closed within its budget and named the block not handed back ({time.time() - t0:.1f} s: {said[:120]})")
    finally:
        os.environ.pop("REL_SLEEP", None)
        try:
            for w in list(app.workers):
                w.popen.kill()
            app.destroy()
        except Exception:              # noqa: BLE001
            pass

    print()
    if CONTROL:
        if len(fails) >= 3:
            print(f"CONTROL OK: {len(fails)} checks fail on the seams as the first merge left them")
            return 0
        print(f"CONTROL BROKEN: only {len(fails)} failed")
        return 1
    print("PASS" if not fails else f"FAIL {len(fails)}")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
