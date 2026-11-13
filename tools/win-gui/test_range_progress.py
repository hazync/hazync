"""A range shows how far through it the run is, and a finished range is not started again (Xesktop, 11-13).

On 1,342-2,000 the per-block bar filled and emptied every 32 s and read as stalling, with nothing saying 16 of 659
were done. And with the window restarting finished workers, a finished RANGE would be started again every two
seconds for ever (every block proven, exit 0, again).

    python test_range_progress.py            # must PASS
    python test_range_progress.py --control  # Activity.set_range a no-op: MUST FAIL
"""
import os
import sys
import tempfile

import activity

CONTROL = "--control" in sys.argv
if CONTROL:
    activity.Activity.set_range = lambda self, rng: None
    print("CONTROL: no range tracking -- the checks below MUST fail")
fails = 0


def check(ok, what):
    global fails
    print(("  ok   " if ok else "  FAIL ") + what)
    fails += 0 if ok else 1


a = activity.Activity("run")
a.set_range("1342-2000")
for line in ("✓ range 1342: the coordinator re-verified your proof and put it on the board as 'Windows App'.",
             "✓ range 1343: the coordinator re-verified your proof and put it on the board as 'Windows App'.",
             "✓ range 1344 was already proved by someone else — nothing to do.",
             "✓ range 7: the coordinator re-verified your proof and put it on the board as 'x'."):
    a.feed(line)
check(a.range_progress == (3, 659), f"3 of 659: two proved here, one by someone else, one outside ignored ({a.range_progress})")
a.feed("✓ range 1343: the coordinator re-verified your proof and put it on the board as 'Windows App'.")
check(a.range_progress == (3, 659), "the same block twice counts once")
b = activity.Activity("run")
check(b.range_progress is None, "no range asked for: no range line")

os.environ["HAZYNC_HOME"] = tempfile.mkdtemp()
try:
    import hazync_gui
    hazync_gui.App.refresh = lambda self: None
    hazync_gui.App.rescan = lambda self: None
    app = hazync_gui.App()
except Exception as e:                  # noqa: BLE001
    print(f"  SKIP the window part: {e}")
    app = None
if app is not None:
    app.withdraw()
    w = hazync_gui.Worker(1, None, os.devnull, "run", extra=("1342-2000",))
    w.activity.set_range("1342-2000")
    app.workers = [w]
    for blk in (1342, 1343):
        app._on_worker_line((w, f"✓ range {blk}: the coordinator re-verified your proof and put it on the board as 'W'."))
    app._on_worker_line((w, "claimed block 1344 (yours for 60 min)"))
    app._home_refresh()
    check("2 of 659 done in 1,342–2,000" in app.v_head.get(), f"Home says how far through the range: {app.v_head.get()!r}")
    app.workers = []
    app._again(w, "done")
    check(not app._respawns and "Range 1,342–2,000 finished" in app.feed.get("1.0", "end"),
          "a finished range is not started again, and says it finished")
    plain = hazync_gui.Worker(1, None, os.devnull, "run")
    app._again(plain, "done")
    check(len(app._respawns) == 1, "a plain Start (no range) still takes the next block")
    app._stop()
    app.destroy()

if CONTROL:
    if fails >= 2:
        print(f"CONTROL OK: {fails} checks fail without range tracking, as they must")
        sys.exit(0)
    print(f"CONTROL BROKEN: only {fails} failed")
    sys.exit(1)
print("PASS" if fails == 0 else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
