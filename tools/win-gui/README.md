# Hazync on Windows — a GUI for the prover

A window that runs the prover, for people who do not use a terminal.

```
python hazync_gui.py
```

Needs Python 3.8+ with `tkinter` (bundled with the python.org installer) and `cryptography`
(`pip install cryptography` — the worker signs every claim and cannot run without it).

## ⛔ Read this first: what is proven and what is not

| | state |
|---|---|
| Windows CUDA build compiles, embeds the canonical guest | ✅ #631/#644 |
| `host.exe method-id` prints the canonical `37987b85…` on real hardware | ✅ measured 2026-10-04, GTX 1050 Ti |
| `host.exe regress` — full consensus path | ✅ measured, same machine |
| **CPU** build proves a block on Windows | ✅ measured |
| **CUDA** build proves a block on Windows | ⛔ **never completed** |
| This GUI drives a real worker end to end on Windows | ⛔ **not yet run on Windows** |

Native Windows **CUDA** proving aborts on the first GPU call:

```
execution time: 1.4686725s        ← guest executed, 54 segments
preflight: Segment { po2: 17 }    ← witness generation fine
prove_segment_core                ← dies here
fatal runtime error: Rust cannot catch foreign exceptions, aborting
```

Two theories were tested and **killed**: it is not out of memory (`HAZYNC_SEG_PO2` 18 and 17 abort
instantly and identically on an idle 4 GB card) and not an arch mismatch (the card *is* the `sm_61`
the binary embeds natively). Open: the build used CUDA 12.9.1 against a driver reporting 12.7.

⇒ **So today this GUI is useful for the CPU path** and for everything around proving — checking a
binary is canonical, managing identity, supervising workers, surfacing errors. The CUDA path is an
open question, and that is said here rather than discovered by a user.

## How it is put together

- **`supervisor.py`** — all the logic, headless and importable. Run it on its own:
  `python supervisor.py <host-binary> <hazync-worker>` prints the same preflight the GUI shows.
- **`hazync_gui.py`** — the window. Layout, threads, text on screen. No decisions.
- **`winshim/fcntl.py`** — a Windows stand-in for POSIX `fcntl`.
- **`test_supervisor.py`** — the tests, plus `--control`.

A supervisor whose only entry point is a window cannot be tested on a build box, which is why the
split exists. The logic is tested against the **real** Windows `host.exe` (via WSL interop) and the
**real** released `hazync-worker`, not mocks.

## What it replaces, and what it does not

A Linux contributor runs three things: the `host` binary, the `hazync-worker` Python CLI, and
`run-workers.sh` — a supervisor loop that starts N workers, retries transient failures and stops on a
configuration failure. **This GUI replaces the third.** The host binary and the worker are used
exactly as they ship, because `hazync-worker` is a release asset whose source is not in this
repository and a forked worker would drift from the one the fleet runs.

## The four Windows gaps in `hazync-worker`

| gap | fixed from outside the worker? |
|---|---|
| module-level `import fcntl` — the worker **dies at startup** | ✅ `winshim/fcntl.py` on `PYTHONPATH` |
| `/tmp/hazync-gpu.lock` — a POSIX path | ✅ `HAZYNC_GPU_LOCK` |
| hardcoded `hazync-host-x86_64-linux-gnu[-cuda]` | ✅ `HAZYNC_HOST` |
| `os.killpg(…, SIGKILL)` ×2 | ⛔ **no** |

⚠ The `import fcntl` one is not a graceful degradation. `gpu_lock()` has an
`except OSError: proceed rather than refuse to work` fallback that *looks* like it would cope, but
the import fails first, so the worker never starts at all.

⛔ **Residual risk:** `os.killpg` has no Windows equivalent and sits on the stall/timeout path, so a
hung worker will raise `AttributeError` instead of killing its child cleanly. That needs the worker's
source. Not papered over — recorded here.

⭐ The shim is a **real** lock (`msvcrt.locking`), not a stub, so `gpu_lock()` keeps serialising GPU
work across workers. A no-op would have let N workers share one card with no serialisation, which on
a small card turns "slow" into "out of memory".

## Why one worker by default

`gpu_lock()` serialises GPU work across workers on a box. With the shim that is genuine, so N
workers are safe. But if the shim is ever absent, the worker's own fallback is to proceed anyway, and
N workers would then share one GPU unserialised. Default 1; raise it once a run has shown the lock
working.

## Things the GUI will not do

- **Start past a fatal check.** A binary whose `METHOD_ID` is not canonical produces proofs the
  coordinator rejects with exit 78. `run-workers.sh` once had three GPUs proving for a day into
  guaranteed rejection, looking busy throughout.
- **Retry `EX_CONFIG` (78).** That means retrying cannot help. The supervisor stops and says why.
- **Treat `EX_TEMPFAIL` (75) as a fault.** That is a busy board, not an error.

## Not done yet

- Never run on Windows. The logic is tested; the window is not.
- No packaging. Needs a Python install today; PyInstaller would remove that, and matters if this is
  ever aimed at people who have not installed Python.
- No identity/key onboarding. Claims are signed, so a newcomer needs a key — `hazync-worker id`
  and `rotate` exist, and the GUI only displays what they report.
- `os.killpg` on the stall path, above.
