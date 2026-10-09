#!/usr/bin/env python3
"""Press Start on the Home page and watch it tell the truth, through the real process plumbing.

⛔ WHY THIS EXISTS. test_activity.py proves the READER is right; test_gui_builds.py proves the
window BUILDS. Neither proves the two are connected — that a line printed by a child process
reaches the headline, moves the bar, lands in the feed and is counted. That path crosses a pipe, a
reader thread and a queue back to the main thread, and every one of those has silently dropped
output in this window before.

So this starts a REAL child process through the window's own `_spawn`, exactly as Start does. The
child is a stand-in for the worker that prints the worker's real lines; nothing touches the network,
the coordinator or anyone's identity.

    test_home_flow.py          # needs a display; SKIPS without one, like test_gui_builds.py
"""
import os
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

fails = []

# ⚠ ONE LINE, THEN A LONG SILENCE, THEN THE REST. The middle of a prove is what a person stares at,
# so the test has to be able to look at the window WHILE the child is mid-prove — and a child that
# prints everything at once gives it no such moment.
STAND_IN = r'''
import sys, time
def say(s):
    print(s, flush=True)
say("claimed block 144263 (yours for 60 min; if you stop, it reopens by itself)")
say("range [144263..144263] (bridge): executed, 61 segments at po2 21 -- proving")
say("    segment 30/61  101s elapsed, ~104s left")
time.sleep(4)
say("    segment 61/61  205s elapsed, ~0s left")
say("    assembled 61 segment receipts in 43s")
say("proved range [144263..144263] from bridge bundle in 263.0s -> range_144263.hzk")
say("✓ range 144263: the coordinator re-verified your proof and put it on the board as 'tester'.")
time.sleep(1)
'''


def check(ok, what):
    print(("  ok   " if ok else "  FAIL ") + what)
    if not ok:
        fails.append(what)


def spin(app, secs, until=None):
    end = time.time() + secs
    while time.time() < end:
        app.update()
        if until and until():
            return True
        time.sleep(0.02)
    return bool(until and until())


def main():
    tmp = tempfile.mkdtemp()
    os.environ["HAZYNC_HOME"] = tmp      # never the real ~/.hazync of whoever runs this
    try:
        import tkinter  # noqa: F401
        import hazync_gui
    except Exception as e:             # noqa: BLE001
        print(f"  SKIP cannot import the window: {e}")
        return 0
    # ⚠ The window must not go and fetch things, scan the disk or call the coordinator while this
    # runs: the test is about one path, and a download racing it would make a failure unreadable.
    hazync_gui.App.refresh = lambda self: None
    hazync_gui.App.rescan = lambda self: None
    try:
        app = hazync_gui.App()
    except Exception as e:             # noqa: BLE001
        if "display" in str(e).lower():
            print(f"  SKIP no display: {e}")
            return 0
        raise

    stand_in = os.path.join(tmp, "stand_in_worker.py")
    with open(stand_in, "w", encoding="utf-8") as fh:
        fh.write(STAND_IN)
    try:
        app.withdraw()
        app.worker_var.set(stand_in)
        app.ident_var.set(tmp)
        app._setup_ok, app._steps = True, [object()]
        app._sync_buttons()

        print("── before Start ──")
        app._home_refresh()
        check(app.v_head.get() == "Ready when you are", f"headline: {app.v_head.get()!r}")
        check(str(app.start_btn["state"]) == "normal" and bool(app.start_btn.winfo_manager()),
              "Start is live and on screen")
        check(not app.stop_btn.winfo_manager(), "Stop is not on screen while nothing runs")

        print("── while it proves ──")
        base = os.path.join(tmp, "gui-workers")
        os.makedirs(base, exist_ok=True)
        from pathlib import Path
        app._spawn(1, Path(base), "run", ())
        app._sync_buttons()
        got = spin(app, 15, lambda: "Piece 30 of 61" in app.v_sub.get())
        check(got, f"the child's own progress line reached the page: {app.v_sub.get()!r}")
        check("144,263" in app.v_head.get(), f"the headline names the block: {app.v_head.get()!r}")
        check(400 < int(float(app.bar["value"])) < 600, f"the bar is about half way ({app.bar['value']})")
        check("Running for" in app.v_meta.get() and "last heard" in app.v_meta.get(),
              f"it says how long it has run and when it last heard: {app.v_meta.get()!r}")
        check(bool(app.stop_btn.winfo_manager()) and not app.start_btn.winfo_manager(),
              "⛔ Stop has taken Start's place — one button, not a live one beside a dead one")
        check(app._landed["run"] == 0, "nothing is counted as landed while it is still proving")

        print("── when the block lands, and after the child exits ──")
        got = spin(app, 20, lambda: app._landed["run"] == 1)
        check(got, "the accepted block is counted, once")
        got = spin(app, 15, lambda: not app.workers)
        check(got, "the window notices the child has exited")
        feed = app.feed.get("1.0", "end")
        for want in ("Took block 144,263", "Finished the proof of block 144,263",
                     "checked and accepted"):
            check(want in feed, f"the feed says: {want!r}")
        check("segment" not in feed.lower() and "piece 30" not in feed.lower(),
              "the feed lists what happened, not every progress line")
        check(bool(app.start_btn.winfo_manager()) and str(app.start_btn["state"]) == "normal",
              "Start is back once nothing is running")
        check("1 block proven" in app.v_meta.get(), f"the session count stays: {app.v_meta.get()!r}")
    finally:
        for w in list(app.workers):
            try:
                w.popen.kill()
            except Exception:          # noqa: BLE001
                pass
        try:
            app.destroy()
        except Exception:              # noqa: BLE001
            pass

    print()
    if fails:
        print(f"FAIL {len(fails)}")
        return 1
    print("Start to landed block, through a real child process: the Home page told the truth")
    return 0


if __name__ == "__main__":
    sys.exit(main())
