#!/usr/bin/env python3
"""The window actually BUILDS, and the controls people need are in it.

⛔⛔ WHY THIS EXISTS. hazync_gui.py is ~1,200 lines of widget construction and NOTHING tested that it
runs. theme.py has a render self-test; the window itself had none. So a typo in `_build`, a renamed
palette key, or a widget added to the wrong parent is caught only when a person launches it -- on
Windows, which is the one platform CI cannot run the real thing on. Every Windows-only bug this
project has hit was invisible on Linux until somebody opened the app.

⚠ This does not prove the layout LOOKS right; nothing here can. It proves the window constructs
without raising and that the controls a contributor must be able to find are present and reachable,
which is the part that fails silently and turns into "it does not open".

    test_gui_builds.py          # build the window headlessly and inspect it

⚠ SKIPS, not fails, without a display: the same contract theme.py uses, so a box with no X server
reports honestly instead of going red for a reason that has nothing to do with the code.
"""
import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

fails = []


def check(ok, what):
    print(("  ok   " if ok else "  FAIL ") + what)
    if not ok:
        fails.append(what)


def walk(w, out=None):
    """Every widget under w, depth first."""
    out = [] if out is None else out
    for child in w.winfo_children():
        out.append(child)
        walk(child, out)
    return out


def texts(widgets):
    """The visible text of everything that has any — buttons, labels, radios, tabs."""
    seen = []
    for x in widgets:
        for key in ("text",):
            try:
                v = x.cget(key)
            except Exception:          # noqa: BLE001 - not every widget has every option
                continue
            if v:
                seen.append(str(v))
    return seen


def main():
    # ⚠ POINT THE APP AT A TEMP HOME FIRST. Otherwise this reads, and could write, the real
    # ~/.hazync of whoever runs it -- including on a contributor's own machine.
    tmp = tempfile.mkdtemp()
    os.environ["HAZYNC_HOME"] = tmp

    try:
        import tkinter  # noqa: F401
    except Exception as e:             # noqa: BLE001
        print(f"  SKIP no tkinter: {e}")
        return 0
    try:
        import hazync_gui
    except Exception as e:             # noqa: BLE001
        print(f"  FAIL the module does not even import: {type(e).__name__}: {e}")
        return 1

    try:
        app = hazync_gui.App()
    except Exception as e:             # noqa: BLE001
        msg = str(e).lower()
        if "display" in msg or "no display name" in msg:
            print(f"  SKIP no display: {e}")
            return 0
        # ⛔ THIS IS THE FAILURE THIS FILE EXISTS FOR. A window that raises while building is a
        # window nobody can open, and the traceback is the whole point -- print it, do not
        # summarise it into "could not start".
        import traceback
        print("  FAIL the window raised while building:")
        traceback.print_exc()
        return 1

    try:
        app.withdraw()                 # built, never shown
        widgets = walk(app)
        label = texts(widgets)
        blob = " | ".join(label)
        check(len(widgets) > 50, f"the window built {len(widgets)} widgets")

        # ⚠ Tabs come off the notebook, not from cget("text").
        tabs = []
        for x in widgets:
            if x.winfo_class() == "TNotebook":
                tabs += [x.tab(i, "text") for i in range(len(x.tabs()))]
        # ⚠ Substring, not equality: the tab labels carry padding spaces ("  Setup  ") for the
        # look of them, and an exact match reported all five missing on a window that plainly has
        # them. A test that fails on cosmetics teaches people to ignore it.
        for want in ("Setup", "Dashboard", "Block map", "Prove", "Settings"):
            check(any(want in t for t in tabs), f"the {want} tab exists")

        # ⭐ THE CONTROLS A CONTRIBUTOR MUST BE ABLE TO FIND. If any of these silently stops being
        # built, the app still opens and simply cannot be used for that job.
        for want, why in [
            ("Start", "Start"),
            ("Stop", "Stop"),
            ("Help the spine (absorb)", "the one serial job nothing else can do"),
            ("Fold instead of prove", "folding, the job that parallelises"),
            ("CPU build", "the build that works on every machine"),
            ("GPU build", "the CUDA build"),
            ("below the supported floor", "the opt-in for an unsupported card"),
        ]:
            check(any(want in t for t in label), f"{why} is present ({want!r})")

        # ⚠ The identity import must exist, or work gets credited to the wrong handle — which has
        # already happened once on a real run.
        check("Import a key" in blob, "importing an identity is reachable from the window")
    finally:
        try:
            app.destroy()
        except Exception:              # noqa: BLE001
            pass

    print()
    if fails:
        print(f"FAIL {len(fails)}")
        return 1
    print("the window builds and every control a contributor needs is in it")
    return 0


if __name__ == "__main__":
    sys.exit(main())
