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

## Counting trie, trimmed (11-04, measured)

Profiling c10t's aggregate (risc0 pprof, 962,000) showed hashing was a minority of the trie's cost: building the trie
from the witness, 32-byte copies (memcpy 36 M) and paging (~40 M) dominated. Two trims:

1. **One pass** (40269e1): the guest reads the witness once in preorder and computes each expanded node's old and new
   hash together; new subtrees are built straight from the sorted inserts; no tree in memory. Coin leaf one compression
   (value and spent flag in the spender record's spare words). Chunk side: coin key = txid xor vout (no hash), script
   hash one raw compression, transaction parsed once. Result (c10t2, 00ec71b3): chunks -13 M per block, but the
   aggregate got WORSE (+63.3% whole): memcpy rose to 62 M (hashes copied through return values and the reader).
2. **In place** (trim 2, c10t3 73158901): hashes written through out-parameters, witness hashes read by reference,
   updates sorted by index. A random test (300 blocks) checks the one-pass roots equal the reference `apply`.

| block | c10s live script tree | c10t trie | **c10t3 trie, trimmed** |
|---|---|---|---|
| 170,060 | +14.3% | +9.7% | +5.6% |
| 650,000 | +55.6% | +46.9% | +26.1% |
| 692,261 | +68.0% | +58.0% | +32.7% |
| 700,000 | +67.8% | +58.8% | +32.8% |
| 750,000 | +78.5% | +68.0% | +38.5% |
| 800,000 | +78.0% | +58.8% | +36.3% |
| 850,000 | +72.0% | +63.6% | +39.6% |
| 900,000 | +54.3% | +39.4% | +24.0% |
| 962,000 | +35.9% | +35.3% | +21.3% |
| 970,000 | +27.3% | +27.5% | +16.9% |
| **9 modern blocks** | **+60.2%** | **+51.3%** | **+30.0%** |

- Against main on 962,000 (CORE ~4,430 M): c10t3 534 M = **8.3x** (base 10.1x).
- Address reuse (HAZYNC_TTREE_SUB=1024 other coins per spent-from address): 962,000 +24.9%, 750,000 +44.1%
  (vs +21.3% / +38.5% at 2).
- What is left is hashing: the aggregate's extra cycles per trie compression are 368 (962,000), 281 (750,000) and
  288 (850,000), against ~283 per compression measured alone. 962,000 still has ~16 M of non-hash work; the heavy
  blocks have almost none. Further cuts must remove compressions: a shorter address leaf (count and value packed with
  a 160-bit script hash, one compression instead of two), fewer modelled addresses (2.32 B at 962,000 is deliberately
  high; each halving is about one level), dropping the coin key from the chunk journal for outputs. Each is a few
  percent.

**Trim 3** (c10t4, 58bab460): address leaf one compression (160-bit script hash + count u32 + value in one block) and
no output coin key in the chunk journal (the aggregate derives txid xor vout). 9 modern blocks **+28.9%** (trim 2:
+30.0%); per block 962,000 +20.4%, 750,000 +36.9%, 850,000 +38.0%, 970,000 +16.1%. Against main on 962,000: 530 M =
**8.36x**. As predicted, a few percent: the trie is at its hashing floor. Not done: the real address count (a whole-chain
scan; the model's 2.32 B is high, each halving ~1 level).

**Fixes, nothing given up** (c10t5, c7c1463e; ea35dd8): the address leaf binds the full 256-bit script hash again
(count and value 64-bit, two compressions with counts; the 160-bit leaf let two scripts one party controls collide in
~2^80), and the coin key XORs the output index into the txid's FIRST bytes (in the last bytes, one transaction's coins
at one address shared 224 bits and needed a ~224-node chain). 9 modern blocks **+28.0%** (trim 3: +28.9%): the
coin-key fix more than pays for the full binding (800,000 +31.9% vs +34.6%, 850,000 +34.6% vs +38.0%). Per block:
650,000 +25.7%, 692,261 +31.5%, 700,000 +31.9%, 750,000 +36.9%, 900,000 +21.8%, 962,000 +19.8%, 970,000 +15.6%.
Against main on 962,000: 527 M = **8.40x**.

## What per-block history costs: coin log vs SwiftSync (11-04, measured)

c10sw (9035a44d): the libzync image with SwiftSync accounting (design B: hint bits from the published hints file,
stop height 970,000, every block covered), on the same 10 blocks. SwiftSync proves one state at its stop height; the
coin log proves a state at EVERY block, with each spent coin's spender (history).

| block | coin log vs SwiftSync | coin log + trie vs SwiftSync |
|---|---|---|
| 170,060 | +0.3% | +5.7% |
| 650,000 | +5.2% | +32.1% |
| 692,261 | +2.0% | +34.2% |
| 700,000 | +2.1% | +34.7% |
| 750,000 | +0.9% | +38.2% |
| 800,000 | -1.8% | +29.5% |
| 850,000 | +1.5% | +36.6% |
| 900,000 | +2.8% | +25.3% |
| 962,000 | +2.8% | +23.2% |
| 970,000 | +2.6% | +18.6% |
| **9 modern blocks** | **+2.1%** | **+30.7%** |

Per-block soundness and coin history (the coin log) cost about 2% over SwiftSync's one-height proof. Absence, address
history and balances (the trie) are the remaining ~28%.

## Trie in the aggregate vs trie as a parallel receipt (11-04, measured, execute mode)

c10tsp (e48145b7): the aggregate recomputes the trie update list's digest (`ttree::list_digest`) instead of updating
the trie; the zyncAPP `triebench` guest does the update alone from the same witness and commits root + digest
(`zyncapp bench-trie`, input dumped by the core host with HAZYNC_TRIE_DUMP). Compared with c10t5 (trie in the aggregate):

| block | aggregate, trie inside | aggregate, split | trie guest | serial aggregate saved |
|---|---|---|---|---|
| 650,000 | 228.0 M | 107.5 M | 133.2 M | 120.5 M (53%) |
| 692,261 | 229.7 M | 110.0 M | 136.2 M | 119.8 M (52%) |
| 700,000 | 212.7 M | 96.1 M | 131.3 M | 116.6 M (55%) |
| 750,000 | 238.2 M | 110.3 M | 145.4 M | 127.9 M (54%) |
| 800,000 | 211.0 M | 113.5 M | 117.7 M | 97.5 M (46%) |
| 850,000 | 239.8 M | 118.4 M | 139.6 M | 121.3 M (51%) |
| 900,000 | 110.8 M | 59.5 M | 59.7 M | 51.3 M (46%) |
| 962,000 | 140.8 M | 95.0 M | 58.3 M | 45.8 M (33%) |
| 970,000 | 113.4 M | 80.0 M | 43.7 M | 33.4 M (29%) |

- 9 modern blocks: serial aggregate 192 M -> 99 M (no-tree base 77 M); the trie guest averages 107 M and runs beside the
  chunks. Whole-block cycles **+2.4%** for the split (the update list is hashed on both sides, plus reading).
- Not in these numbers: verifying one more receipt in the aggregate (a recursion step at proving time) and real GPU
  wall-clock. Both belong to the GPU benchmarks.
- The split aggregate is still 22 M over the no-tree base: building the keys, marks and inserts and the digest. A
  hunt target.

## Cycle hunt, batch 1 (11-04, measured; E2a)

Three read-only agents profiled the chunk, the aggregate and the trie guest (`~/libzync-gpu/e1/prof`). Batch 1 = the
no-risk items (6862ec5, image c10b1 b9e00ee6, split build): libsecp SHA-256 through the accelerator
(patches/libzync/0103), `cached_tx` moves the parsed transaction, the aggregate reads chunk journals straight into words,
host pubkey hints also cover taproot output keys / control-block internal keys / P2PK and bare-multisig prevout keys.
Against c10tsp (user cycles):

| block | chunks | aggregate | whole |
|---|---|---|---|
| 650,000 | -1.5% | -2.8% | -1.7% |
| 692,261 | -1.1% | -3.0% | -1.5% |
| 700,000 | -0.9% | -2.6% | -1.2% |
| 750,000 | -1.0% | -2.9% | -1.4% |
| 800,000 | -7.8% | -3.2% | -6.8% |
| 850,000 | -9.0% | -3.0% | -7.7% |
| 900,000 | -14.2% | -3.6% | -12.5% |
| 962,000 | -2.9% | -6.4% | -3.6% |
| 970,000 | -11.3% | -6.8% | -10.6% |

- Padded (proven) cycles are now in every row: on 962,000 the whole block is 464 M user, 563 M padded (+21%: paging and
  segment padding). Earlier tables are user cycles only.
- The streamed update-list digest measured SLOWER in the trie guest (962,000 58.3 -> 59.8 M; 750,000 145.4 -> 147.3 M)
  and was reverted.
- Two corrections from the agents: an accelerated SHA-256 compression is ~74 cycles (risc0 circuit source), not the
  ~283 measured through a byte-converting wrapper, so the trie was NOT at its hashing floor (hashing ~21-24% of the trie
  guest); and the aggregate journal embeds the image id, so journal hashes differ between builds by design.

## Live-coin hash for instant IBD (11-04, measured; E2b candidate)

c10u (4b87b5ab, 909b07f): c10b1 + `utxohash` — the chain state's ECMH total tracks the LIVE coin set under the coin
log (every coin that outlives its block added, every earlier coin a block spends subtracted), so a new node checks a
downloaded UTXO set (~10 GB) against one value. Against c10b1:

| block | user cycles | padded cycles |
|---|---|---|
| 650,000 | +4.44% | +4.06% |
| 692,261 | +5.71% | +5.32% |
| 700,000 | +5.69% | +5.23% |
| 750,000 | +6.60% | +6.12% |
| 800,000 | +6.96% | +6.44% |
| 850,000 | +6.52% | +6.01% |
| 900,000 | +4.66% | +4.11% |
| 962,000 | +1.96% | +1.81% |
| 970,000 | +1.33% | +1.23% |
| **9 modern blocks** | **+4.95%** | **+4.56%** |

About 2.4k cycles per ECMH operation (750,000: ~14k operations, +33 M). Recent blocks are cheap because most of their
coins are created and spent inside the block (never enter the set). The estimate given before measuring was ~2.5%.

## Cycle hunt, batch 2: aggregate (11-04, measured)

80c3043, image c10b2 (c0bd9f2f; split + live-coin hash): created coins through a per-block-salted hash table (was a
sort of 40-byte rows + binary searches), external double spend and distinct-txid checks through the same table,
coin-log spends ordered by an index sort, no unused trie key list. Against c10u (batch 1 + live-coin hash):

| block | aggregate | whole (padded) |
|---|---|---|
| 650,000 | -25.2% | -4.6% |
| 692,261 | -27.2% | -6.2% |
| 700,000 | -27.1% | -5.8% |
| 750,000 | -28.9% | -7.3% |
| 800,000 | -30.9% | -8.7% |
| 850,000 | -28.3% | -7.7% |
| 900,000 | -25.5% | -5.0% |
| 962,000 | -32.3% | -6.5% |
| 970,000 | -32.6% | -5.7% |
| **9 modern blocks** | **-28.5%** | **-6.4%** |

Same contents: the aggregate journals of c10b2 and a baseline built from 909b07f (HAZYNC_JOURNAL_DUMP) differ only in
`self_id` and `anchor_id` (the test anchor's journal embeds the image id) on 962,000 and 750,000; every other word
(coin-log roots, live-coin hash, trie digest, chain fields) is identical. The rejection rules touched (spend before
create, in-block and external double spend) are covered by the Core-comparison rig in the full test pass (same messages).
