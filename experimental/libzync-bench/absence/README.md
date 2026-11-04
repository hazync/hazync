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

## Bootstrap-at-a-height design: three measurements (11-04; not decided)

The design: the whole-chain proof carries no trees; one bootstrap proof builds both trees from the coin log at a height
(host sorts, guest checks order + a multiset hash equal to the coin log's live set); from then on each tip block's own
proof updates both trees (no lag).

**1. Bootstrap per-unit cost** (`zyncapp bench-boot`, execute mode):

| unit | cycles |
|---|---|
| spent coin-log leaf (fold into the forest root) | 279 |
| live coin (coin leaf from preimage, fold, both keys, both multiset sums) | 2,775 |
| sorted tree entry, per tree (order, leaf, fold, multiset sum) | 1,748 |

At 970,000 (~1.45 B transactions; coin log ~2.5-3.6 B leaves, estimated; ~180 M live): spent leaves 0.64-0.95 T +
live coins 0.50 T + two sorted trees 0.63 T = **~1.8-2.1 T cycles once, ~0.7-0.8% of a whole-chain proof (~$1k at
A40)**, input reading not counted. Every pass is range-parallel. The trees start with only live keys (depth ~27.4,
what the cost table modelled); dead leaves accumulate afterwards (~+1 level a year).

**2. Overhead trim ceiling** (cost table + exact compression counts from each block's witness x 283 cycles):

| block | tree cost in aggregate | hashing | non-hash (trim ceiling) |
|---|---|---|---|
| 970,000 | 83 M | 52 M | 37% |
| 962,000 | 114 M | 70 M | 38% |
| 750,000 | 324 M | 192 M | 41% |
| 650,000 | 306 M | 188 M | 39% |

Correction to 11-03: hashing is ~62% of the tree cost (old and new paths are both recomputed), not about half. A
realistic trim is ~20-25%. Chunk key hashing (+38 M on 962,000, both trees) is separate and could shrink to the net
entries only (not measured).

**3. Tip latency** (`tip_latency.sh`, laptop executor, timestamped host output):

| block | aggregate execution, no trees | both trees | added |
|---|---|---|---|
| 962,000 | 6.3 s | 20.3 s | +14 s |
| 750,000 | 10.0 s | 38.7 s | +29 s |

The harness's host preparation also grew (962,000: 13.1 -> 16.9 s; 750,000: 14.5 -> 40.6 s), but that is the
measurement build's collect run plus witness synthesis; a production host serves the witness from its own tree store
(not measured). Against a ~200 s tip block: +7-15% wall-clock from execution.

## History-keeping script tree (11-04, measured)

Variant `htree` (guest feature, `HAZYNC_HTREE=1`, commit 32b47d6). The script tree never removes: a spent key stays,
its leaf becomes `parent(59, live(key,next), spender)` with the coin log's spender (height, tx, vin). The tree holds
every coin ever made at every script, so it gives complete address history (each coin, unspent or spent-by) at every
proven height. Modelled at the coin log's size (962,000: 2.32 B keys; 750,000: 1.24 B) instead of 180 M live keys.
Images c10h b704daf3, c10oh cba70157 (outpoint tree still live-only + history script tree).

Whole-block cycles vs c10b (no trees), same 10 blocks:

| block | c10s (live script) | **c10h (history script)** | c10os (both, live) | **c10oh (outpoint + history script)** |
|---|---|---|---|---|
| 170,060 | +14.3% | +9.4% | +26.8% | +21.9% |
| 650,000 | +55.6% | +38.7% | +104.7% | +87.7% |
| 692,261 | +68.0% | +55.8% | +127.1% | +114.8% |
| 700,000 | +67.8% | +54.2% | +127.2% | +113.5% |
| 750,000 | +78.5% | +65.8% | +146.6% | +133.8% |
| 800,000 | +78.0% | +73.2% | +142.3% | +137.5% |
| 850,000 | +72.0% | +63.5% | +133.4% | +124.8% |
| 900,000 | +54.3% | +42.2% | +100.1% | +87.8% |
| 962,000 | +35.9% | +29.6% | +60.5% | +54.2% |
| 970,000 | +27.3% | +21.7% | +44.9% | +39.3% |
| **9 modern blocks** | **+60.2%** | **+49.7%** | **+110.7%** | **+100.1%** |

Keeping history is **cheaper** than removing, despite the deeper tree: a mark rewrites one leaf, a remove rewrites
the removed leaf and relinks its predecessor. 962,000: 4,324 touched leaves / 78,451 siblings (history) vs 6,866 /
94,726 (live); 750,000: 14,029 / 217,994 vs 20,142 / 246,374. The ~3.7 extra levels cost less than the leaves saved.
Aggregate extra per modern block: 201 M (c10h) vs 251 M (c10s); 439 M (c10oh) vs 489 M (c10os).

Against main on 962,000 (CORE ~4,430 M): base 440 M = 10.1x; c10h 570 M = 7.8x; c10oh 679 M = 6.5x (c10s 7.4x,
c10os 6.3x). Not measured: an outpoint tree that also keeps history (same mark trick; expected to save similarly).

⚠ **Correction (11-04): c10h / c10oh undercount the history tree.** The tree takes the coin log's appends, and the
coin log skips coins created and spent inside the same block (`agg_lite.rs`: `if spent[i] { continue; }`). At 962,000
that is 5,464 of 7,246 created coins (the history tree inserted 1,782). A complete address history must also insert
those coins, already marked. Their cost is not in the table above, so "+49.7%, cheaper than live-only" is not yet a
valid result. Re-measure with the within-block coins inserted marked.

## Corrected history tree and the counting trie (11-04, measured)

Commit 0bc7dff. **c10h2** (ab3f1ac1): the history script tree inserting EVERY created coin (one spent in its own block
inserted already marked), modelled at 2x the coin log's size (962,000: 4.64 B keys). **c10t** (a87a541e): the counting
trie (`libzync/src/ttree.rs`), one two-level binary trie: address (S) then coin (O), coins never removed (spender
written), each address leaf holding its unspent count and value. Modelled with as many addresses as coin-log coins
(962,000: 2.32 B, deliberately high) and 2 other coins under each address the block spends from.

Whole-block cycles vs c10b:

| block | c10s live script tree | c10h2 history tree (corrected) | **c10t counting trie** | c10os two live trees |
|---|---|---|---|---|
| 170,060 | +14.3% | +10.2% | +9.7% | +26.8% |
| 650,000 | +55.6% | +40.5% | +46.9% | +104.7% |
| 692,261 | +68.0% | +60.6% | +58.0% | +127.1% |
| 700,000 | +67.8% | +57.4% | +58.8% | +127.2% |
| 750,000 | +78.5% | +70.8% | +68.0% | +146.6% |
| 800,000 | +78.0% | +84.2% | +58.8% | +142.3% |
| 850,000 | +72.0% | +72.2% | +63.6% | +133.4% |
| 900,000 | +54.3% | +48.8% | +39.4% | +100.1% |
| 962,000 | +35.9% | +61.2% | +35.3% | +60.5% |
| 970,000 | +27.3% | +51.0% | +27.5% | +44.9% |
| **9 modern blocks** | **+60.2%** | **+60.5%** | **+51.3%** | **+110.7%** |

- The correction matters most on recent blocks: 962,000 creates 7,246 coins, 5,464 of them spent in the same block
  (750,000: 310). c10h2's inserts go from 1,782 to 7,246 there, and its cost from +29.6% to +61.2%.
- The counting trie gives every fact in ONE tree (coin unspent / spent by X / never existed, an address's complete
  history, its unspent count and value) for less than the live-only script tree: a coin is one leaf with no neighbour
  to relink, and an address's coins share one path.
- Counts are nearly free: without them (HAZYNC_TTREE_COUNTS=0) 962,000 +34.7% (vs +35.3%), 750,000 +66.5% (vs +68.0%).
- Address reuse is the sensitive assumption: 1,024 other coins under each spent-from address (HAZYNC_TTREE_SUB=1024):
  962,000 +44.0%, 750,000 +82.1%. Real reuse lies between (most addresses are used once or twice; exchanges hold
  millions); not measured from the chain.
- Against main on 962,000 (CORE ~4,430 M): c10t 595 M = 7.4x (base 10.1x, c10h2 6.2x, c10os 6.3x).
- Not tried: the trie in the guest uses boxed nodes and recursion (a flat array would cut non-hash work); the 37-41%
  non-hash share measured for the indexed tree suggests the order of what a trim can win.
