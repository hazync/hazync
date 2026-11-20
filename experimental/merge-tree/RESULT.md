# Would a wider merge tree make Hazync v2 cheaper?

**Question.** The whole-chain cycle split is chunks 67.5%, trie 17.0%, aggregate 15.6%
(`experimental/libzync-bench/cost/survey_c14.out`, the unweighted sum over its 19 sample blocks). If more receipts were
merged per step (a k-ary join tree, or several blocks per aggregate), how many cycles per block would merging cost?

**Short answer.** Almost none of the 15.6% is merging, so a wider tree cannot touch it. The merging Hazync actually pays
for is the **join** in the backfill tree (one per block). That is about **1.4 M cycles a block, ~0.4% of the chain's
cycles** (with mainnet-size journals). A 16-ary tree would save ~1.3 M of it. **~80% of that saving needs no wider tree**:
the join reads its two journals as serde `Vec<u8>`, one u32 word per byte, and reading them with `read_slice` (as the
aggregate already does, #136) cuts a binary join from 1.41 M to 0.38 M cycles. In card time the ceiling is the joins'
whole budget, ~1,700 PRO 6000 card-hours (~$4.3k, ~1.5% of the chain); a k-ary tree could recover most of it, but
how much depends on recursion costs the executor does not count.

Everything here is **executor (execute-only) cycles** on `hazync-coord`, from origin/main `cac5a2e` (core image
46666b6f) plus the experimental patch `merge-tree.patch` (guest modes 19/20/21 and host `mt-join`; the patched image is
`ab669f6d` then, with mode 21, a later id: never the shipped image). Projections are labelled.

## 1. What a merge step costs, from the code

v2 has two merge steps (`prover/methods/guest/src/main.rs`):

| step | merges | guest work per merged receipt | guest work per session | per block of data |
|---|---|---|---|---|
| **aggregate** (mode 5) | 16 chunk receipts (2 on the board) + the trie receipt + the previous chain proof (none on a leaf) | `env::verify(self_id, journal)` (hashes the claim with the accelerator and records an assumption); the chunk journal is then consumed by `validate_block_lite` as block data | startup (`__libc_init_array`, the allocator check, `assert_core_constants`), decoding the previous `ChainState`, committing the new one | everything else: merkle and witness commitment, BIP30/34/68, inputs covered once, the BIP30 SMT, coin-log apply, ECMH, undo hash, the trie list check, forest append |
| **join** (mode 18, backfill tree) | 2 chain proofs | `env::read::<Vec<u8>>()` of the journal, `env::verify`, serde decode of a `ChainState`, field checks; `link_digest` (re-serialise + SHA-256d) of the left | startup, committing the joined `ChainState` | none |

**What the executor does not count.** In execution `env::verify` only records an assumption. It is paid at proving
time by risc0's recursion programs: each segment is **lifted**, the segment receipts are **joined**, each assumption is
**resolved** (one recursion proof per merged receipt), and the result is compressed to succinct. The project's own code
says so (`prover/host/src/lz.rs`, `report_agg_execute_only`: "In execute mode `env::verify` merely RECORDS an
assumption; RESOLVING it is recursion, and that cost lands in PROVING"). This project has only ever measured those
costs as GPU **wall time**, never as cycles:

- **resolve: 0.28 s each, linear in the count** — `docs/history/TEN_MINUTE_BLOCK.md` §(b), block 741,000, 4 and 16
  chunks (v1 aggregate, 2026-08, not a PRO 6000). v2: the aggregate's tail after the last chunk is ~3.5 s with 16
  chunks + trie (`experimental/libzync-bench/gpu/GPU_RESULTS.md`, 969,122, 4x RTX PRO 6000).
- **join: 6.4 s each, one RTX PRO 6000**, one 2^20 segment — `GPU_RESULTS.md` (top), c14, 7 real joins of blocks 1..8.
  The time is the prove call: execute + segment proof + lift + 2 resolves + compression.

So a merge has (a) guest cycles per merged receipt, (b) guest cycles per session (startup, commit, padding to a power
of two), (c) recursion per merged receipt (resolve) and per segment (lift, segment join). Only (a) and (b) are measured
below; (c) is bounded from the wall-time records above.

## 2. Measured: today's merge cycles

### 2a. The aggregate: per merged chunk receipt

`sweep_agg.sh`: the same block, `HAZYNC_CHUNKS` = 1..32, `agg-exec-fake` (coin log and trie around the block MODELLED,
as in the cost survey), segments 2^21. Data: `data/agg_sweep.out`.

AGG_TABLE

**The aggregate's cost does not depend on how many receipts it merges**: each extra chunk receipt adds ~7 k cycles
(the claim hash and bookkeeping). Merging 16 chunks against 1 costs AGG16 cycles more on 962,000, AGG16PCT of the
aggregate. The aggregate's 15.6% is block validation, not merging.

The **chunks** do get dearer per receipt (the `chunk_user` column): splitting a block into more chunks repeats some
work per chunk (CHUNK_PER on 300,000 and 962,000 per extra chunk). That is a leaf-side cost of parallelism, not of
merging; it was not profiled here. It is not in the 67.5% either: the survey ran one chunk per block.

### 2b. The join: per merged receipt and per session

`sweep_join.sh` and the mode-21 runs: one join session executed over k synthetic `ChainState` journals, correctly
chained (`anchor_id = link_digest(previous)`), assumptions as unresolved claims (exactly what an execute-only run of a
real join sees). Journal size is what drives the cost, so three sizes:
*small* = 2 coin-log roots + 2 forest roots (1,508 bytes, the early chain); *realistic* = 16 + 10 Some roots
(4,412 bytes; near the tip the coin log has ~32 levels and the forest 20, about half of them occupied);
*worst* = 32 + 20 (7,844 bytes). Data: `data/join_sweep_mode18_19.out`, `data/join_sweep_mode21.out`.

**Harness check:** the shipped binary join (mode 18) on *small* journals is 537,572 user cycles, one 2^20 segment —
the GPU run of real joins of blocks 1..8 was also one 2^20 segment. Mode 19 at k=2 matches mode 18 to 0.2%.

| | user cycles | padded (proved) | segments |
|---|---|---|---|
| empty session (mode 20: startup + a 4-byte commit) | 70,226 | 262,144 | 1 |
| binary join today (mode 18), small | 537,572 | 1,048,576 | 1 |
| binary join today (mode 18), **realistic** | **1,408,147** | **2,097,152** | 1 |
| binary join today (mode 18), worst | 2,436,020 | 3,145,728 | 2 |
| binary join with `read_slice` (mode 21), realistic | 381,704 | 1,048,576 | 1 |

Where the binary join's cycles go (pprof, worst size, `pprof_sum.py`): **67% is `Vec<u8> as Deserialize`** — the two
journals arrive through `env::read()` as `Vec<u8>`, which risc0 serde reads one u32 word per byte (the cost #135/#136
removed from chunks and the aggregate). With `read_slice` the rest is the `ChainState` commit (~28%: `FdWriter`, 213 k),
`link_digest`'s re-serialisation, and the decode.

Per merge (one merge = one block joined in), realistic journals; user / padded cycles per merge:

| k (receipts per join) | joins per block | mode 19: serde `Vec<u8>` read (as today) | mode 21: `read_slice` |
|---|---|---|---|
| 2 (today) | 1 | **1,408 k / 2,097 k** | 382 k / 1,049 k |
| 4 | 1/3 | 876 k / 1,049 k | 191 k / 350 k |
| 8 | 1/7 | 723 k / 899 k | 136 k / 300 k |
| 16 | 1/15 | 662 k / 717 k | 114 k / 142 k |
| 32 | 1/31 | 635 k / 677 k | 105 k / 135 k |
| 64 | 1/63 | 622 k / 666 k | 100 k / 133 k |

Fit (realistic): mode 19 ≈ 0.19 M + 0.61 M·k; mode 21 ≈ 0.19 M + 0.095 M·k per session. The per-session part
(0.19 M: startup 70 k + the joined state's commit) is what a wider tree amortises; the per-receipt part is paid per
block whatever the width. Small and worst sizes scale the same way (tables in the data files).

## 3. Not measurable in the executor

- **Resolves.** Each merged receipt is resolved once, whatever the width: a k-ary tree over N leaves has N−1+(N−1)/(k−1)
  edges, so resolves per block fall from 2 (binary) to k/(k−1) (1.07 at k=16). Recursion cycles are not counted by
  the executor; the project's measure is 0.28 s per resolve (v1, not this card).
- **Lift and segment joins** scale with segments (padded cycles above), plus one compression per session: sessions
  per block fall from 1 to 1/(k−1).
- **Per-proof fixed time.** The 6.4 s join is ~1.3 s of segment proving at the PRO 6000's measured 1.13-1.26 s/Mcycle
  (`GPU_RESULTS.md`, 10-29) and 2 × 0.28 s of resolves if that rate holds; the other ~4.6 s is per session (setup,
  lift, compression) — an **inference**, never measured apart. If true, a join's card time is mostly per-session, which
  a wider tree amortises far better than cycles suggest.
- **Multi-block aggregates** (B blocks in one aggregate session) were **not built**: `agg_lite`'s hint, coin-log and
  trie state are per-run globals (`swiftsync::install`, `agg_lite::set_log_witness`, `set_trie_journal`), so a loop over
  blocks is a guest restructure, not a measurement harness. Bounded in §4 from measured parts instead.

## 4. Projections (labelled: arithmetic on the measurements above)

Chain: N = 970,000 blocks; the c14 survey's 335.0 T cycles = 345 M user cycles a block on average (chunks + aggregate
+ trie, one chunk per block, **no joins** — the joins are on top). Join cost per block = cost per merge × 1.

| per block, realistic journals | user cycles | saved vs today | whole chain saved | % of 335 T |
|---|---|---|---|---|
| binary join today | 1,408 k | — | — | (joins = 1.37 T, 0.41%) |
| binary + `read_slice` | 382 k | 1,026 k | 1.00 T | 0.30% |
| 4-ary + `read_slice` | 191 k | 1,217 k | 1.18 T | 0.35% |
| 16-ary + `read_slice` | 114 k | 1,294 k | 1.26 T | 0.38% |
| 16-ary, serde read (as today) | 662 k | 746 k | 0.72 T | 0.22% |
| no joins at all (ceiling) | 0 | 1,408 k | 1.37 T | 0.41% |

Padded (what the prover pays): today 2,097 k a block; 16-ary + `read_slice` 142 k; saving 1.96 M a block, 1.90 T for
the chain. Arithmetic, e.g. 16-ary: (1,408,147 − 1,716,660/15) × 970,000 = 1.255 T; 1.255 / 335.0 = 0.375%.
Early blocks have small journals (537 k today, 187 k binary `read_slice`), so the chain total is below these
realistic-size figures.

**Card time (projection, inferred constants).** The project's estimate for the backfill's joins is ~975 k joins ×
6.4 s ≈ 1,700 PRO 6000 card-hours ≈ $4.3k, ~1.5% on top of the block proofs (`GPU_RESULTS.md`). Floor for any k-ary
tree: ~1.07 resolves a block × 0.28 s ≈ 0.3 s, so the most a wider tree can save is ~6.1 of 6.4 s a block, ≲ 1,640
card-hours ≲ $4.1k (≈1.4% of the c14 estimate's ~$278-295k on PRO 6000). At realistic journal sizes today's join is a
2^21 segment, not 2^20, so the real join budget is somewhat larger than 1,700 card-hours.

**Several blocks per aggregate (bound).** A B-block leaf session removes, per block, (B−1)/B of: one join (above), the
aggregate's per-session part, and its last-segment padding. Measured per-session parts: startup 70 k cycles (mode 20);
the `ChainState` commit ~118 k (aggregate pprof of block 100,000, `FdWriter`); padding of the last 2^21 segment
0.45 M (100,000) to 1.9 M (300,000) padded cycles. **Not** saved: the early aggregate's floor is the BIP30 SMT
(~2.2 M of 2.97 M on block 100,000, 74.5% in `smt_roots`) and the rest of block validation, which are per block.
So B → ∞ saves at most ~1.4 M (join) + ~0.2 M (session) = ~1.6 M user cycles a block (0.46% of the chain), plus
0.45-1.9 M padded cycles of last-segment padding, plus one proof process per block (~8.5 s on a PRO 6000 in
`cost/estimate.py`'s model; 970,000 × 8.5 s ≈ 2,290 card-hours ≈ $5.7k). It is the largest of the levers here, and the
most work.

## 5. Verdict and what a proving run must confirm

- **No saving in the aggregate's 15.6%** from wider merges: measured ~7 k cycles per merged chunk receipt (§2a).
  Confidence high (executor counts, 4 blocks × 6 widths).
- **The join is the merge cost**: ~1.4 M cycles a block (0.41% of the chain). A 16-ary tree saves ≤ 0.38%;
  **`read_slice` in the existing binary join saves 0.30% alone**, with no change to the tree, the host's join
  scheduling or the verifier. Confidence high for cycles (synthetic but correctly chained journals; the harness matches
  the shipped mode 18 and the GPU run's segment size).
- **In card time** the joins are ~1.5% of the chain and a wider tree could recover most of it, but only if the join's
  card time is per-session as inferred. Confidence low: resolve, lift and compression costs are not in any executor
  count and the 0.28 s/resolve figure is from another card and another guest.

A real proving run would have to measure, on one RTX PRO 6000 with real (non-dev) receipts of adjacent blocks at a
mainnet height (journal ~4.4 KB):
1. today's mode-18 join: card time, and how it splits into segment proving, lift, resolves and compression;
2. the same join with `read_slice` (mode 21, k=2): does halving the padded cycles (2^21 → 2^20) show in card time;
3. a k-ary join at k = 4, 16: time per session and per resolve, i.e. whether a join's time is per session (wider tree
   wins) or per receipt (it does not);
4. only then, whether a multi-block aggregate is worth the guest restructure.

## Files

- `merge-tree.patch` — the experimental guest modes 19 (k-ary join), 20 (empty session), 21 (k-ary join reading
  journals with `read_slice`) and the host's `mt-join`. Applied by `build.sh` to a copy of origin/main.
- `build.sh` — builds the patched host on hazync-coord (TEST_LINEAGE mainnet variant; prints the image id).
- `sweep_join.sh` — join width × journal size; `sweep_agg.sh` — aggregate cycles × chunk count.
- `pprof_sum.py` — self/inclusive cycles from a risc0 pprof.
- `data/` — raw outputs.
