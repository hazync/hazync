"""Home says "Preparing the graphics card" while a new card compiles its GPU code (Xesktop, 11-13).

    python test_gpuprep.py            # must PASS
    python test_gpuprep.py --control  # gpuprep.to_prepare reduced to "never": MUST FAIL
"""
import os
import sys
import tempfile

import activity
import gpuprep

CONTROL = "--control" in sys.argv
if CONTROL:
    gpuprep.to_prepare = lambda cards, cuda, prepared: []
    print("CONTROL: the message never shown -- the checks below MUST fail")
fails = 0


def check(ok, what):
    global fails
    print(("  ok   " if ok else "  FAIL ") + what)
    fails += 0 if ok else 1


PRO6000 = {"index": 0, "name": "NVIDIA RTX PRO 6000 Blackwell", "driver": "581.15", "cc_major": 12, "cc_minor": 0}
RTX4090 = {"index": 0, "name": "NVIDIA GeForce RTX 4090", "driver": "581.15", "cc_major": 8, "cc_minor": 9}

# the rule
check(gpuprep.to_prepare([PRO6000], True, []) == [PRO6000], "an RTX PRO 6000 (12.0) on a GPU run: preparing")
check(gpuprep.to_prepare([RTX4090], True, []) == [] or CONTROL, "an RTX 4090 (8.9) has native code: not preparing")
check(gpuprep.to_prepare([PRO6000], False, []) == [] or CONTROL, "a CPU run: not preparing")
check(gpuprep.to_prepare([PRO6000], True, [gpuprep.card_key(PRO6000)]) == [] or CONTROL,
      "a card already prepared on this machine: not again")

# the parser marks the first piece
a = activity.Activity("run")
a.feed("claimed block 1339 (lease 3600s)")
check(not a.pieces, "claimed, no piece yet: pieces is False")
a.feed("executed, 41 segments")
check(not a.pieces, "split into pieces but none proved: still False")
a.feed("segment 1/41  95s elapsed, ~3800s left")
check(a.pieces, "the first piece reported: pieces is True")

# the window: Home shows it until the first piece, then remembers the card
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
    w = hazync_gui.Worker(1, None, os.devnull, "run")
    app.workers = [w]
    app._gpu_prep = gpuprep.to_prepare([PRO6000], True, app.cfg.get("gpu_prepared"))
    app._on_worker_line((w, "claimed block 1339 (lease 3600s)"))
    app._home_refresh()
    check(gpuprep.HEADLINE in app.v_head.get() and "10 minutes" in app.v_sub.get(),
          f"Home, block claimed, nothing moving: {app.v_head.get()!r}")
    app._on_worker_line((w, "segment 1/41  95s elapsed, ~3800s left"))
    app._home_refresh()
    check(gpuprep.HEADLINE not in app.v_head.get() and "Piece 1" in app.v_sub.get(),
          f"Home, first piece: the message gives way to the progress ({app.v_sub.get()!r})")
    import firstrun
    saved = firstrun.load_config().get("gpu_prepared") or []
    check(gpuprep.card_key(PRO6000) in saved, f"the card is remembered for next time ({saved})")
    app.workers = []
    app.destroy()

if CONTROL:
    if fails >= 3:
        print(f"CONTROL OK: {fails} checks fail without the message, as they must")
        sys.exit(0)
    print(f"CONTROL BROKEN: only {fails} failed")
    sys.exit(1)
print("PASS" if fails == 0 else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
