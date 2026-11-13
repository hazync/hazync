# Build and release

How Hazync's host, guest and verifiers are built, how the image ids are pinned and reproduced, and how a release
is cut, gated, signed and checked. §3.2 is the flow as run for **v0.24.0** (2026-11-12): **Hazync v2** (the libzync
engine) on **RISC Zero 3.1.0** (`risc0-zkvm` 3.1.0, circuits 4.1.0). Merged from `BUILDS.md` and
`RELEASE_PROCESS.md` (September 2026); the parts marked **v1** are the record of Hazync v1 (the original "CORE"
design, last released as v0.22.1) and are kept as history, not as instructions.

⚠ **Hazync v1 (CORE) was retired from the active tree on 2026-11-09** ([`../history/README.md`](../history/README.md)).
The v1 verifiers (`verifier/`, `verifier-ffi/`, `verifier-wasm/`), the SNARK fixtures (`prover/testdata/snark/`)
and `scripts/rebaseline-id.sh` named below are now under `history/` at those paths, the host is libzync-only, and
`reproduce/METHOD_ID` names Hazync v2's published mainnet image. v0.23.0 (the first v2 release, RISC Zero 3.0.5,
image `863c9ce3…`) is **withdrawn**: 3.1.0 fixes soundness bugs in RISC Zero's proof system and 3.1.0 verifiers
refuse 3.0.x receipts ([`RELEASE_NOTES_v0.24.0.md`](RELEASE_NOTES_v0.24.0.md)).

**The release tooling is Hazync v2's from v0.23.0** (the v1 text below is the record for v0.22.1 and earlier):

- **Assets** are listed once, in `scripts/release-assets.txt` (read by `scripts/release.sh` and `release-sign.yml`):
  the libzync host for the published mainnet image (`reproduce/LIBZYNC_IDS`) as `hazync-host-x86_64-linux-gnu`,
  `-cuda` and `hazync-host-x86_64-windows-cuda.zip` (`host.exe` + `cudart64_12.dll`); the libzync verifier
  (`swiftsync-verify`) as `hazync-verify-x86_64-linux-gnu`, `hazync-verify-aarch64` (built by `release-sign.yml`),
  `hazync-verify.wasm` and its glue `hazync-verify.js`; `hazync-worker`, `hazync-coordinator.py`,
  `hazync-run-workers.sh`; `SHA256SUMS.txt` and `.asc`. Nine are uploaded by `release.sh`, three added by
  `release-sign.yml`: the release must carry **exactly those 12**.
- **Images** (v0.24.0): mainnet `8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4`, signet
  `d4210dae…`, headers zyncAPP `2db34e12…`. `reproduce/libzync.Dockerfile` rebuilds them bit for bit (it prints the
  ids and ELF hashes); `reproduce/LIBZYNC_IDS` pins them.
- **Hosts:** `prover/build-release.sh` embeds the published mainnet ELF (built by `reproduce/libzync.Dockerfile`, or
  passed as `HAZYNC_GUEST_ELF` and checked against its pinned sha256). The Windows host is built by `windows-prove.yml`
  on GitHub's `windows-2022` runners: dispatch it on the release commit first (`gh workflow run windows-prove.yml
  --ref main -f skip_cpu_prove=true`); `scripts/windows-host.sh` fetches the artifact of that run and checks it from
  its bytes.
- **Gates before publishing:** every host that runs on the release box answers `method-id` with the published id;
  one that cannot (CUDA without a driver, Windows) is checked from its bytes, or, stronger, by
  `HAZYNC_ATTEST_<asset>=<id>` from an operator who ran it. `scripts/check-release-verifiers.sh` runs the staged
  verifiers (CLI, WASM through its glue, runnable hosts) on the real mainnet 1..200 receipt (`DEMO`) and on tampered
  copies (`FAIL`).

The scripts are authoritative: `scripts/release.sh`, `scripts/package-release.sh`, `scripts/check-dist.sh`,
`scripts/windows-host.sh`, `scripts/check-release-verifiers.sh`, `.github/workflows/release-sign.yml`,
`scripts/check-deployed-verifier.sh`. Coordinator deployment and day-to-day operation stay in
[`coordinator/deploy/RUNBOOK.md`](../coordinator/deploy/RUNBOOK.md) and, for the v2 board,
[`coordinator/deploy/libzync/SWITCH.md`](../coordinator/deploy/libzync/SWITCH.md); see also
[OPERATIONS.md](OPERATIONS.md), [COORDINATOR.md](COORDINATOR.md) and the v1
[security model](history/SECURITY_MODEL_CORE.md).

---

## 1 · Builds (v1 record: the CORE channels)

Everything in §1 measured Hazync v1's guest (the CORE/GHOST channels on Bitcoin Core's C++), not Hazync v2.
v2's measurements are in `experimental/libzync-bench/results/` (for example `CYCLES_REAL_STATE.md`: 19.9x fewer
proven cycles than v1 on block 200,000, 9.5x across eight blocks 200k–300k).

### 1.1 The three channels — one canonical guest id (hazync#225)

| channel | role | contributes to the board? |
|---|---|---|
| **stock** | the digest **oracle**: the fidelity floor every acceleration is checked against | no |
| **CORE** (§1.3) | **canonical** v0.21.0–v0.22.1: what the v1 release binary built and the v1 board counted | was **yes** |
| **GHOST** (§1.4) | experimental, where new levers land first | no |

Stock is kept because it is how a claim that an acceleration left consensus untouched is made. Core was
cleared against it on block 962,000: journal `4fb3e3c5…4656d` byte-identical, 4.095x fewer cycles.

⛔ **A Ghost build proves into rejections.** It changes guest source, so it carries a different `METHOD_ID`,
and the coordinator re-verifies every submission against its own:

```
receipt rejected: your prover's guest image id (METHOD_ID) does not match this
coordinator's — you built a different guest.
```

`run-workers.sh` checks the id only at STARTUP (hazync#99), so a mismatched worker proves indefinitely into
nothing. To contribute, use the **release binary** or the reproducible build.

⚠ Promoting a channel is a new `METHOD_ID`, a full cutover, and **every existing proof invalidated** (v0.21.0
spent the whole board). Do not quote a block count for that cost; it is whatever `/api/meta` reports at the
swap. `provision-vps.sh` can apply the #139 lever (patch 0005, phase 5b) and warns *"⛔ It MOVES METHOD_ID,
so a box provisioned with this must never produce a shipped proof"* / *"This box is for benchmarking only."*

### 1.2 Measured results — two L40S, block 962,000, 16 chunks (2026-09-02)

Chunk times, stragglers and aggregate are measured; each build's 16 chunks were proved **serially on one
card** (`scripts/gpu-benchmark.sh`), so the **card count is computed** for a 600 s block, not observed.

| build | chunk work | straggler | aggregate | **cards** |
|---|---|---|---|---|
| **CORE** | 4,029 s | **1.295** | 473 s | **10** |
| **GHOST** | 1,713 s | **1.438** | 473 s | **5** |

Every build commits `4fb3e3c5e80417c87584a617d23b53d8c49940348c0e8d455f66299b4bd4656d` with `all_valid=1`,
`binds=8006`, byte-identical to stock; whole-block cycles reproduced the laptop's execute-mode figures
exactly (GHOST 1,198,904,653).

Fleet check: CORE on 8 × L40S, block 966,108, one chunk per card: **15m13s measured vs 15.2 min projected**,
putting a sub-10-minute block at **~13 L40S** (`docs/history/BENCH_8xL40S_2026-09-08.md`). Different block
from the table, so the card counts are not directly comparable.

⛔ **Derived figures moved the wrong way when measured.** Stragglers predicted within a few percent (CORE
1.210 → 1.295, GHOST 1.469 → 1.438); the cycles→wall-clock conversion was optimistic by 14% for Core
(chunk work 3,521 → 4,029 s; 9 → 10 cards) and 36% for Ghost (1,257 → 1,713 s; 5 cards). The fitted
aggregate (616 s) came in at 473 s. Cycle ratios predict the SHAPE of a distribution, not its absolute cost.

**Run a worker on the coordinator.** `seg-serve` distributes but does not prove; with one worker the
coordinator's GPU sits at 0%:

```
1 remote worker, coordinator IDLE            772.4 s
1 remote worker + coordinator ALSO working   473.1 s   <- 1.63x, zero extra hardware
2 remote workers, coordinator idle           405.6 s   <- 1.90x
```

The serial execute phase hazync#207 estimated at ~107 s measured **13.2 s** (likely fixed by `read_slice`,
#136), so #207 should be closed as already fixed. ⚠ CORE's aggregate on 962,000 is **assumed** equal to
GHOST's (same 16 receipts, 323 segments at po2 21), not measured.

#119 on an L40S: 1 in 16 proves, recovered on first retry (81 s). ✅ Fixed in v0.21.1 (#245): a
non-canonical witness cell from rv32im accum phase 3, ~1 in 2,200 po2=21 segments, not the card.

### 1.3 CORE — *Core's own code decides*

```bash
# patches (secp256k1 tree)
0012-select-field-bigint2-backend.patch
0013-lift-x-via-witness-hint.patch
# build
HAZYNC_FIELD_BIGINT2=1 HAZYNC_LIFTX_HINT=1 HAZYNC_ECMULT_WINDOW=21 cargo build --release
# packing constants -- Core's per-curve fit, BUILT IN since v0.21.0 (c12ad67); shown for reference
HAZYNC_COST_EC_OP=417798 HAZYNC_COST_SCHNORR_OP=462435
HAZYNC_COST_INPUT_BYTE=2 HAZYNC_COST_INPUT_BASE=41387
# run time -- every chunked command (prove-chunk, prove-seg, seg-serve) also needs this
HAZYNC_LIFTX_HINT=1
```

- The constants are the **per-curve fit** (2026-09-01): ECDSA:Schnorr measured 1.11x (the earlier curve-blind
  constants assumed 1.00x), straggler 1.311 → 1.210 (8.89 → 8.28 cards; eight cards needs 1.163).
- Core's cost model fits its data at **6.6%** mean error vs Ghost's **15.2%**: Core's cost is close to linear in
  `(ecdsa, schnorr, bytes, inputs)`, Ghost's is not.
- libsecp keeps its wNAF, GLV, ECDSA logic and every check. Only the field *backend* changes, plus a pubkey-Y
  hint that libsecp's own `fe_sqr`/`fe_equal` verify; a wrong or missing hint falls back to the real sqrt.

### 1.4 GHOST — *fastest wins*

```bash
# patches (secp256k1)                               # patches (bitcoin-core)
0005-ecdsa-verify-group-arith-via-bigint2.patch     0009-sha-transform-fastpath.patch
0006-schnorr-verify-group-arith-via-bigint2.patch   0010-transformd64-via-accelerator.patch
0008-scalar-inverse-via-bigint2.patch
0012-select-field-bigint2-backend.patch
0013-lift-x-via-witness-hint.patch
# build
HAZYNC_BIGINT2_ECDSA=1 HAZYNC_LIFTX_HINT=1 HAZYNC_FIELD_BIGINT2=1 HAZYNC_BIGINT2_SCHNORR=1 \
HAZYNC_SCALAR_INV_ACCEL=1 HAZYNC_SHA_FASTPATH=1 HAZYNC_AGG_READSLICE=1 HAZYNC_SHA_D64_ACCEL=1 \
HAZYNC_ECMULT_WINDOW=21 cargo build --release
# packing constants -- Ghost's OWN fit (#227). The built-in defaults are Core's; do not use them here
HAZYNC_COST_EC_OP=85636  HAZYNC_COST_SCHNORR_OP=168542
HAZYNC_COST_INPUT_BYTE=2 HAZYNC_COST_INPUT_BASE=53162
```

- Built-in constants are global consts in `prover/host/src/main.rs`, each overridable by its `HAZYNC_COST_*`
  variable; there is no per-channel default. Before `c12ad67` "defaults" meant the older curve-blind constants.
- **Ghost calibration, measured 2026-09-05** (`af0534c`, #227): two L40S, block 962,000, 16 chunks, arms
  differing only in the four constants. Fitted on block 965,500 (7.7% Schnorr), intercept zero;
  Schnorr:ECDSA **1.97x** (not 13.77x). Pre-`c12ad67` defaults: straggler 1.462x (max 149 s, mean 102 s);
  Ghost refit: **1.189x** (124 s, 104 s).
- ⚠ Neither fit transfers: Core's refit moved Ghost's straggler the wrong way (1.407 → 1.884, cycle-based), and
  Ghost's constants are unmeasured on Core.
- ⚠ **Re-run an outlier before believing it.** The refit first measured 1.567x from one chunk at 4.89 s/segment
  (31 of 32 sat at 2.4–3.0); re-run twice: 94 s, 2.69 s/segment. Prefer the segment straggler (57 → 49 max
  segments); proving bills whole segments.
- #139 and the field backend stack: 10.676x → 11.467x execute-mode cycles (+7.4%).
- ⚖ **hazync#209 (Ghost's next build) closed 2026-09-16 as superseded.** Its levers (MSM batch verification,
  `docs/history/MSM_BATCH_VERIFY.md`; wholesale bigint2, never actually measured before `patches/0014`,
  `9b767b5`; re-measuring the G3 Schnorr lane and scalar inverse; the aggregate) are shippable only on a
  re-baseline. The 1.189 refit meets the ≤ 1.35 four-card straggler target but the card count was not
  re-derived. Byte term: both fits use 2 cycles/byte (measured 3.13; the earlier 6 was ~2x too high).

### 1.5 Not yet measured

1. CORE's aggregate on 962,000 (measured elsewhere: 203.8 s over 8 workers on 966,108; 241.2 s over 27 on
   966,256, `docs/history/MILESTONE_966256_RUN4_2026-09-10.md`).
2. The straggler above 8 chunks on a fleet (the ~13-L40S figure assumes it does not worsen).
3. Ghost's card count at its refit constants.

### 1.6 Reproducing the measurements

```bash
scripts/field-backend-tests.sh                     # correctness gates, no GPU, ~3 min
HAZYNC_BLOCK=<block_962000.json> HAZYNC_CHUNKS=1 HAZYNC_PROFILE_EXEC=1 \
  ./target/release/host chunk-profile               # cycles + journal digest
HAZYNC_CHUNKS=16 ...                                # per-chunk cycles + measured straggler
```

⚠ `chunk-profile` executes the block **twice** (count-packed and cost-packed). ⚠ Read only the **measured**
straggler: the cost packer balances its own predictor and always reports a perfect 1.00x predicted.

---

## 2 · The image id, reproducible builds and pins

`reproduce/METHOD_ID` is the source of truth; the canonical id is Hazync v2's published mainnet image
`8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4` (`reproduce/LIBZYNC_IDS`, built by
`reproduce/libzync.Dockerfile`).

**Hazync v2 (v0.24.0):**

```bash
docker build -t hazync-libzync-repro -f reproduce/libzync.Dockerfile .
docker run --rm hazync-libzync-repro      # prints the mainnet, signet and headers ids and ELF hashes
```

The id still depends on absolute build paths, so the Dockerfile fixes them (`HOME=/root`, the repo at
`/hazync-zkvm`); an id built anywhere else differs by design. The guest is built once there and every host embeds
those bytes: `prover/methods/build.rs` refuses an ELF whose sha256 or image id is not the pinned one, so the host's
own build location does not move the id. v0.24.0 moved every id with no guest source change, because the image id
depends on the RISC Zero toolchain (3.0.5 → 3.1.0).

The browser verifier is reproducible only with the same **host** Rust: `swiftsync-verify-wasm/build.sh` under Rust
**1.99.0** gives the published `hazync-verify.wasm` (`446fdaeb…`); 1.97.1 gives other bytes. Build it with
`swiftsync-verify-wasm/build.sh`, which pins Rust 1.99.0 itself since 2026-11-12 (installing it if missing), so the
machine's default Rust no longer matters.

**Hazync v1 (record).** The rest of this section describes the CORE images `reproduce/Dockerfile` built;
the CORE id it names, `37987b85`, is superseded (v0.22.1 is the last v1 release, and v1 proofs still verify with
it). Also pinned: `reproduce/GUEST_ELF_SHA256`,
`reproduce/LINEAGE.tsv` (`scripts/lineage.sh --write` appends a row from git history once an id change is
committed; `lineage.sh --check` is the CI gate).

**What moves the id:** any change to guest source (a Ghost build, the #139 lever), and **the absolute build
paths**, which the ELF embeds. The same tree measured three ids on 2026-08-03:

| built at | id |
|---|---|
| `/home/…/dev/projects/hazync` (dev box) | `1bed31ef…` |
| `/root/hazync-rebuild` (coordinator, scratch) | `1112670d…` |
| `/hazync-zkvm` (container / canonical at the time; superseded) | `37987b85…` |

Only the canonical path produces a publishable proof; anything else verifies against nothing published.
The canonical id comes **only** from the container:

```bash
docker build -t hazync-repro -f reproduce/Dockerfile .
docker run --rm hazync-repro                 # prints the canonical METHOD_ID
```

To prove or wrap outside the container, reproduce its environment exactly, then build and check:

```
HOME=/root                       # so CARGO_HOME=/root/.cargo
HAZYNC_BASE=/root/hazync-build   # Core + secp sources
REPO_DIR=/hazync-zkvm            # the checkout itself
```

```bash
REPO_DIR=/hazync-zkvm HAZYNC_PROVISION=build ./provision-vps.sh
/hazync-zkvm/prover/target/release/host method-id   # MUST equal reproduce/METHOD_ID, else everything it proves is scrap
```

**Never read an id off a binary you did not just watch get built** (the most repeated mistake: three times
in one night in the v0.16.0 re-baseline, twice reporting a stale id as live). A failed build leaves the old
binary, and `host method-id` answers cheerfully. Every time:

```bash
BEFORE=$(stat -c %Y "$D/prover/target/release/host" 2>/dev/null || echo 0)
REPO_DIR="$D" HAZYNC_PROVISION=build ./provision-vps.sh; RC=$?
AFTER=$(stat -c %Y "$D/prover/target/release/host" 2>/dev/null || echo 0)
[ "$RC" -eq 0 ] && [ "$AFTER" -gt "$BEFORE" ] || { echo "build did not produce a new binary"; exit 1; }
```

1. Check the right exit code: after `ssh … | tail`, `$?` is `tail`'s. Use `PIPESTATUS[0]` or don't pipe.
2. Check the mtime advanced (catches an early failure leaving yesterday's binary).
3. Only then read the id.

- **Empty is not "different"**: a probe on a wrong path prints a mismatch that looks like an emergency.
- **A disagreement between two builds is a question**: check commit, dirty files and where the binary was
  actually built. `strings host | grep -oE '[^ ]*coinbase-smt/src/[a-z_]+\.rs'` shows the recorded paths.

**Check artifacts by EMBEDDED ID, never by size or hash.** Swapping one 64-hex literal for another is
length-preserving (the stale and correct `.wasm` were both 1,063,349 B, 2026-08-02), and sha256 + PGP attest
only that the bytes are the bytes:

```bash
strings <artifact> | grep -c <new-id>   # want 1
strings <artifact> | grep -c <old-id>   # want 0
```

---

## 3 · The release process

### 3.1 Before the release: lands on `main` first

`release.sh` refuses unless the tree is clean, `HEAD` is `origin/main` and CI passed on that commit, so all
of this is merged first:

- [ ] **Release notes** in a file, passed as `NOTES=<file>`: `docs/RELEASE_NOTES_vX.Y.Z.md` (v0.23.0 and v0.24.0
      are there; v1's are in `docs/history/releases/`).
- [ ] **`CHANGELOG.md`** entry, newest first; mark it **re-baseline** if the id moved.
- [ ] **`docs/OPERATIONS.md`**: bump "The current release is **vX.Y.Z**". `scripts/check-versions.sh` check 4
      reads that line: newer than the newest tag passes, older fails.
- [ ] (No status page since 2026-11-06: `docs/history/STATUS.md` is the last dated snapshot; the release, date and
  shipped items go in `CHANGELOG.md` and the release body.)
- [ ] **`docs/COORDINATOR_REFERENCE.md`**, if `coordinator/server.py`, `coordinator/hazync` or
      `coordinator/run-workers.sh` changed: `python3 scripts/gen-coordinator-reference.py` (CI's `--check`).
- [ ] **`swiftsync-verify-wasm/README.md`** size and sha256 if the wasm changed (`check-deployed-verifier.sh`
      fails until its `raw` figure equals the deployed module).
- [ ] On an id change, everything in §3.5.

### 3.2 Cutting it: `scripts/release.sh` (as run for v0.24.0)

```bash
gh workflow run windows-prove.yml --ref main -f skip_cpu_prove=true   # FIRST, on the release commit (Windows host)
RUSTUP_TOOLCHAIN=1.99.0 ./scripts/release.sh vX.Y.Z --dry-run         # preflight + build + gate; publishes nothing
RUSTUP_TOOLCHAIN=1.99.0 NOTES=docs/RELEASE_NOTES_vX.Y.Z.md ./scripts/release.sh vX.Y.Z   # the release
./scripts/release.sh vX.Y.Z --verify-only                              # resume at signing for a published, unsigned release
```

`RUSTUP_TOOLCHAIN=1.99.0` is what makes step 3's wasm byte-identical to the published `446fdaeb…` (§2); the
script does not pin it. The tag must match `^v[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.]+)?$` (also enforced by
`release-sign.yml`). The script is resumable; each phase skips work already done.

1. **Preflight.** Clean tree; `HEAD == origin/main`; tag free locally and on origin; `check-versions.sh`
   passes; at least one passing and no failed CI run on `HEAD` (cancelled runs count as neither);
   `docker`, `gh`, `node`, `python3` present; **≥ 25 GB free**; `reproduce/METHOD_ID` equals the
   `libzync-product mainnet` id of `reproduce/LIBZYNC_IDS`.
2. **Build the Linux hosts**: `prover/build-release.sh cpu` and `cuda`, with `SKIP_GROTH16=1` and `RZUP_TIMEOUT`
   defaulting to 7200. **A prebuilt host is accepted** (not rebuilt) when `dist/.built-from-<asset>` equals `HEAD`
   **and** it carries the canonical id: asked with `method-id` where it runs, read from its bytes
   (`scripts/embeds-method-id.sh`) where it cannot (the CUDA host on a box with no driver).
   `build-release.sh` stages the published mainnet ELF at `dist/.libzync-guest/mainnet.bin` (or set
   `HAZYNC_GUEST_ELF`).
3. **The Windows host** (step 2b): never built here. `scripts/windows-host.sh fetch <HEAD> …` takes the
   `hazync-host-windows-x86_64-cuda` artifact of a `windows-prove.yml` run **of this commit** whose `build-cuda`
   job passed, checks that the zip holds only `host.exe` + `cudart64_*.dll`, that `host.exe` embeds the published
   guest byte for byte and carries the image id, and zips it as `hazync-host-x86_64-windows-cuda.zip`.
   Artifacts expire after 14 days, so dispatch the workflow close to the release.
4. **Package** (`scripts/package-release.sh` with `TAG`): `hazync-worker` (`VERSION` stamped from the tag and
   asserted), `hazync-coordinator.py`, `hazync-run-workers.sh`, `hazync-verify.js` (from
   `swiftsync-verify-wasm/swiftsync-verify.js`) and `hazync-verify.wasm` via `swiftsync-verify-wasm/build.sh`, only
   if a `wasm32-unknown-unknown` target is installed (otherwise it warns; stage a wasm built elsewhere in `dist/`).
   The x86_64 verifier (`swiftsync-verify`) is built every time.
5. **Gate.** `dist/` must hold **exactly** the nine `release` assets of `scripts/release-assets.txt`, nothing
   else: it is not cleaned between releases, so move stray files out first. Then `scripts/check-dist.sh dist`:
   every staged artifact must belong to the canonical guest. The CUDA host, if it cannot run here, is **checked
   from its bytes** unless an operator passes `HAZYNC_ATTEST_hazync_host_x86_64_linux_gnu_cuda=<id measured on a
   GPU>` (preferred; the bytes must still agree). The Windows zip is checked from its bytes;
   `HAZYNC_ATTEST_hazync_host_x86_64_windows_cuda_zip=<id>` may be passed by an operator who ran it. Then
   `scripts/check-release-verifiers.sh dist` runs the staged verifiers on the real mainnet 1..200 receipt and on
   tampered copies.
6. **Publish.** Annotated tag pushed, then `gh release create` with **all nine** assets in one call
   (`release-sign.yml` signs what is attached at publish and picks up nothing later); the release is read back
   and must carry exactly those nine, each the staged size.
7. **Wait for signing**: the `Sign release` run whose `headBranch` is the tag. It builds `hazync-verify-aarch64`,
   checks it (under qemu) with `check-release-verifiers.sh --cli`, and adds `SHA256SUMS.txt` and `.asc`. On
   failure or timeout the script prints how to re-run it and keep `latest` on the last signed release.
8. **Manifest**: `SHA256SUMS.txt` must list the nine assets plus `hazync-verify-aarch64`, with the staged files'
   hashes; the release must carry exactly the **12** assets of `scripts/release-assets.txt`;
   `SHA256SUMS.txt.asc` should verify (reported, not fatal, if the key is not imported locally).
9. **`latest`**: set `make_latest`, read `/releases/latest` back, then download
   `/releases/latest/download/hazync-verify-x86_64-linux-gnu` until it contains the canonical id.

Check the count yourself afterwards:

```bash
gh release view vX.Y.Z --json assets --jq '.assets|length'    # want 12
```

**`release-sign.yml`** (on `release: published`, or `workflow_dispatch` with a tag): builds
`hazync-verify-aarch64` from the tag, asserts its embedded id, runs it on the real and tampered receipts, and
attaches it; downloads the `hazync-*` assets, refuses unless they are exactly `scripts/release-assets.txt` at the
tag, writes `SHA256SUMS.txt`, signs it with the `GPG_PRIVATE_KEY` secret, uploads `SHA256SUMS.txt` and
`SHA256SUMS.txt.asc`.

**The CUDA kernels' targets.** `prover/build-release.sh` sets `NVCC_FLAGS` to native SASS for sm_80, 86, 89, 90 and
100 plus compute_100 PTX. Cards outside that list run from the PTX: Blackwell workstation and server cards such as
the **RTX PRO 6000 (sm_120)** JIT-compile the kernels on the **first prove of a fresh machine** (about 10 minutes),
then use the driver's cache. Adding a target costs build time (ptxas assembles each architecture in turn, ~6–7 min
per kernel file, per the script's note) and was not done for v0.24.0.

**Not automated, still required:** smoke-test the CUDA hosts on a real GPU (`method-id`, then
`host libzync-prove-range 1 1 testdata/libzync/mainnet genesis out && host libzync-verify out/block_1.rcpt`, as
`release.sh` prints), and repoint running provers (`run-workers.sh` checks the id only at startup, #99).

### 3.3 Verifying the published assets

```bash
curl -fLO https://github.com/hazync/hazync/releases/download/vX.Y.Z/hazync-verify.wasm
curl -fLO https://github.com/hazync/hazync/releases/download/vX.Y.Z/SHA256SUMS.txt
curl -fLO https://github.com/hazync/hazync/releases/download/vX.Y.Z/SHA256SUMS.txt.asc
gpg --verify SHA256SUMS.txt.asc SHA256SUMS.txt && sha256sum -c --ignore-missing SHA256SUMS.txt
```

Then confirm `latest`, via the **`latest`** URL (the versioned URL can be right while `latest` is wrong):

```bash
gh api repos/<owner>/<repo>/releases/latest --jq .tag_name    # must be the tag just cut
curl -sL -H 'Cache-Control: no-cache' -o v \
  https://github.com/<owner>/<repo>/releases/latest/download/hazync-verify-x86_64-linux-gnu
grep -ac <new-id> v    # want 1
grep -ac <old-id> v    # want 0
```

### 3.4 Deploying the browser verifier

Served at `https://hazync.org/verify/` (from `hazync/hazync-web`). A stale module has the same size and
exports as a correct one, so it is caught only by calling it. Deploy after every release whose
`hazync-verify.wasm` differs from the deployed one (every v2 release so far: a new RISC Zero version or image moves
it), and on an id change in the **same cutover** as the coordinator binary swap and board reset (earlier rejects the
live old board; later rejects the new one). Since v0.23.0 the module is the libzync verifier
(`swiftsync-verify-wasm`), and its glue `hazync-verify.js` has the libzync API (`loadVerifier`, `verify({network,
image, receipt})`, `imageId(network)`), not v1's: serve the release's `.wasm` and `.js` **together**.

Note: the source says "both sites"; bitcoinghost.org/hazync was retired 2026-09-16, so hazync.org is the only
site serving the verifier.

**`hazync/hazync-web`** (its `README.md` lists the pins):

1. Copy the verified wasm to `verify/hazync-verify.wasm`, and the release's `hazync-verify.js` (this repo's
   `swiftsync-verify-wasm/swiftsync-verify.js` at the tag) to `verify/hazync-verify.js`.
2. Update the release version and wasm sha256 where pinned: `deploy/deploy.sh` (`want=` and its message),
   `tools/check.py` (`WASM_SHA` and its message), `README.md`, `devs/index.html` (hash, release link, and on a
   re-baseline the program-id table), `explorer/index.html` (release named beside the in-browser verifier).
3. `deploy/deploy.sh` (dry run; refuses on a wasm hash mismatch, runs `tools/check.py`), then
   `deploy/deploy.sh --go`.

**The web box** (`152.53.86.216`): back up the live module under its old id or tag, then install:

```bash
sudo cp -p /var/www/hazync/verify/hazync-verify.wasm \
           /var/www/hazync/verify/hazync-verify.wasm.<old id or tag>
sudo install -o www-data -g www-data -m 644 hazync-verify.wasm \
           /var/www/hazync/verify/hazync-verify.wasm
```

If the loader changed, install `hazync-verify.js` beside it the same way (its path is inferred from the
served URL, not a deploy script).

**Check over the wire:** `./scripts/check-deployed-verifier.sh` (hazync.org is the `HAZYNC_SITE` default).
It requires: the served loader byte-identical to `swiftsync-verify-wasm/swiftsync-verify.js`; the served wasm
size equal to the release asset and `swiftsync-verify-wasm/README.md`'s `raw`; `imageId('mainnet')` and
`/api/meta`'s `method_id` equal to `reproduce/METHOD_ID`; the live spine verified (verdict `ok` or `demo`) and
genesis-anchored in the served module; a one-bit-flipped spine not verified. (`release.sh`'s closing message still
says this script drives the v1 module; the script itself was rewritten for the libzync glue.)

After the release: restart provers on the
new binaries (on a re-baseline, purge their bundle caches, §3.5).

### 3.5 Re-baseline (the guest id changed)

**Hazync v2.** The ids live in `reproduce/LIBZYNC_IDS` and `reproduce/METHOD_ID`. Pin new ones from two agreeing
`reproduce/libzync.Dockerfile` runs with `python3 reproduce/pin-libzync-ids.py <ids file> <source commit>`, which
also rewrites the verifier tables in `swiftsync-verify` and `swiftsync-verify-ffi`; and `check-versions.sh` checks the documented sites.
The v2 board restarts from genesis on an id change, because every leaf bundle is stamped with the image that
prepared it: for v0.24.0 (`863c9ce3` → `8888d59c`) that was step 0 of
[`coordinator/deploy/libzync/SWITCH.md`](../coordinator/deploy/libzync/SWITCH.md). The rest of this section is the
**v1** procedure (`rebaseline-id.sh` is now `history/scripts/rebaseline-id.sh`, the SNARK fixtures and v1 verifiers
are under `history/`, and the bridge has no v2 counterpart), kept as the record.

A guest change moves `METHOD_ID`; the coordinator then (correctly) rejects every proof on the board, which
restarts from genesis. Batch guest changes deliberately.

**The mechanical half is scripted. Do not hand-edit it.**

```bash
docker build -t hazync-repro -f reproduce/Dockerfile .
docker run --rm hazync-repro                 # the canonical METHOD_ID
./scripts/rebaseline-id.sh <that id>         # rewrites every known site
./scripts/check-versions.sh                  # the backstop, not the discovery mechanism
```

If `check-versions` names a site the script missed, **add it to the script**.

⛔ **Known bug (verified 2026-09-14, not yet fixed): `rebaseline-id.sh` exits 1 at its last check**
(`::error::reproduce/METHOD_ID does not end with the new bare id`): it checks `tail -1`, but the file ends
with a comment line. Every substitution has already been applied. Confirm the bare line yourself
(`grep -nE '^[0-9a-f]{64}$' reproduce/METHOD_ID`) and work the closing checklist from the end of the script:
supersession note, regenerate SNARK fixtures, `check-versions.sh`, `check-utreexo.sh`, `check-spec.sh`, cut
the release, deploy the wasm, `check-deployed-verifier.sh`.

**What the script will not do:**

1. Write the **supersession note** into `reproduce/METHOD_ID` (why the id moved and what it cost).
2. **Regenerate `prover/testdata/snark/*.snark`** per `prover/testdata/snark/README.md`. They are proofs by
   the old guest and cannot be re-pointed; until then `snark-verify` / `ci_snark_verify.sh` fail, as they
   should. Needs Groth16 on a **CPU** host (it crashed on every CUDA build tested, #20; not re-measured).
3. Check the wasm and published verifiers **by embedded id** (§2).

**Sites that must change with the id** (each fails looking like something else):

- [ ] `reproduce/METHOD_ID`, FIRST; `reproduce/LINEAGE.tsv` (`scripts/lineage.sh --write`).
- [ ] `CHANGELOG.md`, marked **re-baseline**.
- [ ] hazync.org's pins, including the program-id table in `devs/index.html` (§3.4).
- [ ] `verifier/src/lib.rs` and `verifier-ffi/src/lib.rs` `METHOD_ID_HEX` (literals; `check-versions.sh` fails
      on drift). Published verifier binaries come from `release-sign.yml`: cut a release.
- [ ] `verifier-wasm` embeds `METHOD_ID_HEX`: the served `.wasm` is a pinned artifact and, left stale, rejects
      every new proof.
- [ ] `prover/testdata/snark/*.snark`.
- [ ] **The archive bridge's binary**: it produces the bundles everyone consumes; missing it stalled the board.
- [ ] Docs stating the current id (`docs/OPERATIONS.md`, `SECURITY.md`, `README.md`); `check-versions.sh` enforces.

**Ride-along worth batching into any re-baseline:** journal byte-packing (was #22). risc0 serde commits each
`u8` as a 32-bit word, so a 32-byte Utreexo root costs 128 B; packing saves 96 B per root, ~20% on a
genesis-anchored `[1..1000]` wrap (3,441 → ~2,770 B) and ~25% projected full-chain (~5,360 → ~4,020 B), an
inferred two-point fit. Not worth a re-baseline alone.

**The coordinator swap.** The coordinator derives its expected id from its **own** `HAZYNC_HOST` binary
(`expected_method_id()`, served at `/api/meta`): new binary in, board cleared, workers restarted. Read the real
paths off the unit with `systemctl show` (not `systemctl cat`, which prints superseded drop-in lines; see
[Backup & restore](../coordinator/deploy/RUNBOOK.md#backup--restore)):

```bash
systemctl show hazync-coordinator -p WorkingDirectory -p Environment | tr ' ' '\n' \
  | grep -E 'WorkingDirectory|COORD_DB|COORD_PROOFS|COORD_SPINE|HAZYNC_HOST'
```

Production: `COORD_DB=/var/lib/hazync/coordinator.db`, `COORD_PROOFS=/var/lib/hazync/proofs`,
`HAZYNC_HOST=/usr/local/bin/hazync-host`, checkout `/opt/hazync` (moved from `/root` on 2026-08-02, #58).
Coordinator and bridge both resolve `/usr/local/bin/hazync-host`: one path to swap.

```bash
# 1. BACK UP FIRST (the old ledger + receipts are the record of the previous baseline)
COORD_DB=/var/lib/hazync/coordinator.db COORD_PROOFS=/var/lib/hazync/proofs \
  BACKUP_DIR=/var/lib/hazync/backups /opt/hazync/coordinator/deploy/backup.sh
cd /var/lib/hazync/backups/<STAMP> && sha256sum -c SHA256SUMS    # verify before relying on it
#    Without BACKUP_REMOTE the snapshot is on the SAME disk: copy the DB off-box anyway.

# 2. Stop BOTH services, swap the host (KEEP the old one: it re-verifies archived proofs)
systemctl stop hazync-coordinator hazync-bridge
cp /usr/local/bin/hazync-host /root/hazync-host.bak.<OLD_ID_PREFIX>
install -m755 ./host-new /usr/local/bin/hazync-host
/usr/local/bin/hazync-host method-id                             # MUST equal reproduce/METHOD_ID

# 3. Clear the board: archive, never delete
mv /var/lib/hazync/coordinator.db /var/lib/hazync/coordinator.db.<OLD_ID_PREFIX>
mv /var/lib/hazync/proofs /var/lib/hazync/proofs.<OLD_ID_PREFIX>
mkdir /var/lib/hazync/proofs && chown hazync:hazync /var/lib/hazync/proofs
#    THE SPINE TOO: read COORD_SPINE from the unit (production overrides the default)
SPINE=$(systemctl show hazync-coordinator -p Environment --value | tr ' ' '\n' \
        | grep -oP 'COORD_SPINE=\K.*')
SPINE=${SPINE:-/opt/hazync/coordinator/spine}
echo "spine is at: $SPINE"          # CHECK THIS before moving anything
mv "$SPINE" "$SPINE.<OLD_ID_PREFIX>"
mkdir "$SPINE" && chown hazync:hazync "$SPINE"

# 4. Start BOTH: init_db() reseeds the open ranges; the frontier restarts at 0
systemctl start hazync-coordinator hazync-bridge
curl -s localhost:8899/api/meta      # method_id == the new id, frontier == 0

# 5. Verify no component was left behind
./scripts/check-deployment.sh --local
```

**Then purge every prover's bundle cache.** The contributor CLI caches by height (`BUNDLE_DIR/bundle_<h>.json`)
and never re-fetches an existing file, so a stale bundle keeps failing after the bridge is fixed:

```bash
./coordinator/run-workers.sh 4 --stop           # stops loops AND children (bare pkill misses `python3 ./hazync-worker run`)
rm -rf "$BUNDLE_DIR" ~/.hazync/bundles          # whatever BUNDLE_DIR the workers use
./coordinator/run-workers.sh 4                   # re-fetches; refuses to start on an id mismatch
```

Confirm a bundle re-fetched (fresh mtime, `txs` present). Provers pre-flight with `hazync selftest` against
the new `/api/meta` id, so update every worker's `HAZYNC_HOST` too.

---

## 4 · Release pitfalls (each broke or nearly broke a release)

Hazync v2:

- **The wasm's bytes depend on the host Rust.** Rust 1.99.0 gives the published `446fdaeb…`; 1.97.1 gives
  `a3a85b7f…`. `build.sh` pins 1.99.0 since 2026-11-12; a new pin means a new published sha (§2).
- **`dist/` must hold exactly the release assets.** It is never cleaned, so a previous release's file or a stray
  build output stops step 5 of §3.2; move it out rather than adding it to the list.
- **The Windows host comes from a `windows-prove.yml` run of the release commit.** Dispatch it before releasing;
  its artifact expires after 14 days.
- **Blackwell workstation cards JIT on first prove.** The release CUDA host has no sm_120 SASS, so an RTX PRO 6000
  spends about 10 minutes compiling PTX on its first prove on a fresh machine (§3.2). It is not a hang.

Hazync v1 (record; several still apply to any release):

- **Stale id read off an old binary** (v0.16.0, three times): exit code + mtime before `method-id` (§2).
- **Non-canonical build paths** (2026-08-03): three ids from one tree; only `/hazync-zkvm` is publishable.
- **Size/sha256/PGP do not detect a stale id**: check embedded id (§2).
- **`make_latest` silently not applied** (v0.15.0): a single PATCH with `draft=false` + `make_latest=true`
  published but left `latest` on the previous tag; a second PATCH with only `make_latest=true` fixed it.
  `gh release edit --draft=false` on a draft did nothing and reported no error.
- **`/latest/download/` CDN caches** for minutes: served the old verifier (1,723,592 B vs 1,726,504 B), both
  passing sha256 against their own manifest; only the embedded id told them apart (§3.3).
- **CUDA release build needs ≥ 25 GB free** (`df -h /`; 21 GB is NOT enough): it went 21 → 4.4 GB at
  **3.7 GB/min** during the toolkit unpack, nearly filling root on a box running services. Reclaim with
  `docker system prune -af` (5.3 GB), `/usr/local/cuda-13.x` (~4.8 GB; the release build brings CUDA 12 in its
  container because risc0-sys's kernels did not compile under CUDA 13, per `prover/build-release.sh`), `~/.hazync/receipts`
  (~3.7 GB for 17k; local copies only).
- **groth16 is ~5 GB; set `RZUP_TIMEOUT` and `SKIP_GROTH16=1`.** `build-release.sh` forwards `RZUP_TIMEOUT`
  only when the caller sets it; at the default the download burns 3 x 300 s of retries (non-fatal: groth16 is a
  runtime rzup component the host does not link). Measured 2026-09-18: **5,314,631,872 bytes** in four files
  (`stark_verify_final.zkey` 3,620,786,504; `preprocessed_coeffs.bin` 1,442,123,704;
  `stark_verify_graph.bin` 251,721,088; `fuzzed_msm_results.bin` 576). ⛔ 488 MB is the Rust toolchain, not
  groth16. Even `RZUP_TIMEOUT=7200` needs ~740 KB/s for two hours:
  `RZUP_TIMEOUT=7200 ./prover/build-release.sh cpu`.
- **Bridge left on the old host** (v0.10.0 stall): it emitted bundles without `txs` and every prover panicked
  with "missing field `txs`". Swap both, run `check-deployment.sh --local`.
- **Old-guest spine left live** (2026-08-02, again 2026-08-03): `COORD_SPINE` is untracked and survives
  `git checkout`; production sets `/var/lib/hazync/spine`, so archiving the default moved an empty directory.
  Only symptom: `spine_hi 8156` with proven 0 / frontier 0, and `/api/spine/proof` failing the new verifier.
- **Ownership** (2026-08-02): services run as user `hazync` since #58; anything recreated by hand must be
  chowned, including `/var/lib/hazync` itself, or sqlite fails with "unable to open database file".
- **Stale prover bundle caches**: purge them; the coordinator served the right bundle all along.
- **Wasm deployed out of step** with the board reset rejects one board or the other: deploy together.
