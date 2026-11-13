"""The technical log is drawn, tall, and holds what the workers said; the segment size is a list (11-13).

Xesktop, 11-13: "nothing shows on technical log, it's blank". Every line was in the log, but it was the
last card on a tab needing ~1,124 px, so on an ordinary window it was never drawn. And the segment size
was a free text box; it is now a list, where 22 (chosen on a big card) must reach the worker as 22.

    python test_log_visible.py            # must PASS
    python test_log_visible.py --control  # the log back at the foot of Options: MUST FAIL
"""
import os
import sys
import tempfile
import time

CONTROL = "--control" in sys.argv
fails = 0


def check(ok, what):
    global fails
    print(("  ok   " if ok else "  FAIL ") + what)
    fails += 0 if ok else 1


def main():
    os.environ["HAZYNC_HOME"] = tempfile.mkdtemp()
    try:
        import hazync_gui
        import cards
        import supervisor
        import firstrun
    except Exception as e:                 # noqa: BLE001
        print(f"  SKIP cannot import the window: {e}")
        return 0
    hazync_gui.App.refresh = lambda self: None
    hazync_gui.App.rescan = lambda self: None
    if CONTROL:
        import tkinter as tk

        def old_layout(self):              # the log as the last card on Options, as it was
            t = self.adv.nametowidget(self.adv.tabs()[1])
            logf = self._card(t, "THE TECHNICAL LOG")
            logf.pack(fill="both", expand=True, padx=10, pady=8)
            self.log = tk.Text(logf)
            self.log.pack(fill="both", expand=True)
            self.log.configure(state="disabled")
        hazync_gui.App._tab_log = old_layout
        hazync_gui.App.LOG_TAB = 1
        print("CONTROL: the log at the foot of Options -- the checks below MUST fail")
    try:
        app = hazync_gui.App()
    except Exception as e:                 # noqa: BLE001
        if "display" in str(e).lower():
            print(f"  SKIP no display: {e}")
            return 0
        raise
    app.geometry("1100x760")
    for i in range(40):
        app._say(f"[run 0] segment {i}/40  {i}s elapsed", "sys")

    def settle():
        end = time.time() + 1.5
        while time.time() < end:
            app.update()
            time.sleep(0.02)
    app._open_advanced(app.LOG_TAB)
    settle()
    text = app.log.get("1.0", "end")
    check("segment 39/40" in text, "the log holds what the worker said")
    check(app.log.winfo_ismapped() and app.log.winfo_height() >= 200,
          f"'Show the technical log' shows it, drawn and tall ({app.log.winfo_ismapped()}, "
          f"{app.log.winfo_height()} px in a 760 px window)")
    check(app.log.winfo_rooty() + app.log.winfo_height() <= app.winfo_rooty() + app.winfo_height() + 2,
          "the log ends inside the window, not below its edge")

    if not CONTROL:
        check(app.po2_label.get() == "Automatic (recommended)" and app.po2.get() == "",
              f"a fresh install shows 'Automatic' and stores blank ({app.po2_label.get()!r})")
        lab22 = dict(hazync_gui.SEG_CHOICES)["22"]
        app.po2_label.set(lab22)
        combo = [c for c in _walk(app) if c.winfo_class() == "TCombobox"]
        combo[0].event_generate("<<ComboboxSelected>>")
        app.update()
        check(app.po2.get() == "22", f"choosing 22 stores 22 ({app.po2.get()!r})")
        check(firstrun.load_config().get("seg_po2") == "22", "and saves it")
        big = {"vram_mb": 97887, "index": 0}
        small = {"vram_mb": 24564, "index": 1}
        check(cards.seg_po2_for(big, "22", supervisor._SEG_PO2_FOR_VRAM) == "22",
              "22 reaches the worker on a 96 GB card (was clamped to 21)")
        check(cards.seg_po2_for(small, "22", supervisor._SEG_PO2_FOR_VRAM) == "20",
              "a 24 GB card beside it still gets its own smaller size")
        app.po2.set("21")
        app.update()
        check(app.po2_label.get().startswith("21 "), "a recommended 21 shows as the 21 line")
    app.destroy()

    if CONTROL:
        if fails >= 1:
            print(f"CONTROL OK: {fails} checks fail with the log at the foot of Options, as they must")
            return 0
        print("CONTROL BROKEN: nothing failed")
        return 1
    print("PASS" if fails == 0 else f"FAIL ({fails})")
    return 1 if fails else 0


def _walk(w):
    for c in w.winfo_children():
        yield c
        yield from _walk(c)


if __name__ == "__main__":
    sys.exit(main())
