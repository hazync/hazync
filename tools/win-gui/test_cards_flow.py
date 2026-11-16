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
block = 144263 + {"GPU-sml": 1, "1": 1}.get(os.environ.get("CUDA_VISIBLE_DEVICES") or "", 0)
if (os.environ.get("STAND_IN_DIE_ON") and os.environ.get("CUDA_VISIBLE_DEVICES") == os.environ["STAND_IN_DIE_ON"]
        and sys.argv[1:2] != ["fold"]):
    say("FATAL: guest id mismatch — every proof this worker makes will be REJECTED.")
    sys.exit(78)
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



def _started(app, secs=30):
    """Press Start and wait for its checks, which run off the window's thread (review of c881d582, M2)."""
    import time as _t
    app._start()
    end = _t.time() + secs
    while getattr(app, "_starting", False) and _t.time() < end:
        app.update()
        _t.sleep(0.02)

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
        _started(app)
        check(len(app.workers) == 2, f"Start launched one worker per card (got {len(app.workers)})")
        got = spin(app, 15, lambda: given(1) and given(2))
        check(bool(got), "both children started and reported what they were given")
        e1, e2 = given(1) or {}, given(2) or {}
        check((e1.get("CUDA_VISIBLE_DEVICES"), e2.get("CUDA_VISIBLE_DEVICES")) == ("GPU-big", "GPU-sml"),
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
        _started(app)
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

        print("── one card unticked: it is left alone ──")
        machine["cards"] = TWO
        check(not app._card_vars, "a one-card machine is offered no choice of cards")
        app._show_cards(TWO)
        check(sorted(app._card_vars) == ["GPU-big", "GPU-sml"]
              and all(v.get() for v in app._card_vars.values()),
              "two cards: a tick box for each, both ticked to begin with")
        try:
            os.remove(os.path.join(base, "bundles_1", "env.json"))
        except OSError:
            pass
        app._card_vars["GPU-big"].set(False)
        app._toggle_card("GPU-big")
        check(app.cards_off == {"GPU-big"}, "unticking the 4090 is remembered")
        import firstrun
        check(firstrun.load_config().get("cards_off") == ["GPU-big"], "and saved, so it survives a restart")
        _started(app)
        check(len(app.workers) == 1 and app.workers[0].label == "RTX 3060 Ti",
              f"Start launches one worker, on the card still ticked ({[w.label for w in app.workers]})")
        spin(app, 15, lambda: given(1))
        e1 = given(1) or {}
        check(e1.get("CUDA_VISIBLE_DEVICES") == "GPU-sml" and e1.get("HAZYNC_SEG_PO2") == "18",
              f"⛔ the CHILD PROCESS sees only the 3060 Ti: {e1.get('CUDA_VISIBLE_DEVICES')!r}")
        check("Left alone, as you chose: RTX 4090" in app.feed.get("1.0", "end"),
              "and the feed says which card was left alone")
        stop_all(app)
        app._card_vars["GPU-sml"].set(False)
        app._toggle_card("GPU-sml")
        check(app._card_vars["GPU-sml"].get() and app.cards_off == {"GPU-big"}
              and "At least one" in app.v_cards_note.get(),
              "⛔ the last card cannot be unticked, and the page says why")

        print("── one card stops on an error, the other keeps going: Home still shows both ──")
        app.cards_off = set()
        os.environ["STAND_IN_DIE_ON"] = "GPU-sml"
        try:
            _started(app)
            got = spin(app, 15, lambda: len(app.workers) == 1 and "stopped" in app.v_sub.get())
        finally:
            os.environ.pop("STAND_IN_DIE_ON", None)
        sub = app.v_sub.get()
        check(got and "RTX 3060 Ti:  stopped" in sub and "RTX 4090:" in sub,
              f"⛔ the stopped card keeps its line on Home and says it stopped: {sub!r}")
        check(app.v_head.get().startswith("Proving on 1 graphics card") and "1 stopped" in app.v_head.get(),
              f"and the headline counts it: {app.v_head.get()!r}")
        check(any(t.startswith("RTX 3060 Ti: misconfigured") for _, t, _tag in app._events),
              "the feed names the card that stopped")
        check(len(app.workers) == 1 and app.workers[0].label == "RTX 4090", "only that card stopped")
        stop_all(app)

        print("── the folder beside a prover stops with it when its card stops ──")
        app.also_fold.set(True)
        os.environ["STAND_IN_DIE_ON"] = "GPU-sml"
        try:
            _started(app)
            got = spin(app, 15, lambda: "stopped" in app.v_sub.get()
                       and not any(w.label == "RTX 3060 Ti" for w in app.workers))
        finally:
            os.environ.pop("STAND_IN_DIE_ON", None)
            app.also_fold.set(False)
        check(got and {w.label for w in app.workers} == {"RTX 4090"},
              f"⛔ the 3060 Ti's folder was stopped with its prover; the 4090's pair runs on "
              f"({sorted((w.label, w.job) for w in app.workers)})")
        stop_all(app)

        print("── a CUDA prover and only a card it cannot use: refused before anything starts (M1) ──")
        from tkinter import messagebox as _mb
        asked, errors = [], []
        real_ask, real_err = hazync_gui.messagebox.askyesno, hazync_gui.messagebox.showerror
        hazync_gui.messagebox.askyesno = lambda title, msg, **kw: (asked.append(msg) or answer["yes"])
        hazync_gui.messagebox.showerror = lambda title, msg, **kw: errors.append(msg)
        answer = {"yes": False}
        maxwell_only = [{"index": 0, "uuid": "GPU-mxw", "name": "NVIDIA GeForce GTX 970", "vram_mb": 4096,
                         "driver": "566.14", "cc_major": 5, "cc_minor": 2}]
        machine["cards"] = maxwell_only
        app.force_gpu.set(True)
        cpu_host = os.path.join(tmp, "cpu", "host.exe")
        os.makedirs(os.path.dirname(cpu_host), exist_ok=True)
        open(cpu_host, "wb").write(b"MZ")
        real_classify = supervisor.classify_host
        _info = lambda kind: {"kind": kind, "needs_driver": kind == "cuda", "driver_present": True,  # noqa: E731
                              "dlls": ["nvcuda.dll"] if kind == "cuda" else ["kernel32.dll"], "size": 2, "startable": True}
        supervisor.classify_host = lambda path: _info("cpu" if path == cpu_host else "cuda")
        app.host_var.set(os.path.join(tmp, "cuda-host.exe"))
        app.host_cpu_var.set("")
        _started(app)
        check(not app.workers and errors and "5.2" in errors[-1] and "no CPU prover" in errors[-1],
              f"⛔ one compute 5.x card, opted in, no CPU prover: nothing starts, and it says why ({errors[-1:]})")
        app.host_cpu_var.set(cpu_host)
        _started(app)
        check(not app.workers and asked and "processor" in asked[-1].lower() and "24 minutes" in asked[-1],
              "with a CPU prover: it is OFFERED, with the measured cost; declined, nothing starts")
        answer["yes"] = True
        try:
            os.remove(os.path.join(base, "bundles_1", "env.json"))
        except OSError:
            pass
        real_env = supervisor.worker_env
        seen = {}
        supervisor.worker_env = lambda *a, **kw: (seen.update(kw) or real_env(*a, **kw))
        try:
            _started(app)
            spin(app, 15, lambda: app.workers and (given(1) or {}).get("HAZYNC_GPU_LOCK"))
        finally:
            supervisor.worker_env = real_env
        check(app.host_var.get() == cpu_host and len(app.workers) == 1 and app.workers[0].card is None,
              f"accepted: it switches to the CPU prover and starts one unpinned worker ({app.host_var.get()})")
        check(not seen.get("spark_min_major"), f"⛔ and never asks sppark for a floor below 6 ({seen.get('spark_min_major')})")
        check(any("processor" in t.lower() and "24 minutes" in t for _, t, _tag in app._events),
              "Home's feed says it is on the processor, and how slow that is, measured")
        stop_all(app)
        supervisor.classify_host = real_classify
        hazync_gui.messagebox.askyesno, hazync_gui.messagebox.showerror = real_ask, real_err
        app.force_gpu.set(False)
        machine["cards"] = TWO
        app.host_var.set("")

        print("── 'try my old card anyway': the old card can be switched off too ──")
        old = {"index": 2, "uuid": "GPU-old", "name": "NVIDIA GeForce GTX 1060", "vram_mb": 6144,
               "driver": "566.14", "cc_major": 6, "cc_minor": 1}
        machine["cards"] = TWO + [old]
        app.cards_off = set()
        app.force_gpu.set(False)
        app._show_cards(machine["cards"])
        check("GPU-old" not in app._card_vars, "not forced: the old card has no live tick box, and gets no worker")
        app.force_gpu.set(True)
        app._show_cards(machine["cards"])
        check("GPU-old" in app._card_vars and app._card_vars["GPU-old"].get(),
              "⛔ forced: the old card is in the list with a LIVE box — it is about to get a worker")
        _started(app)
        check(sorted(w.label for w in app.workers) == ["GTX 1060", "RTX 3060 Ti", "RTX 4090"],
              f"and Start agrees with the list: {sorted(w.label for w in app.workers)}")
        stop_all(app)
        maxwell = {"index": 3, "uuid": "GPU-mxw", "name": "NVIDIA GeForce GTX 970", "vram_mb": 4096,
                   "driver": "566.14", "cc_major": 5, "cc_minor": 2}
        machine["cards"] = TWO + [old, maxwell]
        app._show_cards(machine["cards"])
        _started(app)
        check(sorted(w.label for w in app.workers) == ["GTX 1060", "RTX 3060 Ti", "RTX 4090"],
              f"⛔ forced, a compute 5.x card still gets NO worker (no GPU code for it, #727): "
              f"{sorted(w.label for w in app.workers)}")
        stop_all(app)
        machine["cards"] = TWO + [old]
        app._show_cards(machine["cards"])
        app._card_vars["GPU-old"].set(False)
        app._toggle_card("GPU-old")
        _started(app)
        check(sorted(w.label for w in app.workers) == ["RTX 3060 Ti", "RTX 4090"],
              f"⛔ unticked, it gets no worker even while forced: {sorted(w.label for w in app.workers)}")
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
