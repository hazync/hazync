# Join the Hazync Proof Party

> **The board runs Hazync v2** (the libzync engine, image `8888d59c`, release
> [v0.24.0](https://github.com/hazync/hazync/releases/tag/v0.24.0) on RISC Zero 3.1.0) since 12 November 2026, and
> proves the chain again from genesis. What it proves and how: [`docs/libzync/LIBZYNC.md`](docs/libzync/LIBZYNC.md).
> Hazync v1 (the original CORE guest `37987b85`) is no longer proved; its last release is v0.22.1, which still
> verifies v1 proofs, and its documents are in [`docs/history/`](docs/history/README.md). Release v0.23.0 is
> **withdrawn**: do not use it (see [`CHANGELOG.md`](CHANGELOG.md)).

You prove one block of Bitcoin's history on your own machine, sign it, and submit it. Your name goes on the board at https://hazync.org/, and the proof is public for anyone to download and check. This guide takes you from nothing to your first proof.

## What you need

- A Linux machine (x86-64), Ubuntu 22.04+ (glibc 2.34+) for the prebuilt binaries. A cloud GPU box works well.
  **Windows works too**, natively or through WSL2 — see [Windows](#windows) below. macOS cannot prove.
- An NVIDIA GPU and a driver recent enough for it. **No CUDA toolkit or runtime install is needed** on Linux — the prebuilt prover links only `libcuda.so.1`, which ships with the driver, so a stock cloud GPU image works as it comes (CUDA 13 included). No GPU still works (the CPU binary proves the early blocks, just slower).
- **No build.** Grab the prebuilt binary below.

⚠ **Blackwell workstation and server cards (RTX PRO 6000, sm_120) compile the prover's kernels on their first
prove**, about 10 minutes on a fresh machine, and use the cached result afterwards. The release CUDA host carries
native kernels for sm_80/86/89/90/100 only ([`docs/BUILD_AND_RELEASE.md`](docs/BUILD_AND_RELEASE.md)). A first
prove that sits silent for minutes on such a card is this, not a hang; on the board's first v2 run the start-up GPU
smoke test (600 s limit, below) timed out once this way and passed when retried
(`experimental/libzync-bench/RESULT_board_run_2026-11-12.txt`).

### Windows

Two routes, both proving on an NVIDIA card.

**Native: the Windows CUDA host.** Every release carries `hazync-host-x86_64-windows-cuda.zip`: `host.exe` and
`cudart64_12.dll`, zipped so the DLL keeps the name Windows looks for. It proves: mainnet blocks 170 and 400,000 were
proved on an RTX PRO 6000 under Windows' WDDM driver model, and both receipts verified on Windows and again on Linux
(`experimental/libzync-bench/windows/RESULT_xesktop_rtxpro6000.txt`, measured on the v0.23.0 build of the same
engine). The way to drive it without a terminal is the Windows GUI in [`tools/win-gui/`](tools/win-gui/README.md):
it needs Python 3.8+ with `tkinter` (bundled with the python.org installer) and `cryptography`, fetches the worker
from the latest release by itself, and starts from `Hazync.bat`. Its README says what has been measured on Windows
and what has not; read its table before relying on it.

**WSL2: the ordinary Linux binary.** ⭐ You do not need a Linux machine — you need a Linux *userspace*, and Windows
ships one. The prebuilt Linux prover links only `libcuda.so.1`, and NVIDIA's WSL2 support provides exactly that file
inside WSL from the **Windows** driver.

1. `wsl --install -d Ubuntu-22.04` in an admin PowerShell, then reboot.
2. Install the **NVIDIA driver on Windows** — the normal one from nvidia.com. ⛔ Do **not** install
   a driver *inside* WSL; that breaks the passthrough rather than enabling it.
3. In the Ubuntu shell, check the one thing that matters:

   ```bash
   ls -l /usr/lib/wsl/lib/libcuda.so.1 && nvidia-smi
   ```

   If both answer, follow the rest of this guide unchanged. If `libcuda.so.1` is missing, the
   passthrough is not set up and the CPU binary is your path until it is.

⚠ **WSL2 is not tested by us.** This is the documented mechanism — `libcuda.so.1` is the prover's only CUDA
dependency on Linux, and WSL2 supplies it — but nobody has yet reported a Hazync block proved this way. Step 3 is
the check that tells you *before* you download the prover. If you get a block proved on WSL2, say so in an issue
and this paragraph can come out.

### macOS

⛔ **macOS cannot prove, and this is not something we have left unwired.** The circuit that proves
blocks ships CUDA and C++ kernels only — `risc0-circuit-rv32im-sys/kernels/` has `cuda/` and `cxx/`
and no Metal directory, and its `metal` feature is declared empty. NVIDIA is the only GPU path on
any operating system.

⭐ A Mac **can** verify, which needs no GPU: the browser verifier (`hazync-verify.wasm`) runs anywhere.

## Minimum spec, by what you want to do

| You want to | You need |
|-------------|----------|
| Verify a proof someone else made | Any Linux x86-64 box, no GPU, a couple of GB of RAM — download the verifier, done. A Mac or a browser works too |
| Prove blocks for the board | An NVIDIA GPU + its driver (or the CPU binary, slower) |
| Run your own party | See [`docs/RUN_YOUR_OWN_COORDINATOR.md`](docs/RUN_YOUR_OWN_COORDINATOR.md) and [`docs/COORDINATOR.md`](docs/COORDINATOR.md) |

**Proving memory.** A proving *segment* is the unit that has to fit in memory, and its size is
`HAZYNC_SEG_PO2` — each step up doubles the working set. The binary picks the default (21 on the CUDA build,
which peaks around 22 GB and fits a 24 GB card; 20 on CPU, where the extra memory buys nothing; `host seg-po2`
prints it), and the worker retries smaller if a prove fails. But **swapping is not a failure**, so if your box
is RAM-tight it will crawl rather than fall back — set `HAZYNC_SEG_PO2=20` (or 19) explicitly.

## Step 1: get the prover (no build needed)

Download the prebuilt prover from the **latest release** — it embeds the published image, so the coordinator accepts your proofs.

⛔ **Take v0.24.0 or later, and restart anything started on an older release.** The board accepts only proofs
by image `8888d59c` (v0.24.0). A v0.23.0 host proves under `863c9ce3`, which RISC Zero 3.1.0 verifiers refuse, and
a v0.22.x host proves Hazync v1. `run-workers.sh` checks the image id **only when it starts**, so a worker loop
started on an older release keeps producing proofs the board refuses until you restart it.

```
# the checksum list first — everything below is verified against it before it runs
curl -LO https://github.com/hazync/hazync/releases/latest/download/SHA256SUMS.txt
# the prover binary (GPU)
curl -LO https://github.com/hazync/hazync/releases/latest/download/hazync-host-x86_64-linux-gnu-cuda
# the contributor CLI and the fleet launcher, both covered by SHA256SUMS.txt.asc
curl -fLO https://github.com/hazync/hazync/releases/latest/download/hazync-worker
curl -fLO https://github.com/hazync/hazync/releases/latest/download/hazync-run-workers.sh
ln -sf hazync-run-workers.sh run-workers.sh   # shorter to type; the real file keeps the asset name,
                                              # which is what SHA256SUMS.txt lists

# ⛔ CHECK WHAT ARRIVED BEFORE RUNNING IT. Three OK lines, or stop.
grep -E ' (hazync-host-x86_64-linux-gnu-cuda|hazync-worker|hazync-run-workers.sh)$' SHA256SUMS.txt \
  | sha256sum -c -

chmod +x hazync-host-x86_64-linux-gnu-cuda hazync-worker hazync-run-workers.sh
ln -sf hazync-worker hazync      # a shorter name to type; the real file keeps the asset name
sudo apt install -y python3-cryptography   # Debian/Ubuntu — other systems below
```

⚠ **`cryptography` is the only thing here that wants root**, and only on Debian and Ubuntu. The
worker needs it for one job — signing your claims — and says so if it is missing. Elsewhere, or
without sudo, use `pip install --user cryptography`; on **Debian 12 and Ubuntu 24.04 and newer** pip
refuses to touch the system Python (PEP 668), so it is
`python3 -m venv ~/.hazync-venv && ~/.hazync-venv/bin/pip install cryptography`, then run the worker
with `~/.hazync-venv/bin/python3`.

(`run-workers.sh` is optional — it keeps several workers going for you. Keep it **next to** the
CLI; it finds `hazync-worker` or `hazync` beside itself. `MODE=fold` runs folders instead of
provers, `MODE=mixed` runs both plus one spine worker, and `MODE=spine` runs only the spine, which
only ever needs one.)

**No GPU?** Use the CPU binary instead (`hazync-host-x86_64-linux-gnu`) — it proves too, just slower.
Both binaries prove in-process; there is nothing else to install.

**Keep the asset filenames.** `SHA256SUMS.txt` lists them, so downloading with `-LO` lets the check
above match them. Renaming on download (`-o host`) makes it report *"no file was verified"* — which
looks like a signature problem and is not. Symlink or rename afterwards if you want shorter names.

⚠ The check greps the three lines it wants and pipes them to `sha256sum -c -`, rather than using
`--ignore-missing`: that form exits 1 on a tampered file, on an **absent** file, and when nothing was
downloaded. `--ignore-missing` cannot catch the absent case by definition.

`SHA256SUMS.txt.asc` signs that list with the maintainer's key, for anyone wanting the stronger
check: see [`SECURITY.md`](SECURITY.md#verifying-releases). Take the CLI from the **release**, not from a raw source
URL: it holds your ed25519 signing key and decides what gets submitted under your name.

Stronger still: run `./hazync-host-x86_64-linux-gnu-cuda method-id` (or the CPU binary) and confirm it prints
`8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4`, the id in
[`reproduce/METHOD_ID`](reproduce/METHOD_ID) and [`reproduce/LIBZYNC_IDS`](reproduce/LIBZYNC_IDS).

> **Building from source instead?** You *must* embed the **published image**, or the coordinator rejects every
> proof you submit. A guest built anywhere but the reproducible container has a different id. Rebuild it with
> `docker build -t hazync-libzync-repro -f reproduce/libzync.Dockerfile .` and `docker run --rm hazync-libzync-repro`
> (prints both ids and ELF hashes), then build the host around that ELF as [`prover/README.md`](prover/README.md)
> § Build describes. The prebuilt binary sidesteps all of this.

## Step 2: set your name and point at the party

```
# Both of these are OPTIONAL — the CLI defaults to the public party and finds the prover
# beside itself. Set them only if you are pointing somewhere else.
export COORD_URL=https://api.hazync.org
export HAZYNC_HOST=$PWD/hazync-host-x86_64-linux-gnu-cuda
./hazync id yourname
```

Your name can be anything. It is tied to a signing key the tool makes for you and keeps in `~/.hazync`, so nobody else can claim your blocks. Back that folder up if you care about keeping the same identity.

**Back it up now, not later.** Losing the box loses the key, and the blocks it proved stay under a name
you can no longer sign for. This has already happened once on the Hazync v1 board: 29,669 blocks sit under
an identity whose key died with a deleted machine, and nothing can recover them.

**Proved on the v1 board?** Your handle and key were carried over to the v2 board at the switch (13 contributors);
your v1 block counts were not, because they count v1 proofs. Keep using the same `~/.hazync`.

### Moved to a new box, or rebuilt one?

Copy your old `~/.hazync` folder (`key.hex` and `handle`) onto the new machine. The same identity can
prove from several boxes at once, and your blocks stay with that key — but read
[Several workers on one key](#several-workers-on-one-key) first.

`hazync rotate`, which moved one key's blocks onto another, is switched off on the public coordinator
(#311): anyone holding a copy of your `key.hex` could have used it to move your blocks to a key of their own.

**Before you prove, run a pre-flight:**

```
./hazync selftest
```

It confirms your prover is present, its image id matches the coordinator (the common trap — a different build has every proof rejected), and it can verify a real board proof end to end. Better to catch a setup problem now than after a long prove.

## Step 3: prove

```
./hazync run              # takes the coordinator's suggestion
./hazync run 5            # or name any block you like
./hazync run 30000-30049  # or a span — BOTH ENDS INCLUSIVE: this is 50 blocks
```

> ⛔ **Range bounds are inclusive at both ends, so the next range starts at `hi + 1`.**
> `30000-30050` is **51** blocks, not 50, and the range after it begins at `30051`.
>
> Getting it wrong is silent and expensive. On 2026-09-11 a contributor to the v1 board submitted 50-block chunks
> as `30000-30050`, `30050-30100`, … — each starting on the block its predecessor ended on. Every proof was valid,
> but a genesis-anchored chain needs each range to begin exactly one block after the last, so none of them could
> join it, and the frontier froze for thirteen hours. The coordinator now refuses those bounds at submit, and
> `hazync run` prints the span and the block count before it starts proving.

**A bare `run` claims its block; a named height or span claims nothing.** `./hazync run` with no
argument takes the earliest open block through `/api/claim` and heartbeats while it proves. The claim is
released on its own: after `CLAIM_GRACE` (600 s by default) if it never sent a heartbeat, after
`CLAIM_TTL` (3,600 s) without one, and at `CLAIM_MAX` (86,400 s) regardless. `./hazync run 5` or a span
takes no claim at all. The coordinator accepts a valid proof at any height from anyone, and a lapsed claim cancels
nothing.

`run` fetches the block's leaf bundle (the state before the block, prepared by the board), proves it on your machine, signs the receipt, and submits it. The coordinator re-verifies your proof, and when the tool prints a `✓`, your name is on the board at https://hazync.org/. Prove as many as you like — just run it again.

**Which blocks can be proved?** Any block the board has prepared. Hazync v2 has no bridge: a prep pass on the board's
server writes one leaf bundle per block, paced a little ahead of the provers, and the coordinator serves it. A block
not yet prepared has nothing to serve, and `run` tells you so before it starts.

**Leaving it running, or running several at once?** One `run` proves one block and exits, so use the
supplied loop — it keeps N workers going and refuses to start if your image id doesn't match the coordinator's, or
if the GPU cannot prove block 1 (a 600 s smoke test; `SKIP_GPU_SMOKE=1` skips it):

```
HAZYNC_HOST=./hazync-host-x86_64-linux-gnu-cuda ./run-workers.sh 4          # 4 workers, logs in ~/hazync-workers
./run-workers.sh 4 --stop
```

That pre-flight matters: a worker on the wrong image id proves happily and has **every** submission
rejected, burning GPU hours for nothing.

### Several workers on one key

- **At most 4 open claims per key.** The coordinator gives one key at most `CLAIM_OPEN_MAX` live claims (4 by
  default on the public board); past that, a claim is refused until one is proven or lapses. So **more than four
  claiming workers on one key starve themselves**: on the board's first v2 run, nine workers on one key proved about
  100 blocks an hour until the cap was raised for the run
  ([`RESULT_board_run_2026-11-12.txt`](experimental/libzync-bench/RESULT_board_run_2026-11-12.txt)): count every worker
  on every box that shares your `~/.hazync`, not only one box's. If you want more, use a separate identity per
  group of four (`HAZYNC_HOME` points the CLI at another one).
- **Don't restart workers mid-run without stopping them cleanly.** A worker killed mid-prove leaves its claim open
  on your key. A claim on a key that is still active looks alive for up to `CLAIM_TTL` (60 min), so it both uses one
  of your four slots and, if it was the frontier block, **stalls the board's frontier until it lapses**. On that run the frontier stopped at each orphaned
  block. Stop with `./run-workers.sh N --stop` and let the current proves finish where you can. If you did orphan a
  block, prove it directly (`./hazync run <block>`): that is what cleared them on that run.
- **One GPU job at a time per box.** Workers on one box share a GPU lock (`HAZYNC_GPU_LOCK`, default
  `/tmp/hazync-gpu.lock`), so extra workers on one card queue rather than collide. Turning it off is not faster:
  on a 96 GB RTX PRO 6000 with six workers it measured worse (7 blocks in 5 minutes against 15 on each locked
  card). Early blocks are mostly fixed per-block proving work, so more workers do not help; more cards do. A box with several cards can pin
  workers to cards with `CUDA_VISIBLE_DEVICES` and give each card its own lock path (the GPU lock's notes in
  `coordinator/hazync`), within the four-claim limit above.

### Get a push when a worker needs you

A worker that stops (no usable GPU, a prover binary left behind by a release), keeps failing, or has its claims
refused (a clock that is wrong, for example) says so in its log, which nobody reads while it runs unattended.
Point it at [ntfy](https://ntfy.sh) and it pushes to your phone instead:

```
./hazync notify new          # makes a private topic, saves it, sends a test push, prints the topic
./hazync notify test         # send another test
./hazync notify off          # stop
./hazync notify <topic or https://your-ntfy-server/topic>    # a topic or server of your own
```

Install the ntfy app and subscribe to the topic it prints. **The topic is the only secret:** anyone who knows
it can read your alerts and send you fake ones, so keep the generated one rather than choosing a guessable
name. Each problem is pushed at most once an hour (`HAZYNC_NTFY_REPEAT`). `run-workers.sh` also pushes after
5 failures in a row (`NOTIFY_FAIL_STREAK`), again when the worker recovers, and when it refuses to start; a
board with nothing free to claim is not a failure. A push that cannot be sent never stops the work.

**Running workers for days?** [`docs/OPERATIONS.md`](docs/OPERATIONS.md) is the operator's manual (§0 is the v2 board,
§3 the board workers), and every worker setting is listed in
[`docs/COORDINATOR_REFERENCE.md`](docs/COORDINATOR_REFERENCE.md).

## Step 4: two other ways to help, both cheaper than proving

Proving is the expensive job. These two turn what has already been proved into something a stranger
can check in one download.

```
./hazync fold          # join two adjacent proven ranges into one wider proof
./hazync fold 20       # do twenty of them
```

The board holds a receipt per block. Folding joins adjacent ones — `[100..199] + [200..299]` becomes
`[100..299]` — and the result can be folded again with *its* neighbour. It is a tree, so any adjacent
pair may be folded in any order and any number of people may do it at once. The coordinator suggests pairs whose
fold does not exist yet, and if someone beats you to one your submission is simply discarded. A fold is a libzync
join: its guest checks that the two proofs meet, and it re-proves no block.

```
./hazync spine 5       # absorb the next 5 chunks into the genesis-anchored head
```

The **spine** is the one artifact that says *"every block from genesis to N is valid"*. It advances by
joining the next adjacent chunk onto it, so it is complete after every step. This is the only job that has to
happen in order, so one machine at a time makes progress on it; duplicate effort is harmless but wasted.

Absorbing wide chunks is far better than absorbing single blocks: it is **one join either way**. So
folding first and then absorbing is how the spine catches up quickly. `hazync spine` absorbs the widest verified
chunk that starts where the spine ends, and falls back to a narrower one if a wide chunk will not absorb. More cards
on `fold` therefore make the spine faster; a second card on `spine` does not.

Whoever advances the spine cannot corrupt it. Every absorption is re-verified against the published image and
pinned to genesis, and because every per-block receipt is retained, anyone can rebuild the spine without
re-proving anything.

## Just want to check a proof, not make one?

You never have to trust the party. The spine — every block from genesis to N in one receipt — is public at
`https://api.hazync.org/api/spine/proof`, and so is every proven block at `https://api.hazync.org/api/proof/<block>`.
Check the spine with the standalone verifier, no GPU and no build:

```
curl -LO https://github.com/hazync/hazync/releases/latest/download/hazync-verify-x86_64-linux-gnu
chmod +x hazync-verify-x86_64-linux-gnu
curl -fLOJ https://api.hazync.org/api/spine/proof     # -J: lands as hazync-spine-1-<N>.hzk
./hazync-verify-x86_64-linux-gnu --network mainnet --product hazync-spine-1-*.hzk
```

(Check the verifier binary itself first with the SHA256 + PGP steps in [`SECURITY.md`](SECURITY.md#verifying-releases).)

The verifier has three answers. `SWIFTSYNC_VERIFY_OK` (exit 0): a proof from genesis with Bitcoin's minimum chain
work. **`SWIFTSYNC_VERIFY_DEMO` (exit 2): every check passed, but the chain proved so far is below the minimum work
— every early board proof answers this, and it is not a failure.** `SWIFTSYNC_VERIFY_FAIL` (exit 1): refused. Add
`--mempool` or `--expect <height>:<hash>` to compare the proven tip with the real chain;
[`swiftsync-verify/README.md`](swiftsync-verify/README.md) has every option. The same verifier runs in a browser
(`hazync-verify.wasm`) and on hazync.org.

The `.hzk` is a **binary receipt** (a RISC Zero proof, a few hundred KB), not text. A single block's proof from
`/api/proof/<block>` is not anchored at genesis on its own (it starts from the state before its block); the prover
host checks one with `host verify-any <file> <block>`, which needs that block's leaf bundle, so the spine is the one
to check by hand.

## If something breaks

- `./host: cannot execute` or a `GLIBC` error — the prebuilt Linux binaries need glibc 2.34+ (Ubuntu 22.04+). On an older distro, run in a container or build from source (see [`prover/README.md`](prover/README.md)).
- The Linux CUDA prover needs **only the NVIDIA driver** (`libcuda.so.1`), not a CUDA toolkit or runtime:

  ```
  $ ldd hazync-host-x86_64-linux-gnu-cuda | grep -i cuda
  	libcuda.so.1 => /lib/x86_64-linux-gnu/libcuda.so.1
  ```

  CUDA 12 is a BUILD requirement, not a runtime one (see `prover/build-release.sh`). If you are downloading the
  prebuilt binary it does not apply to you. If proving cannot find the GPU, check the driver with `nvidia-smi`.
- `this coordinator accepts only signed claims: update your worker` — your worker is older than v0.21.5 and does
  not sign its claims. Download the current `hazync-worker` from the latest release; your identity in
  `~/.hazync` is unchanged.
- The coordinator rejects your proof with an image id (`METHOD_ID`) mismatch — your host is not v0.24.0's (a v0.23.0
  or v0.22.x host, or one built from source). Use the latest release, and restart any `run-workers.sh` loop
  started on an older one.
- Claims refused because the key already holds 4 — see [Several workers on one key](#several-workers-on-one-key).
- Anything else: open an issue with the **Board or prover problem** template.

## Running your own party

The coordinator (`coordinator/`) is optional and reusable. If you want to run your own proving effort, for signet, a
private run or a test, [`docs/RUN_YOUR_OWN_COORDINATOR.md`](docs/RUN_YOUR_OWN_COORDINATOR.md) and
[`docs/COORDINATOR.md`](docs/COORDINATOR.md) walk through standing one up; `coordinator/deploy/RUNBOOK.md` is the
operator's runbook for the public board's own box, and `coordinator/deploy/libzync/SWITCH.md` records how it moved
to Hazync v2. Contributors point their `COORD_URL` at your coordinator, and their proofs land in your ledger, not
ours. Each coordinator is its own island: separate ledger, separate frontier, separate stored proofs. The proofs
themselves are universal, so anyone can verify a proof from any party.

## Reviewing the code

If you would rather try to break it than prove blocks, that is the most valuable thing you can do. [`SECURITY.md`](SECURITY.md) is the map of where the soft spots are, and [`docs/SOUNDNESS.md`](docs/SOUNDNESS.md) and [`docs/EXTERNAL_REVIEW.md`](docs/EXTERNAL_REVIEW.md) say what a proof rests on and how to review it.

Changes to the code go through a pull request; the template asks for evidence and for the change's effect on the
image id (any change the guest compiles moves it).

Everyone taking part is covered by [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md); conduct and security reports go through the private form it names.
