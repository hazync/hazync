# Hazync on Windows — a GUI for the prover

A window that runs the prover, for people who do not use a terminal. It drives Hazync v2 (the libzync engine):
the canonical image it checks for is v0.26.0's mainnet `46666b6f…` (`supervisor.py`, `CANONICAL_METHOD_ID`).

```
python hazync_gui.py
```

Needs Python 3.8+ with `tkinter` (bundled with the python.org installer) and `cryptography`
(`pip install cryptography` — the worker signs every claim and cannot run without it).

## Getting it onto a Windows machine

**Most people: the installer.** `hazync-setup-x86_64-windows.exe` (built by `.github/workflows/windows-installer.yml`
from `installer/hazync.iss`) installs Hazync for the current user, with no admin prompt, Start-menu and desktop
shortcuts and an uninstaller. Everything is inside: the app, the worker, the GPU prover and (from v0.26.1) the CPU
prover. Python and git are not needed. From v0.27.0 the installed app updates itself (§ Updating). It is not Authenticode-signed yet, so Windows SmartScreen says "Windows protected your PC": choose More info,
then Run anyway. Its sha256 is in the release's PGP-signed `SHA256SUMS.txt`. Uninstalling never touches
`%USERPROFILE%\.hazync`, where the signing key lives.

From source (developers):

```
git clone --depth 1 --filter=blob:none --sparse https://github.com/hazync/hazync
cd hazync
git sparse-checkout set tools/win-gui
cd tools/win-gui
```

(The GUI is on `main`.) Then **double-click `Hazync.bat`**. It finds Python, says what to install if there is none, and opens
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

While it proves, Home shows the card's memory, load and temperature, RAM and CPU, with a 5-minute chart, and the
technical log records each block's peak card memory (`stats.py`, from v0.27.0). The **Automatic** segment size is the
detected card's (`supervisor.py`, `_SEG_PO2_FOR_VRAM`): a 4 GB GTX 1050 Ti given the host's default 21 ran out of
memory, and proved block 4401 at 18 (19.11.2026).

## ⛔ Read this first: what is proven and what is not

| | state |
|---|---|
| Windows CUDA build compiles, embeds the canonical guest | ✅ #631/#644 |
| `host.exe method-id` prints the then-canonical CORE id `37987b85…` (since superseded) on real hardware | ✅ measured 2026-10-04, GTX 1050 Ti |
| `host.exe regress` — full consensus path (the CORE host; gone from the libzync host) | ✅ measured, same machine |
| **libzync CUDA** host proves mainnet leaves on a real GPU under WDDM (blocks 170 and 400,000), verified on Windows and Linux | ✅ **measured 2026-11-09, RTX PRO 6000** (`experimental/libzync-bench/windows/`) |
| The libzync Windows CUDA host is a **release asset** (`hazync-host-x86_64-windows-cuda.zip`: `host.exe` + `cudart64_12.dll`), built by `windows-prove.yml` and checked by `scripts/release.sh` | ✅ since v0.23.0 |
| The Windows **CPU** host is a release asset too (`hazync-host-x86_64-windows-cpu.zip`: `host.exe` alone), from `windows-prove.yml`'s `prove` job, which ran it on Windows (`method-id`, then mainnet block 1 proved and verified), and the installer ships it in `prover-cpu\` | ✅ since v0.26.1 |
| **CPU** build proves a block on Windows | ✅ measured (v1 host) |
| This GUI opens and works on Windows | ✅ **tested on Windows Python 3.13** |
| This GUI drives a prove end to end on Windows | ✅ **measured 2026-10-05 with the v1 host: `PROVED in 2220.7s — receipt VERIFIED against METHOD_ID`**; not yet re-run with the libzync host |
| v1 (CORE) **CUDA** build proves a block on a GTX 1050 Ti | ⛔ never completed: the card is below the prover's floor (below) |

The CUDA host needs a card of **compute capability 7.0 or newer**. Release CUDA hosts carry native code for sm_80
to sm_100 (`prover/build-release.sh`); a newer card such as the RTX PRO 6000 (sm_120) compiles the kernels from PTX
on its first prove on a fresh machine (about 10 minutes, then cached), so a long silent first prove is expected.

### v1 history: why CUDA proving failed on a GTX 1050 Ti

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

⇒ It was **untested, not broken**: it had never run on a card sppark accepts. It has since run on one: the
libzync CUDA host proved mainnet blocks 170 and 400,000 on an RTX PRO 6000 under WDDM (2026-11-09, row above).

⭐ **Resolved by hazync#727 — a GTX 1050 Ti proves.** With the fixes, the 1050 Ti proved mainnet
block 1 on its GPU (197.7 s, receipt verified, driver 566.14, 15.11.2026). Three things were needed:

1. **The floor.** `HAZYNC_SPPARK_MIN_MAJOR=6` lowers sppark's compute floor from 7 (`all_gpus.cpp`).
   The app sets it for you only when **"Try my GPU even if it is below the supported floor"** is ticked
   and the worker's card is compute 6.x; each worker is also pinned to its card with
   `CUDA_VISIBLE_DEVICES`. Below 6 is refused: no build carries code for it.
2. **The real "operation not supported".** rv32im's lookup tables used `::cuda::atomic<uint32_t>`,
   which defaults to *system* scope. On Pascal that puts `EIATTR_ATOM_SYS_INSTR_OFFSETS` in the module
   and the Windows driver refuses to load the whole module (error 801) on a card without host-native
   atomics. They are now device-scope, and `scripts/gpu_code_archs.py` refuses any build that carries
   a system-scope atomic.
3. **Memory pools.** sppark now asks whether the device has memory pools before using
   `cudaMallocAsync`, and falls back to `cudaMalloc`/`cudaFree` where it does not (the 1050 Ti's own
   pools tested fine; this is defensive).

Other cards below compute 7.0 remain **unproven**. The history below is what it looked like before.

With `HAZYNC_SPPARK_MIN_MAJOR=6` on a build from before hazync#727, the 1050 Ti was accepted —
`cooperativeLaunch=1`, so that second condition was never the obstacle — and then failed further in,
in the NTT:

```
GPU SCAN: 1 device(s), need major >= 6 and cooperativeLaunch
  [0] NVIDIA GeForce GTX 1050 Ti  compute 6.1  cooperativeLaunch=1  -> USED
CUDA ERROR: cudaGetLastError()@sppark/ntt/ntt.cuh:97 failed: "operation not supported"
panicked at risc0-zkp-3.0.5/src/hal/cuda.rs:708: Failure during zk_shift
```

That error was raised earlier, by the refused rv32im module (point 2 above) — `cudaGetLastError()`
reports an error left pending by an earlier call, so the line number was not the culprit. hazync#727.

The original abort, for reference — this is what you get on a build older than 2026-10-04, where the
real error was thrown away:

```
prove_segment_core                ← dies here
fatal runtime error: Rust cannot catch foreign exceptions, aborting
```

Two theories were tested and **killed**: it is not out of memory (`HAZYNC_SEG_PO2` 18 and 17 abort
instantly and identically on an idle 4 GB card) and not an arch mismatch (the card *is* the `sm_61`
the binary embeds natively). Open: the build used CUDA 12.9.1 against a driver reporting 12.7.

The Pascal NTT failure this section left open was resolved by hazync#727 (above).

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

**Schedule** — prove only inside a window you set, for example overnight (#725); below.

⛔ **There is no "pick any open block" button, on purpose.** The coordinator owns allocation: it
hands out work and tracks claims, and `/api/blockstatus` deliberately excludes claims because "they
change by the second". Letting someone choose an arbitrary open block would hand out work another
prover already holds. `/api/pick` is the coordinator's own answer and the Dashboard shows it.

## Updating

The installed app asks GitHub for the latest release at startup and every 6 hours, and shows "Hazync vX is
available" with an **Update** button on Home. Nothing is downloaded until that is pressed. Update fetches
`hazync-setup-x86_64-windows.exe`, `SHA256SUMS.txt` and `SHA256SUMS.txt.asc`; it runs the installer only if
(1) the signature is good under the maintainer key pinned in `appupdate.py` (the one in SECURITY.md), (2) the
manifest's signed first line names that release, (3) the installer's sha256 matches it, and (4) the release is newer
than this app. The installer is not Authenticode-signed, so the PGP-signed manifest is the trust anchor. The app then
closes so its files can be replaced. A copy run from source has no version and never updates itself.

⚠ **Rotating the release key needs a manual reinstall** for every installed app: an app refuses a manifest signed by
any key but the pinned one (it fails closed). The key expires 2028-04-25; plan the rotation before then.

On the first start of a new version, a remembered prover that does not make the board's image (when the installed one
does) and a remembered worker older than the installed one are replaced by the installed ones, and the feed says so.
On any other start an old copy is named on the Home card with one button to use the installed one.

## Proving on a schedule

Under **Advanced → Schedule** you can tell the app to prove only inside a window, say 23:00 to 07:00, on the days
you tick. Press Start as usual: the app waits for the window, proves while it is open, and stops taking blocks
when it closes. The logic is in `provewindow.py` and `winpower.py`, tested by `test_schedule.py` (pure, with real
daylight-saving nights) and `test_schedule_flow.py` (the real window and a stand-in worker).

- **Drain, don't kill.** When the window closes, the block in hand is finished and put on the board; no new
  block is taken. A long-running anchor job (which holds no claim) is stopped at the end of the window.
- **Don't start what can't finish.** Before each block the app compares the time left in the window with how
  long this card's recent blocks took, claim to board: the median of the last five, plus a quarter, plus two
  minutes. With no block measured on the card yet, it starts one only with an hour or more left. The board proves
  in height order, so the next block is a neighbour of the last ones; the margin covers it being a little bigger.
- **Local wall-clock time.** The window is set in the PC's local time and the time left is real seconds, so on
  the nights the clocks change a 23:00 to 07:00 window lasts 7 or 9 hours. The window belongs to the day it starts
  on: ticking only Friday gives Friday night into Saturday morning.
- **Keep awake.** While the app proves inside the window it asks Windows not to sleep
  (`SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)`; the screen can still turn off). It keeps asking
  while the last block finishes after the window closes, and stops asking when nothing is running or when you
  press Stop. Off the schedule it never asks.
- **Wake timer, off unless you tick it.** A task in Task Scheduler, `Hazync\Wake for proving`, with "wake the
  computer" set, at the window's start. It runs nothing (`cmd /c exit 0`): the app has to be left open. It cannot
  wake a PC that is shut down or hibernating, and does nothing when "Allow wake timers" is off in the power plan;
  the app reads that setting with `powercfg` when you save, and says so. Unticking it removes the task.
- **Handing a block back.** Stop (or closing the app) while a block is claimed hands it back to the board at once
  with `hazync-worker release <n> <claim>`, signed by the same key, instead of leaving it held for up to an hour. The
  claim is the id the worker printed on its `claimed block` line: only that claim is released, so a release that
  lands late cannot free the block the same key has claimed again since
  (`POST /api/release`, docs/COORDINATOR_REFERENCE.md). On Windows the worker is ended with `taskkill /F`, which
  it cannot see, so the app does the release for it. A worker from before this change has no `release` command;
  the app says so and the block reopens by itself, as before.

What it does not do yet: "only when plugged in" and "pause while the PC is in use" from #725 are not built.

## Testing this machine

The **Prove** tab has two buttons that run what you would otherwise type:

| | |
|---|---|
| Will the board accept this prover's proofs? | `method-id` — must print the image the **board** checks proofs against (its `/api/meta` `method_id`, asked at Start, in Checks and here), or proofs are rejected; only when the board cannot be reached does the image this app was built for (`CANONICAL_METHOD_ID`, `46666b6f…`) stand in. A prover the board accepts passes even if this app is older or newer than it; the app's own image fails once the board has moved, with "install the latest Hazync". Runs at startup. ⚠ Where `method-id` cannot run (a CUDA `host.exe` on a machine with no NVIDIA driver), the fallback reads the embedded guest from the file, pinned by `GUEST_SHA256` and `GUEST_SIZE` in `supervisor.py` to the published mainnet guest of `reproduce/LIBZYNC_IDS` (the `GUEST_ELF_SHA256` on its mainnet line, 70,199,652 bytes: v0.26.0's guest, the one behind `46666b6f…`; the same file as windows-prove run 43835465944's guest artifact). This fallback can say only that the file carries the image this app was built for, not that the board accepts it. A new image means updating both |
| Can it actually prove? | the client's `selftest` — checks the prover against the board, verifies a real board proof, then proves mainnet block 1 for real from the board's leaf bundle |

⭐ **No block is claimed and nothing is submitted.** The selftest fetches block 1's bundle and a board proof
to check against, so a machine that cannot prove learns that without holding up the board. (The CORE host's
offline `regress` and `prove-block` do not exist in the libzync host.)

Each verdict reads the **output**, not the exit code — a selftest exiting 0 without both its prove line and
`ALL GOOD` is reported as a failure, because a check that passes on silence is not a check. The last few
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
exactly as they ship (the worker's source is `coordinator/hazync`), because a forked worker would drift from the
one the fleet runs.

## The four Windows gaps in `hazync-worker`

| gap | status |
|---|---|
| module-level `import fcntl` — the worker **died at startup** | ✅ fixed in the worker (hazync#616: `fcntl` or `msvcrt`); `winshim/fcntl.py` on `PYTHONPATH` covered older workers |
| `/tmp/hazync-gpu.lock` — a POSIX path | ✅ `HAZYNC_GPU_LOCK` |
| host names searched are the Linux ones (`hazync-host-x86_64-linux-gnu[-cuda]`, …) | ✅ `HAZYNC_HOST` |
| `os.killpg(…)` on the stall/timeout path | ✅ fixed in the worker (hazync#616): on Windows it uses `taskkill /F /T`, which the worker itself marks **untested on Windows** |

⭐ The shim is a **real** lock (`msvcrt.locking`), not a stub, so `gpu_lock()` keeps serialising GPU
work across workers. A no-op would have let N workers share one card with no serialisation, which on
a small card turns "slow" into "out of memory".

## Why one worker by default

`gpu_lock()` serialises GPU work across workers on a box. With the shim that is genuine, so N
workers are safe. But if the shim is ever absent, the worker's own fallback is to proceed anyway, and
N workers would then share one GPU unserialised. Default 1; raise it once a run has shown the lock
working.

## No graphics card the prover can use: the CPU prover

The installer carries a second prover, `prover-cpu\host.exe`, which needs no NVIDIA card or driver (from v0.26.1).
Start asks before it launches anything whether a graphics-card prover can run here: no NVIDIA driver, or no card at
compute 7.0 or newer (6.x with "Try my GPU…" ticked). If none can, it offers the CPU prover instead and says what that
costs, measured: mainnet block 1, the smallest block there is, took **1,433 s (about 24 minutes)** on a 4-core Windows
runner (windows-prove run 42540389071, 2026-11-09); most blocks are bigger and take longer. Nothing bigger has been
measured on a CPU, so nothing bigger is promised. The choice can be changed under Advanced → Options ("CPU build" /
"GPU build"). A machine where `nvidia-smi` cannot be asked is not refused: unknown is not "no card".

The processor is offered **for that session only**: the saved choice stays the graphics card, so installing the
NVIDIA driver later needs no setting changed (Advanced → Options → CPU build makes it permanent). The same offer comes
when the graphics card fails while proving and nothing else is running, and when the remembered prover file is gone.

**No Visual C++ runtime needed.** The CPU `host.exe` is built with a static C runtime and imports only DLLs Windows
ships (checked at release by `scripts/pe_imports.py --cpu`). The CUDA `host.exe` still imports `VCRUNTIME140.dll`,
`VCRUNTIME140_1.dll` and `MSVCP140.dll`; the installer copies them beside it, so a PC without the Visual C++
Redistributable runs it. Someone unzipping the CUDA release asset by hand needs that redistributable.

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
- Advanced → Options lists the cards with a tick box each, all ticked to begin with. An unticked
  card is left alone; the choice is remembered by the card's UUID, so it follows the card if the
  cards are moved between slots. The last card cannot be unticked.
- Home shows **one line per card of the session**, including a card whose worker stopped on an error
  (`RTX 3060 Ti:  stopped — …`, and the headline counts it: "Proving on 1 graphics card · 1 stopped") or is
  between two blocks. Only that card stops; the others keep going. Before, a stopped card simply vanished.
- ⚠ The opt-in proves on Pascal only with a prover that has hazync#727 (PR #739), which releases carry from v0.26.1.
  An older prover, ticked on a compute 6.x card, gets "operation not supported" from the GPU.
- "Try my GPU even if it is below the supported floor" never reaches below compute 6: the builds carry no
  GPU code under sm_61 (hazync#727), so a compute 5.x card gets no worker even when forced.

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

## Getting the prover

`hazync-worker` is a public release asset and downloads with no login (measured: HTTP 200
unauthenticated), so the Setup tab just gets it. Since v0.23.0 the Windows **prover** is one too, and the installer
carries both provers (`prover\` and `prover-cpu\`), which the app finds first. Run from source, download
`hazync-host-x86_64-windows-cuda.zip` (or `-cpu.zip`) from the latest release and "Extract All" in Downloads or your
home folder; the GUI searches one folder level under both for `host.exe`. ⚠ From source the GUI does not fetch it
itself: its Setup step names the release zip and asks you to extract it and point at `host.exe`.

`./publish_host_asset.sh` is now only for a build **between** releases: it finds the newest successful
`windows-prove` run, **verifies the embedded guest** (the published mainnet guest of `reproduce/LIBZYNC_IDS`, so it
needs `HAZYNC_GUEST_ELF=dist/.libzync-guest/mainnet.bin`) before publishing anything, and attaches the binary to a
clearly-marked prerelease. It is a dry run unless you pass `--publish`.

## Packaging

`Hazync.bat` removes the terminal. `hazync-gui.spec` removes Python:

```
pip install pyinstaller
pyinstaller --clean --noconfirm hazync-gui.spec
```

⚠ `winshim` ships as a **data file**, not a bundled module — the GUI puts its *directory* on
`PYTHONPATH` for the worker's separate interpreter, so there has to be a real directory on disk.

## Not done yet

- Fetching the Windows prover from the release when run from source (the installer carries it; above).
- A full prove under this window with the libzync host and a GPU.
- No identity/key onboarding. Claims are signed, so a newcomer needs a key — `hazync-worker id`
  and `rotate` exist, and the GUI only displays what they report.
- The worker's Windows kill path (`taskkill /T`) is untested on Windows, above.
