#!/usr/bin/env python3
"""Hazync for Windows — a window that runs the prover, for people who do not use a terminal.

    python hazync_gui.py

⛔ THE LOGIC IS NOT IN HERE. Anything that can be WRONG lives in a tested module:

    supervisor.py   is this binary canonical, what does this exit code mean, what env does a
                    worker need                                  -> test_supervisor.py (+control)
    hazync_api.py   the coordinator's endpoints, and the block map's geometry
                                                                  -> test_api.py (+control, +live)
    brand.py        the real palette and logo, from hazync.org    -> brand.py self-test

This file is layout, threads, and putting pixels on screen. A supervisor whose only entry point is
a window cannot be tested on a build box.

⚠ WHAT IT REPLACES. A Linux contributor runs the `host` binary, the `hazync-worker` CLI and
`run-workers.sh` — a supervisor loop. This replaces the THIRD. The host and the worker are used
exactly as they ship.

⛔ READ tools/win-gui/README.md FIRST for what is proven and what is not. In short, as of
2026-10-05: this window runs on Windows and has driven a complete prove through it (block 170,
2,220.7 s, receipt VERIFIED). Native Windows CUDA proving has still never completed — but that is a
CARD-SUPPORT question, not a Windows one: sppark keeps only GPUs at compute capability 7.0 or newer,
and the card it was tested on is 6.1.
"""

import os as _os, sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
import winconsole as _wc  # noqa: E402
_wc.fix()   # ⛔ BEFORE anything prints: a ✅ on a cp1252 console raises, not degrades

import os
import queue
import subprocess
import sys
import threading
import time
import tkinter as tk
from pathlib import Path
from tkinter import filedialog, messagebox, ttk

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import activity  # noqa: E402
import brand  # noqa: E402
import firstrun  # noqa: E402
import hazync_api as api  # noqa: E402
import supervisor  # noqa: E402
import theme  # noqa: E402

POLL_MS = 150
REFRESH_S = 60          # the website refreshes the map every minute; match it rather than hammer


class Worker:
    def __init__(self, index, popen, log_path, job):
        self.index, self.popen, self.log_path, self.job = index, popen, log_path, job
        self.started = time.time()
        self.stopping = False
        self.activity = activity.Activity(job)
        self.heard = time.time()         # when the prover last printed anything at all


class App(tk.Tk):
    def __init__(self):
        super().__init__()
        self.title("Hazync")
        self.geometry("1040x740")
        self.minsize(900, 620)
        # ⛔ SETTINGS MUST SURVIVE A RESTART. Re-typing four paths every launch is its own reason to
        # give up, and the first version of this window saved nothing at all.
        self.cfg = firstrun.load_config()
        self.dark = tk.BooleanVar(value=bool(self.cfg.get("dark")))
        self.p = brand.palette(self.dark.get())
        self.q = queue.Queue()
        # ⛔ Results from worker threads come back HERE and are applied by _drain on the main
        # thread. Calling self.after() from a thread is itself a Tk call — the same class of
        # bug as reading a StringVar there, and it is why the first version loaded no data.
        self.results = queue.Queue()
        # The last failure per setup step, so a card can say why its own button did nothing. Without
        # it the reason goes only to the log pane, which _tab_run builds on a DIFFERENT TAB.
        self.fix_errors = {}
        self.workers = []
        self.runs, self.meta, self.prog = [], {}, {}
        self._cells = []                 # (item_id, lo, hi, state) for the map
        self._explained = set()          # each known failure explained once, not every line
        self._auto_diag_done = False     # the cheap checks run once per launch, not on every rescan
        self._auto_fix_done = False      # ...and so does fetching what setup can fetch by itself
        self._setup_ok = False           # until the first scan says otherwise
        self._steps = []
        self._problem = ""               # the last thing that stopped work, in plain words
        self._events = []                # (clock time, sentence, tag) for the Home feed
        self._landed = {j: 0 for j in activity.JOBS}     # what has landed since the window opened
        self._diag_running = None
        self._build()
        self.after(1000, self._home_tick)
        self.after(POLL_MS, self._drain)
        self.after(400, self.refresh)
        self.after(700, self.rescan)
        self.protocol("WM_DELETE_WINDOW", self._on_close)

    # ── chrome ──────────────────────────────────────────────────────────────────────────────────
    def _build(self):
        # ⛔ THEME ttk FIRST, BEFORE ANY WIDGET EXISTS. 24 ttk widgets here render in the OS theme
        # unless styled, which is why dark mode looked like grey Windows chrome on a near-black
        # page: brand.palette() only ever reached the hand-coloured tk widgets.
        p = self.p = theme.apply(self, self.dark.get())
        self.configure(bg=p["fog"])

        head = tk.Frame(self, bg=p["fog"])
        head.pack(fill="x", padx=20, pady=(16, 6))
        logo = tk.Canvas(head, width=44, height=44, bg=p["fog"], highlightthickness=0)
        logo.pack(side="left")
        brand.draw_logo(logo, 2, 2, 40, dark=self.dark.get())
        tk.Label(head, text="Hazync", bg=p["fog"], fg=p["ink"],
                 font=("Segoe UI", 20, "bold")).pack(side="left", padx=(12, 0))
        tk.Label(head, text="prove Bitcoin's history, one block at a time",
                 bg=p["fog"], fg=p["slate"], font=("Segoe UI", 10)).pack(side="left", padx=10)
        ttk.Checkbutton(head, text="dark", variable=self.dark,
                        command=self._retheme).pack(side="right")
        # Who the board credits, where a person can always see it: work under the wrong name is
        # public and permanent, and it has already happened once on a real run.
        self.v_who = tk.StringVar(value=getattr(self, "_who", ""))
        tk.Label(head, textvariable=self.v_who, bg=p["fog"], fg=p["slate"],
                 font=("Segoe UI", 10)).pack(side="right", padx=14)

        # ⚠ THE STATUS LINE IS PACKED BEFORE THE NOTEBOOK. Packed after it, the notebook's
        # expand=True took every pixel and the line was squeezed off the bottom of the window.
        self.status = tk.Label(self, text="starting…", anchor="w", bg=p["mist"], fg=p["slate"],
                               padx=12, pady=3)
        self.status.pack(fill="x", side="bottom")

        # Shared by Home and the Advanced pages, so it has to exist before either is built.
        self.mode = tk.StringVar(value=self.cfg.get("mode") or "auto")

        # ⭐ THREE PLACES, NOT FIVE. Home is the whole program for most people: one choice, one
        # button, and what is happening. The board is for looking. Everything a newcomer should
        # never need — paths, builds, segment sizes, the raw log — is behind Advanced.
        self.nb = ttk.Notebook(self)
        self.nb.pack(fill="both", expand=True, padx=14, pady=6)
        self._tab_home()
        self._tab_board()
        advf = tk.Frame(self.nb, bg=p["fog"])
        self.nb.add(advf, text="  Advanced  ")
        self.adv = ttk.Notebook(advf)
        self.adv.pack(fill="both", expand=True, padx=6, pady=(8, 6))
        self._tab_setup()
        self._tab_run()
        self._tab_settings()
        self.mode.trace_add("write", lambda *_: self._paint_tiles())
        self._paint_tiles()
        self._sync_buttons()
        self._home_refresh()

    def _retheme(self):
        # ⚠ A full re-theme of a live widget tree is fiddly and easy to get half-right. Rebuilding
        # is honest and instant at this size; a half-themed window looks broken.
        # ⚠ Keep what was chosen: _build re-creates every Tk variable from self.cfg.
        try:
            self.cfg = self._snapshot()
        except Exception:      # noqa: BLE001 - mid-build, there is nothing to keep yet
            pass
        for w in self.winfo_children():
            w.destroy()
        self._build()          # re-themes ttk as well as rebuilding the tk widgets
        self._render_home_setup()
        self.refresh()

    def _card(self, parent, title):
        p = self.p
        f = tk.Frame(parent, bg=p["mist"], highlightbackground=p["haze"], highlightthickness=1)
        tk.Label(f, text=title, bg=p["mist"], fg=p["slate"],
                 font=("Segoe UI", 9)).pack(anchor="w", padx=12, pady=(8, 0))
        return f

    def _big(self, parent, var):
        return tk.Label(parent, textvariable=var, bg=self.p["mist"], fg=self.p["ink"],
                        font=("Segoe UI", 22, "bold"))

    # ── home ────────────────────────────────────────────────────────────────────────────────────
    # ⭐ THE WHOLE PROGRAM, FOR MOST PEOPLE. One choice (what to do), one button (Start), and a
    # plain account of what is happening. Nothing on this page names a file, a flag or a build.
    #
    # ⛔ AND IT IS NEVER SILENT. A prove runs for minutes on a GPU and most of an hour on a CPU; a
    # window that only says "running" for that long reads as broken, and was reported as exactly
    # that from a real machine. So this page always shows which block, which piece of how many,
    # how long it has been running, and how long since the prover last said anything at all.
    PLAIN_STEPS = {
        "crypto": ("Signing tools", "Installing the signing tools…"),
        "worker": ("The Hazync client", "Downloading the Hazync client…"),
        "host": ("The prover program", ""),
        "identity": ("Your name on the board", ""),
    }
    START_TEXT = {"auto": "Start proving", "range": "Start proving",
                  "fold": "Start folding", "spine": "Start anchoring"}
    MODE_JOB = {"auto": "run", "range": "run", "fold": "fold", "spine": "spine"}

    def _tab_home(self):
        p = self.p
        t = tk.Frame(self.nb, bg=p["fog"])
        self.nb.add(t, text="  Home  ")
        self._home = t

        hero = tk.Frame(t, bg=p["mist"], highlightbackground=p["haze"], highlightthickness=1)
        hero.pack(fill="x", padx=10, pady=(12, 8))
        self._hero = hero
        self.v_head = tk.StringVar(value="Getting ready…")
        self.v_sub = tk.StringVar(value="Looking at this machine.")
        self.v_meta = tk.StringVar(value="")
        tk.Label(hero, textvariable=self.v_head, bg=p["mist"], fg=p["ink"], anchor="w",
                 justify="left", wraplength=940, font=("Segoe UI", 20, "bold")
                 ).pack(fill="x", padx=24, pady=(20, 2))
        tk.Label(hero, textvariable=self.v_sub, bg=p["mist"], fg=p["slate"], anchor="w",
                 justify="left", wraplength=940, font=("Segoe UI", 11)
                 ).pack(fill="x", padx=24)
        self.bar = ttk.Progressbar(hero, mode="determinate", maximum=1000)
        self.bar.pack(fill="x", padx=24, pady=(16, 6))
        self._bar_busy = False
        tk.Label(hero, textvariable=self.v_meta, bg=p["mist"], fg=p["slate"], anchor="w",
                 font=("Segoe UI", 9)).pack(fill="x", padx=24)
        row = tk.Frame(hero, bg=p["mist"])
        row.pack(anchor="w", padx=24, pady=(14, 20))
        self.start_btn = ttk.Button(row, text="Start proving", style="Hero.TButton",
                                    command=self._start, state="disabled")
        self.start_btn.pack(side="left")
        # ⭐ ONE BUTTON AT A TIME. Stop takes Start's place while work runs, so the page never shows
        # a greyed-out button next to a live one and leaves a person to work out which is which.
        self.stop_btn = ttk.Button(row, text="Stop", style="Hero.TButton", command=self._stop,
                                   state="disabled")

        # Only on screen while something is still needed; see _render_home_setup.
        self.home_setup = tk.Frame(t, bg=p["fog"])

        pick = tk.Frame(t, bg=p["fog"])
        pick.pack(fill="x", padx=4, pady=(4, 0))
        self._pick = pick
        tk.Label(pick, text="WHAT WOULD YOU LIKE TO DO?", bg=p["fog"], fg=p["slate"],
                 font=("Segoe UI", 9)).grid(row=0, column=0, columnspan=3, sticky="w", padx=8,
                                            pady=(4, 4))
        self._tiles = {}
        for col, val in enumerate(("auto", "fold", "spine")):
            job = self.MODE_JOB[val]
            f = tk.Frame(pick, bg=p["mist"], highlightbackground=p["haze"], highlightthickness=1,
                         cursor="hand2")
            f.grid(row=1, column=col, sticky="nsew", padx=6, pady=2)
            pick.columnconfigure(col, weight=1, uniform="tiles")
            title = tk.Label(f, text=activity.JOB_NAMES[job], bg=p["mist"], fg=p["ink"],
                             anchor="w", font=("Segoe UI", 13, "bold"), cursor="hand2")
            title.pack(fill="x", padx=14, pady=(12, 2))
            blurb = tk.Label(f, text=activity.JOB_BLURBS[job], bg=p["mist"], fg=p["slate"],
                             anchor="w", justify="left", wraplength=280, font=("Segoe UI", 9),
                             cursor="hand2")
            blurb.pack(fill="x", padx=14, pady=(0, 12))
            for w in (f, title, blurb):
                w.bind("<Button-1>", lambda _e, v=val: self._choose(v))
            self._tiles[val] = (f, title)
        self.v_pick_note = tk.StringVar(value="")
        tk.Label(pick, textvariable=self.v_pick_note, bg=p["fog"], fg=p["slate"],
                 font=("Segoe UI", 9)).grid(row=2, column=0, columnspan=3, sticky="w", padx=8)

        feedf = self._card(t, "WHAT HAS HAPPENED")
        feedf.pack(fill="both", expand=True, padx=10, pady=(8, 10))
        ttk.Button(feedf, text="Show the technical log", style="Quiet.TButton",
                   command=lambda: self._open_advanced(1)).place(relx=1.0, x=-10, y=6, anchor="ne")
        self.feed = tk.Text(feedf, wrap="word", height=6, bg=p["mist"], fg=p["ink"],
                            relief="flat", font=("Segoe UI", 10), padx=4, pady=2,
                            highlightthickness=0, cursor="arrow")
        self.feed.pack(fill="both", expand=True, padx=10, pady=(8, 10))
        self.feed.tag_configure("when", foreground=p["slate"])
        self.feed.tag_configure("good", foreground=p["good"])
        self.feed.tag_configure("bad", foreground=p["bad"])
        self.feed.tag_configure("sys", foreground=p["ink"])
        self.feed.configure(state="disabled")
        self._render_feed()

    def _open_advanced(self, index):
        self.nb.select(2)
        self.adv.select(index)

    def _choose(self, val):
        # ⚠ Not while work is running: the choice on screen would then describe a job that is not
        # the one in progress.
        if self.workers:
            self.v_pick_note.set("Stop first to switch to something else.")
            return
        self.mode.set(val)
        self._persist()

    def _paint_tiles(self):
        if not getattr(self, "_tiles", None):
            return
        p, mode = self.p, self.mode.get()
        for val, (frame, title) in self._tiles.items():
            on = val == mode
            frame.configure(highlightbackground=p["lamp"] if on else p["haze"],
                            highlightthickness=2 if on else 1)
            title.configure(fg=p["lamp_text"] if on else p["ink"])
        self.v_pick_note.set("A specific range is chosen under Advanced." if mode == "range" else "")
        self._sync_buttons()

    def _sync_buttons(self):
        """Start and Stop say what they will do, and only one of them is ever live."""
        if not hasattr(self, "start_btn"):
            return
        running = bool(self.workers)
        self.start_btn.configure(text=self.START_TEXT.get(self.mode.get(), "Start"),
                                 state="normal" if (self._setup_ok and not running) else "disabled")
        self.stop_btn.configure(state="normal" if running else "disabled")
        if running and not self.stop_btn.winfo_manager():
            self.start_btn.pack_forget()
            self.stop_btn.pack(side="left")
        elif not running and not self.start_btn.winfo_manager():
            self.stop_btn.pack_forget()
            self.start_btn.pack(side="left")

    def _feed(self, text, tag="sys"):
        """One plain sentence about something that HAPPENED, newest first."""
        self._events.append((time.strftime("%H:%M"), text, tag))
        del self._events[:-200]
        self._render_feed()

    def _render_feed(self):
        if not hasattr(self, "feed"):
            return
        f = self.feed
        f.configure(state="normal")
        f.delete("1.0", "end")
        if not self._events:
            f.insert("end", "Nothing yet. What this machine does will be listed here as it happens.",
                     "when")
        for when, text, tag in reversed(self._events[-60:]):
            f.insert("end", f"{when}   ", "when")
            f.insert("end", text + "\n", tag)
        f.configure(state="disabled")

    def _set_bar(self, fraction, busy):
        """A real fraction fills the bar; work with no measurable length gets a moving one."""
        if fraction is not None:
            if self._bar_busy:
                self.bar.stop()
                self._bar_busy = False
            self.bar.configure(mode="determinate")
            self.bar["value"] = int(max(0.0, min(1.0, fraction)) * 1000)
        elif busy:
            if not self._bar_busy:
                self.bar.configure(mode="indeterminate")
                self.bar.start(40)
                self._bar_busy = True
        else:
            if self._bar_busy:
                self.bar.stop()
                self._bar_busy = False
            self.bar.configure(mode="determinate")
            self.bar["value"] = 0

    def _session_line(self):
        parts = [activity.tally(j, n) for j, n in self._landed.items() if n]
        return " · ".join(parts)

    def _home_refresh(self):
        """Say what is true right now. Called on every worker line and once a second."""
        if not hasattr(self, "v_head"):
            return
        now = time.time()
        if self.workers:
            lead = self.workers[0]
            a = lead.activity
            stopping = any(w.stopping for w in self.workers)
            self.v_head.set("Stopping…" if stopping else a.headline)
            self.v_sub.set("Letting the prover finish cleanly." if stopping else a.detail)
            self._set_bar(a.fraction, busy=not a.waiting)
            bits = [f"Running for {activity.clock(now - min(w.started for w in self.workers))}",
                    f"last heard from the prover {activity.clock(now - max(w.heard for w in self.workers))} ago"]
            if len(self.workers) > 1:
                bits.append(f"{len(self.workers)} at once")
            if self._session_line():
                bits.append(self._session_line() + " this session")
            self.v_meta.set(" · ".join(bits))
            return
        self.v_meta.set((self._session_line() + " this session") if self._session_line() else "")
        if self._diag_running:
            name, t0 = self._diag_running
            title = next((t for n, t, *_ in supervisor.DIAGNOSTICS if n == name), name)
            self.v_head.set("Checking this machine…")
            self.v_sub.set(f"{title}  ·  {activity.clock(now - t0)}")
            self._set_bar(None, busy=True)
        elif getattr(self, "_fixing", None):
            self.v_head.set("Getting ready…")
            self.v_sub.set(self._fixing)
            self._set_bar(None, busy=True)
        elif self._problem:
            self.v_head.set("Stopped — this needs a look")
            self.v_sub.set(self._problem)
            self._set_bar(None, busy=False)
        elif not self._steps:
            self.v_head.set("Getting ready…")
            self.v_sub.set("Looking at this machine.")
            self._set_bar(None, busy=True)
        elif not self._setup_ok:
            left = [s for s in self._steps if not s.done]
            self.v_head.set("Almost ready")
            self.v_sub.set(f"{len(left)} thing{'' if len(left) == 1 else 's'} left before you can "
                           f"start — see below.")
            self._set_bar(None, busy=False)
        else:
            job = self.MODE_JOB.get(self.mode.get(), "run")
            self.v_head.set("Ready when you are")
            self.v_sub.set(activity.JOB_BLURBS[job])
            self._set_bar(None, busy=False)

    def _home_tick(self):
        self._home_refresh()
        self.after(1000, self._home_tick)

    def _render_home_setup(self):
        """What is still needed before Start can work, with the button that deals with each."""
        if not hasattr(self, "home_setup"):
            return
        p, box = self.p, self.home_setup
        for w in box.winfo_children():
            w.destroy()
        todo = [s for s in self._steps if not s.done]
        if self._setup_ok or not todo:
            box.pack_forget()
            return
        box.pack(fill="x", padx=10, pady=(0, 6), after=self._hero)
        card = tk.Frame(box, bg=p["mist"], highlightbackground=p["lamp"], highlightthickness=1)
        card.pack(fill="x")
        done = [self.PLAIN_STEPS.get(s.key, (s.title,))[0] for s in self._steps if s.done]
        tk.Label(card, text="BEFORE YOU CAN START" + (f"      ✓ {' · ✓ '.join(done)}" if done else ""),
                 bg=p["mist"], fg=p["slate"], font=("Segoe UI", 9)
                 ).pack(anchor="w", padx=14, pady=(10, 4))
        worker_ok = any(s.key == "worker" and s.done for s in self._steps)
        for s in todo:
            name = self.PLAIN_STEPS.get(s.key, (s.title, ""))[0]
            row = tk.Frame(card, bg=p["mist"])
            row.pack(fill="x", padx=14, pady=(2, 8))
            left = tk.Frame(row, bg=p["mist"])
            left.pack(side="left", fill="x", expand=True)
            tk.Label(left, text=name, bg=p["mist"], fg=p["ink"], anchor="w",
                     font=("Segoe UI", 11, "bold")).pack(anchor="w")
            err = self.fix_errors.get(s.key)
            if s.key == "host":
                say = ("This is the program that does the proving. It is not included yet, so it "
                       "has to be picked by hand: choose the host.exe you downloaded.")
            elif s.key == "identity":
                say = ("Every block you prove is credited to this name, publicly and permanently."
                       if worker_ok else "This can be set once the Hazync client has arrived.")
            else:
                say = "Fetching this for you." if not err else "This could not be fetched."
            tk.Label(left, text=say, bg=p["mist"], fg=p["slate"], anchor="w", justify="left",
                     wraplength=620, font=("Segoe UI", 9)).pack(anchor="w")
            if err:
                tk.Label(left, text=err, bg=p["mist"], fg=p["bad"], anchor="w", justify="left",
                         wraplength=620, font=("Segoe UI", 9)).pack(anchor="w")
            if s.key == "host":
                ttk.Button(row, text="Choose it…", style="Accent.TButton",
                           command=self._choose_host).pack(side="right")
                found = firstrun.find_hosts()
                if found:
                    ttk.Button(row, text=f"Use the one in {Path(found[0]['path']).parent.name}",
                               command=lambda pth=found[0]["path"]: self._use_host(pth)
                               ).pack(side="right", padx=8)
            elif s.key == "identity" and worker_ok:
                ttk.Button(row, text="Use this name", style="Accent.TButton",
                           command=self._set_handle).pack(side="right")
                ttk.Entry(row, textvariable=self.handle_var, width=22).pack(side="right", padx=8)
            elif s.fix and err:
                ttk.Button(row, text="Try again",
                           command=lambda st=s: self._run_fix(st)).pack(side="right")

    # ── advanced: setup ─────────────────────────────────────────────────────────────────────────
    def _tab_setup(self):
        p = self.p
        t = tk.Frame(self.adv, bg=p["fog"])
        self.adv.add(t, text="  Setup  ")

        tk.Label(t, text="Four things, then you can prove.", bg=p["fog"], fg=p["ink"],
                 font=("Segoe UI", 12, "bold")).pack(anchor="w", padx=14, pady=(12, 2))
        tk.Label(t, bg=p["fog"], fg=p["slate"], font=("Segoe UI", 9), justify="left",
                 wraplength=940,
                 text=("Anything with a button, this program will do for you. The prover binary is "
                       "the one download it cannot fetch yet — it is not published as a release "
                       "asset, and GitHub will not serve a build artefact without a login.")
                 ).pack(anchor="w", padx=14, pady=(0, 8))

        self.steps_frame = tk.Frame(t, bg=p["fog"])
        self.steps_frame.pack(fill="x", padx=10)

        row = tk.Frame(t, bg=p["fog"])
        row.pack(fill="x", padx=14, pady=10)
        ttk.Button(row, text="Check again", command=self.rescan).pack(side="left")
        self.v_setup = tk.StringVar(value="checking…")
        tk.Label(row, textvariable=self.v_setup, bg=p["fog"], fg=p["lamp_text"],
                 font=("Segoe UI", 10, "bold")).pack(side="left", padx=12)

        namef = self._card(t, "WHAT SHOULD THE BOARD CALL YOU?")
        namef.pack(fill="x", padx=10, pady=8)
        tk.Label(namef, bg=p["mist"], fg=p["ink"], font=("Segoe UI", 9), justify="left",
                 wraplength=920,
                 text=("Every block you prove is credited to this name, publicly and permanently. "
                       "Without one, the credit goes to a machine-generated label like "
                       "ghost:a1b2c3. The key that signs it lives in your identity folder — back "
                       "that file up, because losing it means losing the credit.")
                 ).pack(anchor="w", padx=12, pady=(2, 6))
        nrow = tk.Frame(namef, bg=p["mist"])
        nrow.pack(anchor="w", padx=12, pady=(0, 10))
        self.handle_var = tk.StringVar(value="")
        ttk.Entry(nrow, textvariable=self.handle_var, width=26).pack(side="left")
        ttk.Button(nrow, text="Use this name", command=self._set_handle).pack(side="left", padx=8)
        self.v_handle = tk.StringVar(value="")
        tk.Label(nrow, textvariable=self.v_handle, bg=p["mist"], fg=p["slate"],
                 font=("Segoe UI", 9)).pack(side="left", padx=8)

    def _render_steps(self, steps):
        p = self.p
        for w in self.steps_frame.winfo_children():
            w.destroy()
        for s in steps:
            card = tk.Frame(self.steps_frame, bg=p["mist"], highlightbackground=p["haze"],
                            highlightthickness=1)
            card.pack(fill="x", pady=3)
            left = tk.Frame(card, bg=p["mist"])
            left.pack(side="left", fill="x", expand=True, padx=12, pady=8)
            tk.Label(left, text=("✓  " if s.done else "•  ") + s.title, bg=p["mist"],
                     fg=p["good"] if s.done else p["ink"],
                     font=("Segoe UI", 10, "bold")).pack(anchor="w")
            tk.Label(left, text=s.detail, bg=p["mist"], fg=p["slate"], font=("Segoe UI", 9),
                     justify="left", wraplength=760).pack(anchor="w")
            if s.manual:
                tk.Label(left, text=s.manual, bg=p["mist"], fg=p["lamp_text"],
                         font=("Segoe UI", 8), justify="left",
                         wraplength=760).pack(anchor="w", pady=(2, 0))
            # ⛔⛔ THE REASON A FIX FAILED MUST APPEAR ON THIS CARD. It used to go only to _say(),
            # which writes to the log pane built in _tab_run -- a DIFFERENT TAB. So pressing
            # "Download it" and having it fail reset this row to "not downloaded yet" with the
            # explanation written somewhere the person was not looking. Reported from a real machine
            # as "it wont download the client", with no error visible anywhere on the screen.
            #
            # ⚠ Every fetch_worker failure path already returns a precise, different sentence --
            # "could not download <url>" vs "saved to <path> but it does not run" -- and all of that
            # care was wasted by showing none of it here.
            err = self.fix_errors.get(s.key)
            if err and not s.done:
                tk.Label(left, text=f"⛔  {err}", bg=p["mist"], fg=p["bad"],
                         font=("Segoe UI", 8), justify="left",
                         wraplength=760).pack(anchor="w", pady=(4, 0))
            if not s.done and s.fix:
                ttk.Button(card, text=s.fix_label or "Fix",
                           command=lambda st=s: self._run_fix(st)).pack(side="right", padx=12)
            if s.key == "host" and not s.done:
                ttk.Button(card, text="Choose host.exe…",
                           command=self._choose_host).pack(side="right", padx=4)
                found = firstrun.find_hosts()
                if found:
                    ttk.Button(card, text=f"Use the one in {Path(found[0]['path']).parent.name}",
                               command=lambda pth=found[0]["path"]: self._use_host(pth)
                               ).pack(side="right", padx=4)

    def rescan(self):
        self.v_setup.set("checking…")
        cfg = self._snapshot()
        threading.Thread(target=self._rescan_thread, args=(cfg,), daemon=True).start()

    def _rescan_thread(self, cfg):
        # ⭐ FIND THE BINARIES BEFORE JUDGING THE SETUP. Pointing a program at a file it could have
        # found itself is the setup step that makes someone give up before they start — and
        # classify_host can tell a CPU build from a CUDA one by its import table, so both slots can
        # be filled without asking. ⚠ Only ever fills what is EMPTY; a deliberate choice stands.
        if not (cfg.get("host") and cfg.get("host_cpu") and cfg.get("host_cuda")):
            try:
                adopted = firstrun.adopt_hosts(cfg)
                if adopted != cfg:
                    cfg = adopted
                    self.results.put((self._apply_adopted, adopted))
            except Exception as e:      # noqa: BLE001 - discovery must never stop the window
                self.results.put((self._say, f"[setup] could not scan for a prover: {e}"))
        steps = firstrun.setup_steps(cfg)
        self.results.put((self._apply_steps, steps))

    def _apply_adopted(self, cfg):
        """Put discovered paths into the fields, on the main thread."""
        for key, var in (("host", self.host_var), ("host_cpu", self.host_cpu_var),
                         ("host_cuda", self.host_cuda_var)):
            if cfg.get(key) and not var.get().strip():
                var.set(cfg[key])
                self._say(f"[setup] found a prover: {cfg[key]}", "sys")
        if cfg.get("build_kind") and not self.build_kind.get():
            self.build_kind.set(cfg["build_kind"])
        self._persist()

    def _apply_steps(self, steps):
        self._render_steps(steps)
        self.v_setup.set(firstrun.summarise(steps))
        for s in steps:
            if s.key == "identity":
                self.v_handle.set(s.detail[:48])
        ok = firstrun.ready(steps)
        self._steps, self._setup_ok = steps, ok
        for s in steps:
            if s.key == "identity" and s.done:
                self._who = "proving as " + s.detail.split("  (key")[0]
                self.v_who.set(self._who)
        self._render_home_setup()
        self._sync_buttons()
        self._home_refresh()
        # ⭐ WHAT THE PROGRAM CAN FETCH, IT FETCHES. One at a time, each tried once per launch: a
        # download that fails says so on its own row with a "Try again", and is not retried in a
        # loop behind the person's back.
        if not ok:
            tried = self.__dict__.setdefault("_auto_fix_tried", set())
            nxt = next((s for s in steps if not s.done and s.fix and s.key not in tried), None)
            if nxt and not getattr(self, "_fixing", None):
                tried.add(nxt.key)
                self._run_fix(nxt)
        # ⚠ Only once setup is READY. Running them against a half-configured machine produces
        # failures that are about the setup, not the machine, and that is the Setup tab's job to say.
        if ok:
            self._auto_diagnostics()

    def _run_fix(self, step):
        self.v_setup.set(f"{step.fix_label or 'fixing'}…")
        self._fixing = self.PLAIN_STEPS.get(step.key, ("", ""))[1] or f"{step.fix_label or 'Fixing'}…"
        self._feed(self._fixing)
        self._home_refresh()

        def go():
            try:
                ok, detail = step.fix()
            except Exception as e:
                ok, detail = False, f"{type(e).__name__}: {e}"
            self.results.put((self._after_fix, (step.key, ok, detail)))
        threading.Thread(target=go, daemon=True).start()

    def _after_fix(self, args):
        key, ok, detail = args
        self._fixing = None
        name = self.PLAIN_STEPS.get(key, (key,))[0]
        self._feed(f"{name}: ready" if ok else f"{name}: could not be fetched — {detail}",
                   "good" if ok else "bad")
        self._say(f"[setup] {key}: {'done' if ok else 'failed'} — {detail}",
                  "proved" if ok else "cuda-error")
        # ⛔ KEEP IT. rescan() rebuilds every card from firstrun.setup_steps(), whose detail for a
        # missing worker is the generic "not downloaded yet" -- so without this the reason the fix
        # just failed is overwritten a few milliseconds after it is produced.
        if ok:
            self.fix_errors.pop(key, None)
        else:
            self.fix_errors[key] = detail
        if ok and key == "worker":
            self.worker_var.set(detail)
        self._persist()
        self.rescan()

    def _choose_host(self):
        p = filedialog.askopenfilename(title="Select host.exe",
                                       filetypes=[("Windows executable", "*.exe"), ("All", "*.*")])
        if p:
            self._use_host(p)

    def _use_host(self, path):
        self.host_var.set(path)
        info = supervisor.classify_host(path)
        self._say(f"[setup] host: {supervisor.host_kind_sentence(info)}",
                  "proved" if info.get("startable") else "oom")
        self._persist()
        self.rescan()

    def _set_handle(self):
        name = self.handle_var.get().strip()
        if not name:
            messagebox.showinfo("A name", "Type the name you want the board to credit.")
            return
        cfg = self._snapshot()
        self.v_handle.set("setting…")

        def go():
            ok, detail = firstrun.set_handle(cfg["worker"], name, cfg["host"], cfg["identity"],
                                            cfg["coord"])
            self.results.put((self._after_handle, (ok, detail)))
        threading.Thread(target=go, daemon=True).start()

    def _after_handle(self, args):
        ok, detail = args
        self.v_handle.set(detail[:60])
        if ok:
            self.fix_errors.pop("identity", None)
            self._feed(f"Your name on the board is set: {self.handle_var.get().strip()}", "good")
        else:
            self.fix_errors["identity"] = detail
        self._say(f"[setup] name: {detail}", "proved" if ok else "cuda-error")
        self.rescan()

    # ── config ──────────────────────────────────────────────────────────────────────────────────
    def _snapshot(self):
        """Every setting, read on the MAIN thread — Tk variables may not be touched elsewhere."""
        return {"host": self.host_var.get().strip(), "worker": self.worker_var.get().strip(),
                "identity": self.ident_var.get().strip(), "coord": self.coord_var.get().strip(),
                "workers": int(self.nworkers.get()), "seg_po2": self.po2.get().strip(),
                "dark": bool(self.dark.get()), "mode": self.mode.get(),
                "host_cpu": self.host_cpu_var.get().strip(),
                "host_cuda": self.host_cuda_var.get().strip(),
                "build_kind": self.build_kind.get(),
                "force_gpu": bool(self.force_gpu.get())}

    def _persist(self):
        self.cfg = self._snapshot()
        ok, detail = firstrun.save_config(self.cfg)
        if not ok:
            self._say(f"[setup] {detail}", "oom")

    # ── tab: dashboard ──────────────────────────────────────────────────────────────────────────
    def _tab_board(self):
        """How far the whole project has got: the numbers, and the map of every block."""
        p = self.p
        t = tk.Frame(self.nb, bg=p["fog"])
        self.nb.add(t, text="  The board  ")
        self._tab_dash(t)
        self._tab_map(t)

    def _tab_dash(self, t):
        p = self.p

        self.v_proven = tk.StringVar(value="—")
        self.v_folded = tk.StringVar(value="—")
        self.v_anchored = tk.StringVar(value="—")
        self.v_pct = tk.StringVar(value="—")
        grid = tk.Frame(t, bg=p["fog"])
        grid.pack(fill="x", padx=4, pady=10)
        for col, (title, var, note) in enumerate([
                ("PROVEN", self.v_proven, "blocks with a verified proof"),
                ("FOLDED", self.v_folded, "proofs combined into ranges"),
                ("ANCHORED", self.v_anchored, "one receipt back to genesis"),
                ("OF THE CHAIN", self.v_pct, "share of all blocks mined")]):
            c = self._card(grid, title)
            c.grid(row=0, column=col, sticky="nsew", padx=6)
            self._big(c, var).pack(anchor="w", padx=12)
            tk.Label(c, text=note, bg=p["mist"], fg=p["slate"],
                     font=("Segoe UI", 8), wraplength=190,
                     justify="left").pack(anchor="w", padx=12, pady=(0, 10))
            grid.columnconfigure(col, weight=1)

        mid = tk.Frame(t, bg=p["fog"])
        mid.pack(fill="x", padx=4, pady=4)
        self.v_next = tk.StringVar(value="—")
        self.v_folds = tk.StringVar(value="—")
        self.v_contrib = tk.StringVar(value="—")
        for col, (title, var, note) in enumerate([
                ("NEXT BLOCK TO PROVE", self.v_next, "the next one waiting for a prover"),
                ("WAITING TO BE COMBINED", self.v_folds, "pairs of finished proofs"),
                ("PEOPLE PROVING", self.v_contrib, "everyone who has landed a block")]):
            c = self._card(mid, title)
            c.grid(row=0, column=col, sticky="nsew", padx=6)
            tk.Label(c, textvariable=var, bg=p["mist"], fg=p["lamp_text"],
                     font=("Segoe UI", 15, "bold")).pack(anchor="w", padx=12)
            tk.Label(c, text=note, bg=p["mist"], fg=p["slate"],
                     font=("Segoe UI", 8)).pack(anchor="w", padx=12, pady=(0, 10))
            mid.columnconfigure(col, weight=1)

    # ── tab: block map ──────────────────────────────────────────────────────────────────────────
    def _tab_map(self, t):
        p = self.p
        bar = tk.Frame(t, bg=p["fog"])
        bar.pack(fill="x", padx=10, pady=(10, 4))
        ttk.Button(bar, text="Refresh", style="Quiet.TButton",
                   command=self.refresh).pack(side="right")
        tk.Label(bar, text="Every block, in order — each square is a range.",
                 bg=p["fog"], fg=p["slate"], font=("Segoe UI", 9)).pack(side="left")
        self.v_percell = tk.StringVar(value="")
        tk.Label(bar, textvariable=self.v_percell, bg=p["fog"], fg=p["slate"],
                 font=("Segoe UI", 9)).pack(side="left", padx=10)

        leg = tk.Frame(t, bg=p["fog"])
        leg.pack(fill="x", padx=10)
        for st in (api.ANCHORED, api.FOLDED, api.PROVEN, api.OPEN):
            sw = tk.Frame(leg, bg=brand.state_colours(self.dark.get())[st],
                          width=14, height=14, highlightbackground=p["haze"], highlightthickness=1)
            sw.pack(side="left", padx=(10, 4))
            tk.Label(leg, text=api.STATE_NAMES[st], bg=p["fog"], fg=p["slate"],
                     font=("Segoe UI", 8)).pack(side="left")

        # ⚠ BLACK, in both themes — the site's map ground. The colours were chosen against it.
        self.map_canvas = tk.Canvas(t, bg=brand.MAP_SURFACE, highlightthickness=0)
        self.map_canvas.pack(fill="both", expand=True, padx=10, pady=8)
        self.map_canvas.bind("<Button-1>", self._map_click)
        self.map_canvas.bind("<Configure>", lambda e: self._draw_map())

        self.v_cell = tk.StringVar(value="select a square to see its range")
        tk.Label(t, textvariable=self.v_cell, bg=p["fog"], fg=p["ink"],
                 font=("Segoe UI", 10)).pack(anchor="w", padx=12, pady=(0, 10))

    def _draw_map(self):
        c = self.map_canvas
        c.delete("all")
        self._cells = []
        if not self.runs or not self.meta.get("tip"):
            c.create_text(14, 14, anchor="nw", text="Loading the board…",
                          fill="#93a0aa", font=("Segoe UI", 10))
            return
        w = max(c.winfo_width(), 200)
        h = max(c.winfo_height(), 120)
        cell, gap = 9, 2
        per_row = max(10, (w - 20) // (cell + gap))
        rows_max = max(4, (h - 20) // (cell + gap))
        rows, per_cell = api.map_rows(self.runs, self.meta["tip"], per_row, rows_max)
        self.v_percell.set(f"{per_cell:,} blocks per square · {self.meta['tip']:,} blocks total")
        cols = brand.state_colours(self.dark.get())
        for r, row in enumerate(rows):
            for i, (st, lo, hi) in enumerate(row):
                x = 10 + i * (cell + gap)
                y = 10 + r * (cell + gap)
                iid = c.create_rectangle(x, y, x + cell, y + cell, fill=cols[st], outline="")
                self._cells.append((iid, lo, hi, st))

    def _map_click(self, ev):
        hit = self.map_canvas.find_closest(ev.x, ev.y)
        if not hit:
            return
        for iid, lo, hi, st in self._cells:
            if iid == hit[0]:
                n = hi - lo + 1
                self.v_cell.set(f"blocks {lo:,}–{hi:,}  ({n:,} block{'s' if n > 1 else ''})  —  "
                                f"least advanced state here: {api.STATE_NAMES[st]}")
                return

    # ── tab: run ────────────────────────────────────────────────────────────────────────────────
    def _tab_run(self):
        p = self.p
        t = tk.Frame(self.adv, bg=p["fog"])
        self.adv.add(t, text="  Options and log  ")

        box = self._card(t, "WHAT TO WORK ON")
        box.pack(fill="x", padx=10, pady=10)
        # ⛔ NO "pick any open block" OPTION, DELIBERATELY. The coordinator owns allocation: it hands
        # out work and tracks claims, and /api/blockstatus deliberately excludes claims because
        # "they change by the second". Letting a user choose an arbitrary open block would hand out
        # work someone else already holds.
        for val, label, note in [
            ("auto", "Prove — the coordinator chooses the block  (recommended)",
             "asks for the next block that needs proving, proves it, and sends the proof in"),
            ("range", "Prove a specific range",
             "only if you were asked to — the coordinator still has to agree you may take it"),
            ("spine", "Anchor — join finished proofs onto the chain",
             "the one job that must be done in order, one step at a time; light work"),
            ("fold", "Fold — combine finished proofs",
             "two adjacent proofs become one; this is what anchoring is fed by"),
        ]:
            ttk.Radiobutton(box, text=label, value=val, variable=self.mode).pack(anchor="w", padx=14)
            tk.Label(box, text=note, bg=p["mist"], fg=p["slate"],
                     font=("Segoe UI", 8)).pack(anchor="w", padx=36, pady=(0, 6))
        rowr = tk.Frame(box, bg=p["mist"])
        rowr.pack(anchor="w", padx=36, pady=(0, 10))
        tk.Label(rowr, text="range", bg=p["mist"], fg=p["slate"]).pack(side="left")
        self.range_var = tk.StringVar(value="")
        ttk.Entry(rowr, textvariable=self.range_var, width=18).pack(side="left", padx=6)
        tk.Label(rowr, text="e.g. 142141 or 142141-142150", bg=p["mist"], fg=p["slate"],
                 font=("Segoe UI", 8)).pack(side="left")

        ctrl = self._card(t, "HOW")
        ctrl.pack(fill="x", padx=10, pady=4)
        row = tk.Frame(ctrl, bg=p["mist"])
        row.pack(anchor="w", padx=14, pady=10)
        tk.Label(row, text="workers", bg=p["mist"], fg=p["ink"]).pack(side="left")
        self.nworkers = tk.IntVar(value=int(self.cfg.get("workers") or 1))
        ttk.Spinbox(row, from_=1, to=8, width=4, textvariable=self.nworkers).pack(side="left", padx=6)
        tk.Label(row, text="segment size (HAZYNC_SEG_PO2)", bg=p["mist"], fg=p["ink"]
                 ).pack(side="left", padx=(16, 4))
        self.po2 = tk.StringVar(value=self.cfg.get("seg_po2") or "")
        ttk.Entry(row, textvariable=self.po2, width=6).pack(side="left")
        tk.Label(row, text="blank = default; lower uses less graphics memory", bg=p["mist"],
                 fg=p["slate"], font=("Segoe UI", 8)).pack(side="left", padx=6)
        tk.Label(ctrl, text="Start and Stop are on the Home page.", bg=p["mist"], fg=p["slate"],
                 font=("Segoe UI", 8)).pack(anchor="w", padx=14, pady=(0, 8))

        diag = self._card(t, "TEST THIS MACHINE  (no claims are made, nothing is submitted)")
        diag.pack(fill="x", padx=10, pady=4)
        tk.Label(diag, bg=p["mist"], fg=p["slate"], font=("Segoe UI", 8), justify="left",
                 wraplength=920,
                 text=("Run these before proving for real. They use only the block-170 fixtures "
                       "built into the prover — no network, no coordinator, and no block is "
                       "claimed, so nothing here can hold anyone else up.")
                 ).pack(anchor="w", padx=12, pady=(2, 6))
        # ⭐ ONE ROW PER CHECK, EACH WITH ITS OWN RESULT. A single shared status line showed only
        # whichever check ran last, so three results collapsed into one and the earlier two looked
        # as though they had never been run.
        self.diag_btns, self.diag_vars = {}, {}
        for name, title, note, _timeout, auto in supervisor.DIAGNOSTICS:
            drow = tk.Frame(diag, bg=p["mist"])
            drow.pack(fill="x", anchor="w", padx=12, pady=(0, 4))
            btn = ttk.Button(drow, text=title, width=24,
                             command=lambda n=name: self._diagnose(n))
            btn.pack(side="left", padx=(0, 10))
            self.diag_btns[name] = btn
            var = tk.StringVar(value="runs automatically" if auto else "not run — press when ready")
            self.diag_vars[name] = var
            tk.Label(drow, textvariable=var, bg=p["mist"], fg=p["slate"],
                     font=("Segoe UI", 9), justify="left", wraplength=700, anchor="w"
                     ).pack(side="left", fill="x", expand=True)
        self.v_diag = tk.StringVar(value="")
        tk.Label(diag, textvariable=self.v_diag, bg=p["mist"], fg=p["ink"],
                 font=("Segoe UI", 9), justify="left", wraplength=920
                 ).pack(anchor="w", padx=12, pady=(4, 10))

        logf = self._card(t, "THE TECHNICAL LOG")
        logf.pack(fill="both", expand=True, padx=10, pady=8)
        self.log = tk.Text(logf, wrap="word", bg=p["fog"], fg=p["ink"],
                           insertbackground=p["ink"], relief="flat")
        self.log.pack(side="left", fill="both", expand=True, padx=(10, 0), pady=8)
        sb = ttk.Scrollbar(logf, command=self.log.yview)
        sb.pack(side="right", fill="y", pady=8)
        self.log.configure(yscrollcommand=sb.set, state="disabled")
        for tag, key in (("cuda-error", "bad"), ("abort", "bad"), ("oom", "lamp_text"),
                         ("proved", "good"), ("accepted", "good"), ("claimed", "lamp_text"),
                         ("sys", "slate")):
            self.log.tag_configure(tag, foreground=p[key])

    def _import_key(self):
        """Bring an existing signing key onto this machine."""
        path = filedialog.askopenfilename(
            title="Select key.hex from the machine that already has your identity",
            filetypes=[("Signing key", "*.hex"), ("All files", "*.*")])
        if not path:
            return
        ok, msg = firstrun.import_identity(path, self.ident_var.get() or None)
        self.v_key.set(("\u2713 " if ok else "\u26d4 ") + msg)
        self._say(f"[identity] {'imported' if ok else 'refused'} — {msg}",
                  "proved" if ok else "cuda-error")
        if ok:
            self.rescan()

    def _backup_key(self):
        """⚠ Copy the signing key somewhere safe. Losing it means losing every block's credit."""
        path = filedialog.asksaveasfilename(
            title="Save a copy of your signing key", initialfile="hazync-key.hex",
            defaultextension=".hex", filetypes=[("Signing key", "*.hex")])
        if not path:
            return
        ok, msg = firstrun.backup_identity(path, self.ident_var.get() or None)
        self.v_key.set(("\u2713 " if ok else "\u26d4 ") + msg)

    def _update_app(self):
        """Pull the newest version of this program, from inside it."""
        self.v_update.set("checking\u2026")

        def go():
            ok, msg = firstrun.app_update()
            self.results.put((self._after_update, (ok, msg)))
        threading.Thread(target=go, daemon=True).start()

    def _after_update(self, args):
        ok, msg = args
        self.v_update.set(("\u2713 " if ok else "\u26d4 ") + msg)
        self._say(f"[update] {msg}", "proved" if ok else "cuda-error")

    def _apply_recommended(self):
        """Set this machine up from what the card actually is.

        ⛔ THE POINT IS THAT NOBODY SHOULD HAVE TO KNOW ANY OF THIS. Which of two host binaries,
        whether this GPU is eligible at all, and what segment size fits its VRAM are answerable
        from nvidia-smi in under a second — and every one of them was previously learned by hitting
        it as a failure.
        """
        # ⛔ IT DOES THE LOOKING TOO. Driven headlessly 2026-10-05, this ran before the background
        # scan had finished and answered "choose the CPU build above and it will be used" — a
        # button called "Work it out for me" telling someone to work it out themselves. Reading
        # nvidia-smi and scanning for binaries are both I/O, so both happen off the main thread.
        self.v_build.set("looking at this machine\u2026")
        cfg = self._snapshot()

        def go():
            rec = supervisor.recommend()
            found = {}
            try:
                found = firstrun.adopt_hosts(cfg)
            except Exception as e:      # noqa: BLE001 - a failed scan must not lose the advice
                found = {"_error": str(e)}
            self.results.put((self._after_recommend, (rec, found)))
        threading.Thread(target=go, daemon=True).start()

    def _after_recommend(self, args):
        rec, found = args
        want = rec["build"]
        for key, var in (("host_cpu", self.host_cpu_var), ("host_cuda", self.host_cuda_var)):
            if found.get(key) and not var.get().strip():
                var.set(found[key])
        self._say(f"[setup] recommended: the {want.upper()} build — {rec['why']}", "sys")
        if rec.get("seg_po2"):
            self.po2.set(str(rec["seg_po2"]))
        have = (self.host_cpu_var if want == "cpu" else self.host_cuda_var).get().strip()
        if have:
            self.build_kind.set(want)
            self._use_build()
            self.v_build.set(rec["why"])
        else:
            # ⚠ Only now is "I could not find one" true, and it says what to get rather than just
            # which box to fill in.
            which = "hazync-host-windows-x86_64-" + ("cpu" if want == "cpu" else "cuda")
            self.v_build.set(rec["why"] + f"  \u2014 no {want.upper()} build found on this machine. "
                                          f"Download the `{which}` artifact and press Browse.")

    def _pick_build(self, kind):
        """Choose a prover for one slot, and CHECK it is the build that slot is for.

        ⛔ The two binaries are both called host.exe and both ~251 MB on this project, so a person
        cannot tell them apart in a file dialog — measured: two sat side by side differing by 1,024
        bytes. classify_host reads the import table, so it can say which is which; silently
        accepting a CUDA build as "the CPU one" would send someone back to the abort they were
        trying to escape.
        """
        path = filedialog.askopenfilename(title=f"Select the {kind.upper()} host.exe",
                                          filetypes=[("host.exe", "host*.exe"), ("All", "*.*")])
        if not path:
            return
        info = supervisor.classify_host(path)
        got = info.get("kind", "unknown")
        (self.host_cpu_var if kind == "cpu" else self.host_cuda_var).set(path)
        if got != kind and got != "unknown":
            self.v_build.set(f"⚠ that looks like the {got.upper()} build, not {kind.upper()} — "
                             f"{supervisor.host_kind_sentence(info)}")
        else:
            self.v_build.set(supervisor.host_kind_sentence(info))
        self.build_kind.set(kind)
        self._use_build()

    def _use_build(self):
        """Make the selected slot the active prover."""
        kind = self.build_kind.get()
        path = (self.host_cpu_var if kind == "cpu" else self.host_cuda_var).get().strip()
        if not path:
            self.v_build.set(f"no {kind.upper()} build chosen yet — press Browse")
            return
        # ⚠ REFRESH THE LINE, DO NOT LEAVE THE LAST ONE. Selecting a slot before filling it set
        # "no CUDA build chosen yet" and nothing ever cleared it, so a correctly configured row sat
        # under a message saying it was not configured — seen on a real machine.
        info = supervisor.classify_host(path)
        self.v_build.set(supervisor.host_kind_sentence(info))
        self.host_var.set(path)
        # ⚠ Changing the prover invalidates every check that was about the PREVIOUS binary, so they
        # are run again rather than left on screen describing a file that is no longer in use.
        self._auto_diag_done = False
        for k, var in getattr(self, "diag_vars", {}).items():
            var.set("not run for this prover yet")
        self._say(f"[setup] prover: using the {kind.upper()} build — {path}", "sys")
        self._persist()
        self.rescan()

    # ── diagnostics ─────────────────────────────────────────────────────────────────────────────
    def _diagnose(self, name, auto=False, then=None):
        """Run one diagnostic. `auto` = started by the program, so never interrupt with a dialog."""
        host = self.host_var.get().strip()
        po2 = self.po2.get().strip()
        if not host:
            # ⛔ A messagebox from the startup path would be a modal nobody asked for, on a window
            # that has only just opened, about a step the Setup tab is already reporting.
            if not auto:
                messagebox.showinfo("No prover", "Choose the prover program on the Home page first.")
            return
        title = next((t for n, t, *_ in supervisor.DIAGNOSTICS if n == name), name)
        timeout = next((d[3] for d in supervisor.DIAGNOSTICS if d[0] == name), 600)
        for b_ in self.diag_btns.values():
            b_.configure(state="disabled")
        self._diag_running = (name, time.time())
        self.diag_vars[name].set("running…")
        self.v_diag.set(f"{title}  — running…")
        self._say(f"[test] {name}: started", "sys")
        self._home_refresh()
        self._tick_diag()

        def go():
            env = dict(os.environ)
            if po2:
                env["HAZYNC_SEG_PO2"] = po2
            # ⚠ The prover forces RISC0_PROVER=local when unset; leave it alone rather than
            # second-guessing which backend it should pick.
            # ⛔ STREAMED. _run hands everything back only when the process EXITS, which for a
            # 2,875 s prove is forty minutes of a window saying "running…" and nothing else —
            # reported from a real machine as "there isn't any form of progress being shown".
            rc, out = supervisor.run_stream(
                supervisor.diagnostic_command(host, name), env=env, timeout=timeout,
                on_line=lambda l: self.results.put((self._diag_line, l)))
            self.results.put((self._after_diag, (name, title, rc, out, then)))
        threading.Thread(target=go, daemon=True).start()

    def _diag_line(self, line):
        """One line of live output from a running check."""
        self._say(f"    {line[:200]}", supervisor.interesting_line(line) or "sys")

    def _tick_diag(self):
        """Keep an elapsed count on the running row.

        ⚠ A long job that prints nothing for minutes is indistinguishable from a hung one, and the
        honest reading of a frozen window is "it broke". The clock moving is the cheapest possible
        proof that it has not.
        """
        if not getattr(self, "_diag_running", None):
            return
        name, t0 = self._diag_running
        secs = int(time.time() - t0)
        self.diag_vars[name].set(f"running… {secs // 60}m {secs % 60:02d}s")
        self.after(1000, self._tick_diag)

    def _after_diag(self, args):
        name, title, rc, out, then = args
        elapsed = int(time.time() - self._diag_running[1]) if getattr(self, "_diag_running", None) else 0
        self._diag_running = None
        ok, verdict = supervisor.diagnostic_verdict(name, rc, out)
        verdict = f"{verdict}  ({elapsed // 60}m {elapsed % 60:02d}s)" if elapsed else verdict
        self.diag_vars[name].set(f"{'PASSED' if ok else 'FAILED'}: {verdict}")
        self.v_diag.set(f"{title}  —  {'PASSED' if ok else 'FAILED'}: {verdict}")
        self._say(f"[test] {name}: {'passed' if ok else 'FAILED'} — {verdict}",
                  "proved" if ok else "cuda-error")
        self._feed(f"Check {'passed' if ok else 'FAILED'}: {title}"
                   + ("" if ok else f" — {verdict}"), "good" if ok else "bad")
        if not ok:
            self._problem = supervisor.explain(out) or f"{title} — {verdict}"
        self._home_refresh()
        # ⚠ The output was already streamed into the log line by line, so repeating the tail here
        # would print everything twice. Only the explanation is added below.
        if not ok:
            why = supervisor.explain(out)
            if why:
                self._say(f"  ⇒ {why}", "oom")
        for b_ in self.diag_btns.values():
            b_.configure(state="normal")
        # ⚠ The chain is advanced from HERE, on the main thread, not from the worker thread that ran
        # the check — self.after() and every widget call are Tk calls, and Tk is main-thread only.
        if then:
            then()

    # ⭐ THE CHEAP CHECKS RUN THEMSELVES. Pressing two buttons in a fixed order before every session
    # is work the program can do, and a person who does not press them gets no answer at all rather
    # than a wrong one. Only the ones flagged auto= in supervisor.DIAGNOSTICS run here: the third
    # takes 2,875 s on four CPU cores, and starting that uninvited would pin the machine for the
    # better part of an hour.
    def _auto_diagnostics(self):
        if self._auto_diag_done or not self.host_var.get().strip():
            return
        self._auto_diag_done = True
        queue_ = list(supervisor.AUTO_DIAGNOSTICS)

        def step():
            if not queue_:
                return
            nxt = queue_.pop(0)
            # ⚠ Sequential, not parallel: both drive the same binary and the same GPU, and two at
            # once would make a timing-sensitive check answer about a machine under load.
            self._diagnose(nxt, auto=True, then=step)
        step()

    # ── tab: settings ───────────────────────────────────────────────────────────────────────────
    def _tab_settings(self):
        p = self.p
        t = tk.Frame(self.adv, bg=p["fog"])
        self.adv.add(t, text="  Settings  ")

        paths = self._card(t, "WHERE THINGS ARE")
        paths.pack(fill="x", padx=10, pady=10)
        # ⚠ Saved values first, then a guess — so a path chosen once is never asked for again.
        c = self.cfg
        self.host_var = tk.StringVar(value=c.get("host") or self._guess("host.exe"))
        # ⭐ TWO PROVERS, REMEMBERED SEPARATELY. A machine that can run CUDA at all still needs the
        # CPU build when the GPU is below sppark's floor (hazync#631), so switching between them is
        # a normal thing to do — not a reason to go and find a path again. Re-typing one is how a
        # person ends up proving with whichever binary happened to be in the box.
        self.host_cpu_var = tk.StringVar(value=c.get("host_cpu") or "")
        self.host_cuda_var = tk.StringVar(value=c.get("host_cuda") or "")
        self.worker_var = tk.StringVar(value=c.get("worker") or self._guess("hazync-worker"))
        self.ident_var = tk.StringVar(value=c.get("identity") or str(firstrun.hazync_home()))
        self.coord_var = tk.StringVar(value=c.get("coord") or api.DEFAULT_COORD)
        inner = tk.Frame(paths, bg=p["mist"])
        inner.pack(fill="x", padx=12, pady=8)
        for r, (label, var, kind) in enumerate([
                ("host.exe (the prover)", self.host_var, "file"),
                ("hazync-worker (the client)", self.worker_var, "file"),
                ("identity folder (your key)", self.ident_var, "dir"),
                ("coordinator", self.coord_var, None)]):
            tk.Label(inner, text=label, bg=p["mist"], fg=p["ink"], width=26,
                     anchor="w").grid(row=r, column=0, sticky="w", pady=3)
            ttk.Entry(inner, textvariable=var, width=74).grid(row=r, column=1, sticky="we", pady=3)
            if kind:
                ttk.Button(inner, text="Browse…",
                           command=lambda v=var, d=(kind == "dir"): self._browse(v, d)
                           ).grid(row=r, column=2, padx=6)
        inner.columnconfigure(1, weight=1)

        build = self._card(t, "WHICH PROVER TO USE")
        build.pack(fill="x", padx=10, pady=4)
        tk.Label(build, justify="left", wraplength=920, bg=p["mist"], fg=p["ink"],
                 font=("Segoe UI", 9),
                 text=("Keep both and switch. The GPU build only works on a card sppark will "
                       "accept — compute capability 7.0 or newer; an older card is rejected with "
                       "\u201cno CUDA-capable device is detected\u201d however much VRAM it has. "
                       "The CPU build works anywhere and is slow.")
                 ).pack(anchor="w", padx=12, pady=(2, 6))
        bi = tk.Frame(build, bg=p["mist"])
        bi.pack(fill="x", padx=12, pady=(0, 10))
        self.build_kind = tk.StringVar(value=c.get("build_kind") or "")
        for r, (kind, label, var) in enumerate([
                ("cpu", "CPU build  (always works)", self.host_cpu_var),
                ("cuda", "GPU build  (CUDA)", self.host_cuda_var)]):
            ttk.Radiobutton(bi, text=label, value=kind, variable=self.build_kind,
                            command=self._use_build).grid(row=r, column=0, sticky="w", pady=3)
            ttk.Entry(bi, textvariable=var, width=62).grid(row=r, column=1, sticky="we", padx=8)
            ttk.Button(bi, text="Browse\u2026",
                       command=lambda k=kind: self._pick_build(k)).grid(row=r, column=2)
        # ⚠ OPT-IN, AND HONEST ABOUT WHAT IT IS. sppark refuses cards below compute 7.0 outright,
        # so an older card never gets as far as trying. We have ONE measurement of lowering that
        # floor -- a GTX 1050 Ti, which is then accepted and still cannot be driven (hazync#631) --
        # and nothing at all about other older cards. Without this nobody can find out whether
        # theirs works, because the env var is invisible to anyone who has not read sppark.
        # ⛔ Not automatic: for a Pascal card this REPLACES a clean "too old, use the CPU build"
        # with a deeper and more confusing failure, so it has to be a choice someone makes.
        self.force_gpu = tk.BooleanVar(value=bool(c.get("force_gpu")))
        ttk.Checkbutton(bi, variable=self.force_gpu,
                        text="Try my GPU even if it is below the supported floor "
                             "\u2014 unproven, and known to fail on Pascal (hazync#631)"
                        ).grid(row=2, column=0, columnspan=3, sticky="w", pady=(6, 0))
        bi.columnconfigure(1, weight=1)
        ttk.Button(build, text="Work it out for me", style="Accent.TButton",
                   command=self._apply_recommended).pack(anchor="w", padx=12, pady=(6, 2))
        self.v_build = tk.StringVar(value="")
        tk.Label(build, textvariable=self.v_build, bg=p["mist"], fg=p["slate"],
                 font=("Segoe UI", 9), justify="left", wraplength=920
                 ).pack(anchor="w", padx=12, pady=(0, 8))

        ident = self._card(t, "YOUR IDENTITY")
        ident.pack(fill="x", padx=10, pady=4)
        tk.Label(ident, justify="left", wraplength=920, bg=p["mist"], fg=p["ink"],
                 font=("Segoe UI", 9),
                 text=("Every block you prove is signed with a key in the identity folder, and the "
                       "board credits that key's handle. Keep it — losing it means losing the "
                       "credit for everything you have proved. `hazync-worker rotate` moves your "
                       "proved blocks to a new key if you need to replace it.")
                 ).pack(anchor="w", padx=12, pady=(4, 6))
        idrow = tk.Frame(ident, bg=p["mist"])
        idrow.pack(anchor="w", padx=12, pady=(0, 10))
        ttk.Button(idrow, text="Show my identity", style="Quiet.TButton",
                   command=self._show_identity).pack(side="left")
        ttk.Button(idrow, text="Import a key\u2026", style="Quiet.TButton",
                   command=self._import_key).pack(side="left", padx=(8, 0))
        ttk.Button(idrow, text="Back it up\u2026", style="Quiet.TButton",
                   command=self._backup_key).pack(side="left", padx=(8, 0))
        self.v_key = tk.StringVar(value="")
        tk.Label(ident, textvariable=self.v_key, bg=p["mist"], fg=p["slate"],
                 font=("Segoe UI", 9), justify="left", wraplength=900
                 ).pack(anchor="w", padx=12, pady=(0, 6))
        self.v_ident = tk.StringVar(value="not checked")
        tk.Label(idrow, textvariable=self.v_ident, bg=p["mist"], fg=p["lamp_text"],
                 font=("Consolas", 9)).pack(side="left", padx=10)

        upd = self._card(t, "THIS PROGRAM")
        upd.pack(fill="x", padx=10, pady=4)
        urow = tk.Frame(upd, bg=p["mist"])
        urow.pack(anchor="w", padx=12, pady=(4, 2))
        ttk.Button(urow, text="Update Hazync", style="Quiet.TButton",
                   command=self._update_app).pack(side="left", padx=(0, 10))
        self.v_update = tk.StringVar(value="")
        tk.Label(urow, textvariable=self.v_update, bg=p["mist"], fg=p["slate"],
                 font=("Segoe UI", 9), justify="left", wraplength=780).pack(side="left")
        tk.Label(upd, bg=p["mist"], fg=p["slate"], font=("Segoe UI", 9), justify="left",
                 wraplength=900,
                 text=("Pulls the newest version from GitHub. Close and reopen afterwards \u2014 a "
                       "running program keeps the code it started with.")
                 ).pack(anchor="w", padx=12, pady=(0, 8))

        chk = self._card(t, "CHECKS")
        chk.pack(fill="both", expand=True, padx=10, pady=8)
        crow = tk.Frame(chk, bg=p["mist"])
        crow.pack(anchor="w", padx=12, pady=8)
        ttk.Button(crow, text="Run checks", command=self._run_checks).pack(side="left")
        self.verdict = tk.Label(crow, text="not checked yet", bg=p["mist"], fg=p["ink"],
                                font=("Segoe UI", 10, "bold"))
        self.verdict.pack(side="left", padx=12)
        self.checks_box = tk.Text(chk, height=9, wrap="word", bg=p["fog"], fg=p["ink"],
                                  relief="flat")
        self.checks_box.pack(fill="both", expand=True, padx=12, pady=(0, 10))
        self.checks_box.configure(state="disabled")

    def _guess(self, name):
        here = Path(__file__).resolve().parent
        for base in (here, Path.cwd(), Path.home() / "Downloads", Path.home()):
            for sub in ("", "hazync-host-windows-x86_64-cuda", "hazync-host-windows-x86_64-cpu"):
                cand = base / sub / name if sub else base / name
                if cand.is_file():
                    return str(cand)
        return ""

    def _browse(self, var, want_dir):
        p = filedialog.askdirectory() if want_dir else filedialog.askopenfilename()
        if p:
            var.set(p)

    # ── data ────────────────────────────────────────────────────────────────────────────────────
    def refresh(self):
        # ⛔⛔ READ EVERY Tk VARIABLE HERE, ON THE MAIN THREAD. `StringVar.get()` from a worker
        # thread raises "RuntimeError: main thread is not in main loop" — intermittently, because
        # it depends on what the interpreter is doing when the thread happens to run. Caught by
        # actually running the window, not by reading the code.
        self.status.configure(text="refreshing from the coordinator…")
        coord = self.coord_var.get().strip() or api.DEFAULT_COORD
        threading.Thread(target=self._refresh_thread, args=(coord,), daemon=True).start()

    def _refresh_thread(self, coord):
        out = {}
        try:
            out["prog"] = api.progress(coord)
            out["meta"], out["runs"] = api.block_status(coord)
        except api.ApiError as e:
            out["err"] = str(e)
        except Exception as e:            # ⚠ anything else, or the thread dies with no trace at all
            out["err"] = f"{type(e).__name__}: {e}"
        # ⛔ THE BOARD IS SHOWN NOW, NOT AFTER THE EXTRAS. The two calls below are decoration — one
        # number each — and /api/pick was measured at 0.4 to 7.1 s on the coordinator itself
        # (2026-10-09). Waiting for them held the totals and the whole block map, which had arrived
        # in half a second, behind "refreshing…" for as long as the slowest took.
        self.results.put((self._apply, out))
        if "err" in out:
            return
        extra = {}
        for key, path in (("pick", "/api/pick"), ("fold", "/api/foldable?limit=1")):
            try:
                extra[key] = api.fetch(path, coord)
            except Exception:             # noqa: BLE001 - a missing extra is a dash, not an error
                extra[key] = None
        self.results.put((self._apply_extras, extra))

    def _apply(self, out):
        # ⚠ Scheduled FIRST, on both paths. It used to sit after the early return, so one failed
        # refresh — a laptop waking from sleep is enough — meant the board never refreshed again.
        self.after(REFRESH_S * 1000, self.refresh)
        if "err" in out:
            self.status.configure(text=f"could not reach the coordinator — {out['err']}")
            return
        self.prog, self.meta, self.runs = out["prog"], out["meta"], out["runs"]
        p = self.prog
        self.v_proven.set(f"{p.get('proven', 0):,}")
        self.v_folded.set(f"{p.get('folded', 0):,}")
        self.v_anchored.set(f"{p.get('spine_hi', 0):,}")
        self.v_pct.set(f"{p.get('pct', 0)}%")
        self.v_contrib.set(str(p.get("contributors", "—")))
        self._draw_map()
        self.status.configure(text=api.summarise_progress(p))

    def _apply_extras(self, extra):
        pick = extra.get("pick") or {}
        self.v_next.set(activity.pretty(pick.get("range", "—")) if pick else "—")
        fold = extra.get("fold") or {}
        self.v_folds.set(f"{fold.get('count', '—')}+" if fold else "—")

    def _show_identity(self):
        # ⚠ Same rule: snapshot the Tk variables on this thread first.
        host, worker = self.host_var.get(), self.worker_var.get()
        ident, coord = self.ident_var.get() or None, self.coord_var.get().strip() or None

        def go():
            env = supervisor.worker_env(host, worker, ident, None, coord_url=coord)
            rc, out = supervisor._run(
                supervisor.worker_command(sys.executable, worker, "id"),
                env=env, timeout=60)
            txt = (out or "").strip().splitlines()
            msg = txt[-1][:90] if rc == 0 and txt else f"could not read it (exit {rc})"
            self.results.put((self.v_ident.set, msg))
        threading.Thread(target=go, daemon=True).start()

    # ── checks ──────────────────────────────────────────────────────────────────────────────────
    def _run_checks(self):
        self.verdict.configure(text="checking…")
        host, worker = self.host_var.get(), self.worker_var.get()
        ident = self.ident_var.get() or None
        threading.Thread(target=self._checks_thread, args=(host, worker, ident),
                         daemon=True).start()

    def _checks_thread(self, host, worker, ident):
        checks, fatal = supervisor.preflight(host, worker, identity_dir=ident)
        lines = [f"{'ok  ' if c.ok else 'FAIL'}  {c.name}: {c.detail}" for c in checks]
        self.results.put((lambda a: self._show_checks(*a),
                          (lines, supervisor.summarise(checks), bool(fatal))))

    def _show_checks(self, lines, verdict, fatal):
        self.checks_box.configure(state="normal")
        self.checks_box.delete("1.0", "end")
        self.checks_box.insert("end", "\n".join(lines))
        self.checks_box.configure(state="disabled")
        self.verdict.configure(text=verdict, fg=self.p["bad"] if fatal else self.p["good"])
        self._sync_buttons()
        if fatal:
            self.start_btn.configure(state="disabled")

    # ── output ──────────────────────────────────────────────────────────────────────────────────
    def _say(self, text, tag="sys"):
        self.q.put((tag, text))

    def _drain(self):
        wrote = False
        try:
            while True:
                tag, text = self.q.get_nowait()
                self.log.configure(state="normal")
                self.log.insert("end", text.rstrip() + "\n", tag)
                self.log.configure(state="disabled")
                wrote = True
        except queue.Empty:
            pass
        if wrote:
            self.log.see("end")
        try:
            while True:
                fn, arg = self.results.get_nowait()
                try:
                    fn(arg)
                except Exception as e:                 # a bad result must not stop the window
                    self._say(f"could not apply a background result: {e}")
        except queue.Empty:
            pass
        self._reap()
        self.after(POLL_MS, self._drain)

    # ── workers ─────────────────────────────────────────────────────────────────────────────────
    def _start(self):
        checks, fatal = supervisor.preflight(self.host_var.get(), self.worker_var.get(),
                                             identity_dir=self.ident_var.get() or None)
        if fatal:
            messagebox.showerror("Cannot start", fatal[0].detail)
            self._show_checks([f"{'ok  ' if c.ok else 'FAIL'}  {c.name}: {c.detail}" for c in checks],
                              supervisor.summarise(checks), True)
            self._open_advanced(2)
            return
        self._problem = ""
        mode = self.mode.get()
        extra = ()
        job = "run"
        if mode == "fold":
            job = "fold"
        elif mode == "spine":
            # ⛔⛔ THE JOB THE BOARD WAS MISSING. Measured 2026-10-06: the spine had not advanced
            # for 20.4 h with proof 136,432 waiting and 7,746 blocks of proven work above it,
            # because nobody was running this — the window offered prove, a range and fold, and
            # not the one serial job that anchors any of it.
            job = "spine"
        elif mode == "range":
            rng = self.range_var.get().strip()
            if not rng:
                messagebox.showerror("No range", "Enter a range, or let the coordinator choose.")
                return
            extra = (rng,)
        base = Path(self.ident_var.get() or Path.home() / ".hazync") / "gui-workers"
        base.mkdir(parents=True, exist_ok=True)
        for i in range(1, max(1, int(self.nworkers.get())) + 1):
            self._spawn(i, base, job, extra)
        self._feed(f"Started — {activity.JOB_NAMES[job].lower()}"
                   + (f", {len(self.workers)} at once" if len(self.workers) > 1 else ""))
        self._sync_buttons()
        self._home_refresh()
        self.status.configure(text=f"{len(self.workers)} worker(s) running — {job}")

    def _spawn(self, i, base, job, extra):
        bundle = base / f"bundles_{i}"
        bundle.mkdir(parents=True, exist_ok=True)
        log_path = base / f"worker_{i}.log"
        # ⚠ Only when the box is ticked AND the card is actually below the floor: setting it on a
        # modern card would change nothing and make the log claim something untrue about the run.
        floor = None
        if self.force_gpu.get():
            g = supervisor.gpu_facts()
            if g and g["cc_major"] < 7:
                floor = g["cc_major"]
                self._say(f"[setup] trying your {g['name']} anyway — HAZYNC_SPPARK_MIN_MAJOR="
                          f"{floor} lowers sppark's floor. This is unproven; if it fails, the CPU "
                          f"build works.", "sys")
        env = supervisor.worker_env(self.host_var.get(), self.worker_var.get(),
                                    self.ident_var.get() or None, bundle,
                                    coord_url=self.coord_var.get().strip() or None,
                                    seg_po2=(self.po2.get().strip() or None),
                                    spark_min_major=floor)
        cmd = supervisor.worker_command(sys.executable, self.worker_var.get(), job, extra)
        kwargs = {}
        if supervisor.IS_WINDOWS:
            kwargs["creationflags"] = getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
        p = subprocess.Popen(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                             text=True, bufsize=1, errors="replace", **kwargs)
        w = Worker(i, p, log_path, job)
        self.workers.append(w)
        self._say(f"[{job} {i}] started (pid {p.pid}) — log {log_path}")
        threading.Thread(target=self._pump, args=(w,), daemon=True).start()

    def _pump(self, w):
        try:
            with open(w.log_path, "a", encoding="utf-8", errors="replace") as fh:
                for line in w.popen.stdout:
                    fh.write(line)
                    fh.flush()
                    # ⚠ Handed to the main thread: reading it updates the Home page, and every
                    # widget call is a Tk call.
                    self.results.put((self._on_worker_line, (w, line)))
                    kind = supervisor.interesting_line(line)
                    # ⚠ Remembered on the WORKER, not globally: with several workers running, one
                    # card's hard failure must not make another worker's ordinary exit look like a
                    # GPU fault. _explained below is deliberately global (say it once), but this
                    # decides whether to invite a retry, so it has to be per worker.
                    if kind == "cuda-error":
                        w.saw_cuda_error = True
                    if kind:
                        self.q.put((kind, f"[{w.job} {w.index}] {line.rstrip()}"))
                        # ⭐ A known failure gets its plain-language meaning right underneath,
                        # once — every one of these cost real time to diagnose the first time.
                        if kind in ("cuda-error", "abort", "oom") and kind not in self._explained:
                            why = supervisor.explain(line)
                            if why:
                                self._explained.add(kind)
                                self.q.put(("sys", f"  ⇒ {why}"))
        except Exception as e:
            self.q.put(("sys", f"[{w.job} {w.index}] stopped reading output: {e}"))

    def _on_worker_line(self, arg):
        """One line from a prover, on the main thread: keep the Home page current."""
        w, line = arg
        w.heard = time.time()
        before = w.activity.done
        event = w.activity.feed(line)
        landed = w.activity.done - before
        if landed > 0:
            self._landed[w.activity.job] += landed
        if event:
            self._feed(event, "good" if landed > 0 else "sys")
        kind = supervisor.interesting_line(line)
        if kind in ("cuda-error", "abort", "oom"):
            # ⭐ The plain-language meaning, not the raw line: "Rust cannot catch foreign
            # exceptions" tells a newcomer nothing, and each of these cost real time to learn.
            why = supervisor.explain(line) or line.strip()[:240]
            if why != self._problem:
                self._problem = why
                self._feed(why, "bad")
        self._home_refresh()

    def _reap(self):
        for w in list(self.workers):
            rc = w.popen.poll()
            if rc is None:
                continue
            self.workers.remove(w)
            if w.stopping:
                self._say(f"[{w.job} {w.index}] stopped")
                self._feed("Stopped")
                continue
            kind, msg = supervisor.classify_exit(rc, saw_cuda_error=getattr(w, "saw_cuda_error", False))
            # ⛔ THE DEFAULT WAS "oom", so any exit this code did not recognise was presented as the
            # GPU running out of memory — a specific, actionable and usually wrong claim, the same
            # mistake as the log matcher that read the worker's own advice as evidence. When the
            # window does not know what happened it must not invent a cause.
            tag = {"config": "cuda-error", "gpu": "cuda-error",
                   "idle": "sys", "done": "proved"}.get(kind, "sys")
            self._say(f"[{w.job} {w.index}] {msg}", tag)
            self._feed(msg, {"cuda-error": "bad", "proved": "good"}.get(tag, "sys"))
            if kind in ("config", "gpu") and not self._problem:
                self._problem = msg
            if kind in ("config", "gpu"):
                self._say("⛔ not restarting — and do not just press Start again: a box that cannot "
                          "prove claims blocks and abandons them, which holds up everyone else.",
                          "cuda-error")
        # ⛔ str(), because ttk returns a Tcl_Obj that never compares equal to a string. Without it
        # this condition silently never fires and Start never comes back. tk returns a plain str,
        # which is why the mistake is invisible in a quick test.
        if not self.workers and str(self.stop_btn["state"]) == "normal":
            self._sync_buttons()
            self._home_refresh()
            self.status.configure(text=api.summarise_progress(self.prog) if self.prog else "idle")

    def _stop(self):
        for w in self.workers:
            w.stopping = True
            try:
                subprocess.run(supervisor.stop_command(w.popen.pid), capture_output=True, timeout=30)
            except Exception as e:
                self._say(f"[{w.job} {w.index}] could not stop cleanly: {e}")
        if self.workers:
            self._feed("Stopping — letting the prover finish cleanly")
            self._home_refresh()
        self.status.configure(text="stopping…")

    def _on_close(self):
        if self.workers and not messagebox.askokcancel(
                "Quit", f"{len(self.workers)} worker(s) are still running. Stop them and quit?"):
            return
        self._persist()
        self._stop()
        self.destroy()


def main():
    if not supervisor.IS_WINDOWS:
        print("note: not on Windows — the fcntl shim stays OFF and the real module is used.")
    App().mainloop()
    return 0


if __name__ == "__main__":
    sys.exit(main())
