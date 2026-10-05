"""Make ttk obey the palette, in both modes.

⛔⛔ WHY DARK MODE LOOKED BROKEN. This window hand-coloured every `tk` widget from brand.palette()
and then used 24 `ttk` widgets — buttons, entries, tabs, radios, the spinbox, the scrollbar — which
ignore all of that and render in the operating system's theme. In light mode that is merely plain.
In dark mode it is grey Windows chrome sitting on a near-black page, which is worse than no dark
mode at all: the thing the person asked for is the thing that looks wrong.

⚠ `ttk.Style` was not used ANYWHERE before this file. Hand-colouring `tk` widgets while leaving
`ttk` alone cannot be finished — the two sets are themed by different machinery, and a window that
mixes them will always look half-done.

⭐ THE BASE THEME IS `clam`, DELIBERATELY. The default on Windows is `vista`, whose widgets are drawn
by the OS and IGNORE background and bordercolor — you can set them and nothing happens, which is a
very confusing afternoon. `clam` is drawn by Tk itself, so every colour here actually applies.

⚠ RESTRAINT IS THE POINT. The ask was "minimalistic": no gradients, no rounded-corner emulation, no
third accent. One accent (the logo's orange) on the one button that starts work, one border colour,
one surface, and generous padding so a control is easy to hit. Everything that is not those is text.
"""
import os as _os, sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
import winconsole as _wc  # noqa: E402
_wc.fix()   # ⛔ BEFORE anything prints: a ✅ on a cp1252 console raises, not degrades

from tkinter import ttk  # noqa: E402

import brand  # noqa: E402

# One scale for the whole window. A control that is 22px tall is hard to hit and looks cramped;
# these are the numbers the rest of the layout is spaced against.
PAD_X, PAD_Y = 14, 7
FONT = ("Segoe UI", 10)
FONT_SMALL = ("Segoe UI", 9)
FONT_BOLD = ("Segoe UI", 10, "bold")


def apply(root, dark=False):
    """Theme every ttk widget class this window uses. Returns the palette it used."""
    p = brand.palette(dark)
    st = ttk.Style(root)
    try:
        st.theme_use("clam")
    except Exception:          # noqa: BLE001 - a Tk without clam still works, just OS-themed
        pass

    st.configure(".", background=p["fog"], foreground=p["ink"], fieldbackground=p["mist"],
                 bordercolor=p["haze"], lightcolor=p["mist"], darkcolor=p["mist"],
                 focuscolor=p["lamp"], font=FONT)

    # ── buttons ─────────────────────────────────────────────────────────────────────────────────
    # ⚠ FLAT, WITH REAL PADDING. The default ttk button on Windows is a small bevelled grey box;
    # at this size, with this many of them, the window reads as a form rather than a tool.
    st.configure("TButton", background=p["mist"], foreground=p["ink"], bordercolor=p["haze"],
                 relief="flat", padding=(PAD_X, PAD_Y), font=FONT, anchor="center")
    st.map("TButton",
           background=[("pressed", p["haze"]), ("active", p["haze"]), ("disabled", p["fog"])],
           foreground=[("disabled", p["slate"])],
           bordercolor=[("focus", p["lamp"])])

    # The one button that starts work. ⭐ Exactly one accent in the window: more than one and
    # neither means anything.
    st.configure("Accent.TButton", background=p["lamp"], foreground="#ffffff",
                 bordercolor=p["lamp"], relief="flat", padding=(PAD_X + 4, PAD_Y + 1),
                 font=FONT_BOLD)
    st.map("Accent.TButton",
           background=[("pressed", p["lamp_text"]), ("active", p["lamp_text"]),
                       ("disabled", p["haze"])],
           foreground=[("disabled", p["slate"])])

    # Quiet button for secondary actions, so a row of five does not shout five times.
    st.configure("Quiet.TButton", background=p["fog"], foreground=p["slate"],
                 bordercolor=p["haze"], relief="flat", padding=(PAD_X - 2, PAD_Y - 1),
                 font=FONT_SMALL)
    st.map("Quiet.TButton", background=[("active", p["mist"])],
           foreground=[("active", p["ink"]), ("disabled", p["haze"])])

    # ── fields ──────────────────────────────────────────────────────────────────────────────────
    for cls in ("TEntry", "TSpinbox", "TCombobox"):
        st.configure(cls, fieldbackground=p["mist"], background=p["mist"], foreground=p["ink"],
                     bordercolor=p["haze"], lightcolor=p["haze"], darkcolor=p["haze"],
                     insertcolor=p["ink"], arrowcolor=p["slate"], padding=(6, 5))
        st.map(cls, bordercolor=[("focus", p["lamp"])], fieldbackground=[("readonly", p["fog"])])

    # ── choices ─────────────────────────────────────────────────────────────────────────────────
    for cls in ("TRadiobutton", "TCheckbutton"):
        st.configure(cls, background=p["fog"], foreground=p["ink"], font=FONT,
                     indicatorcolor=p["mist"], indicatorbackground=p["mist"], padding=(2, 4))
        st.map(cls, background=[("active", p["fog"])], foreground=[("disabled", p["slate"])],
               indicatorcolor=[("selected", p["lamp"]), ("pressed", p["lamp_text"])])

    # ── tabs ────────────────────────────────────────────────────────────────────────────────────
    # ⚠ The selected tab carries the accent as a thin cue, not as a filled block: this strip is
    # navigation, and a brightly filled tab competes with the one button that matters.
    st.configure("TNotebook", background=p["fog"], bordercolor=p["haze"], tabmargins=(2, 6, 2, 0))
    st.configure("TNotebook.Tab", background=p["fog"], foreground=p["slate"],
                 bordercolor=p["fog"], padding=(18, 9), font=FONT)
    st.map("TNotebook.Tab",
           background=[("selected", p["mist"]), ("active", p["mist"])],
           foreground=[("selected", p["ink"])],
           bordercolor=[("selected", p["haze"])])

    # ── the rest ────────────────────────────────────────────────────────────────────────────────
    st.configure("TScrollbar", background=p["mist"], troughcolor=p["fog"],
                 bordercolor=p["fog"], arrowcolor=p["slate"], relief="flat")
    st.map("TScrollbar", background=[("active", p["haze"])])
    st.configure("TProgressbar", background=p["lamp"], troughcolor=p["mist"],
                 bordercolor=p["haze"], lightcolor=p["lamp"], darkcolor=p["lamp"])
    st.configure("TSeparator", background=p["haze"])
    st.configure("TFrame", background=p["fog"])
    st.configure("TLabel", background=p["fog"], foreground=p["ink"], font=FONT)

    # ⚠ The window itself, and the dialogs Tk draws from these option-database values. Without this
    # a file dialog and a messagebox stay white in dark mode — the two places a person is most
    # likely to be reading carefully.
    root.configure(bg=p["fog"])
    for opt, val in (("*Background", p["fog"]), ("*Foreground", p["ink"]),
                     ("*selectBackground", p["lamp"]), ("*selectForeground", "#ffffff")):
        try:
            root.option_add(opt, val)
        except Exception:      # noqa: BLE001
            pass
    return p


def self_test():
    """Check the theme applies and that dark really differs from light, without showing a window."""
    # ⛔ A MISSING tkinter IS A REAL FAILURE, NOT A SKIP — this whole program is a Tk window, so a
    # Python without it cannot run Hazync at all. But say WHICH thing is missing: the first CI run
    # died on a bare `ModuleNotFoundError: No module named 'tkinter'` after installing only an X
    # server, which reads as a code fault rather than a one-package environment gap.
    try:
        import tkinter as tk
    except ImportError as e:
        print(f"  FAIL tkinter is not installed in this Python ({e}). Hazync is a Tk window, so "
              f"this is fatal, not skippable. On Debian/Ubuntu: apt-get install python3-tk")
        return 1
    bad = 0
    try:
        root = tk.Tk()
        root.withdraw()
    except Exception as e:     # noqa: BLE001 - headless build box
        print(f"  SKIP no display: {e}")
        return 0
    seen = {}
    for dark in (False, True):
        p = apply(root, dark)
        st = ttk.Style(root)
        seen[dark] = {
            "theme": st.theme_use(),
            "button_bg": st.lookup("TButton", "background"),
            "accent_bg": st.lookup("Accent.TButton", "background"),
            "tab_fg": st.lookup("TNotebook.Tab", "foreground"),
            "entry_bg": st.lookup("TEntry", "fieldbackground"),
        }
    for dark in (False, True):
        name = "dark" if dark else "light"
        s = seen[dark]
        ok = s["theme"] == "clam"
        print(f"  {'ok  ' if ok else 'FAIL'} {name}: base theme is clam (got {s['theme']!r})")
        bad += 0 if ok else 1
        for k in ("button_bg", "accent_bg", "entry_bg", "tab_fg"):
            ok = bool(s[k])
            if not ok:
                print(f"  FAIL {name}: {k} is unset — ttk would fall back to the OS theme")
                bad += 1
    # ⛔ THE ONE THAT MATTERS: dark must actually differ. A "dark mode" that themes nothing is the
    # bug this file was written for.
    same = [k for k in ("button_bg", "entry_bg") if seen[False][k] == seen[True][k]]
    ok = not same
    print(f"  {'ok  ' if ok else 'FAIL'} dark and light differ for every surface"
          + (f" — IDENTICAL: {same}" if same else ""))
    bad += 0 if ok else 1
    print(f"  ok   one accent only: {seen[False]['accent_bg']} / {seen[True]['accent_bg']}")
    root.destroy()
    return bad


if __name__ == "__main__":
    import sys
    n = self_test()
    print()
    print("All checks passed." if not n else f"FAIL: {n} check(s)")
    sys.exit(1 if n else 0)
