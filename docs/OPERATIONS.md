# Operations

The operator's reference for proving on the board, and for the **Hazync v1 system**: the CORE guest (`37987b85`,
now superseded), its last release v0.22.1, multi-card fleets on RunPod, and tip runs.

**Since 2026-11-12 the public board is Hazync v2** (the libzync engine, image `8888d59c`, release v0.24.0); §0 covers
it and its proving fleets. §3 (board workers) applies to both boards. §1, §2, §4 and §5 describe v1 and stay as its
reference. It merges the September 2026 pages
PROVING, PROVER_OPERATIONS, FLEET_OPERATIONS, TIP_RUN_OPERATIONS and TOPOLOGY_AND_SETTINGS (now in
`docs/history/`).

⚠ **The host in this tree is libzync-only.** The canonical image is libzync's published mainnet image `8888d59c`
(§2.7), and `prover/host` refuses to build without `HAZYNC_LIBZYNC=1`. The CORE commands below that drive a guest mode
a libzync image refuses (`regress`, `adversarial`, `check-*`, `prove-full`, `prove-seg`, `prove-chain*`, `prove-ibd`,
`prove-range`, `extend-spine`, `verify-range`, `verify-chain`, `prove-snark`, mode 6) exist only in the v0.22.1
release binaries; [`history/README.md`](../history/README.md) lists what was removed. This tree's commands are in
[`prover/README.md`](../prover/README.md), and libzync's GPU scripts in `experimental/libzync-bench/gpu/`. The rest of
this page is the reference for the v1 (CORE) board, which was retired on 2026-11-12.

New to proving: [`CONTRIBUTING.md`](../CONTRIBUTING.md) first. Concepts: [`libzync/LIBZYNC.md`](libzync/LIBZYNC.md).
Board coordinator: [`COORDINATOR.md`](COORDINATOR.md). Soundness: [`SECURITY.md`](history/SECURITY_MODEL_CORE.md). Builds and
releases: [`BUILD_AND_RELEASE.md`](BUILD_AND_RELEASE.md).

**Labels.** MEASURED = in this repo's evidence (CORE guest unless it says *stock*). INFERRED = arithmetic
over measured inputs. UNKNOWN = nobody has measured it.

⚠ **Never quote a fleet size without its framing, its card and its po2. Measure before quoting** — every
number in [§7](#7-superseded-numbers--do-not-quote) was once someone's confident summary.

⚠ **Two things are called "coordinator".** The **board coordinator** (`coordinator/server.py`) hands out
blocks and verifies submitted proofs; it never proves. The **segment coordinator** is the `seg-serve`
process that executes one guest run, pushes work to `seg-connect` workers and assembles the result; it
lives for one prove. They share no code.

## 0. The live board: Hazync v2

The board at `api.hazync.org` switched to Hazync v2 on 2026-11-12 and restarted from genesis under `8888d59c`
([`coordinator/deploy/libzync/SWITCH.md`](../coordinator/deploy/libzync/SWITCH.md)). The v1 board reached frontier
144,769 (145,223 blocks proven); its data and proofs are kept read-only as the record, and 13 contributors carried over
(handles and keys, not block counts). v1 proofs still verify with the v0.22.1 release.

What runs on the board box, where its state lives, how to check it and how to update it:
[`coordinator/deploy/RUNBOOK.md`](../coordinator/deploy/RUNBOOK.md) § "The live board: Hazync v2". In short: the
coordinator is `hazync-lz-coordinator` on port 8899 (state in `/var/lib/hazync-lz`), and there is **no bridge**: a
prep pass on the board box (`libzync-board-prep`) writes one leaf bundle per block (LZLEAF02) under
`/srv/bulk/libzync-board/leaves`, paced by proving. Following the tip with libzync is not running yet. v2 keeps
claims, per-block proofs, folds, the genesis-anchored spine, the leaderboard, sponsorships and the sponsor bot.

Workers: the v0.24.0 release (`hazync-worker`, `hazync-run-workers.sh`, the CUDA host), run as in §3. A worker whose
host is not `8888d59c` is refused at start (§3.4).

**MEASURED, first v2 proving run** (2026-11-12, 4× RTX PRO 6000, 2 h 38 min): 1,287 blocks proven, ~490 blocks/hr,
~$0.021/block on early blocks, $27.13 in all. An early block takes ~22–27 s on an RTX PRO 6000, mostly fixed per-block
proving (trie + aggregate ≈ 3.4 M cycles, plus recursion), not JIT. Early blocks only: later blocks are larger.

### 0.1 Proving fleets: four things measured on 2026-11-12

- ⛔ **`CLAIM_OPEN_MAX` is per key, default 4** (`coordinator/server.py`). One key running more than 4 proving
  workers starves itself: the fifth and later get "nothing to claim right now" while the board has work. Give each
  box its own key (`HAZYNC_HOME`, §3.9), or have the operator raise `CLAIM_OPEN_MAX` on the coordinator.
- ⛔ **Restarting workers mid-run orphans their claims.** The claims stay on that key, and because the key is
  still live they look alive for up to `CLAIM_TTL` (60 min). They count against the key's 4, and an orphaned claim
  on the **frontier** block stalls the frontier until it lapses. Stop gracefully (§3.4) where you can; if the
  frontier waits on an orphaned claim, prove that block directly (`hazync run <n>`, §3.3).
- **Keep the GPU lock.** `HAZYNC_GPU_LOCK` serialises GPU jobs per box (§3.2). Turning it off on a 96 GB card was
  measured **worse**: 7 blocks per 5 min against 15 with the lock.
- **Blackwell workstation and server cards (RTX PRO 6000, `sm_120`) compile the kernels on first use.** The release
  CUDA host carries native kernels for `sm_80/86/89/90/100` and `compute_100` PTX; an `sm_120` card JIT-compiles from
  the PTX for **~10 min on the first prove on a fresh machine**, cached afterwards. A fresh pod that looks stuck on
  its first block is usually doing this — allow for it in selftests and in pod timeouts, and do not judge a card's
  speed by its first block.

## 1. Topology

### 1.1 Two workloads

**Backfill** (G2) is sized by budget and calendar; per-block latency is irrelevant and inputs are a closed,
exhaustively testable set. **Tip-following** (G6) is sized by the block interval; latency matters once
caught up and inputs do not exist yet. ⇒ **While backfilling, size for throughput (§1.3).**

### 1.2 Latency — one block, tip to receipt, inside 600 s

**MEASURED — 8 × L40S, 8 chunks (one per card), po2 21, aggregate over 8 `seg-connect` workers**
(`history/BENCH_8xL40S_2026-09-08.md`):

| block | prevouts | chunk phase (slowest card) | straggler | aggregate | compute total |
|---|---|---|---|---|---|
| 966,108 | 8,562 | 709 s | 1.118 | 203.8 s | **912.8 s** |
| 966,107 | 7,961 | 602 s | 1.048 | 205.8 s | **807.8 s** |
| 966,106 | 6,644 | 522 s | 1.059 | 189.8 s | **711.8 s** |

**INFERRED card curve** (fitted to 966,108): `block ≈ (5,075 / N) × straggler + 19.2 + (184.6 × 8 / N)` →
8 cards 15.2 min *(measured 15m13s)*, 10 → 12.3, 12 → 10.3, **13 → ~9.8**, 16 → 7.8 min. ⇒ **~13 L40S for a
sub-10-minute CORE block.** Only N=8 is measured; the straggler likely worsens above 8 chunks, and the ~19 s
execute has since come off the critical path (#236).

**MEASURED — rented RTX 4090s, block 966,256** (4,741 txs, 9,079 inputs), po2 21, one chunk per card
(`history/MILESTONE_966256_RUN4_2026-09-10.md`): run 1, 26 cards: chunks 243.9 s + aggregate 139.4 s =
**485.2 s** (attested from logs), $1.191. Run 4, 27 cards: 247.9 s + 241.2 s = **544.0 s** (receipt on
disk), $1.387.

⚠ At 26–27 cards the aggregate is as large as the chunk phase, but not a fixed floor: on the same 27
receipts the #235 N-sweep on A40s measured assembly **162.5 s at N=4 → 63.5 s at N=26**. Run 4's 128.7 s
assembly is most likely coordinator round trips across a spread fleet — **inferred** (#252).

### 1.3 Throughput — bounded lag

Concurrency comes from proving *different* blocks at once (`prove-chunk` takes no previous receipt;
per-block aggregates are independent). **INFERRED**, block 966,108:

5,075 chunk card-s + 184.6 × 8 aggregate card-s + 19.2 s ≈ **6,570 card-s per block ⇒ 6,570 / 600 ≈ 11
L40S**. ⚠ One near-tip block, aggregate from the fit; the §1.2 blocks span 3,942–5,075 chunk card-s.

**The chain fold does not threaten it.** MEASURED (stock): **3.76 s/fold** on an L40S, flat in range length
(`history/prover/evidence/fold_and_snark_wrap_1_1000.txt`); 2.0–3.0 s/fold by concurrency
(`history/prover/evidence/fold_concurrency_2xL40S.txt`). ⚠ Both on early receipts; per-fold VRAM at high-UTXO
heights is UNKNOWN.

### 1.4 The card

- ✅ **L40S 46 GB** — the baseline (§1.2). ✅ **RTX 4090 24 GB** — runs po2 21, peak **22,478 MiB** of
  24,564 (91%); ⚠ cannot run po2 22 (a stock chunk peaked at 40.6 GB). **RTX PRO 6000** — the tip-run card
  (§5), $2.09/hr.
- ✗ H100 80 GB **0.95x** (stock); ✗ B200 8.7% slower than L40S even with native `sm_100` (stock)
  (`history/ACCELERATION.md`); ✗ L4 37% more expensive per proof (stock, `history/TEN_MINUTE_BLOCK.md`).

⭐ **Card type is the largest single lever on wall-clock, and price/hour is the wrong objective.**
MEASURED 2026-09-21, 9 runs on block 741000 (#448), 3 cards: all-4090 **272.2 s** mean (267.9 / 271.7 /
276.9, spread 9.0 s), billed **$0.243**; fleets containing an A40 382.8 s mean (357.9 / 380.2 / 410.4,
spread 52.5 s), 4090+2×A40 billed **$0.262**. The A40 ($0.49/hr) loses to the 4090 ($0.74/hr) on price
per proof.

### 1.5 Boxes and roles

| box / role | what runs there |
|---|---|
| board coordinator | `api.hazync.org` (`/api/meta`, `/api/state`), board at <https://hazync.org/>, live page `hazync.org/live`. [`COORDINATOR.md`](COORDINATOR.md) |
| tip driver box | `coordinator/tip_smoke.py`; needs the identity (`/var/lib/hazync/identity-GHOST`), the RunPod key (`/root/.hazync/runpod.key`), the live rig (`/root/hazync-live-rig`) and the publish key (`/root/.ssh/hazync_publish`, dest `root@152.53.86.216:`). Repo at `/opt/hazync`. **No GPU needed** |
| bridge (**v1 only**; v2 has none, §0) | serves tip bundles (`--bridge-host`); retains every bundle back to the emit floor (1,489, oldest 967,500, 2026-09-28; nothing prunes them) |
| prover box | `run-workers.sh` loops (§3), one GPU job at a time |
| rented pods (RunPod) | `tip_smoke.py` fleets (§4–5); 40 GB container disk on the project's cards |

## 2. Proving a block

Proving is RAM- and GPU-heavy. Build on a provisioned box; a small WSL2 machine can run *execute-mode*
validation but not proving. GPU proving needs CUDA **12.8** (RISC0 3.0.5's kernels did not build against
13.x; the 3.1.0 GPU runs of 2026-11-11 also used 12.8) and the `cuda` feature.

```
GPU=1 REPO_DIR=$PWD ./provision-vps.sh
cd prover && cargo build --release --features cuda
```

Release binaries: `hazync-host-x86_64-linux-gnu` (CPU) and `hazync-host-x86_64-linux-gnu-cuda`, called
`./host` below. Commands and defaults are read from `prover/host/src/main.rs`.

### 2.1 Commands

Execute mode (`check-*`, `regress`, `adversarial`) runs the full consensus path (`VerifyScript`,
`CheckTransaction`, no-inflation, PoW, retarget, merkle, subsidy, weight, sigops, maturity, locktime, BIP68,
accumulator transition) and panics on any violation — a clean run means every rule passed. No GPU.

```
H=./target/release/host
$H regress                       # block-170 consensus regression (CI: .github/workflows/adversarial.yml)
$H adversarial                   # every known hole must REJECT (CI too)
HAZYNC_BLOCK=block_741000.json $H check-full
HAZYNC_WITNESS_DIR=/w HAZYNC_FROM=1 HAZYNC_TO=550 $H check-ibd

HAZYNC_BLOCK=block_741000.json $H prove-full                     # one block, monolithic
HAZYNC_BLOCK=block_741000.json HAZYNC_CHUNKS=16 $H prove-seg     # one block, segmented
HAZYNC_BLOCK=block_741000.json HAZYNC_CHUNKS=16 $H chunk-profile # predict chunk costs, instant
HAZYNC_BLOCK=block_741000.json HAZYNC_CHUNKS=16 HAZYNC_PROFILE_EXEC=1 $H chunk-profile  # + real cycles, no GPU

$H prove-chain                   # fold 170 -> 171 -> 172 (IVC), verify the tip
$H prove-chain-bad               # wrong self_id must be REJECTED
HAZYNC_WITNESS_DIR=/w HAZYNC_FROM=1 HAZYNC_TO=170 HAZYNC_TIP=3 $H prove-ibd
$H verify-chain <chain.bin>      # verify + PIN anchor to dsha256(genesis_anchor)

$H prove-range <n>               # backfill: one block as range [n..n]
$H fold-range <left.hzk> <right.hzk> <out.hzk>
$H verify-range <out.hzk>        # verify + PIN leftmost boundary to genesis
$H verify-any <bin>              # no genesis pin; emits boundary digest (coordinator's per-range check)
NGPU=2 LO=1 HI=550 HAZYNC_WITNESS_DIR=/w bash rangecluster.sh   # multi-GPU fan-out -> one receipt
```

- `prove-full` commits a `ChainState` in one guest run. `prove-seg` proves chunks in parallel (`chunk_prove`,
  mode 4) then aggregates (mode 5); each chunk commits an `input_bind` digest the aggregate re-checks. Both
  assert `self_id == METHOD_ID`.
- **Chunk packing** is by equal predicted **cost** (#132), not input count (which left block 741000's
  straggler at 1.22x the mean). Constants in §6.1 (Core's fit; Ghost has its own); repeat keys discounted
  (`COST_PER_EC_OP_REPEAT`); `HAZYNC_CHUNK_PACK=count` restores the old split. Read `chunk-profile`'s
  `straggler: max ... = N.NNx` line (sample `history/prover/evidence/chunk_packing_741000.txt`). `HAZYNC_CHUNKS` is a
  request — an uneven block can pack into fewer runs.
- `verify-chain` checks `self_id`, the `KIND_CHAIN` tag and the genesis anchor, so a checkpoint-anchored demo
  proof fails it ([`SECURITY.md`](history/SECURITY_MODEL_CORE.md)). A fold checks the full seam (tip, UTXO roots+leaves,
  difficulty, MTP window); [`history/SPEC_CORE.md`](history/SPEC_CORE.md) §10.
- `prove-range-bridge` proves a **board** block in one process on one card; for many cards use mode 6 (§2.4).

### 2.2 One block across many cards: chunks + aggregate (fixture only)

⛔ This path proves a **fixture**; its receipt (`KIND_CHUNK` / mode-5 aggregate) is **not submittable** to
the board, which verifies `KIND_RANGE`. For a board block use mode 6 (§2.4).

```sh
# card i, for i in 0..N-1 — SAME block file and HAZYNC_CHUNKS everywhere
HAZYNC_LIFTX_HINT=1 HAZYNC_BLOCK=block_966256.json HAZYNC_CHUNKS=27 \
  HAZYNC_OUT=chunk_$i.hzk ./host prove-chunk $i

# segment coordinator — reads chunk_<i>.hzk (fallback chunk_<i>.bin) from $HAZYNC_RECEIPTS
HAZYNC_BLOCK=block_966256.json HAZYNC_CHUNKS=27 HAZYNC_AGG=1 HAZYNC_PORT=9110 ./host seg-serve
# every worker
HAZYNC_WORKER_ID=w1 ./host seg-connect <coordinator-host>:9110
```

- `HAZYNC_CHUNKS` defaults to **2** — set it identically on every command; each recomputes `chunk_bounds`.
- **One chunk per card.** 8 vs 16 chunks on 8 cards: mean **+0.2%** over three blocks, sign inconsistent,
  all six receipts digest-identical. Not a tuning lever.
- Chunk receipts are verified against `METHOD_ID` on the way in. The block receipt goes to `$HAZYNC_OUT`
  (default `aggregate_receipt.bin`); a verified receipt that cannot be saved exits 3.
- `seg-serve` in chunk mode (no `HAZYNC_AGG`, `HAZYNC_CHUNK=<i>`) distributes one chunk, same `$HAZYNC_OUT`
  default — set it.
- **Put a worker on the coordinator's own card**: `seg-serve` distributes but does not prove. One remote
  worker: 772.4 s with the coordinator's card idle, 473.1 s with a worker on it.
- One box in-process: `agg-chunks` with the same `HAZYNC_CHUNKS` and `HAZYNC_RECEIPTS` (`scripts/gpu-benchmark.sh`).

**Who does what in `seg-serve`.** The segment coordinator executes (streaming segments as produced, #236),
merges the session journal and assumptions into the last claim, resolves assumptions (aggregate only;
**on the coordinator by default**, #446, because the chain is serial — `HAZYNC_RESOLVE_LOCAL=0` sends them
to workers), verifies every returned receipt, assembles and verifies against `METHOD_ID`. Workers prove and
lift each segment, prove the last segment (`NOLIFT_TAG`, #158), lift the merged last segment (`LIFT_TAG`)
and build the join tree (`JOIN_TAG`, `log2(N)` levels). Wire tags are job-index bits: `JOIN_TAG` 31,
`RESOLVE_TAG` 30, `NOLIFT_TAG` 29, `LIFT_TAG` 28; a worker tests resolve before join.

### 2.3 ⛔ `HAZYNC_LIFTX_HINT=1` at run time (chunk path)

The canonical guest is built with the lift_x hint (`patches/0013`) and **reads a hint table before its
first `VerifyScript`**; only a host run with `HAZYNC_LIFTX_HINT=1` writes one (`write_chunk_inputs`).
Without it the input stream desynchronises and the guest dies with `DeserializeUnexpectedEnd`, which reads
as a corrupt fixture ([`history/LIFTX_HINT.md`](history/LIFTX_HINT.md) §6).

**Set it for** `prove-chunk`, `prove-seg`, `seg-serve` (chunk mode), `segment-size`, `exec-time`,
`segment-mem`, `seg-distribute`, `seg-coordinate`, `seg-coordinate-tree`, `chunk-profile` (when executing).
**Not read by** `prove-range-bridge`, `fold-range`, mode 6 (`write_range_env`), the aggregate (`agg-chunks`,
`seg-serve` with `HAZYNC_AGG`) or `seg-connect` workers, so board workers are unaffected. `provision-vps.sh`
exports it in `.bashrc`; a release binary run from any other shell does not inherit it.
`HAZYNC_FIELD_BIGINT2` and `HAZYNC_ECMULT_WINDOW` are **build-time only**; the host never reads them.

> Note: FLEET_OPERATIONS' "passing levers" section listed `HAZYNC_LIFTX_HINT` as build-time only. It is
> read at both build and run time (history/LIFTX_HINT.md §6; `prover/host/src/chunks.rs`). The run-time rule stands.

### 2.4 A board block across many cards (mode 6)

`seg-serve` serves a **bridge range** (#361/#364): N cards, and the `KIND_RANGE` receipt the coordinator accepts.

```sh
# segment coordinator — ONE block from its bridge bundle. No HAZYNC_BLOCK, no chunks.
HAZYNC_RANGE=74928 HAZYNC_BRIDGE_OUT=/workspace HAZYNC_PORT=9110 HAZYNC_BIND=0.0.0.0 ./host seg-serve
# every worker, one per card
CUDA_VISIBLE_DEVICES=0 HAZYNC_WORKER_ID=w0 ./host seg-connect <coordinator-host>:9110
```

- Bundle read from **`$HAZYNC_BRIDGE_OUT/bundle_<n>.json`** (default `/root/bridge_bundles`).
  `HAZYNC_CHUNKS` is not used — parallelism is segments across cards. Put a worker on the coordinator's card.
- ⛔ **Without `HAZYNC_BIND`, remote workers get `Connection refused`**: `seg-serve` binds `127.0.0.1` by
  default because the wire is **unauthenticated** (#365); the shell says so as it binds (#401).
  `HAZYNC_SEG_REMOTE=1` implies `0.0.0.0`.
- ⚠ `0.0.0.0` lets **anyone who can reach the port feed work in**. Use a trusted network or tunnel
  (`ssh -N -L 9110:127.0.0.1:9110 …`, worker at `127.0.0.1:9110`); rented pods usually publish only the
  ports declared at creation, so the tunnel is often the only option.

**Driven by the CLI under your key (#367):**

```sh
hazync run --distributed                                    # claim earliest free block, serve, wait, submit
HAZYNC_BIND=0.0.0.0 hazync run 74928 --distributed --workers=4   # specific block + 4 local workers
```

- `--workers` defaults to 0; external `host seg-connect <this-box>:9110` may join in any order — a worker
  arriving before the server retries (#402). Receipt is the same `range_<n>.hzk`; submission unchanged.
- ⛔ **One block only**: `hazync run 100-200 --distributed` is refused.
- ⛔ **A distributed prove must beat its claim.** Mode 6 prints progress as `91/2352 segments …` (number
  first, plural) vs single-card `segment 91/2352`. Workers before #368 cannot parse it, never beat, and lose
  the block after `CLAIM_GRACE` (600 s). **Use a worker from v0.21.7 or later.** A wedged fleet still loses
  the claim, as intended.
- ⚠ **Pooling costs throughput.** MEASURED 2026-09-18, block 230,000: four cards on one block **96.0
  s/block**, a block each **82.9 s/block**. It buys latency on one block (3.45x). Use it for the frontier
  block, a large block, or the tip.

**MEASURED first mode-6 run** — block 74,928 on 3 × RTX 4090, canonical guest verified on all three
(`history/BENCH_MODE6_3xRTX4090_2026-09-17.md`), in the v0.22.1 summary shape:

```
prologue            6.2 s   <- serial: bind + allocate + spawn
segment phase    2622.7 s   <- 2,352 segments produced AND proved, overlapped
  of which execution 123.6 s   <- INSIDE the line above, not additional
assembly          948.1 s   <- serial: last segment + join tree + resolves
TOTAL            3577.0 s   <- MEASURED, not the sum
serial floor      954.3 s   <- prologue + assembly; no fleet size reduces it
```

⚠ **Do not add these columns** — the pre-v0.22.1 summary did, giving 3694.4 s, ~123.6 s too high (#567).
Receipt 229,754 bytes, identical in size to the single-card one; both paths share one input writer. Pod 2
waited 1,855 s; a join is ~0.4 s of GPU and ~47 s of round trip.

### 2.5 Segment size and the retry ladder

`HAZYNC_SEG_PO2` sets the executor's `segment_limit_po2` on **every** prove path (`prove-range`,
`prove-range-bridge`, `fold-range`, `prove-chunk`, chunk-aggregate, replay, IVC step, SNARK wrap). Host-side:
**does not affect `METHOD_ID`**; the receipt is identical either way.

- **Default 21 on `cuda` builds, 20 on CPU** (since v0.10.0). `host seg-po2` prints it; the CLI's ladder
  starts there. MEASURED on an L40S: block 130000 **23.0 s at po2 21 vs 24.4 s at 20**, flat to 22. po2 21
  needs ~2x the memory: an 11 GB CPU box proving block 170 peaked at **8.7 GB RSS and swapped**.
- **Segment-boundary panic** (`risc0-circuit-rv32im` 4.0.5, `cycles <= 1 << segment.po2`, ~10% of blocks,
  CPU and CUDA; liveness only, never a wrong proof): the CLI retries a failed prove *or fold* at
  `HAZYNC_SEG_PO2` 21→20→19→18.
- **The ladder is for failures, not stalls** (#256): a prove that stops progressing or hits the outer
  timeout is retried **once at the same size**, then abandoned with how far it got (`_run_with_seg_retry`
  in `coordinator/hazync`). ⚠ Swapping is not a failure, so the ladder never fires on it — it just crawls.
- **#119** (accum-pass LogUp bug → non-canonical field element → receipt fails `verify()`): fixed in v0.21.1
  (#245) by vendored `risc0-circuit-rv32im-sys` 4.0.3; upstream shipped the same fix in 4.1.0 (risc0 3.1.0), so
  since v0.24.0 the vendored 4.1.0 copy carries only the Windows build fix (#631). `prove-chunk`, `agg-chunks` (#240) and every
  `seg-connect` worker also retry via `prove_segment_resilient` (up to 3 attempts, only the known transient
  verify failure, a `[#119]` line per retry). **A retry now is new information — report it on the issue.**

### 2.6 SNARK wrap (Groth16, optional)

```
./target/release/host prove-snark
```

MEASURED: block 170 (CPU, guest `3f52baff`) **825.7 s**, 9.7 GB peak RSS, whole receipt **2033 bytes**;
folded `[1..1000]` genesis-anchored **66–77 s**, 1.4 GB, **3,441 bytes**; `[500..500]` mid-chain 5,633 bytes
(`history/prover/evidence/groth16_snark_wrap.txt`, `history/prover/evidence/fold_and_snark_wrap_1_1000.txt`). Wrap cost is flat in chain length; size follows boundary content (popcount of Utreexo roots: min 7 / mean
~14 / max 22). INFERRED full-chain wrap: ~3.4 KB best / ~5.4 KB typical / ~7.6 KB worst — say **"a few KB"**.
The Groth16 proof proper is ~128–256 bytes; the journal dominates (u8-per-word serde, #22).

⚠ **Groth16 fails on CUDA builds** (#20, upstream `sppark` illegal memory access, won't-fix). Wrap on CPU;
it runs once per artifact. `SKIP_GROTH16=1` boxes cannot `snark-wrap` at all.

### 2.7 METHOD_ID and reproducibility

**Releases.** The current release is **v0.25.0**, libzync on RISC Zero 3.1.0; `scripts/check-versions.sh` requires this
line to name the newest tag (or one ahead of it). It ships the published libzync image `8888d59c`. v0.23.0 (`863c9ce3`, risc0
3.0.5) was the first libzync release and v0.22.1 (`37987b85`) the CORE design's last; 3.0.5 receipts do not verify under 3.1.0.

**The canonical id** (`reproduce/METHOD_ID`) is libzync's published mainnet image
`8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4`, with the signet image in
[`reproduce/LIBZYNC_IDS`](../reproduce/LIBZYNC_IDS). `reproduce/libzync.Dockerfile` reproduces both bit for bit and the
`reproducible-image-id` job rebuilds them on every push. Release v0.24.0 ships it (and the board runs it since
2026-11-12).

A receipt verifies only against the guest that produced it. A mismatch prints `STARK verification FAILED.
This is almost certainly a guest image-id (METHOD_ID) MISMATCH` — a build mismatch, not a bad proof.

```
./target/release/host method-id
docker build -f reproduce/libzync.Dockerfile -t hazync-libzync-repro .
docker run --rm hazync-libzync-repro  # must equal reproduce/LIBZYNC_IDS (mainnet: reproduce/METHOD_ID)
```

The CORE bullets below describe `reproduce/Dockerfile`, which built the CORE images.

- Pins: `risc0-zkvm`/`risc0-build` `=3.0.5`; `rzup` rust 1.94.1 / cpp 2024.1.5 / `cargo-risczero`+`r0vm`
  3.0.5; Bitcoin Core `v28.0`; secp256k1 `v0.5.1`; committed `Cargo.lock`; fixed container paths. CI job
  `reproducible-image-id` asserts it. A host built **outside** the container may differ (absolute paths).
- CPU and CUDA binaries embed the same guest ELF, so the CPU Dockerfile attests both.
- Re-baselines are hard cuts; every canonical id is in `reproduce/LINEAGE.tsv`, reasons in `reproduce/METHOD_ID`.
- **Budget:** a 488 MB toolchain download can exceed an hour on a domestic link (#53) — raise `RZUP_TIMEOUT`
  (default 3600 s). Disk: ~15 GB for CPU; CUDA consumed ~18 GB and completed only with 25 GB free.
- ⚠ **Disk exhaustion masquerades as a toolchain bug**: nvcc dies, cc-rs reports `exit status: 139`. If
  nvcc segfaults, `df -h` first.
- ⛔ **Do not switch branches while `prover/build-release.sh` runs** — it bind-mounts the tree and compiles
  whatever is checked out per crate; the result looks healthy but mixes branches. Use a separate clone.

### 2.8 Release binaries (maintainer)

```bash
./prover/build-release.sh cpu     # -> dist/hazync-host-x86_64-linux-gnu
./prover/build-release.sh cuda    # -> dist/hazync-host-x86_64-linux-gnu-cuda
```

Builds at `HOME=/root` in `ubuntu:22.04` (glibc 2.34), prints `METHOD_ID` / `seg-po2` / glibc / SASS
coverage beside the canonical id. CUDA uses the CUDA 12 toolchain with `NVCC_APPEND_FLAGS="-gencode
arch=compute_80,code=sm_80 -gencode arch=compute_86,code=sm_86 -gencode arch=compute_89,code=sm_89 -gencode
arch=compute_90,code=sm_90 -gencode arch=compute_100,code=sm_100 -gencode arch=compute_100,code=compute_100"`
(`sm_100` needs 12.8+, #181); check `cuobjdump --list-elf <host>` shows `sm_80 sm_86 sm_89 sm_90 sm_100`.
⛔ Do not build natively on a CUDA 13 box. Smoke-test each binary: `host method-id`, `host
bundle-roundtrip-test`, `host verify-any` on a served proof, and for CUDA **`host prove-snark`** (no CI GPU;
Groth16 shipped broken v0.8.0–v0.10.0, #20/#23). `history/prover/e2e_bundle_test.sh` covers served-bundle → prove →
verify. Process: [`BUILD_AND_RELEASE.md`](BUILD_AND_RELEASE.md), `scripts/release.sh`.

## 3. Prover operations (board workers)

### 3.1 Quick start: one box, one GPU

```
# 1. download prover, CLI and launcher keeping asset names; verify (../SECURITY.md#verifying-releases)
./hazync id yourname
cp ~/.hazync/key.hex /somewhere/safe/    # lose this and your blocks stay under a name nobody can sign for
./hazync selftest                        # before spending GPU hours
./hazync notify new                      # (v0.21.5) push alerts
HAZYNC_HOST=$PWD/hazync-host-x86_64-linux-gnu-cuda ./run-workers.sh 2
tail -f ~/hazync-workers/worker_1.log
```

### 3.2 Worker modes

`run-workers.sh N` starts N loops, each re-running one CLI command when it exits.

| `MODE` | the loops run | notes |
|---|---|---|
| `prove` (default) | `hazync run`: claim earliest open block, prove, submit | moves the frontier |
| `fold` | `hazync fold`: merge two adjacent proven ranges | seconds per fold |
| `spine` | `hazync spine`: absorb next chunk into the genesis-anchored spine | serial: **one spine worker across all your boxes** |
| `mixed` | N−2 prove, 1 folds, 1 spine (spine only when N > 2) | `mixed` on **one** box, `prove` on the rest |

- ⚠ **Someone must be proving new blocks** — folding and the spine only use existing proofs (the frontier
  stood still 40 min on 2026-09-14 when the only proving card was switched to folding).
- **One GPU job at a time per box**: workers take `/tmp/hazync-gpu.lock` (`HAZYNC_GPU_LOCK`); two provers on
  one card fail when memory peaks coincide. On v2, disabling the lock on a 96 GB card was measured worse, not
  better (7 vs 15 blocks / 5 min, §0.1). Extra workers help only during fetch/coordinator waits; the
  project's cards run two. A direct `host prove-*` does not take the lock (#97).
- **One prove per card**: ~22 GB VRAM at po2 21 (L40S, 2026-08-28).

### 3.3 Claims

A bare `hazync run` claims; `hazync run <n>` or a span claims nothing. The coordinator accepts a valid proof
at any height from anyone; claims only decide who is **offered** a block. Public board runs the defaults
(2026-09-14):

| rule | default | meaning |
|---|---|---|
| heartbeat | while proving | a beating worker keeps its block however slow |
| `CLAIM_GRACE` | 600 s | a claim that never beat is released |
| `CLAIM_TTL` | 3,600 s | released an hour after the last heartbeat |
| `CLAIM_MAX` | 86,400 s | released after a day regardless |
| `CLAIM_OPEN_MAX` | 4 | live claims per key across all boxes; a fifth is "nothing to claim right now" |
| `CLAIM_RETAKE_WAIT` | 3,600 s | a key does not get back a block its own claim let lapse |
| signed claims | v0.21.5+ workers | nobody else can use up your 4 claims |

⛔ **More than 4 proving workers on one key starve themselves**, and **restarting workers mid-run orphans their
claims** for up to `CLAIM_TTL` — an orphaned frontier claim stalls the frontier until it lapses (measured on v2,
2026-11-12; §0.1).

⚠ **Proving by hand** (`hazync run <n>`, for a block stuck behind a claim): not on a box whose loop may be
proving the same block — both use `~/.hazync/receipts/.work-<n>` and the loop's prove crashes. Check
`worker_<i>.log` or use another box.

### 3.4 Start, stop, restart

- **Start** refuses if the prover's guest id differs from `/api/meta`, or if the GPU cannot prove a test
  block; warns if no handle is set.
- **Quick stop**: `./run-workers.sh N --stop`. An in-flight block stays claimed up to an hour (`CLAIM_TTL`), and
  counts against the key's 4 claims meanwhile (§0.1).
- **Graceful stop** (before an upgrade):

```
for p in $(ps -eo pid,args | awk '$2 ~ /^hazync-(worker|fold|spine)-loop-/ {print $1}'); do kill $p; done
for p in $(ps -eo pid,args | awk '$3 == "./hazync-worker" && $4 == "spine" {print $1}'); do kill $p; done
while ps -eo args | grep -qE '^python3 ./hazync-worker (run|fold)'; do sleep 5; done
./run-workers.sh N --stop     # confirms nothing is left
```

⚠ **Use `ps | awk`, not `pkill -f` inside `ssh host '…'` / `bash -c '…'`** — the pattern is in the shell's
own command line, so it kills that shell. `--stop` likewise counts any process mentioning `hazync-worker`
as a survivor.

**Restart**: `run-workers.sh N` again.

### 3.5 systemd

```ini
# /etc/systemd/system/hazync-workers.service
[Unit]
Description=Hazync workers
Wants=network-online.target
After=network-online.target

[Service]
Type=oneshot
RemainAfterExit=yes
User=prover
WorkingDirectory=/opt/hazync
Environment=HAZYNC_HOST=/opt/hazync/hazync-host-x86_64-linux-gnu-cuda
Environment=LOG_DIR=/home/prover/hazync-workers
ExecStart=/opt/hazync/hazync-run-workers.sh 2
ExecStop=/opt/hazync/hazync-run-workers.sh 2 --stop
TimeoutStopSec=120

[Install]
WantedBy=multi-user.target
```

`sudo systemctl daemon-reload && sudo systemctl enable --now hazync-workers`. `systemctl stop` is the quick
stop. `HAZYNC_HOME` is the service user's `~/.hazync` — run `hazync id`/`notify` as that user. Tested
2026-09-14 as a user unit with a stand-in worker; not yet on a GPU box. Pods without systemd (RunPod): use
the provider's start command.

### 3.6 Upgrading

1. Download new files beside the old (with `SHA256SUMS.txt`, `SHA256SUMS.txt.asc`) and verify:
   `gpg --verify SHA256SUMS.txt.asc SHA256SUMS.txt`, `sha256sum -c --ignore-missing SHA256SUMS.txt`.
2. Check the guest id — a mismatched prover has every proof rejected:
   ```
   ./hazync-host-x86_64-linux-gnu-cuda method-id
   curl -s 'https://api.hazync.org/api/meta' | python3 -c 'import json,sys; print(json.load(sys.stdin)["method_id"])'
   ```
3. Graceful stop, swap files (keep the old until the new has proved something), start, `./hazync selftest`.

A worker whose proof is rejected for a mismatch stops itself (exit 78).

### 3.7 Watching it

```
curl -s 'https://api.hazync.org/api/state?slim=1' | python3 -c '
import json, sys
d = json.load(sys.stdin); p = d["progress"]; b = d.get("blocked") or {}
print("frontier", p["frontier"], "| proven", p["proven"], "| spine", p["spine_hi"], "| tip", p["tip"])
print("next block for the frontier:", b.get("block"), b.get("status"), "| waited", b.get("stalled_for"), "s |", b.get("why", ""))
print("needs attention:", b.get("needs_attention"))'
```

**frontier** = genesis to here proven and joined. **blocked** = the block the frontier needs, its status,
wait and reason; a big block under a beating claim is waiting, not stuck — `needs_attention` is the
coordinator's "actually stuck". `leaderboard` rows carry `handle`, `proved`, `folded`, `anchored`, `version`.

Logs: `LOG_DIR/worker_<i>.log` (default `~/hazync-workers`): `claimed block N` (offered N); `✓ range N: the
coordinator re-verified your proof` (landed); `range N was already proved by someone else` (race lost, not a
failure); `nothing to claim right now: …` (nothing free or 4 claims held; waits 30 s); `FATAL: …` (stopped; says why).

**Exit codes**: `0` done; `1` failed, retry after 5 s; `75` nothing to claim, wait 30 s; `78` stopped for
good (no usable GPU, guest id mismatch), loop exits.

**Alerts** (v0.21.5): `hazync notify new` creates a private ntfy topic. Pushes on stop-for-good, rejected
proof, claim refused for a box-side reason, launcher refusal, `NOTIFY_FAIL_STREAK` failures in a row
(default 5), and recovery; one push per problem per hour. **The topic is the only secret — keep it private.**

### 3.8 Disk (MEASURED 2026-09-14)

| what | where | grows | measured |
|---|---|---|---|
| receipts | `~/.hazync/receipts/<range>.bin` | one per block, never deleted | 228 KB largest; 2,225 in 881 MB since 2026-09-11 |
| bundles | `LOG_DIR/bundles_<i>/` or `~/.hazync/bundles/` | one per block fetched, never deleted | ~22 MB for 2,324 early bundles; at height 220,000 median 2.16 MB, largest 16.87 MB |
| spine scratch | `~/.hazync/receipts/.spine-work/` | removed when the spine worker exits | 720 MB |
| logs | `LOG_DIR/worker_<i>.log` | appended | 1.1 MB after days |

Accepted receipts are never read again; resend an unaccepted one with `hazync submit <range>`. Bundles can be
deleted while no worker runs. Busiest project card used 4.4 GB of 40 GB after days. Segment work dir ~1.6 GB
(stock) — watch `df`.

### 3.9 Identity

`~/.hazync/key.hex` is who you are; `~/.hazync/handle` is only the display name. The same key on several
boxes is fine (the 4-claim limit is per key). `HAZYNC_HOME` selects a different identity. New box: copy
`key.hex` (and `handle`). `hazync rotate` is off on the public coordinator (#311).

### 3.10 Rented GPUs as board provers

- ⚠ **A card can pass boot with no usable GPU** — the launcher's test prove catches it; a worker that hits
  it exits 78 (#261).
- ⚠ **Public SSH is not guaranteed**: RunPod 4090 community hosts often never exposed it (3 of 7 on
  2026-09-11, 3 of 3 on retry, 1 of 4 on 2026-09-14); A40s always did.
- ⚠ **Stopping workers does not stop the pod** — a card idled ~3 h at $0.49/h on 2026-09-12.
- ⚠ **The host can read `~/.hazync/key.hex` and submit as you** (not re-key your blocks, #311). Use hosts you
  trust with your name.

### 3.11 When something is wrong

| you see | likely cause | do |
|---|---|---|
| `FATAL: guest id mismatch` | pre-re-baseline binary | upgrade (§3.6) |
| `FATAL: this box has a GPU but cannot prove on it` / `no usable CUDA device` | driver or device | `nvidia-smi`; replace host or fix driver |
| `nothing to claim right now` constantly | busy board, or >4 proving workers per key | fewer proving workers per key, or fold |
| `claim timestamp outside +/-120s` | wrong clock | `timedatectl set-ntp true` |
| `the claim signature does not match that pubkey` | `key.hex` changed under a worker | restart; check `~/.hazync` |
| `refusing to POST … from a source checkout` | CLI run from a git checkout | release CLI, or `COORD_URL` → your own coordinator |
| frontier waits on a block `claimed` for hours | big block, live worker | if `needs_attention`, read `why`; a box not proving it can `hazync run <n>` |
| disk full | receipts, bundles | §3.8 |

Still stuck: issue with the **Board or prover problem** template, `hazync selftest` output and log lines.

## 4. The GPU fleet (RunPod)

`coordinator/tip_smoke.py` rents pods, stages prover and block, runs the gates, proves, harvests evidence
and releases the pods. Orchestration for the 544.0 s run: `tools/milestone/`.

```sh
python3 coordinator/tip_smoke.py --cards 3 --spares 3 \
    --block 741000 --block-path prover/block_741000.json \
    --rundir /tmp/run1 --repo tools/milestone --live-rig tools/live --key ~/.ssh/hz_smoke
```

`--block`/`--block-path` default 130000; ⚠ `--block-path` is a **local** path pushed from the driver.
`--claim` claims a board block and proves it from its bundle (mode 6). All other flags: §5.3.

### 4.1 Capacity and cost

⛔ **Capacity, not money, caps a fleet.** MEASURED 2026-09-27, 45 requested of each, SECURE stock, same
few minutes: RTX 4090 **1** granted ($0.74/hr), RTX PRO 4500 SE **4** ($0.72/hr), RTX PRO 6000 **38**
($2.09/hr). Two hours later the PRO 6000 granted 17. `stockStatus` said `Low` for all of it. You cannot
survey then decide — only be trying when stock appears.

- One-hour session on ~20 RTX PRO 6000: roughly **$45–65** including gates.
- `--gpu-type auto` (default since #493) ranks SECURE-stock types by **measured** cost per proof, then price,
  and fills from **one** type where possible, saying so when it mixes. A comma list (`"NVIDIA GeForce RTX
  4090,NVIDIA A40"`) means **silent first-available fallback** — slower and dearer when it fires.
- ⚠ **Check the `FLEET:` line before comparing runs**; a mixed fleet prints `⚠ MIXED CARD TYPES — timings
  are NOT comparable` (#449; 142 s of spread was once misattributed to geography).

### 4.2 The fleet ledger

Every run that produced a proof appends to `docs/history/fleet-economics.jsonl`; report with
`python3 coordinator/tip_economics.py`:

```
fleet                   n   mean_s     min     max  spread   mean_$
3x RTX 4090             3    272.2   267.9   276.9     9.0    0.255
1x A40 + 2x RTX 4090    1    357.9   357.9   357.9     0.0    0.262
2x A40 + 1x RTX 4090    2    395.3   380.2   410.4    30.2    0.261
```

Nine runs on one block justify a default, not a settled question; the ledger can overturn it. It ranks types
for **selection**; once paid, `tip_lifecycle.rank` keeps the *fastest* cards and rightly ignores price.

### 4.3 Passing levers to the cards

Any `HAZYNC_*` in the driver's environment is forwarded to the aggregate **and every worker**
(`tip_lifecycle.lever_env`, #445), e.g. `HAZYNC_JOIN_LOCAL_MAX=2 python3 coordinator/tip_smoke.py --cards 3 ...`.
Per-run keys (`HAZYNC_BLOCK`, `HAZYNC_PORT`, `HAZYNC_CHUNKS`, `HAZYNC_RANGE`, …) are **reserved** and never
taken from your shell. ⚠ Before #445 no lever reached anything, and an unreachable lever reads as "no
measurable difference" — confirm a lever arrives before trusting an A/B.

### 4.4 Worker failures and trust

Every returned receipt is checked with `verify_integrity_with_context`; a failed check or dropped
connection requeues the job. A dropped worker reconnects for `HAZYNC_RECONNECT_MAX_S` (600 s). Segment
receipts bind pre/post-state digests and `join` checks continuity: **a malicious worker costs latency, not
soundness.**

### 4.5 Teardown and harvest

- ⛔ **Harvest before release.** `rundir/measurements.json` is harvested before release — the only chance.
  Two September tip runs were torn down with no log fetch and the gap was blamed on "nobody ran it" for ten days.
- Stop with `kill -TERM <driver pid>` (handled; reaches the `finally` that releases pods). ⛔ **`SIGUSR1`
  kills the driver** (`REAL_EXIT=138`) — the stack-dump hook is in the coordinator, not here.
- Then verify: `python3 coordinator/tip_smoke.py --cleanup --rundir <rundir>` releases everything in
  `rented.json` **and** `recruited.json`, and reports unknown pods on the account without touching them.
- ⛔ **`terminate` returning ok is not the pod being gone** — trust only `account check: clean`.
- After a run: `rundir/run.log` (with `REAL_EXIT`), `rundir/billing.json` + a `BILLED:` line,
  `rundir/measurements.json`, a row in `fleet-economics.jsonl`.

## 5. Tip runs

Proving blocks **as the chain mines them**, with real money.

### 5.1 Before you launch

1. **Identity** — board credit is public and permanent:
   ```sh
   export HAZYNC_HOME=/var/lib/hazync/identity-GHOST
   python3 -c "import tip_board; print(tip_board.identity()[2])"   # prints the handle
   ```
   ⛔ Always pass **`--claim-as "G H O S T"`** — every box has *some* identity (a session once claimed as
   `hazync-coordinator`). It refuses **before anything is rented**.
2. `/opt/hazync` at the release you mean to run.
3. Driver box has identity, RunPod key, live rig, publish key (§1.5).

### 5.2 The launch

```sh
export HAZYNC_HOME=/var/lib/hazync/identity-GHOST
python3 -u coordinator/tip_smoke.py \
  --session 1 --budget-usd 150 \
  --fresh-tip --clock-from-tip --claim-as "G H O S T" \
  --cards 30 --min-cards 18 --spares 10 \
  --gpu-type "NVIDIA RTX PRO 6000 Blackwell Server Edition" \
  --agg-candidates 4 \
  --live-rig /root/hazync-live-rig \
  --publish-dest root@152.53.86.216: --publish-key /root/.ssh/hazync_publish \
  --rundir /root/tiprun-$(date +%F) 2>&1 | tee -a /root/tiprun-$(date +%F)/run.log
```

Detach with `setsid nohup … &` to outlive ssh. ⚠ Record the exit status **into the log** — a background
wrapper's exit code is the last command's, not the driver's.

### 5.3 `tip_smoke.py` flags

⛔ **`--cards` is a target, `--min-cards` a floor.** Target-as-floor threw away four healthy fleets on
2026-09-27 (29/30, 28/29, 25/26 twice; ~$45, no blocks). Floor-as-target took a 21-card fleet to 16
(`wait_for_ssh` stopped at the floor) then **10** (tail cut trims *down to* it) — too few for tip pace.
**Rule of thumb:** `--cards` = what you want, `--min-cards` ≈ 60% of it, `--spares` ≈ a third.

| flag | default | what it does |
|---|---|---|
| `--cards N` | 2 | **target**: what to rent, ssh wait, tail-cut size, what counts as surplus |
| `--min-cards M` | 0 (= `--cards`) | **floor**: final gate check, `--adopt` check, fetch-abandon view |
| `--spares S` | 1 | padding against bad pods; unused released immediately |
| `--gpu-type` | `auto` | §4.1; or a name / comma list |
| `--claim` | off | prove a claimed board block from its bundle (mode 6) |
| `--claim-as HANDLE` | — | refuse to start unless the identity is HANDLE |
| `--session HOURS` | off | keep claiming and proving for this long |
| `--budget-usd` | 0 | stop when spend reaches this; charged every loop at the **live** fleet rate |
| `--fresh-tip` | off | prove only blocks mined **after** the fleet is ready; **what makes a tip-hour claim honest** |
| `--clock-from-tip` | off | start the session at the **first tip block**; pair with `--fresh-tip` |
| `--clock-wait-max H` | 2.0 | give up if no tip block arrives; a deferred clock has **no other deadline** |
| `--tip-max-behind N` | 3 | unproved tip bundles allowed to queue before jumping to the newest, **recording every skip**; `0` = never jump |
| `--tip-from H` | — | start the tip floor just below `H` to close a gap; overrides `--fresh-tip`'s floor |
| `--allow-tip-gaps` | off | ⛔ prove a non-child tip block — leaves a hole in the chain; recovery only |
| `--no-board-fill` | off | idle between tip blocks instead of board work (pure tip-latency measurement) |
| `--grow-to N` | 0 | rent and gate toward N in the background; recruits join at a **block boundary**. Target in `$RUNDIR/grow_to`, re-read every 45 s: `echo 32 > $RUNDIR/grow_to`. Lowering never releases a card |
| `--gate-parallel N` | 4 | recruits gated at once (serial gating capped growth at ~1 card / 4 min) |
| `--adopt FILE` | — | reuse a previous `rented.json`; adopted pods **not** released at the end |
| `--release-adopted` | off | with `--adopt`, release them after all |
| `--worker-min-mbit` | 0 | release workers the aggregate cannot push to at this rate; 0 = report only |
| `--agg-candidates` | 3 | cards measured before choosing the aggregate |
| `--cleanup` | — | release everything in `rented.json` + `recruited.json`, exit |

### 5.4 Phases

| phase | typical | notes |
|---|---|---|
| renting | seconds | capacity refusals normal; `--spares` absorbs them |
| waiting for ssh | 1–2 min | a pod RunPod never starts is released |
| aggregate link | ~1 min | measures `--agg-candidates` cards |
| staging the block | seconds | |
| **fetching the prover** | **~14 min cold, ~4 s cached** | 411 MB per card; cached **on the pod**, so relaunching onto survivors skips it |
| GPU gate | 2–5 min | throwaway proof per card |
| reachability | ~30 s | card-to-card |
| **live page** | ~2 min after gates | collector → renderer → publisher |
| **first tip block** | **0–20+ min, not yours to control** | MEASURED 2026-09-28: **21 min**, filled with 20 board blocks; without `--clock-from-tip` it comes out of the hour |
| session | `--session` hours | from the first tip block with `--clock-from-tip` |

The log announces `⏱ the clock has NOT started …` then `⏱ CLOCK STARTS: <h> is the first tip block`. The
summary reports `blocks_ok` and `blocks_ok_on_clock`, `spend_before_clock_usd` and `spend_on_clock_usd`.
⛔ **Quote the on-clock pair for tip-following claims** — `blocks_ok` includes pre-clock gap filling.

⚠ **Until the gates finish, `hazync.org/live` serves the previous run's last frame at HTTP 200.** Judge by
`Last-Modified` (on `frame.png`), never the status code.

### 5.5 The proof chain and gaps

- **The session proves the lowest unproved tip bundle**, not the newest (968,984 and 968,985 mined 5 s
  apart once left 968,984 unproven). Past `--tip-max-behind` it jumps and writes a `skipped` line per height
  to `tip_ledger.jsonl`, the log, and `tip_blocks_skipped` in the summary.
- **Each bundle is linkage-checked** after fetch, before any GPU: `in_tip == witness.header[4:36] ==
  reversed(getblockhash h-1)`. Refusal: `refusing to prove <h>: block <h> builds on <x> but the block we
  proved last hashes to <y> — this is a GAP or a fork, not the next link`. Also refused: wrong stated
  height, or `in_tip` disagreeing with its own header.
- ⚠ The **first** tip block of a session logs `unlinked` with the reason — expected.

**Closing a gap** (`--fresh-tip`'s floor can never reach back):

```sh
python3 coordinator/tip_smoke.py --session 0.5 --tip-from 968984 --claim-as "G H O S T" \
  --bridge-host localhost --cards 20 --min-cards 12 ...
```

⛔ Do not pass `--fresh-tip` on a gap-closing run. The bridge keeps every bundle (§1.5), so a gap can be
closed any time.

### 5.6 When it goes wrong (all seen 2026-09-27)

| symptom | cause | do |
|---|---|---|
| `only N of M cards passed every pre-clock gate` | survivors below `--min-cards` | lower the floor; read the per-card drop reasons |
| a card "never answered ssh" | RunPod never published a port | normal; if several at once, suspect the floor used as a wait target |
| `released … the aggregate could not push to it fast enough` | tail cut | several at once ⇒ tail cut trimming to the wrong number |
| `GPU_BAD` | card cannot prove | the gate working |
| log quiet 10+ min in the fetch gate | one slow card | measure the laggard's bytes twice before killing (one went 78% → 99.8% in 40 s) |
| run dies, pods stay up | driver killed without its `finally` | `--cleanup --rundir <dir>`, or adopt |
| live page shows an old run | nothing publishing yet | `Last-Modified` on `frame.png` |

⛔ **Never judge a run by the absence of error lines** — a stalled run is silent; alert on **silence**, with
the phase attached.

## 6. Settings reference

### 6.1 Host environment (`prover/host/src/main.rs`)

⚠ **Presence-tested** flags (`env::var(..).is_ok()`): `HAZYNC_AGG=0` still turns the aggregate **ON** —
only unsetting disables. ⚠ `HAZYNC_RESOLVE_LOCAL` is the exception: value-tested, **unset = ON**, only an
exact `0` disables (#252/#446).

| variable | default | meaning |
|---|---|---|
| `HAZYNC_SEG_PO2` | **21** CUDA, 20 CPU | segment size (§2.5). po2 21 peaks ~22 GB; po2 22 ~40.6 GB, ~11–12% faster (stock/#139), ≥46 GB cards only and serialised, not measured on CORE |
| `HAZYNC_LIFTX_HINT` | — | **`1` at run time** on chunk commands (§2.3) |
| `HAZYNC_CHUNKS` | 2 | chunk count; **= card count**, identical on every command |
| `HAZYNC_CHUNK` | 0 | chunk index for `seg-serve` chunk mode |
| `HAZYNC_CHUNK_PACK` | cost | `count` = old equal-input split |
| `HAZYNC_PROFILE_EXEC` | unset | `chunk-profile` executes for real cycles |
| `HAZYNC_COST_EC_OP` / `_SCHNORR_OP` / `_INPUT_BASE` / `_INPUT_BYTE` | 417,798 / 462,435 / 41,387 / 2 | chunk cost model (`COST_PER_EC_OP`, `COST_PER_SCHNORR_OP`, `COST_INPUT_BASE`, `COST_PER_INPUT_BYTE`): Core's per-curve fit to execute-mode cycles |
| `HAZYNC_BLOCK` | `block_full.json` | fixture (not mode 6) |
| `HAZYNC_AGG` | unset | serve the mode-5 aggregate (presence-tested) |
| `HAZYNC_RANGE` | unset | `=<n>` serves mode-6 bridge range (parsed) |
| `HAZYNC_BRIDGE_OUT` | `/root/bridge_bundles` | mode-6 `bundle_<n>.json` dir |
| `HAZYNC_RECEIPTS` | `.` | chunk receipt dir |
| `HAZYNC_OUT` | `chunk_<i>.hzk` / `aggregate_receipt.bin` | output path |
| `HAZYNC_PORT` | 9110 | `seg-serve` port |
| `HAZYNC_BIND` | `127.0.0.1` | `seg-serve` address; `0.0.0.0` for remote workers, trusted network only (#365) |
| `HAZYNC_SEG_REMOTE` | unset | `=1` implies `0.0.0.0`; explicit `HAZYNC_BIND` wins |
| `HAZYNC_RECONNECT_MAX_S` | **600** | worker reconnect window from the **current** outage; `0` = exit on drop |
| `HAZYNC_PUSH_DEPTH` | 4 | jobs in flight per worker |
| `HAZYNC_PUSH_BYTES` | 64 MiB | in-flight byte budget, clamps depth — raise this, not the depth |
| `HAZYNC_RESOLVE_LOCAL` | **on** | resolves on the coordinator; `=0` pushes to workers |
| `HAZYNC_JOIN_LOCAL_MAX` | `0` (off) | prove join levels of ≤ N pairs on the aggregate; worth ~0.4–1.6 s, off pending a fleet run (#252/#444) |
| `HAZYNC_WORKER_ID` | `push1` | worker log name; set per worker |
| `HAZYNC_SEG_QUIET` | unset | old sparse worker output (#254; presence-tested) |
| `HAZYNC_WITNESS_DIR` / `_FROM` / `_TO` / `_TIP` | — | witness dir and range for `check-ibd`, `prove-ibd` |

### 6.2 Worker CLI and launcher

| variable | default | meaning |
|---|---|---|
| `HAZYNC_HOST` | — | prover binary path for `run-workers.sh` |
| `MODE` | `prove` | `prove` / `fold` / `spine` / `mixed` (§3.2) |
| `LOG_DIR` | `~/hazync-workers` | worker logs and launcher bundles |
| `HAZYNC_HOME` | `~/.hazync` | identity (key, handle, receipts) |
| `HAZYNC_GPU_LOCK` | `/tmp/hazync-gpu.lock` | per-box GPU lock |
| `NOTIFY_FAIL_STREAK` | 5 | failures in a row before a push |
| `COORD_URL` | — | point at your own coordinator (the release CLI refuses to POST from a source checkout) |

Coordinator claim settings: §3.3.

### 6.3 Per-box and provisioning

| setting | value | label / why |
|---|---|---|
| GPU concurrency | **1** | MEASURED (stock): rejected three times at 0.95–1.03x |
| po2 23 | **do not** | ~79 GB (B200-only) and needs two code changes (§6.5) |
| CUDA | **12.8** | `provision-vps.sh` phase 7; `HAZYNC_CUDA_VER` overrides |
| `SKIP_GROTH16=1` | proving and bench boxes | skips `risc0-groth16`; box cannot `snark-wrap` |
| `HAZYNC_PROVISION` | `deps` then `build` | split provisioning phases |
| `RZUP_TIMEOUT` | 3600 s | toolchain download timeout |
| CUDA release SASS | `sm_80 sm_86 sm_89 sm_90 sm_100` | `prover/build-release.sh` |

⚠ **`nvidia-smi utilization.gpu` is kernel residency, not useful work** — never tune from it.
⚠ **Read the per-box evidence directory before commissioning a run** (a B200 was once provisioned to re-test
an answered po2-23 question).

### 6.4 Guest build settings (all shipped; each moves `METHOD_ID`, so not operator knobs)

| setting | shipped | measured gain (cycles, block 140,000, 212 inputs, stock libsecp) |
|---|---|---|
| C/C++ opt | `-O3` | −0.264% |
| Rust `lto` | `"fat"` | −0.486% |
| Rust `codegen-units` | `1` | −0.361% |
| `ECMULT_WINDOW_SIZE` | **21** | **−1.245%** (19: 376,662,184; 20: 375,914,975; 21: 371,971,773; journal identical) |
| `ECMULT_GEN_KB` | 22 | 0% — sizes the signing table; 2/22/86 bit-identical. 2 would save memory, waits for a re-baseline |
| `NDEBUG` | absent | −0.0018% |
| field backend + `lift_x` hint | `patches/0012` + `0013` | **4.095x** fewer cycles on 962,000 (13,748,003,793 → 3,357,576,338), journal byte-identical (`all_valid=1`, `binds=8006`) |

Codegen/window shipped in v0.20.0 (`42417d2`); `0012`/`0013` canonical in v0.21.0 (`c12ad67`, 2026-09-07),
applied unconditionally by `provision-vps.sh`, and in the trust base ([`SECURITY.md`](history/SECURITY_MODEL_CORE.md)). SHA-256
uses the RISC0 accelerator (patch `0002`). Window 21 is the `build.rs` default and `provision-vps.sh` exports
it. Detail: [`BUILD_AND_RELEASE.md`](BUILD_AND_RELEASE.md), [`history/FIELD_BIGINT2_BACKEND.md`](history/FIELD_BIGINT2_BACKEND.md).

- ⚠ **Small-workload sweeps under-rate large windows** (block 130,000, 10 inputs: window 20 worse than 19).
  Re-run codegen arms at realistic input counts.
- ⛔ **Windows ≤15 cannot be swept naively** — `build.rs` regenerates `precomputed_ecmult.c` only above 15.
  **Any sweep mutates `$HAZYNC_BASE`**: back up and restore `secp256k1/src/precomputed_ecmult.c`.

### 6.5 Not settled / closed

**Open:** card curve between 9 and 26 cards on one fleet; po2 22 on CORE; per-fold cost and VRAM at
high-UTXO heights; po2 23 (needs raising `DEFAULT_MAX_PO2` **and** recomputing and shipping
`allowed_control_root("poseidon2", 23)` — raising the cap alone makes every proof fail verification).

**Closed — do not re-open without new evidence:** chunk count; GPU concurrency; the card axis; CUDA kernel
compiler lever (5 arms, stock best); coordinator egress; `NDEBUG`; C/C++ LTO (`rust-lld` cannot read GCC LTO
bytecode); a newer risc0.

## 7. Superseded numbers — do not quote

| number | status |
|---|---|
| ~29 L40S throughput, 32–48 latency | ⛔ stock guest on 962,000; CORE ~11 / ~13 (§1) |
| ~7–9 cards with #139 | ⛔ projection; measured 10 CORE / 5 Ghost on 962,000 |
| 9.10x wholesale bigint2 | ⛔ never produced by a run (nothing called it until `patches/0014`, `9b767b5`) |
| "aggregate scaling past 2 cards is UNMEASURED" | ⛔ 3 workers 2.78x (stock, `2facde4`); 8 workers 203.8 s; #235 sweep to N=26 |
| "chain fold per-block cost UNKNOWN" | ⛔ 3.76 s/fold (§1.3) |
| "24 GB cards must drop to po2 20" | ⛔ po2 21 fits, 22,478 MiB |
| aggregate 1,575 s / 627 s on 962,000 | ⚠ 473.1 s with the coordinator working, 405.6 s on two workers |
| "the aggregate saturates at N=2" / "is the binding constraint"; ">3,300 s"; `N^1.79` scaling | ⛔ false / stale / artefact of changing block |
| "the GPU is 65% idle" | ⛔ refuted: 91.5% busy |
| "worker processes are worth 1.20x" | ⚠ ceiling ≤1.09x (stock) |
| mode-6 `TOTAL 3694.4 s` | ⛔ double-counted execution; ~3,571 s real (§2.4) |

Evidence records: `history/BENCH_8xL40S_2026-09-08.md`, `history/MILESTONE_966256_RUN4_2026-09-10.md`,
`history/BENCH_MODE6_3xRTX4090_2026-09-17.md`, `history/SEGMENT_DISTRIBUTION.md`,
`history/TOPOLOGY_AND_SETTINGS_2026-09-05.md`, `history/CORE_VS_GHOST.md`, `history/ACCELERATION.md`.
