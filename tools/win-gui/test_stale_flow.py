#!/usr/bin/env python3
"""The Home card for an old prover or worker, the update banner, and the after-an-upgrade reset, in a real window.

    test_stale_flow.py          # needs a display; SKIPS without one, like test_gui_builds.py

⛔ THE CASE (11-18, v0.26.1 on a real laptop): the startup check showed "Stopped — this needs a look ... Download the
prover again." The remembered prover was an old host.exe (image 37987b85) and the remembered worker an old client
that could not read the board's input files; downloading changed neither. Here, through the window's own paths:

  * the startup check (the real _diagnose, running a stand-in host that prints the old image) puts the old path and
    its image on the card with ONE button, "Use the prover installed with Hazync", which switches to it;
  * an old worker gets "Use the client installed with Hazync";
  * Start against a board that has moved past this app's image says "Update Hazync to keep proving" with the button,
    and no error dialog;
  * a newer release shows the banner; Update downloads (mocked), and only a VERIFIED installer is run, then the app
    closes; a refused one is said and nothing runs;
  * the first start of a new app version replaces the remembered old prover and worker with the installed ones,
    and the startup checks wait for that.
Nothing touches the network or the real ~/.hazync.
"""
import os
import stat
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
fails = []


def check(ok, what):
    print(("  ok   " if ok else "  FAIL ") + what)
    if not ok:
        fails.append(what)


OLD = "37987b85ec665970" + "1" * 48


def host_script(path, image):
    """A stand-in `host` whose `method-id` prints `image` (POSIX shell; this test runs where CI's xvfb does)."""
    with open(path, "w") as f:
        f.write(f"#!/bin/sh\necho METHOD_ID {image}\n")
    os.chmod(path, os.stat(path).st_mode | stat.S_IEXEC)
    return path


def main():
    if os.name == "nt":
        print("  SKIP the stand-in prover is a shell script")
        return 0
    tmp = tempfile.mkdtemp()
    os.environ["HAZYNC_HOME"] = tmp
    try:
        import hazync_gui
        import supervisor
        import appupdate
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
    app.withdraw()

    def spin(secs, until=lambda: False):
        end = time.time() + secs
        while time.time() < end:
            app.update()
            if until():
                return True
            time.sleep(0.02)
        return until()

    BOARD = supervisor.CANONICAL_METHOD_ID
    appdir = os.path.join(tmp, "app")
    os.makedirs(os.path.join(appdir, "prover"))
    old_host = host_script(os.path.join(tmp, "old-host"), OLD)
    inst_host = host_script(os.path.join(appdir, "prover", "host.exe"), BOARD)
    old_worker = os.path.join(tmp, "old-worker")
    open(old_worker, "w").write('#!/usr/bin/env python3\nimport sys\nVERSION = "0.21.0"\n')
    inst_worker = os.path.join(appdir, "hazync-worker")
    open(inst_worker, "w").write('#!/usr/bin/env python3\nimport sys\nVERSION = "0.26.1"\n# LZLEAF\n')
    board = {"id": BOARD}
    supervisor.board_image = lambda coord, timeout=None: (board["id"], "")
    supervisor.installed_provers = lambda: {"cuda": inst_host}
    supervisor.installed_worker = lambda: inst_worker
    supervisor.classify_host = lambda p: {"kind": "cuda", "needs_driver": False, "driver_present": True,
                                          "dlls": [], "size": 1, "startable": True}
    errors, infos = [], []
    hazync_gui.messagebox.showerror = lambda *a, **k: errors.append(a)
    hazync_gui.messagebox.showinfo = lambda *a, **k: infos.append(a)
    app.ident_var.set(tmp)
    app._setup_ok, app._steps = True, [object()]
    app.mode.set("spine")
    try:
        print("── startup: the remembered prover is an old copy ──")
        app.worker_var.set(inst_worker)
        app.build_kind.set("cuda")
        app.host_cuda_var.set(old_host)
        app.host_var.set(old_host)
        app._diagnose("method-id", auto=True)
        spin(30, lambda: bool(app._problem))
        app._home_refresh()
        check(app.v_head.get() == "Stopped — the prover in use is an old copy", f"headline: {app.v_head.get()!r}")
        sub = app.v_sub.get()
        check(old_host in sub and "37987b85" in sub and BOARD[:8] in sub and inst_host in sub,
              f"the card names the old path, its image, the board's and the installed prover: {sub!r}")
        check("download" not in sub.lower(), "no 'download the prover again'")
        check(bool(app.fix_btn.winfo_manager()) and app.fix_btn.cget("text") == "Use the prover installed with Hazync",
              f"one button offers the fix: {app.fix_btn.cget('text')!r}")
        app.fix_btn.invoke()
        spin(1)
        check(app.host_cuda_var.get() == inst_host and app.host_var.get() == inst_host,
              f"one click uses the installed prover: {app.host_var.get()!r}")
        check(not app._problem and not app.fix_btn.winfo_manager(), "the card clears and the button goes")

        print("── startup: the remembered worker is an old copy ──")
        app.worker_var.set(old_worker)
        app._diagnose("method-id", auto=True)
        spin(30, lambda: bool(app._problem))
        app._home_refresh()
        check(old_worker in app.v_sub.get() and "cannot read the board's input files" in app.v_sub.get(),
              f"the card names the old worker: {app.v_sub.get()!r}")
        check(app.fix_btn.cget("text") == "Use the client installed with Hazync", f"button: {app.fix_btn.cget('text')!r}")
        app.fix_btn.invoke()
        spin(1)
        check(app.worker_var.get() == inst_worker, f"one click uses the installed client: {app.worker_var.get()!r}")

        print("── Start, when the board has moved past this app's image ──")
        board["id"] = "5" * 64
        real_pf = supervisor.preflight
        supervisor.preflight = lambda *a, **kw: (
            [supervisor.CheckResult("host binary", False, "makes the old image", fatal=True)],
            [supervisor.CheckResult("host binary", False, "makes the old image", fatal=True)])
        supervisor.gpu_cards = lambda: []
        n_err = len(errors)
        app._start()
        spin(15, lambda: not getattr(app, "_starting", False) and bool(app._problem))
        app._home_refresh()
        supervisor.preflight = real_pf
        check(app.v_head.get() == "Update Hazync to keep proving", f"headline: {app.v_head.get()!r}")
        check(len(errors) == n_err, "no error dialog: it is not a fault on this PC")
        check(bool(app.fix_btn.winfo_manager()) and app.fix_btn.cget("text") == "Install the latest Hazync",
              f"with the update button: {app.fix_btn.cget('text')!r}")
        board["id"] = BOARD
        app._problem = ""

        print("── a newer release: the banner, and Update ──")
        app._app_version = "0.26.1"
        appupdate.latest_tag = lambda get_json, timeout=20: "v0.27.0"
        app._update_tick()
        spin(5, lambda: app._update_tag == "v0.27.0")
        check(bool(app.update_bar.winfo_manager()) and "v0.27.0 is available" in app.v_update.get(),
              f"the banner: {app.v_update.get()!r}")
        ran, closed = [], []
        appupdate.launch_installer = lambda p: ran.append(p)
        app._on_close = lambda: closed.append(1)
        hazync_gui.messagebox.askyesno = lambda *a, **k: True
        appupdate.download_verified = lambda tag, dest, get, **k: (False, "BAD signature: not the maintainer key")
        app.update_btn.invoke()
        spin(5, lambda: not app._update_busy and errors)
        check(not ran and not closed and "BAD signature" in str(errors[-1:]),
              f"a refused download is said and nothing runs: {errors[-1:]}")
        inst = os.path.join(tmp, "hazync-setup-v0.27.0.exe")
        appupdate.download_verified = lambda tag, dest, get, **k: (True, inst)
        app.update_btn.invoke()
        spin(5, lambda: closed)
        check(ran == [inst] and closed, f"a verified installer is run and the app closes: {ran}")

        print("── the first start of a new version: old prover and worker are replaced ──")
        app.host_cuda_var.set(old_host)
        app.host_var.set(old_host)
        app.worker_var.set(old_worker)
        app.cfg["app_version_seen"] = "0.25.0"
        app._upgrade_pending = True
        app._auto_diag_done = False
        app._auto_diagnostics()
        check(not app._auto_diag_done, "the startup checks wait for the upgrade check")
        app._after_upgrade_check()
        spin(30, lambda: not app._upgrade_pending)
        check(app.host_cuda_var.get() == inst_host and app.host_var.get() == inst_host,
              f"the prover is the installed one: {app.host_var.get()!r}")
        check(app.worker_var.get() == inst_worker, f"the worker is the installed one: {app.worker_var.get()!r}")
        check(app.cfg.get("app_version_seen") == "0.26.1", "the version is remembered, so this runs once")
        feed = app.feed.get("1.0", "end")
        check(old_host in feed and old_worker in feed, "the feed names what was replaced")
    finally:
        try:
            app.destroy()
        except Exception:              # noqa: BLE001
            pass
    print()
    print("PASS" if not fails else f"FAIL ({len(fails)})")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
