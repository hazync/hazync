"""No black console window for every block (Xesktop, 11-13).

The app has no console, so on Windows every console program it or its worker starts gets a NEW console window: the
worker's host.exe opened one per block, and the app's own helper calls (nvidia-smi, taskkill) flash one each.
hide_child_consoles() makes every child process this Python starts run with CREATE_NO_WINDOW (a console without a
window; output still goes wherever it was piped). The app calls it at start, and frozenpy calls it before running the
worker, so it covers the worker's children too, without touching the worker's code.
"""
import subprocess
import sys

CREATE_NO_WINDOW = 0x08000000


def hide_child_consoles(popen_cls=None, force=False):
    """Default every Popen in this process to CREATE_NO_WINDOW (Windows only; `force` for tests). Idempotent."""
    if sys.platform != "win32" and not force:
        return False
    cls = popen_cls or subprocess.Popen
    orig = cls.__init__
    if getattr(orig, "_hz_quiet", False):
        return True

    def __init__(self, *args, **kwargs):
        kwargs["creationflags"] = (kwargs.get("creationflags") or 0) | CREATE_NO_WINDOW
        return orig(self, *args, **kwargs)

    __init__._hz_quiet = True
    cls.__init__ = __init__
    return True
