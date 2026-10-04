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

⛔⛔ READ tools/win-gui/README.md FIRST. Native Windows CUDA proving has never completed, and this
window has never run on Windows. Both are stated there rather than left for a user to discover.
"""
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
import brand  # noqa: E402
import hazync_api as api  # noqa: E402
import supervisor  # noqa: E402

POLL_MS = 150
REFRESH_S = 60          # the website refreshes the map every minute; match it rather than hammer


class Worker:
    def __init__(self, index, popen, log_path, job):
        self.index, self.popen, self.log_path, self.job = index, popen, log_path, job
        self.started = time.time()
        self.stopping = False


class App(tk.Tk):
    def __init__(self):
        super().__init__()
        self.title("Hazync")
        self.geometry("1040x740")
        self.minsize(900, 620)
        self.dark = tk.BooleanVar(value=False)
        self.p = brand.palette(False)
        self.q = queue.Queue()
        # ⛔ Results from worker threads come back HERE and are applied by _drain on the main
        # thread. Calling self.after() from a thread is itself a Tk call — the same class of
        # bug as reading a StringVar there, and it is why the first version loaded no data.
        self.results = queue.Queue()
        self.workers = []
        self.runs, self.meta, self.prog = [], {}, {}
        self._cells = []                 # (item_id, lo, hi, state) for the map
        self._build()
        self.after(POLL_MS, self._drain)
        self.after(400, self.refresh)
        self.protocol("WM_DELETE_WINDOW", self._on_close)

    # ── chrome ──────────────────────────────────────────────────────────────────────────────────
    def _build(self):
        p = self.p
        self.configure(bg=p["fog"])

        head = tk.Frame(self, bg=p["fog"])
        head.pack(fill="x", padx=16, pady=(14, 6))
        logo = tk.Canvas(head, width=44, height=44, bg=p["fog"], highlightthickness=0)
        logo.pack(side="left")
        brand.draw_logo(logo, 2, 2, 40, dark=self.dark.get())
        tk.Label(head, text="Hazync", bg=p["fog"], fg=p["ink"],
                 font=("Segoe UI", 20, "bold")).pack(side="left", padx=(12, 0))
        tk.Label(head, text="prove Bitcoin's history, one block at a time",
                 bg=p["fog"], fg=p["slate"], font=("Segoe UI", 10)).pack(side="left", padx=10)
        ttk.Checkbutton(head, text="dark", variable=self.dark,
                        command=self._retheme).pack(side="right")
        ttk.Button(head, text="Refresh", command=self.refresh).pack(side="right", padx=8)

        self.nb = ttk.Notebook(self)
        self.nb.pack(fill="both", expand=True, padx=12, pady=6)
        self._tab_dash()
        self._tab_map()
        self._tab_run()
        self._tab_settings()

        self.status = tk.Label(self, text="starting…", anchor="w", bg=p["mist"], fg=p["slate"])
        self.status.pack(fill="x", side="bottom")

    def _retheme(self):
        # ⚠ A full re-theme of a live widget tree is fiddly and easy to get half-right. Rebuilding
        # is honest and instant at this size; a half-themed window looks broken.
        self.p = brand.palette(self.dark.get())
        for w in self.winfo_children():
            w.destroy()
        self._build()
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

    # ── tab: dashboard ──────────────────────────────────────────────────────────────────────────
    def _tab_dash(self):
        p = self.p
        t = tk.Frame(self.nb, bg=p["fog"])
        self.nb.add(t, text="  Dashboard  ")

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
                ("NEXT BLOCK FOR YOU", self.v_next, "chosen by the coordinator (/api/pick)"),
                ("FOLDS WAITING", self.v_folds, "adjacent pairs with no fold yet"),
                ("CONTRIBUTORS", self.v_contrib, "provers on the board")]):
            c = self._card(mid, title)
            c.grid(row=0, column=col, sticky="nsew", padx=6)
            tk.Label(c, textvariable=var, bg=p["mist"], fg=p["lamp_text"],
                     font=("Segoe UI", 15, "bold")).pack(anchor="w", padx=12)
            tk.Label(c, text=note, bg=p["mist"], fg=p["slate"],
                     font=("Segoe UI", 8)).pack(anchor="w", padx=12, pady=(0, 10))
            mid.columnconfigure(col, weight=1)

        # ⛔ The honest note goes on the first screen, not buried in a README nobody opens.
        warn = tk.Frame(t, bg=p["mist"], highlightbackground=p["haze"], highlightthickness=1)
        warn.pack(fill="x", padx=10, pady=12)
        tk.Label(warn, text="Before you start", bg=p["mist"], fg=p["slate"],
                 font=("Segoe UI", 9, "bold")).pack(anchor="w", padx=12, pady=(8, 2))
        tk.Label(warn, justify="left", wraplength=940, bg=p["mist"], fg=p["ink"],
                 font=("Segoe UI", 9),
                 text=("Run the checks on the Settings tab first. A prover whose METHOD_ID is not "
                       "canonical produces proofs the coordinator rejects, and a box that cannot "
                       "prove will claim blocks and abandon them — one machine once claimed and "
                       "abandoned 14 blocks in 15 minutes, which holds up everyone else.\n"
                       "Native Windows CUDA proving has not completed yet; the CPU build works and "
                       "is slow. See tools/win-gui/README.md.")
                 ).pack(anchor="w", padx=12, pady=(0, 10))

    # ── tab: block map ──────────────────────────────────────────────────────────────────────────
    def _tab_map(self):
        p = self.p
        t = tk.Frame(self.nb, bg=p["fog"])
        self.nb.add(t, text="  Block map  ")

        bar = tk.Frame(t, bg=p["fog"])
        bar.pack(fill="x", padx=10, pady=(10, 4))
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

        self.map_canvas = tk.Canvas(t, bg=p["fog"], highlightthickness=0)
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
            c.create_text(14, 14, anchor="nw", text="no data yet — press Refresh",
                          fill=self.p["slate"], font=("Segoe UI", 10))
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
                iid = c.create_rectangle(x, y, x + cell, y + cell, fill=cols[st],
                                         outline=self.p["haze"] if st == api.OPEN else "")
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
        t = tk.Frame(self.nb, bg=p["fog"])
        self.nb.add(t, text="  Prove  ")

        box = self._card(t, "WHAT TO WORK ON")
        box.pack(fill="x", padx=10, pady=10)
        self.mode = tk.StringVar(value="auto")
        # ⛔ NO "pick any open block" OPTION, DELIBERATELY. The coordinator owns allocation: it hands
        # out work and tracks claims, and /api/blockstatus deliberately excludes claims because
        # "they change by the second". Letting a user choose an arbitrary open block would hand out
        # work someone else already holds.
        for val, label, note in [
            ("auto", "Let the coordinator choose  (recommended)",
             "asks for the next block that needs proving, claims it, proves it, submits it"),
            ("range", "A specific range",
             "only if you were asked to — the coordinator still has to agree you may claim it"),
            ("fold", "Fold instead of prove",
             "combines adjacent proofs into ranges; this is what builds the spine"),
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
        self.nworkers = tk.IntVar(value=1)
        ttk.Spinbox(row, from_=1, to=8, width=4, textvariable=self.nworkers).pack(side="left", padx=6)
        tk.Label(row, text="HAZYNC_SEG_PO2", bg=p["mist"], fg=p["ink"]).pack(side="left", padx=(16, 4))
        self.po2 = tk.StringVar(value="")
        ttk.Entry(row, textvariable=self.po2, width=6).pack(side="left")
        tk.Label(row, text="blank = default; lower uses less VRAM", bg=p["mist"], fg=p["slate"],
                 font=("Segoe UI", 8)).pack(side="left", padx=6)
        self.start_btn = ttk.Button(row, text="Start", command=self._start)
        self.start_btn.pack(side="left", padx=(20, 6))
        self.stop_btn = ttk.Button(row, text="Stop", command=self._stop, state="disabled")
        self.stop_btn.pack(side="left")

        logf = self._card(t, "WHAT THE WORKERS ARE DOING")
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

    # ── tab: settings ───────────────────────────────────────────────────────────────────────────
    def _tab_settings(self):
        p = self.p
        t = tk.Frame(self.nb, bg=p["fog"])
        self.nb.add(t, text="  Settings  ")

        paths = self._card(t, "WHERE THINGS ARE")
        paths.pack(fill="x", padx=10, pady=10)
        self.host_var = tk.StringVar(value=self._guess("host.exe"))
        self.worker_var = tk.StringVar(value=self._guess("hazync-worker"))
        self.ident_var = tk.StringVar(value=str(Path.home() / ".hazync"))
        self.coord_var = tk.StringVar(value=api.DEFAULT_COORD)
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
        ttk.Button(idrow, text="Show my identity", command=self._show_identity).pack(side="left")
        self.v_ident = tk.StringVar(value="not checked")
        tk.Label(idrow, textvariable=self.v_ident, bg=p["mist"], fg=p["lamp_text"],
                 font=("Consolas", 9)).pack(side="left", padx=10)

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
        try:
            out["pick"] = api.fetch("/api/pick", coord)
        except api.ApiError:
            out["pick"] = None
        try:
            out["fold"] = api.fetch("/api/foldable?limit=1", coord)
        except api.ApiError:
            out["fold"] = None
        self.results.put((self._apply, out))

    def _apply(self, out):
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
        pick = out.get("pick") or {}
        self.v_next.set(f"{pick.get('range', '—')}" if pick else "—")
        fold = out.get("fold") or {}
        self.v_folds.set(f"{fold.get('count', '—')}+" if fold else "—")
        self._draw_map()
        self.status.configure(text=api.summarise_progress(p))
        self.after(REFRESH_S * 1000, self.refresh)

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
        self.start_btn.configure(state="disabled" if fatal else "normal")

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
            self.nb.select(3)
            return
        mode = self.mode.get()
        extra = ()
        job = "run"
        if mode == "fold":
            job = "fold"
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
        self.start_btn.configure(state="disabled")
        self.stop_btn.configure(state="normal")
        self.status.configure(text=f"{len(self.workers)} worker(s) running — {job}")

    def _spawn(self, i, base, job, extra):
        bundle = base / f"bundles_{i}"
        bundle.mkdir(parents=True, exist_ok=True)
        log_path = base / f"worker_{i}.log"
        env = supervisor.worker_env(self.host_var.get(), self.worker_var.get(),
                                    self.ident_var.get() or None, bundle,
                                    coord_url=self.coord_var.get().strip() or None,
                                    seg_po2=(self.po2.get().strip() or None))
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
                    kind = supervisor.interesting_line(line)
                    if kind:
                        self.q.put((kind, f"[{w.job} {w.index}] {line.rstrip()}"))
        except Exception as e:
            self.q.put(("sys", f"[{w.job} {w.index}] stopped reading output: {e}"))

    def _reap(self):
        for w in list(self.workers):
            rc = w.popen.poll()
            if rc is None:
                continue
            self.workers.remove(w)
            if w.stopping:
                self._say(f"[{w.job} {w.index}] stopped")
                continue
            kind, msg = supervisor.classify_exit(rc)
            tag = {"config": "cuda-error", "idle": "sys", "done": "proved"}.get(kind, "oom")
            self._say(f"[{w.job} {w.index}] {msg}", tag)
            if kind == "config":
                self._say("⛔ not restarting — and do not just press Start again: a box that cannot "
                          "prove claims blocks and abandons them, which holds up everyone else.",
                          "cuda-error")
        # ⛔ str(), because ttk returns a Tcl_Obj that never compares equal to a string. Without it
        # this condition silently never fires and Start never comes back. tk returns a plain str,
        # which is why the mistake is invisible in a quick test.
        if not self.workers and str(self.stop_btn["state"]) == "normal":
            self.stop_btn.configure(state="disabled")
            self.start_btn.configure(state="normal")
            self.status.configure(text=api.summarise_progress(self.prog) if self.prog else "idle")

    def _stop(self):
        for w in self.workers:
            w.stopping = True
            try:
                subprocess.run(supervisor.stop_command(w.popen.pid), capture_output=True, timeout=30)
            except Exception as e:
                self._say(f"[{w.job} {w.index}] could not stop cleanly: {e}")
        self.status.configure(text="stopping…")

    def _on_close(self):
        if self.workers and not messagebox.askokcancel(
                "Quit", f"{len(self.workers)} worker(s) are still running. Stop them and quit?"):
            return
        self._stop()
        self.destroy()


def main():
    if not supervisor.IS_WINDOWS:
        print("note: not on Windows — the fcntl shim stays OFF and the real module is used.")
    App().mainloop()
    return 0


if __name__ == "__main__":
    sys.exit(main())
