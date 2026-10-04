"""The logic behind the Windows GUI, with no tkinter in it — so it can be TESTED.

⛔ WHY THE SPLIT. A supervisor whose only entry point is a window cannot be tested on a build box,
and this project's standing lesson is that a check which cannot run is a check that does not exist.
Everything here is importable and headless; `hazync_gui.py` is a thin shell over it.

## What this replaces

A contributor on Linux runs three things: the `host` binary, the `hazync-worker` Python CLI, and
`run-workers.sh`, which is a supervisor loop — start N workers, restart them on transient failure,
stop them on a configuration failure. The GUI replaces the THIRD of those. The host binary and the
worker are used exactly as they ship, which matters because `hazync-worker` is a release asset whose
source is not in this repository: a forked worker would drift from the one the fleet runs.

## The four Windows gaps, and which are fixable from out here

    import fcntl              ⛔ module-level, so the worker dies at startup   -> winshim/fcntl.py
    /tmp/hazync-gpu.lock      POSIX path                                      -> HAZYNC_GPU_LOCK
    hazync-host-x86_64-...    hardcoded Linux binary names                    -> HAZYNC_HOST
    os.killpg(..., SIGKILL)   no process groups on Windows                    -> NOT fixable here

⚠ The last one is honest residual risk: it sits on the stall/timeout path, so a worker that hangs
will raise AttributeError instead of killing its child cleanly. That needs the worker's source. It
is recorded in the README rather than papered over.

## ⛔ One worker by default on Windows, and the reason is not caution

`gpu_lock()` serialises GPU work across workers on a box. With the shim it is a REAL lock, so N
workers are safe — but if the shim is ever absent or fails, the worker's own fallback is "proceed
rather than refuse to work", and N workers would then share one GPU with no serialisation at all. On
a 4 GB card that converts "slow" into "out of memory". ⇒ Default 1, and only raise it once a run has
shown the lock working.
"""
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

IS_WINDOWS = os.name == "nt"

# The canonical guest image id. A binary printing anything else produces proofs the coordinator
# rejects with EX_CONFIG, so this is the first thing checked and the only one that is fatal.
CANONICAL_METHOD_ID = "37987b85ec665970ac6c5e8031deb8160ac8ed846f09056c3790b5f78c8bb5dd"

EX_CONFIG = 78      # the worker established it can never land anything — STOP, do not retry
EX_TEMPFAIL = 75    # nothing to claim right now — a busy board, not a fault

HEX64 = re.compile(r"\b[0-9a-f]{64}\b")


class CheckResult:
    """One named check with a verdict, its evidence, and whether it blocks starting."""

    def __init__(self, name, ok, detail, fatal=False):
        self.name, self.ok, self.detail, self.fatal = name, ok, detail, fatal

    def __repr__(self):
        return f"<{'ok' if self.ok else 'FAIL'} {self.name}: {self.detail}>"


def _run(cmd, env=None, timeout=60):
    """(rc, combined output). Never raises — a missing binary is a result, not a crash."""
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout,
                           env=env, errors="replace")
        return p.returncode, (p.stdout or "") + (p.stderr or "")
    except FileNotFoundError:
        return 127, f"not found: {cmd[0]}"
    except subprocess.TimeoutExpired:
        return -1, f"timed out after {timeout}s: {' '.join(map(str, cmd))}"
    except OSError as e:
        return -1, f"could not run {cmd[0]}: {e}"


def check_host_binary(host_path):
    """⛔ THE CHECK THAT MATTERS: does this binary carry the canonical METHOD_ID?

    ⚠ Parsed as "the first 64-hex token in the output", because `host method-id` prints the digest
    AND a `u32x8` decomposition, and a looser match would pick up the words. Measured output:

        METHOD_ID 37987b85ec665970ac6c5e8031deb8160ac8ed846f09056c3790b5f78c8bb5dd
          u32x8   [2239469623, ...]
    """
    host = Path(host_path)
    if not host.is_file():
        return CheckResult("host binary", False, f"no such file: {host}", fatal=True)
    rc, out = _run([str(host), "method-id"], timeout=120)
    if rc != 0:
        # ⛔ On Windows a CUDA-linked exe that cannot load its driver DLL fails HERE, and the message
        # is about a missing module rather than anything to do with the guest. Say which it is.
        hint = ""
        if rc == 127 or "not found" in out.lower() or "0xc" in out.lower():
            hint = ("  — this looks like Windows failing to LOAD the exe rather than the exe "
                    "failing. A CUDA build needs the NVIDIA driver (nvcuda.dll) and "
                    "cudart64_*.dll beside it.")
        return CheckResult("host binary", False, f"`method-id` exited {rc}: {out.strip()[:200]}{hint}",
                           fatal=True)
    m = HEX64.search(out)
    if not m:
        return CheckResult("host binary", False, f"no image id in output: {out.strip()[:160]}",
                           fatal=True)
    got = m.group(0)
    if got != CANONICAL_METHOD_ID:
        return CheckResult("host binary", False,
                           f"METHOD_ID is {got}, NOT the canonical {CANONICAL_METHOD_ID}. "
                           f"Proofs from this binary would be rejected (exit {EX_CONFIG}).",
                           fatal=True)
    return CheckResult("host binary", True, f"canonical METHOD_ID {got[:16]}…")


def check_gpu():
    """GPU name, VRAM and the DRIVER's CUDA version — all three matter, and not just for display.

    📏 Measured 2026-10-04: a GTX 1050 Ti with 4 GB and driver CUDA 12.7 ran `method-id` and
    `regress` fine and then aborted on the first GPU proving call. VRAM and the driver's CUDA version
    are the two numbers that make that diagnosable, so they are surfaced rather than hidden.
    """
    rc, out = _run(["nvidia-smi", "--query-gpu=name,memory.total,driver_version",
                    "--format=csv,noheader"], timeout=30)
    if rc != 0:
        rc2, out2 = _run(["nvidia-smi"], timeout=30)
        if rc2 != 0:
            return CheckResult("GPU", False,
                               "nvidia-smi did not run. Without an NVIDIA driver a CUDA build "
                               "cannot start at all; the CPU build still works.")
        return CheckResult("GPU", True, out2.strip().splitlines()[0][:120])
    line = out.strip().splitlines()[0] if out.strip() else ""
    note = ""
    mb = re.search(r"(\d+)\s*MiB", line)
    if mb and int(mb.group(1)) < 8000:
        # ⚠ Not presented as a failure: it has never been measured either way below 24 GB.
        note = ("  ⚠ under 8 GB. The smallest card measured to prove has 24 GB, and one prove at "
                "HAZYNC_SEG_PO2=21 peaked near 22 GB, so expect to lower HAZYNC_SEG_PO2.")
    return CheckResult("GPU", True, line + note)


def check_worker(worker_path, python_exe=None):
    """The worker must be present AND importable — on Windows that means the fcntl shim works."""
    w = Path(worker_path)
    if not w.is_file():
        return CheckResult("worker", False, f"no such file: {w}", fatal=True)
    py = python_exe or sys.executable
    env = worker_env(host_path=None, worker_path=w, identity_dir=None, bundle_dir=None)
    rc, out = _run([py, str(w), "--help"], env=env, timeout=60)
    if rc != 0 and "No module named 'fcntl'" in out:
        return CheckResult("worker", False,
                           "the worker cannot import `fcntl` — the Windows shim is not on "
                           "PYTHONPATH. That is a module-level import, so the worker dies at "
                           "startup rather than degrading.", fatal=True)
    if rc not in (0, 1):      # `--help` exits 0; a bare invocation exits 1 and both mean "it ran"
        return CheckResult("worker", False, f"`--help` exited {rc}: {out.strip()[:200]}", fatal=True)
    return CheckResult("worker", True, f"{w.name} starts and its CLI answers")


def check_signing_library(python_exe=None):
    """The worker signs its claims; without `cryptography` it exits before doing any work."""
    py = python_exe or sys.executable
    rc, out = _run([py, "-c", "import cryptography; print(cryptography.__version__)"], timeout=60)
    if rc != 0:
        return CheckResult("signing library", False,
                           "`cryptography` is not installed for this Python. The worker signs "
                           "every claim, so it cannot run without it:  pip install cryptography",
                           fatal=True)
    return CheckResult("signing library", True, f"cryptography {out.strip()}")


def gpu_lock_path():
    """Where the cross-worker GPU lock lives. ⚠ The worker's default is a POSIX /tmp path."""
    return str(Path(tempfile.gettempdir()) / "hazync-gpu.lock")


def shim_dir():
    return str(Path(__file__).resolve().parent / "winshim")


def worker_env(host_path, worker_path, identity_dir, bundle_dir, coord_url=None,
               seg_po2=None, base_env=None, force_shim=None):
    """The environment one worker runs with.

    ⚠ THE SHIM GOES ON PYTHONPATH ON WINDOWS ONLY — as cheap defence, not because shadowing is
    proven. MEASURED on the dev box: `fcntl` is a BUILT-IN module there (`fcntl.__file__` is
    `<built-in>`), so `BuiltinImporter` resolves it from sys.meta_path before the path finder and the
    shim cannot shadow it. ⚠ But that is a property of the Python BUILD: where `fcntl` ships as a
    shared extension in lib-dynload it is found via sys.path and the shim WOULD win, replacing real
    `flock` with one that refuses and silently ending the fleet's GPU serialisation. The gate costs
    nothing and removes the question; the shim also raises on POSIX rather than pretending, so even
    if it did load the mistake would be loud.
    """
    env = dict(base_env if base_env is not None else os.environ)
    use_shim = IS_WINDOWS if force_shim is None else force_shim
    if use_shim:
        existing = env.get("PYTHONPATH", "")
        env["PYTHONPATH"] = shim_dir() + (os.pathsep + existing if existing else "")
    if host_path:
        env["HAZYNC_HOST"] = str(host_path)
    if identity_dir:
        env["HAZYNC_HOME"] = str(identity_dir)
    if bundle_dir:
        env["BUNDLE_DIR"] = str(bundle_dir)
    if coord_url:
        env["COORD_URL"] = coord_url
    if seg_po2:
        env["HAZYNC_SEG_PO2"] = str(seg_po2)
    # ⚠ Always set, because the worker's default is /tmp and Windows has no /tmp.
    env["HAZYNC_GPU_LOCK"] = gpu_lock_path()
    # Unbuffered, or the GUI's log pane shows nothing until a worker exits.
    env["PYTHONUNBUFFERED"] = "1"
    return env


def classify_exit(rc):
    """(kind, human sentence) for a worker exit. The three cases are NOT interchangeable.

    ⛔ `run-workers.sh` learned this the hard way: retrying EX_CONFIG meant three GPUs proving for a
    day into guaranteed rejection, looking busy the whole time. A supervisor that treats every
    non-zero exit as transient reproduces exactly that.
    """
    if rc == 0:
        return "done", "finished cleanly"
    if rc == EX_CONFIG:
        return "config", ("misconfigured — retrying CANNOT help. Usually the guest id does not "
                          "match the coordinator, so nothing this worker proves would be accepted. "
                          "The supervisor stops rather than burning the GPU.")
    if rc == EX_TEMPFAIL:
        return "idle", "nothing to claim right now — a busy board, not a fault. Waiting."
    return "transient", f"exited {rc} — treated as transient and retried"


def interesting_line(line):
    """Should this worker output line be surfaced prominently? (kind or None)

    ⭐ `CUDA ERROR:` exists because of #631: sppark's throw is a foreign exception on MSVC, so the
    process aborts before any Rust handler runs and the real CUDA error used to be destroyed. It is
    now printed just before the throw, and it is the single most useful line a Windows user can see.
    """
    s = line.strip()
    low = s.lower()
    if s.startswith("CUDA ERROR:"):
        return "cuda-error"
    if "fatal runtime error" in low and "foreign exception" in low:
        return "abort"
    if "out of memory" in low:
        return "oom"
    if "proved" in low and "verified" in low:
        return "proved"
    if low.startswith("✓") or "the coordinator re-verified" in low:
        return "accepted"
    if "claimed block" in low:
        return "claimed"
    return None


def preflight(host_path, worker_path, identity_dir=None, python_exe=None):
    """Every check, in the order a person should read them. Fatal ones block starting."""
    checks = [
        check_host_binary(host_path),
        check_worker(worker_path, python_exe=python_exe),
        check_signing_library(python_exe=python_exe),
        check_gpu(),
    ]
    return checks, [c for c in checks if c.fatal and not c.ok]


def worker_command(python_exe, worker_path, job="run", extra=()):
    """The exact argv for one worker. `job` is one of the worker's own subcommands."""
    return [python_exe or sys.executable, str(worker_path), job, *extra]


def stop_command(pid):
    """How to stop a worker and its children WITHOUT os.killpg, which Windows lacks.

    ⚠ The worker spawns the prover as a child, and killing only the parent orphans a prove that goes
    on holding the GPU — `run-workers.sh` has that exact scar: "the ones already running were
    orphaned, not stopped". `taskkill /T` takes the tree.
    """
    if IS_WINDOWS:
        return ["taskkill", "/PID", str(pid), "/T", "/F"]
    return ["kill", "-TERM", str(pid)]


def summarise(checks):
    """A one-line verdict, for a status bar or a CLI run."""
    bad = [c for c in checks if not c.ok]
    fatal = [c for c in bad if c.fatal]
    if fatal:
        return f"cannot start: {fatal[0].name} — {fatal[0].detail}"
    if bad:
        return f"ready, with {len(bad)} warning(s): " + "; ".join(c.name for c in bad)
    return "ready"


def main(argv=None):
    """Headless preflight, so the checks can be run without a display or a GUI at all."""
    argv = list(sys.argv[1:] if argv is None else argv)
    if not argv or argv[0] in ("-h", "--help"):
        print("usage: python supervisor.py <host-binary> <hazync-worker> [identity-dir]")
        print("  Runs the same preflight the GUI runs, and prints it. No window needed.")
        return 0 if argv else 2
    host = argv[0]
    worker = argv[1] if len(argv) > 1 else ""
    ident = argv[2] if len(argv) > 2 else None
    checks, fatal = preflight(host, worker, identity_dir=ident)
    for c in checks:
        print(f"  {'ok  ' if c.ok else 'FAIL'} {c.name}: {c.detail}")
    print()
    print(summarise(checks))
    return 1 if fatal else 0


if __name__ == "__main__":
    sys.exit(main())
