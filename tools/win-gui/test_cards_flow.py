#!/usr/bin/env python3
"""Press Start on a two-card machine and check each child process really got its own card.

⛔ WHY THIS EXISTS. test_cards.py proves the PLAN is right. It cannot prove the plan reaches the
process: between the decision and the prover sit `_start`, `_spawn`, `worker_env` and a real
`Popen`, and a card chosen but never put in the child's environment would pass every check there
while both workers piled onto one card — which is exactly the fault this change is for.

So this presses the real Start with two cards reported, lets it launch REAL child processes, and
has each child write down the environment it was actually given. The children are stand-ins that
print the worker's real lines; nothing touches a GPU, the network or anyone's identity.

⛔ WHAT IT STILL CANNOT SHOW: that two real cards prove at once. That needs a two-card machine.

    test_cards_flow.py         # needs a display; SKIPS without one, like test_home_flow.py
"""
import json
import os
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

fails = []

# Each child records what it was given, then proves "its" block slowly enough to be looked at.
STAND_IN = r'''
import json, os, sys, time
keep = ("CUDA_VISIBLE_DEVICES", "CUDA_DEVICE_ORDER", "HAZYNC_GPU_LOCK", "HAZYNC_SEG_PO2")
with open(os.path.join(os.environ["BUNDLE_DIR"], "env.json"), "w") as fh:
    json.dump({k: os.environ.get(k) for k in keep}, fh)
def say(s):
    print(s, flush=True)
block = 144263 + int(os.environ.get("CUDA_VISIBLE_DEVICES") or 0)
say(f"claimed block {block} (yours for 60 min; if you stop, it reopens by itself)")
say(f"range [{block}..{block}] (bridge): executed, 61 segments at po2 21 -- proving")
say("    segment 30/61  101s elapsed, ~104s left")
time.sleep(5)
say(f"✓ range {block}: the coordinator re-verified your proof and put it on the board as 'tester'.")
time.sleep(1)
'''

TWO = [{"index": 0, "uuid": "GPU-big", "name": "NVIDIA GeForce RTX 4090", "vram_mb": 24564,
        "driver": "566.14", "cc_major": 8, "cc_minor": 9},
       {"index": 1, "uuid": "GPU-sml", "name": "NVIDIA GeForce RTX 3060 Ti", "vram_mb": 8192,
        "driver": "566.14", "cc_major": 8, "cc_minor": 6}]


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


def stop_all(app):
    for w in list(app.workers):
        try:
            w.popen.kill()
        except Exception:              # noqa: BLE001
            pass
    spin(app, 10, lambda: not app.workers)


def main():
    tmp = tempfile.mkdtemp()
    os.environ["HAZYNC_HOME"] = tmp      # never the real ~/.hazync of whoever runs this
    try:
        import tkinter  # noqa: F401
        import hazync_gui
        import supervisor
    except Exception as e:             # noqa: BLE001
        print(f"  SKIP cannot import the window: {e}")
        return 0
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
    # ⚠ DATA, NOT BEHAVIOUR. What the machine is said to contain is replaced; deciding, starting,
    # building the environment and launching are all the window's own code.
    machine = {"cards": TWO}
    supervisor.gpu_cards = lambda: list(machine["cards"])
    supervisor.classify_host = lambda path: {"kind": "cuda", "needs_driver": True}
    supervisor.preflight = lambda *a, **kw: ([], [])
    base = os.path.join(tmp, "gui-workers")

    def given(i):
        try:
            with open(os.path.join(base, f"bundles_{i}", "env.json"), encoding="utf-8") as fh:
                return json.load(fh)
        except (OSError, ValueError):
            return None

    try:
        app.withdraw()
        app.worker_var.set(stand_in)
        app.ident_var.set(tmp)
        app.mode.set("auto")
        app.nworkers.set(1)
        app.po2.set("20")
        app._setup_ok, app._steps = True, [object()]

        print("── two cards, one worker asked for ──")
        app._start()
        check(len(app.workers) == 2, f"Start launched one worker per card (got {len(app.workers)})")
        got = spin(app, 15, lambda: given(1) and given(2))
        check(bool(got), "both children started and reported what they were given")
        e1, e2 = given(1) or {}, given(2) or {}
        check((e1.get("CUDA_VISIBLE_DEVICES"), e2.get("CUDA_VISIBLE_DEVICES")) == ("0", "1"),
              f"⛔ each CHILD PROCESS sees its own card: {e1.get('CUDA_VISIBLE_DEVICES')!r} "
              f"and {e2.get('CUDA_VISIBLE_DEVICES')!r}")
        check(e1.get("CUDA_DEVICE_ORDER") == e2.get("CUDA_DEVICE_ORDER") == "PCI_BUS_ID",
              "and both count cards the way nvidia-smi does")
        check(e1.get("HAZYNC_GPU_LOCK") and e2.get("HAZYNC_GPU_LOCK")
              and e1["HAZYNC_GPU_LOCK"] != e2["HAZYNC_GPU_LOCK"],
              "⛔ each child has its own lock, so neither waits for the other's card")
        check((e1.get("HAZYNC_SEG_PO2"), e2.get("HAZYNC_SEG_PO2")) == ("20", "18"),
              f"the 8 GB card got a smaller segment size than the 24 GB one "
              f"({e1.get('HAZYNC_SEG_PO2')}, {e2.get('HAZYNC_SEG_PO2')})")

        print("── and the Home page shows both ──")
        got = spin(app, 15, lambda: app.v_sub.get().count("Piece 30 of 61") == 2)
        check(got, f"a line for each card, each with its own progress: {app.v_sub.get()!r}")
        check(app.v_head.get() == "Proving on 2 graphics cards", f"headline: {app.v_head.get()!r}")
        sub = app.v_sub.get()
        check("RTX 4090:" in sub and "RTX 3060 Ti:" in sub and "144,263" in sub and "144,264" in sub,
              "each line names its card and the block that card is on")
        check(400 < int(float(app.bar["value"])) < 600, f"the bar is the two averaged ({app.bar['value']})")
        got = spin(app, 20, lambda: app._landed["run"] == 2)
        check(got, f"both blocks are counted when they land (got {app._landed['run']})")
        feed = app.feed.get("1.0", "end")
        check("on 2 graphics cards: RTX 3060 Ti, RTX 4090" in feed, "the feed says which cards were started")
        check("RTX 4090: Block 144,263 checked and accepted" in feed
              and "RTX 3060 Ti: Block 144,264 checked and accepted" in feed,
              "and says which card landed which block")
        stop_all(app)

        print("── one card: nothing about this machine changes ──")
        machine["cards"] = TWO[:1]
        for i in (1, 2):
            try:
                os.remove(os.path.join(base, f"bundles_{i}", "env.json"))
            except OSError:
                pass
        app._start()
        check(len(app.workers) == 1, f"one worker (got {len(app.workers)})")
        spin(app, 15, lambda: given(1))
        e1 = given(1) or {}
        check(e1.get("CUDA_VISIBLE_DEVICES") == os.environ.get("CUDA_VISIBLE_DEVICES")
              and e1.get("CUDA_DEVICE_ORDER") == os.environ.get("CUDA_DEVICE_ORDER"),
              "⛔ the child is not pinned: its card settings are whatever the machine already had")
        check(str(e1.get("HAZYNC_GPU_LOCK", "")).endswith("hazync-gpu.lock"), "and it uses the same lock as before")
        got = spin(app, 15, lambda: "Piece 30 of 61" in app.v_sub.get())
        check(got and "graphics cards" not in app.v_head.get() and "\n" not in app.v_sub.get(),
              f"the Home page reads as it always did: {app.v_head.get()!r}")
        stop_all(app)

        print("── the setting off: two cards, but left alone ──")
        machine["cards"] = TWO
        app.per_card.set(False)
        app._start()
        check(len(app.workers) == 1 and app.workers[0].card is None,
              "with 'use every graphics card' unticked, one unpinned worker, as before")
        stop_all(app)
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
    print("Two cards, two real child processes, each with its own card and lock")
    return 0


if __name__ == "__main__":
    sys.exit(main())
