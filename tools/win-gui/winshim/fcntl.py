"""A Windows stand-in for POSIX `fcntl`, so the UNMODIFIED release worker starts.

⛔⛔ THIS IS THE ONE HARD BLOCKER TO RUNNING `hazync-worker` ON WINDOWS, and it is not a graceful
degradation — it is an ImportError before the program does anything. The worker does a MODULE-LEVEL
`import contextlib, fcntl`, and `fcntl` is POSIX-only, so on Windows the process dies at import.

⚠ The worker's own `gpu_lock()` has an `except OSError: proceed rather than refuse to work` fallback,
which reads like it would cope. It never gets the chance: the import fails first.

⇒ Put this directory on PYTHONPATH and the release worker runs unmodified. That matters because
`hazync-worker` is shipped as a release asset and its source is not in this repository, so patching
it here is not an option — and a GUI that required a forked worker would drift from the real one.

⭐ AND IT IS A REAL LOCK, NOT A STUB. `msvcrt.locking` gives genuine mandatory byte-range locking on
Windows, so `gpu_lock()` keeps doing its actual job: serialising GPU work across workers on one box.
A no-op stub would have been easier and would have let N workers hammer one GPU simultaneously —
which on a 4 GB card is how you turn "slow" into "out of memory".

⚠ WHAT IS DELIBERATELY NOT IMPLEMENTED. Only `flock`, `LOCK_EX`, `LOCK_SH`, `LOCK_UN` and `LOCK_NB`
— the four names the worker actually uses. `ioctl`, `fcntl` and the rest raise, loudly, rather than
silently doing nothing: a future worker that starts using them should fail where the gap is, not
somewhere downstream.

⛔ A PREVIOUS IMPORT SHIM IN THIS PROJECT BROKE ON PYTHON 3.12 because it used `find_module`, which
3.12 removed — it blocked nothing in CI while passing locally on 3.10. This one is a plain module on
`sys.path`, with no import-system hooks at all, so there is no equivalent to go stale. It is still
tested against the behaviour, not the description.
"""
import os

__all__ = ["flock", "LOCK_EX", "LOCK_SH", "LOCK_UN", "LOCK_NB"]

# The POSIX values, so anything comparing against them behaves.
LOCK_SH = 1
LOCK_EX = 2
LOCK_NB = 4
LOCK_UN = 8

try:                                  # pragma: no cover - present only on Windows
    import msvcrt
except ImportError:                   # pragma: no cover - importing this on POSIX is a mistake
    msvcrt = None

# ⚠ One byte at offset 0 is locked, not the whole file. The worker only ever needs mutual exclusion
# on a lock FILE, and a one-byte range is the portable way to express that with msvcrt.
_NBYTES = 1


def _fileno(fh):
    """Accept a file object or a raw fd, as POSIX flock does."""
    return fh if isinstance(fh, int) else fh.fileno()


def flock(fh, operation):
    """Lock or unlock `fh`. Raises OSError on failure, which is what callers already handle.

    ⚠ BLOCKING IS EMULATED BY RETRYING. `msvcrt.locking` with LK_LOCK retries for about ten seconds
    and then raises, whereas POSIX `LOCK_EX` waits indefinitely. The worker's gpu_lock can wait
    through a long fold — measured folds on this project run to tens of minutes — so a ten-second
    ceiling would turn a busy GPU into a spurious error. ⇒ Retry until it is acquired.
    """
    if msvcrt is None:
        raise OSError("this fcntl shim is for Windows; on POSIX use the real module")
    fd = _fileno(fh)

    if operation & LOCK_UN:
        try:
            msvcrt.locking(fd, msvcrt.LK_UNLCK, _NBYTES)
        except OSError:
            # ⚠ Unlocking something not locked is not worth failing over: the worker unlocks in a
            # `finally`, and raising there would mask whatever actually went wrong inside the block.
            pass
        return

    # LK_NBLCK fails immediately; LK_LOCK retries ~10s then raises. Non-blocking callers get the
    # former and the real OSError; blocking callers get the latter in a loop.
    if operation & LOCK_NB:
        msvcrt.locking(fd, msvcrt.LK_NBLCK, _NBYTES)
        return

    while True:
        try:
            msvcrt.locking(fd, msvcrt.LK_LOCK, _NBYTES)
            return
        except OSError:
            # ⛔ Keep waiting. See the docstring: a ten-second cap would report a busy GPU as a fault,
            # and this project has the scar — every fold error was once reported as a lock error.
            continue


def _unsupported(name):
    def fn(*_a, **_k):
        raise NotImplementedError(
            f"fcntl.{name} is not implemented by the hazync Windows shim. Only flock/LOCK_* are, "
            f"because they are what hazync-worker uses. If the worker now needs {name}, implement "
            f"it here rather than working around it at the call site."
        )
    return fn


ioctl = _unsupported("ioctl")
fcntl = _unsupported("fcntl")
lockf = _unsupported("lockf")


def _self_test():
    """Prove the module's own contract, on whatever platform it is run.

    ⛔ On POSIX this shim must REFUSE rather than pretend: a silent no-op here would mean the real
    serialisation quietly stopped happening on the fleet, which is exactly the failure mode the
    worker's own comment describes ("nothing has been folded from genesis while the cause stayed
    invisible").
    """
    import tempfile
    path = os.path.join(tempfile.gettempdir(), "hazync-fcntl-shim-selftest")
    fh = open(path, "a+")
    try:
        if msvcrt is None:
            try:
                flock(fh, LOCK_EX)
            except OSError as e:
                print(f"  ok   on POSIX the shim REFUSES rather than pretending: {e}")
                return 0
            print("  FAIL on POSIX the shim silently accepted a lock — that would hide a real lock")
            return 1
        flock(fh, LOCK_EX)
        print("  ok   LOCK_EX acquired via msvcrt")
        flock(fh, LOCK_UN)
        print("  ok   LOCK_UN released")
        flock(fh, LOCK_EX | LOCK_NB)
        print("  ok   LOCK_NB acquired when free")
        flock(fh, LOCK_UN)
        with open(path, "a+") as other:
            try:
                flock(other, LOCK_EX | LOCK_NB)
                print("  FAIL a second LOCK_NB succeeded while the first was held")
                return 1
            except OSError:
                print("  ok   a second LOCK_NB is refused while held — it is a REAL lock")
        return 0
    finally:
        try:
            fh.close()
        except Exception:
            pass


if __name__ == "__main__":
    import sys
    sys.exit(_self_test())
