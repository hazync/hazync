"""Start asks for a name when none is set (Xesktop, 11-13: the installed app proved block 1,339 as 'ghost:a3cb01').

The installer supplies the prover and the client, so setup is complete on first launch and the card holding the name
field never showed. Start now asks; Cancel starts nothing; a name given is set before the work starts.

    python test_name_on_start.py            # must PASS
    python test_name_on_start.py --control  # Start without the question: MUST FAIL
"""
import os
import sys
import tempfile

CONTROL = "--control" in sys.argv
fails = 0


def check(ok, what):
    global fails
    print(("  ok   " if ok else "  FAIL ") + what)
    fails += 0 if ok else 1


def main():
    os.environ["HAZYNC_HOME"] = tempfile.mkdtemp()
    try:
        import tkinter  # noqa: F401
        import hazync_gui
        import firstrun
        import supervisor
    except Exception as e:             # noqa: BLE001
        print(f"  SKIP cannot import the window: {e}")
        return 0
    hazync_gui.App.refresh = lambda self: None
    hazync_gui.App.rescan = lambda self: None
    if CONTROL:
        hazync_gui.App._ask_name_first = lambda self: True
        print("CONTROL: Start without the question -- the checks below MUST fail")
    try:
        app = hazync_gui.App()
    except Exception as e:             # noqa: BLE001
        if "display" in str(e).lower():
            print(f"  SKIP no display: {e}")
            return 0
        raise
    app.withdraw()

    started, asked, named = [], [], []

    class Fatal:
        detail = "stopped here by the test"
    # preflight is where _start goes once it decides to start: record it and stop there
    supervisor.preflight = lambda *a, **kw: (started.append(1) or [], [Fatal()])
    hazync_gui.messagebox.showerror = lambda *a, **kw: None
    app._show_checks = lambda *a, **kw: None
    app._open_advanced = lambda *a, **kw: None
    answer = {"v": None}
    hazync_gui.simpledialog.askstring = lambda *a, **kw: (asked.append(1) or answer["v"])
    firstrun.set_handle = lambda worker, name, *a, **kw: (named.append(name) or (True, f"this prover is now {name!r}"))

    unnamed = lambda: [firstrun.Step("identity", "Your name on the board", False, "currently 'ghost:a3cb01'")]  # noqa: E731

    app._steps = unnamed()
    answer["v"] = None
    app._start()
    check(asked and not started, f"no name set: Start asks, and Cancel starts nothing (asked {len(asked)}, started {len(started)})")

    asked.clear(); started.clear(); named.clear()
    app._steps = unnamed()
    answer["v"] = "  xesktop-test "
    app._start()
    check(asked and named == ["xesktop-test"] and started,
          f"a name given is set, then the work starts (set {named}, started {len(started)})")
    check("xesktop-test" in app.v_who.get(), f"the window says who it proves as ({app.v_who.get()!r})")

    asked.clear(); started.clear()
    app._steps = [firstrun.Step("identity", "Your name on the board", True, "alice  (key key.hex)")]
    app._start()
    check(not asked and started, "a name already set: no question, it starts")

    app.destroy()
    if CONTROL:
        if fails >= 2:
            print(f"CONTROL OK: {fails} checks fail without the question, as they must")
            return 0
        print(f"CONTROL BROKEN: only {fails} failed")
        return 1
    print("PASS" if fails == 0 else f"FAIL ({fails})")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
