"""The board tab opens the website's block map, and 'my blocks' opens it on the user's name (11-13).

    python test_map_link.py            # must PASS
    python test_map_link.py --control  # 'my blocks' without the name: MUST FAIL
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


os.environ["HAZYNC_HOME"] = tempfile.mkdtemp()
try:
    import webbrowser
    import firstrun
    import hazync_gui
    hazync_gui.App.refresh = lambda self: None
    hazync_gui.App.rescan = lambda self: None
    if CONTROL:
        hazync_gui.App.map_link = lambda self, mine=False: hazync_gui.App.MAP_URL
        print("CONTROL: the link never carries the name -- the checks below MUST fail")
    app = hazync_gui.App()
except Exception as e:                  # noqa: BLE001
    print(f"  SKIP the window: {e}")
    sys.exit(0)
app.withdraw()
opened = []
webbrowser.open = lambda url, *a, **kw: opened.append(url)
hazync_gui.messagebox.showinfo = lambda *a, **kw: opened.append("<asked for a name>")

app._open_map()
check(opened[-1] == "https://hazync.org/block-map/", f"Open the block map: the site's map ({opened[-1]})")
app._steps = [firstrun.Step("identity", "Your name on the board", True, "Windows App  (key key.hex)")]
app._open_map(mine=True)
check(opened[-1] == "https://hazync.org/block-map/?prover=Windows%20App",
      f"Show my blocks: the map on this prover's name ({opened[-1]})")
app._steps = [firstrun.Step("identity", "Your name on the board", False, "currently 'ghost:a3cb01'")]
app._open_map(mine=True)
check(opened[-1] == "<asked for a name>", f"no name yet: it says so instead of opening a map of nobody ({opened[-1]})")
check(not hasattr(app, "map_canvas"), "no condensed map is drawn in the window any more")
app.destroy()
if CONTROL:
    if fails >= 2:
        print(f"CONTROL OK: {fails} checks fail, as they must")
        sys.exit(0)
    print(f"CONTROL BROKEN: only {fails} failed")
    sys.exit(1)
print("PASS" if fails == 0 else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
