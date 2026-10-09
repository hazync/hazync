# Hazync on Windows — a GUI for the prover

A window that runs the prover, for people who do not use a terminal.

```
python hazync_gui.py
```

Needs Python 3.8+ with `tkinter` (bundled with the python.org installer) and `cryptography`
(`pip install cryptography` — the worker signs every claim and cannot run without it).

## Getting it onto a Windows machine

```
git clone --depth 1 --filter=blob:none --sparse -b feat/windows-gui https://github.com/hazync/hazync
cd hazync
git sparse-checkout set tools/win-gui
cd tools/win-gui
```

Then **double-click `Hazync.bat`**. It finds Python, says what to install if there is none, and opens
the window with no console behind it.

The window opens on **Home**, which is the whole program for most people:

- **one choice** — Prove, Fold or Anchor, each with a sentence saying what it is;
- **one button** — Start, which becomes Stop while work runs;
- **what is happening** — which block, which piece of how many, how long is left, how long it has
  been running, how long since the prover last said anything, and a plain list of what has landed.

On first launch Home fetches what it can by itself (the signing tools and the client) and shows the
one or two things it needs from you: the prover program, and the name the board should credit.

**The board** shows the project's totals and the map of every block. **Advanced** holds everything a
newcomer should never need: the setup checklist, a specific range, worker count and segment size,
the machine tests, the technical log, paths, the CPU/GPU choice and your key.

## ⛔ Read this first: what is proven and what is not

| | state |
|---|---|
| Windows CUDA build compiles, embeds the canonical guest | ✅ #631/#644 |
| `host.exe method-id` prints the canonical `37987b85…` on real hardware | ✅ measured 2026-10-04, GTX 1050 Ti |
| `host.exe regress` — full consensus path | ✅ measured, same machine |
| **CPU** build proves a block on Windows | ✅ measured |
| This GUI opens and works on Windows | ✅ **tested on Windows Python 3.13** |
| This GUI drives a prove end to end on Windows | ✅ **measured 2026-10-05: `PROVED in 2220.7s — receipt VERIFIED against METHOD_ID`** |
| **CUDA** build proves a block on Windows | ⛔ **never completed — and now we know why** |

### Why CUDA proving fails, and what it is not

⛔⛔ **It is a CARD-SUPPORT question, not a Windows one.** Measured 2026-10-05 on a GTX 1050 Ti with
a build that prints the GPU's own error before discarding it:

```
CUDA ERROR: cudaErrorNoDevice@sppark/util/all_gpus.cpp:43
            failed: "no CUDA-capable device is detected"
```

on a machine where `nvidia-smi` lists the card and the driver is current. That line is
`CUDA_OK(cudaErrorNoDevice)` — a **synthetic** error raised when sppark's device list is **empty**.
The enumerator above it keeps only `prop.major >= PROP_MAJOR_MIN`, and sppark sets that to 7,
"Volta and forward". Pascal is compute 6.1, so the card is filtered out.

⭐ **The same card fails identically on Linux.** Windows only *destroyed* the error on its way out
(the foreign-exception abort below), which made a card-support question look like a platform
question for weeks. Two theories — out of memory, and a PTX/arch mismatch — were both wrong, because
both assumed the GPU had been accepted at all.

⇒ **Native Windows CUDA proving is UNTESTED, not broken.** It has never run on a card sppark
accepts. Testing it needs **compute capability 7.0 or newer**.

With `HAZYNC_SPPARK_MIN_MAJOR=6` (a build from 2026-10-05 or later) the 1050 Ti IS accepted —
`cooperativeLaunch=1`, so that second condition was never the obstacle — and then fails further in,
in the NTT:

```
GPU SCAN: 1 device(s), need major >= 6 and cooperativeLaunch
  [0] NVIDIA GeForce GTX 1050 Ti  compute 6.1  cooperativeLaunch=1  -> USED
CUDA ERROR: cudaGetLastError()@sppark/ntt/ntt.cuh:97 failed: "operation not supported"
panicked at risc0-zkp-3.0.5/src/hal/cuda.rs:708: Failure during zk_shift
```

⚠ Whether *that* is a Pascal limit or a Windows one is **not yet known** — `cudaGetLastError()` can
report an error raised by an earlier asynchronous call, so the line number may not be the culprit.
A run with `CUDA_LAUNCH_BLOCKING=1` is what settles it. See hazync#631.

The original abort, for reference — this is what you get on a build older than 2026-10-04, where the
real error was thrown away:

```
prove_segment_core                ← dies here
fatal runtime error: Rust cannot catch foreign exceptions, aborting
```

Two theories were tested and **killed**: it is not out of memory (`HAZYNC_SEG_PO2` 18 and 17 abort
instantly and identically on an idle 4 GB card) and not an arch mismatch (the card *is* the `sm_61`
the binary embeds natively). Open: the build used CUDA 12.9.1 against a driver reporting 12.7.

⇒ **So today this GUI is useful for the CPU path** and for everything around proving — checking a
binary is canonical, managing identity, supervising workers, surfacing errors. The CUDA path is an
open question, and that is said here rather than discovered by a user.

## What is in the window

**Dashboard** — the headline numbers, live from `/api/state`: proven, folded, **anchored**
(`spine_hi`), share of the chain, contributors. Plus the next block the coordinator would give you
(`/api/pick`) and how many folds are waiting (`/api/foldable`).

**Block map** — every block in order, one square per range, coloured by its *least advanced* state.
Click a square for its range. Deliberately pessimistic: a 6,736-block square containing one open
block reads **open**, because a square that showed its best state would paint a mostly-unproved
chain as proved and hide exactly the gaps a prover is looking for.

**Prove** — let the coordinator choose (recommended), a specific range, or fold instead of prove.
Worker count, `HAZYNC_SEG_PO2`, start/stop, and a log that colours only the lines you must not miss.

**Settings** — paths, your identity, coordinator URL, and the checks.

⛔ **There is no "pick any open block" button, on purpose.** The coordinator owns allocation: it
hands out work and tracks claims, and `/api/blockstatus` deliberately excludes claims because "they
change by the second". Letting someone choose an arbitrary open block would hand out work another
prover already holds. `/api/pick` is the coordinator's own answer and the Dashboard shows it.

## Testing this machine

The **Prove** tab has three buttons that run what you would otherwise type:

| | |
|---|---|
| Is this prover genuine? | `method-id` — must print the canonical id or proofs are rejected |
| Does consensus work here? | `regress` — block 170 through the full consensus path, seconds |
| Can it actually prove? | `prove-block` — a real STARK receipt from built-in fixtures |

⭐ **No block is claimed and nothing is submitted.** They use only the block-170 fixtures compiled
into the prover, so a machine that cannot prove learns that without holding up the board.

Each verdict reads the **output**, not the exit code — `regress` exiting 0 while printing no pass
line is reported as a failure, because a check that passes on silence is not a check. The last few
real lines are shown underneath, so the verdict always comes with its evidence.

Known failures are translated. `fatal runtime error: Rust cannot catch foreign exceptions` becomes
*"the GPU reported an error and this build discarded it — a build from 2026-10-04 or later prints a
CUDA ERROR line just above"*.

## Where the colours and logo come from

Both are read off what hazync.org actually serves, not eyeballed:

- the palette from `/assets/site.css` — its own `--ink/--fog/--mist/--haze/--slate/--lamp/--good/--bad`,
  in light **and** dark
- the logo from `/favicon.svg` — three rectangles in a 32×32 viewBox, so `brand.py` reproduces it
  **exactly** on a Canvas with no image file, no Pillow and no SVG renderer, sharp at any size.
  `brand.py`'s self-test asserts the drawn geometry still matches the real favicon byte for byte.

## How it is put together

- **`supervisor.py`** — all the logic, headless and importable. Run it on its own:
  `python supervisor.py <host-binary> <hazync-worker>` prints the same preflight the GUI shows.
- **`hazync_gui.py`** — the window. Layout, threads, text on screen. No decisions.
- **`winshim/fcntl.py`** — a Windows stand-in for POSIX `fcntl`.
- **`hazync_api.py`** — the coordinator's endpoints and the map's geometry, pure and testable.
- **`brand.py`** — the real palette and logo. `python brand.py` self-tests.
- **`test_supervisor.py`**, **`test_api.py`** — the tests, each with `--control`;
  `test_api.py --live` additionally checks the real API still has the shape the fixtures assume.

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

## More than one graphics card

A machine with two or more cards that can prove gets **one worker on each**, started seeing only its
own card (`CUDA_VISIBLE_DEVICES`, with `CUDA_DEVICE_ORDER=PCI_BUS_ID` so CUDA numbers the cards the
way `nvidia-smi` does) and taking a lock that belongs to that card alone. Before this every worker
shared one lock and none was told which card to use, so a second card sat idle. `cards.py` decides;
`test_cards.py` and `test_cards_flow.py` test it.

- The worker count can only grow: one worker asked for on three cards starts three; four asked for
  on two cards starts four, two to a card, taking turns.
- A card below the prover's floor (compute capability 7.0) gets no worker.
- A smaller card beside a bigger one gets its own, smaller segment size.
- Anchoring is never multiplied — it is one step after another, and a second worker only races the first.
- A machine with one usable card is untouched: nothing is pinned and the lock is the one it always was.
- "Use every graphics card" under Advanced → Options turns it off.

⛔ **Not yet run on a real two-card machine.** What is tested: the decision; that each real child
process receives its own card and lock; and (2026-10-09, one A40 on Linux) that the real prover
proves when pinned to the card it has and refuses when pinned to one it does not. Two cards proving
at once under this window has not been watched by anyone.

## Things the GUI will not do

- **Start past a fatal check.** A binary whose `METHOD_ID` is not canonical produces proofs the
  coordinator rejects with exit 78. `run-workers.sh` once had three GPUs proving for a day into
  guaranteed rejection, looking busy throughout.
- **Retry `EX_CONFIG` (78).** That means retrying cannot help. The supervisor stops and says why.
- **Treat `EX_TEMPFAIL` (75) as a fault.** That is a busy board, not an error.

## What running it on real Windows found

Tested under Windows Python 3.14.2 through WSL interop. Three things only that could show:

**A `✅` crashes the Windows console.** It is cp1252, and every marker this project prints is
outside it — `UnicodeEncodeError: 'charmap' codec can't encode character '\u2705'`. Not a garbled
character: an exception, mid-sentence, so the diagnostics would have died before reporting
anything. `winconsole.fix()` now runs before anything prints.

**The shim's own self-test was wrong, not the shim.** It claimed exclusion failed, because it took
two handles in ONE process — which `msvcrt` deliberately allows, unlike POSIX `flock`. What
`gpu_lock` actually needs is exclusion across worker PROCESSES, and that works: measured, the
parent holds it, a child is refused, and the child acquires it after release.

**A `#!/bin/sh` fixture cannot run on Windows**, so five checks "failed" against code that was
fine. The fixture now writes a `.bat` there.

## Two bugs worth recording, both found by running it

**`ttk.Button["state"]` returns a `Tcl_Obj`**, not a `str`, so `== "normal"` is always False. It
*prints* as `normal`. A condition comparing it silently never fired and Start never came back after
workers finished. `tk.Button` returns a plain `str`, which is why the mistake is invisible in a
quick test.

**Tk is main-thread only — including `self.after()`.** Reading a `StringVar` from a worker thread
raises `RuntimeError: main thread is not in main loop`, intermittently, depending on timing.
Scheduling with `after()` from a thread is the same class of bug. Worker threads now snapshot every
variable before they start and post results back through a queue drained on the main thread.

## The one thing it cannot fetch for you

`hazync-worker` is a public release asset and downloads with no login (measured: HTTP 200
unauthenticated), so the Setup tab just gets it. The Windows **prover** is not a release asset — it
exists only as a CI artifact, and GitHub refuses to serve build artifacts anonymously even for a
public repository. So that one download stays manual.

`./publish_host_asset.sh` fixes that permanently: it finds the newest successful `windows-prove`
run, **verifies the embedded guest before publishing anything**, and attaches the binary to a
clearly-marked prerelease. It is a dry run unless you pass `--publish`, because publishing is
outward-facing and this project does not cut releases without going through their contents.

## Packaging

`Hazync.bat` removes the terminal. `hazync-gui.spec` removes Python:

```
pip install pyinstaller
pyinstaller --clean --noconfirm hazync-gui.spec
```

⚠ `winshim` ships as a **data file**, not a bundled module — the GUI puts its *directory* on
`PYTHONPATH` for the worker's separate interpreter, so there has to be a real directory on disk.

## Not done yet

- Never run on Windows. The logic is tested; the window is not.
- No packaging. Needs a Python install today; PyInstaller would remove that, and matters if this is
  ever aimed at people who have not installed Python.
- No identity/key onboarding. Claims are signed, so a newcomer needs a key — `hazync-worker id`
  and `rotate` exist, and the GUI only displays what they report.
- `os.killpg` on the stall path, above.
