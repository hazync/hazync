"""The installed Hazync.exe is its own Python, for the worker and the setup checks.

The app runs the worker as `[sys.executable, hazync-worker, job, ...]` and its checks as `[sys.executable, "-c", code]`.
From source, sys.executable is python.exe. In the installed (PyInstaller) build it is Hazync.exe itself, so those calls
would open a second window instead of running anything. `run_as_python()` is called before the window is built: when
the frozen exe is started as an interpreter (`-c <code>` or `<script> args...`) it runs that code, as python.exe would,
and the process exits with its status; started any other way it returns None and the app opens.

Two things a frozen windowed exe lacks and a script needs:
  - PYTHONPATH is not read, so its entries are put on sys.path here (worker_env points it at winshim/, the fcntl shim);
  - a windowed build starts with sys.stdout / sys.stderr = None, so they are opened on the inherited handles (the app
    reads the worker's output through a pipe); with no handle there, writes go to os.devnull rather than crashing.
"""
import os
import runpy
import sys


def _streams():
    for fd, name in ((1, "stdout"), (2, "stderr")):
        if getattr(sys, name) is None:
            try:
                s = open(fd, "w", encoding="utf-8", errors="replace", buffering=1, closefd=False)
            except OSError:
                s = open(os.devnull, "w", encoding="utf-8")
            setattr(sys, name, s)


def run_as_python(argv=None):
    """Exit status when this process was started as an interpreter, else None (open the app)."""
    argv = list(sys.argv if argv is None else argv)
    if not getattr(sys, "frozen", False) or len(argv) < 2:
        return None
    first = argv[1]
    if first == "-c":
        if len(argv) < 3:
            return None
        mode = ("-c", argv[2], argv[3:])
    elif first.startswith("-") or not os.path.isfile(first):
        return None
    else:
        mode = ("script", first, argv[2:])
    for p in reversed([p for p in os.environ.get("PYTHONPATH", "").split(os.pathsep) if p]):
        if p not in sys.path:
            sys.path.insert(0, p)
    _streams()
    # the worker's own children (host.exe, once per block) get no console window either (winquiet)
    import winquiet
    winquiet.hide_child_consoles()
    kind, what, rest = mode
    try:
        if kind == "-c":
            sys.argv = ["-c", *rest]
            exec(compile(what, "<string>", "exec"), {"__name__": "__main__"})
        else:
            sys.argv = [what, *rest]
            sys.path.insert(0, os.path.dirname(os.path.abspath(what)))
            runpy.run_path(what, run_name="__main__")
    except SystemExit as e:
        code = e.code
        if code is None:
            return 0
        if isinstance(code, int):
            return code
        print(code, file=sys.stderr)
        return 1
    except BaseException:
        import traceback
        traceback.print_exc()
        return 1
    finally:
        for s in (sys.stdout, sys.stderr):
            try:
                s.flush()
            except Exception:
                pass
    return 0
