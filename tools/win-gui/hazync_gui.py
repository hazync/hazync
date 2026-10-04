#!/usr/bin/env python3
"""Hazync for Windows — a window that runs the prover, for people who do not use a terminal.

    python hazync_gui.py

⛔ THE LOGIC IS NOT IN HERE. Everything that can be wrong — is this binary canonical, what does this
exit code mean, which environment does a worker need — lives in `supervisor.py`, which is headless
and tested (`test_supervisor.py`, plus a control). This file is the window: layout, threads, and
putting text on screen. A supervisor whose only entry point is a GUI cannot be tested on a build box.

⚠ WHAT THIS REPLACES, AND WHAT IT DOES NOT. A Linux contributor runs the `host` binary, the
`hazync-worker` Python CLI and `run-workers.sh`. This replaces the THIRD — the supervisor loop. The
host and the worker are used exactly as they ship.

⛔⛔ READ tools/win-gui/README.md BEFORE TRUSTING THIS. At the time of writing, native Windows CUDA
proving has NEVER completed: measured 2026-10-04 on a GTX 1050 Ti, `method-id` printed the canonical
id and `regress` passed, then the first GPU call aborted. The CPU build proves. So this GUI is
useful today for the CPU path and for everything around proving, and the CUDA path is still an open
question — stated here rather than discovered by a user.
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
import supervisor  # noqa: E402

POLL_MS = 150


class Worker:
    """One `hazync-worker run` process, its log, and how it ended."""

    def __init__(self, index, popen, log_path):
        self.index, self.popen, self.log_path = index, popen, log_path
        self.started = time.time()
        self.last_line = ""
        self.stopping = False


class App(tk.Tk):
    def __init__(self):
        super().__init__()
        self.title("Hazync — Windows prover")
        self.geometry("980x680")
        self.minsize(820, 560)
        self.q = queue.Queue()
        self.workers = []
        self._build()
        self.after(POLL_MS, self._drain)
        self.protocol("WM_DELETE_WINDOW", self._on_close)

    # ── layout ──────────────────────────────────────────────────────────────────────────────────
    def _build(self):
        pad = {"padx": 8, "pady": 4}

        setup = ttk.LabelFrame(self, text="1 · Where things are")
        setup.pack(fill="x", **pad)
        self.host_var = tk.StringVar(value=self._guess("host.exe"))
        self.worker_var = tk.StringVar(value=self._guess("hazync-worker"))
        self.ident_var = tk.StringVar(value=str(Path.home() / ".hazync"))
        for r, (label, var, isdir) in enumerate([
                ("host.exe (the prover)", self.host_var, False),
                ("hazync-worker (the client)", self.worker_var, False),
                ("identity folder", self.ident_var, True)]):
            ttk.Label(setup, text=label, width=26).grid(row=r, column=0, sticky="w", padx=6, pady=3)
            ttk.Entry(setup, textvariable=var, width=78).grid(row=r, column=1, sticky="we", pady=3)
            ttk.Button(setup, text="Browse…",
                       command=lambda v=var, d=isdir: self._browse(v, d)).grid(row=r, column=2, padx=6)
        setup.columnconfigure(1, weight=1)

        checks = ttk.LabelFrame(self, text="2 · Check before starting")
        checks.pack(fill="x", **pad)
        ttk.Button(checks, text="Run checks", command=self._run_checks).pack(side="left", padx=6, pady=6)
        self.verdict = ttk.Label(checks, text="not checked yet", font=("", 10, "bold"))
        self.verdict.pack(side="left", padx=10)
        self.checks_box = tk.Text(self, height=7, wrap="word")
        self.checks_box.pack(fill="x", padx=8)
        self.checks_box.configure(state="disabled")

        run = ttk.LabelFrame(self, text="3 · Prove")
        run.pack(fill="x", **pad)
        ttk.Label(run, text="workers").pack(side="left", padx=6)
        # ⛔ ONE BY DEFAULT. See supervisor.py: if the GPU lock is ever not working, N workers share
        # one card with no serialisation, which on a small card turns slow into out-of-memory.
        self.nworkers = tk.IntVar(value=1)
        ttk.Spinbox(run, from_=1, to=8, width=4, textvariable=self.nworkers).pack(side="left")
        ttk.Label(run, text="HAZYNC_SEG_PO2").pack(side="left", padx=(14, 4))
        self.po2 = tk.StringVar(value="")
        ttk.Entry(run, textvariable=self.po2, width=6).pack(side="left")
        ttk.Label(run, text="(blank = default; lower uses less VRAM)").pack(side="left", padx=6)
        self.start_btn = ttk.Button(run, text="Start", command=self._start)
        self.start_btn.pack(side="left", padx=10)
        self.stop_btn = ttk.Button(run, text="Stop", command=self._stop, state="disabled")
        self.stop_btn.pack(side="left")

        logf = ttk.LabelFrame(self, text="4 · What the workers are doing")
        logf.pack(fill="both", expand=True, **pad)
        self.log = tk.Text(logf, wrap="word")
        self.log.pack(side="left", fill="both", expand=True)
        sb = ttk.Scrollbar(logf, command=self.log.yview)
        sb.pack(side="right", fill="y")
        self.log.configure(yscrollcommand=sb.set, state="disabled")
        # ⭐ Colour only the lines a person must not miss; everything else stays plain, or the
        # important ones stop standing out.
        for tag, colour in (("cuda-error", "#b00020"), ("abort", "#b00020"), ("oom", "#b05a00"),
                            ("proved", "#0a6b0a"), ("accepted", "#0a6b0a"), ("claimed", "#004a99"),
                            ("sys", "#555555")):
            self.log.tag_configure(tag, foreground=colour)

        self.status = ttk.Label(self, text="idle", relief="sunken", anchor="w")
        self.status.pack(fill="x", side="bottom")

    def _guess(self, name):
        """Look beside the GUI and in the usual download spots, so most people never browse."""
        here = Path(__file__).resolve().parent
        for base in (here, Path.cwd(), Path.home() / "Downloads", Path.home()):
            for cand in (base / name, base / "hazync-host-windows-x86_64-cuda" / name,
                         base / "hazync-host-windows-x86_64-cpu" / name):
                if cand.is_file():
                    return str(cand)
        return ""

    def _browse(self, var, want_dir):
        p = filedialog.askdirectory() if want_dir else filedialog.askopenfilename()
        if p:
            var.set(p)

    # ── output plumbing ─────────────────────────────────────────────────────────────────────────
    def _say(self, text, tag="sys"):
        self.q.put((tag, text))

    def _drain(self):
        """Pull queued lines onto the screen. ⚠ Tk must only be touched from this thread."""
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
        self._reap()
        self.after(POLL_MS, self._drain)

    # ── checks ──────────────────────────────────────────────────────────────────────────────────
    def _run_checks(self):
        self.verdict.configure(text="checking…")
        threading.Thread(target=self._checks_thread, daemon=True).start()

    def _checks_thread(self):
        checks, fatal = supervisor.preflight(self.host_var.get(), self.worker_var.get(),
                                             identity_dir=self.ident_var.get() or None)
        lines = [f"{'ok  ' if c.ok else 'FAIL'}  {c.name}: {c.detail}" for c in checks]
        verdict = supervisor.summarise(checks)
        self.after(0, lambda: self._show_checks(lines, verdict, bool(fatal)))

    def _show_checks(self, lines, verdict, fatal):
        self.checks_box.configure(state="normal")
        self.checks_box.delete("1.0", "end")
        self.checks_box.insert("end", "\n".join(lines))
        self.checks_box.configure(state="disabled")
        self.verdict.configure(text=verdict)
        self.start_btn.configure(state="disabled" if fatal else "normal")

    # ── workers ─────────────────────────────────────────────────────────────────────────────────
    def _start(self):
        checks, fatal = supervisor.preflight(self.host_var.get(), self.worker_var.get(),
                                             identity_dir=self.ident_var.get() or None)
        if fatal:
            # ⛔ Never start past a fatal check. The whole point of the METHOD_ID check is that a
            # non-canonical binary proves into guaranteed rejection while looking busy.
            messagebox.showerror("Cannot start", fatal[0].detail)
            self._show_checks([f"{'ok  ' if c.ok else 'FAIL'}  {c.name}: {c.detail}" for c in checks],
                              supervisor.summarise(checks), True)
            return
        base = Path(self.ident_var.get() or Path.home() / ".hazync") / "gui-workers"
        base.mkdir(parents=True, exist_ok=True)
        n = max(1, int(self.nworkers.get()))
        for i in range(1, n + 1):
            self._spawn(i, base)
        self.start_btn.configure(state="disabled")
        self.stop_btn.configure(state="normal")
        self.status.configure(text=f"{len(self.workers)} worker(s) running")

    def _spawn(self, i, base):
        bundle = base / f"bundles_{i}"
        bundle.mkdir(parents=True, exist_ok=True)
        log_path = base / f"worker_{i}.log"
        env = supervisor.worker_env(self.host_var.get(), self.worker_var.get(),
                                    self.ident_var.get() or None, bundle,
                                    seg_po2=(self.po2.get().strip() or None))
        cmd = supervisor.worker_command(sys.executable, self.worker_var.get(), "run")
        kwargs = {}
        if supervisor.IS_WINDOWS:
            # ⚠ Its own process group, so stopping can take the TREE. The worker spawns the prover
            # as a child and killing only the parent orphans a prove that keeps holding the GPU.
            kwargs["creationflags"] = getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
        p = subprocess.Popen(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                             text=True, bufsize=1, errors="replace", **kwargs)
        w = Worker(i, p, log_path)
        self.workers.append(w)
        self._say(f"[worker {i}] started (pid {p.pid}), log {log_path}")
        threading.Thread(target=self._pump, args=(w,), daemon=True).start()

    def _pump(self, w):
        """Read one worker's output, mirror it to disk, and surface the lines that matter."""
        try:
            with open(w.log_path, "a", encoding="utf-8", errors="replace") as fh:
                for line in w.popen.stdout:
                    fh.write(line)
                    fh.flush()
                    w.last_line = line.rstrip()
                    kind = supervisor.interesting_line(line)
                    if kind:
                        self.q.put((kind, f"[worker {w.index}] {line.rstrip()}"))
        except Exception as e:                       # a dead pipe must not kill the UI thread
            self.q.put(("sys", f"[worker {w.index}] stopped reading output: {e}"))

    def _reap(self):
        """Report finished workers, using the supervisor's reading of the exit code."""
        for w in list(self.workers):
            rc = w.popen.poll()
            if rc is None:
                continue
            self.workers.remove(w)
            if w.stopping:
                self._say(f"[worker {w.index}] stopped")
                continue
            kind, msg = supervisor.classify_exit(rc)
            tag = {"config": "cuda-error", "idle": "sys", "done": "proved"}.get(kind, "oom")
            self._say(f"[worker {w.index}] {msg}", tag)
            if kind == "config":
                # ⛔ Do not restart. Retrying EX_CONFIG is how three GPUs once proved for a day into
                # guaranteed rejection; the supervisor's job is to notice and stop.
                self._say("⛔ not restarting this worker — see the message above.", "cuda-error")
        # ⛔ str(), BECAUSE ttk RETURNS A Tcl_Obj. `ttk.Button["state"] == "normal"` is ALWAYS
        # False — it prints as "normal" and compares unequal — so this condition silently never
        # fired and Start never came back after the workers finished. tk.Button returns a plain
        # str, which is why the mistake is easy to make and invisible in a quick test.
        if not self.workers and str(self.stop_btn["state"]) == "normal":
            self.stop_btn.configure(state="disabled")
            self.start_btn.configure(state="normal")
            self.status.configure(text="idle")

    def _stop(self):
        for w in self.workers:
            w.stopping = True
            try:
                subprocess.run(supervisor.stop_command(w.popen.pid), capture_output=True, timeout=30)
            except Exception as e:
                self._say(f"[worker {w.index}] could not stop cleanly: {e}")
        self.status.configure(text="stopping…")

    def _on_close(self):
        if self.workers and not messagebox.askokcancel(
                "Quit", f"{len(self.workers)} worker(s) are still running. Stop them and quit?"):
            return
        self._stop()
        self.destroy()


def main():
    if not supervisor.IS_WINDOWS:
        # ⚠ Not refused: the GUI is developed and tested on Linux against the CPU build, and
        # pretending otherwise would make it untestable. Just be clear which path is live.
        print("note: not running on Windows — the fcntl shim stays OFF and the real module is used.")
    App().mainloop()
    return 0


if __name__ == "__main__":
    sys.exit(main())
