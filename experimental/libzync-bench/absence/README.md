# E1: proofs of absence (outpoint-keyed tree) — estimate first

⛔ EXPERIMENT. Nothing here is in the guest.

The coin log (COINLOG.md) is in birth order, so it proves what a coin IS, never that an outpoint is ABSENT
("X was never created", "X is not live"). That needs a second tree keyed by outpoint, updated every block: one
delete per spend, one insert per surviving output.

## Estimate (`estimate.py`, 11-03)

Compressed sparse Merkle tree, uniform keys, 180 M live coins (depth ~27), a block's updates applied as one batch,
old and new root recomputed over the touched paths; cycles per hash from the coin-log bench (100-525).

| block | updates | hashes | cycles (100 / 250 / 525 per hash) | vs whole block |
|---|---|---|---|---|
| 962,000 | 20,850 | ~616 k | 62 / 154 / 323 M | +14% to +74% of 439 M |

(`estimate_962000.out` also has 741,000.) For comparison the whole coin log costs **+2.1%** over 10 blocks.

## Measured hash cost (11-03, `zyncapp bench-hash`, execute mode)

| node hash | cycles per hash |
|---|---|
| one SHA-256 compression from a per-level IV (the coin log's `parent`) | **283** |
| SHA-256(0x01 ‖ L ‖ R), 65 bytes via `sha2` (the block forest's) | 1,105 |

With the coin-log hash, 962,000's ~616 k hashes are **~174 M cycles, about +40% of the block** (439 M), against +2.1%
for the coin log. The coin-log hash is the one to use if this is built.

## What it would unlock

- "Outpoint X does not exist / is not live" with a short proof. Most uses are already covered positively: a spent
  coin has a spent-by proof; a coin that never existed has no creating transaction to show.
- The valuable absence statements are about **scripts**, not outpoints ("these are ALL the coins of my keys", for
  proof of reserves). An outpoint-keyed tree does not give those; a script-keyed tree would, at a similar or higher cost.

## Plan

Build behind a guest feature after the rest of the roadmap, measure on 962,000 in execute mode, and decide with the
measured number. Script-keyed is the variant worth measuring first if absence is wanted for proof of reserves.

## Recommendation (11-03)

Do **not** put the keyed tree in the core: +40% on every block for a fact few zyncAPPs need. Build it as its own
recursive zyncAPP chain beside the core instead: each step verifies the core receipt for block h (assumption), opens
block h in the core's block forest (the hash binds the block's bytes), parses its transactions (no scripts: the core
already checked them) and updates the keyed tree. The core stays at +2.1%; only users who want absence proofs pay for
that chain. A script-keyed variant (all coins of a script, for proof of reserves) fits the same shape.

## Measured: variant images on the 10-block cost table (11-03, `cost_variants.sh`)

Execute mode, 16 chunks + aggregate, real blocks, each tree modelled at 180 M live keys (`HAZYNC_ITREE_N`) as the coin
log is. Images: c10b e7514dc8 (coin log, no tree), c10o 629005ae (outpoint tree), c10s 3c948172 (script tree),
c10os 95df867e (both). Measurement builds only: the tree state comes from the host's witness, nothing binds it across
blocks yet.

| block | base whole | base aggregate | + outpoint | + script | + both | aggregate, outpoint | aggregate, both |
|---|---|---|---|---|---|---|---|
| 970,000 | 445.4 M | 62.9 M | +23% | +27% | +45% | 146 M | 229 M |
| 962,000 | 440.0 M | 74.8 M | +31% | +36% | +60% | 189 M | 303 M |
| 900,000 | 339.4 M | 47.5 M | +50% | +54% | +100% | 206 M | 368 M |
| 850,000 | 486.1 M | 91.3 M | +67% | +72% | +133% | 397 M | 702 M |
| 800,000 | 461.4 M | 84.0 M | +71% | +78% | +142% | 388 M | 701 M |
| 750,000 | 466.2 M | 84.5 M | +74% | +78% | +147% | 409 M | 733 M |
| 700,000 | 482.3 M | 75.0 M | +64% | +68% | +127% | 367 M | 658 M |
| 692,261 | 513.0 M | 86.3 M | +64% | +68% | +127% | 396 M | 704 M |
| 650,000 | 611.9 M | 88.6 M | +53% | +56% | +105% | 395 M | 699 M |
| 170,060 | 26.7 M | 3.2 M | +14% | +14% | +27% | 7 M | 10 M |

**Summed over the nine modern blocks: outpoint tree +55.6%, script tree +60.2%, both +110.7%.** The 11-03 estimate
(+40%) was low: it took 962,000, which has fewer coin updates than mid-history blocks.

- **Almost all of it is the aggregate.** Chunks grow 8.4% (both trees: key hashing per input and output); the
  aggregate grows 2-5x per tree. Cost tracks a block's tree updates (962,000: 2,542 removes + 1,782 inserts;
  650,000-850,000: 12-20 k), not its signature work.
- **Not all of it is hashing.** 962,000's outpoint tree adds ~114 M aggregate cycles for ~95 k siblings; the node
  hashes at 283 cycles account for roughly half. The rest is witness decoding, 64-byte-key map operations and key
  building: an optimisation target if a tree is kept.
- **Whole chain, rough:** the scripts-ON whole-chain estimate (WHOLE_CHAIN_COST.md, ~$114-119k A40) scaled by these
  ratios gives **~+$65k for one tree, ~+$130k for both** (±50%: nine samples, all from 650,000 up; early-chain blocks
  have fewer coin updates per cycle of signature work). Per tip block (962,000) one tree is ~+$0.06, both ~+$0.12 at
  the A40 rate.
- **Tip latency:** the aggregate is one execution, but risc0 proves its segments in parallel; what grows serially is
  executing it (and the join depth). Not measured in seconds here.

## Measured: sharding the outpoint tree by key prefix (11-03, `shard_bench.sh`, `zyncapp bench-shard`)

The design: 2^s shards by the key's top bits, chunks commit a per-shard multiset hash of their keys (SwiftSync's sum),
one shard prover per shard applies its keys, the aggregate checks the sums and folds 2^s roots. The bench executes
one shard prover per shard on each block's real remove / insert counts (random keys, which route like SHA-256 keys).
The shard guest's `read` (serde `env::read` of the witness) is a bench artifact and is left out below; the core reads
its witness by slice.

| block | 1 shard: apply | 16 shards: apply total / slowest | 64 shards: apply total / slowest | multiset sums |
|---|---|---|---|---|
| 970,000 | 68.6 M | 66.6 M / 4.6 M | 65.5 M / 1.6 M | 3.1 M |
| 962,000 | 94.1 M | 91.2 M / 6.3 M | 90.1 M / 2.0 M | 4.5 M |
| 800,000 | 251.7 M | 244.5 M / 15.9 M | 240.4 M / 4.4 M | 14.9 M |
| 650,000 | 260.7 M | 253.0 M / 16.7 M | 248.9 M / 4.5 M | 12.2 M |

(all ten blocks: `~/libzync-gpu/e1/cost/*_shard*.log`)

- **Calibration holds:** 1 shard's apply is ~83-85% of the outpoint tree's measured aggregate growth (962,000: 94 M of
  114 M; 650,000: 261 M of 306 M). The rest is key building and witness handling in the aggregate.
- **Sharding saves no work.** Total apply drops 3-5% (shallower shard trees); the multiset sums add ~5%; each shard is
  also one more proof process (~8.5 s fixed on a PRO 6000). Net: the same dollars or slightly more.
- **It only splits the serial execution:** the slowest of 16 shards is ~1/16 of the total. Since risc0 already proves
  one long execution's segments in parallel, that buys executor time at the tip, not proving cost.

**Verdict:** sharding does not meet the bar ("seriously saves us something"). It is not built further.

## Options B and C measured (11-04)

### Inputs

**Coin lifetimes** (`lifetimes.py`, read-only raw blocks from the bridge node; `lsm_model.py`). Tree updates per block
are what the guest applies (spends of coins not created in the same block + creates not spent in it); the parse
reproduces 962,000's guest counts exactly (2,542 removes, 1,782 inserts).

| coins spent within | 961,000-963,015 (2,016 blocks) | 750,000-751,007 (1,008 blocks) |
|---|---|---|
| the same block | 37.7% | 3.8% |
| 16 blocks | 64.2% | 33.0% |
| 144 blocks (a day) | 79.1% | 61.7% |
| 1,008 blocks (a week) | 87.4% | 75.3% (504) |
| tree updates per block | 9,369 | 10,011 |

**Merge step** (`zyncapp bench-merge`, execute mode): rebuild each input run's root, check order, merge with
cancellation, build the output root. **16-way: ~1,400 cycles per entry; 4-way: ~1,110** (about half hashing, half the
merge loop; flat from 16 k to 320 k entries).

**Batch apply of today's tree** (`bench-shard` with 1 shard): 20.9 k cycles per update at 4 k updates, 19.7 k at 20 k,
18.7 k at 80 k. Bigger batches barely help: the cost is per touched leaf, not shared upper paths.

**Today's tree, per update** (cost table, aggregate growth / updates): 962,000 26.4 k; 750,000 23.1 k.

### Option B: one batch per epoch

Epoch 144: 43.9% (modern) / 50.6% (mid) of the per-block updates survive; epoch 1,008: 26.4% / 27.7%. At ~19 k cycles
per update that is **~2-2.5x cheaper (epoch 144) or ~4x (epoch 1,008)** in tree work, facts a day or a week old.

### Option C: log-structured tree (per-block sorted runs, merged 16 at a time)

Survival after the 16-block merge: 70% / 80%; after the 256-block merge: 38% / 44%; beyond the window no further
cancellation is assumed (conservative). Merges per update: **2.46 / 2.68**. With a block run (~400) plus ~1,400 per
merge: **~3.9 k / ~4.2 k cycles per update, 5.5-6.8x cheaper than today's tree.** Fanout 4 is worse (more levels).

| block, both trees | today | option C (tree work modelled from the measured parts) | + today's chunk key hashing |
|---|---|---|---|
| 962,000 | +266 M (+60%) | 2 x 4,324 x 3.9 k = 34 M | +38 M => **+72 M (+16%)** |
| 750,000 | +684 M (+147%) | 2 x 14,029 x 4.2 k = 117 M | +35 M => **+152 M (+33%)** |

- The per-block critical path carries only the block's own run; merges are separate proofs, range-partitionable.
- What remains is about half chunk key hashing (both keys for every input and output, ~2.5 k cycles per item).
  Hashing only the net entries would cut it to an estimated ~9 M on 962,000 (not measured).
- Whole chain, rough: today's trees ~+111%; option C both trees ~+20-30%.
- Cost of the design: an absence / completeness fact checks every live run (up to ~15 per level, ~4 levels), so
  zyncAPP proofs are larger; merges are new core proof steps.

### Against the 10x bar (962,000, cycles; live main ~4,430 M)

base 440 M = 10.1x; option C both trees ~512 M = 8.7x (~482 M = 9.2x with net-only key hashing); option C script tree
only ~9.3-9.6x. In card-seconds the base itself measured 8.6x, not 10x.
