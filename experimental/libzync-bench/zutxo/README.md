# Zutxo prototype: Core's chainstate as a compact Merkle tree, updated every block (2026-11-02)

⛔ Experiment, not consensus code. Code: `methods/guest/src/zutxo.rs` (shared by guest and host), guest mode 6,
host `zbench-host zutxo-test <trials>` and `zbench-host zutxo <keys file> <n coins> <seed>`; `zutxo_keys.py` extracts a
block's touched coins.

**Tree.** Key = SHA-256(txid ‖ vout LE). hash(S) = 0 if empty, the coin's leaf if one coin, no node for a one-sided
split, else one SHA-256 compression of L ‖ R from a per-depth starting state (the split depth is bound). The root
depends only on the set, so a Core `dumptxoutset` from any height rebuilds it, and it gives membership and
non-membership. **Block update:** the touched coins (deletes = inputs spending earlier coins, inserts = spendable
outputs not spent in the block) plus a witness of untouched summaries; one pass checks the old root and computes the
new one; paths in a block share their upper levels.

**Correctness:** `zutxo-test 300`: on explicit random sets (1..1,021 untouched coins, deletes, leaf changes, inserts)
the batched update equals the from-scratch root of the old and the new set, 300/300.

**Measured** (guest cycles, execute mode). The untouched ~170M coins are modelled: above depth 24 every subtree is
non-empty (opaque summaries); each touched depth-24 bucket gets Poisson(10.1) explicit coins. Input reading (serde)
is excluded; the witness is 1.6–6.8 MB per block.

| block | touched (del / ins) | tree M | keys + sort M | per node hash | block today M (cost/survey.out, scripts on) |
|---|---|---|---|---|---|
| 970,000 | 2,920 (2,133 / 787) | 87.6 | 10.8 | 950 | 390.6 |
| 962,000 | 4,324 (2,542 / 1,782) | 124.8 | 16.7 | 951 | ~427 (r17m) |
| 900,000 | 6,265 (3,694 / 2,571) | 174.6 | 25.1 | 952 | 280.4 |
| 850,000 | 13,529 (4,987 / 8,542) | 350.8 | 58.9 | 957 | 430.4 |
| 800,000 | 14,213 (3,521 / 10,692) | 366.2 | 62.0 | 957 | 423.3 |
| 750,000 | 14,029 (6,113 / 7,916) | 362.8 | 61.0 | 956 | 412.5 |
| 700,000 | 12,183 (6,206 / 5,977) | 318.6 | 52.5 | 956 | 423.4 |
| 650,000 | 11,602 (8,725 / 2,877) | 305.7 | 49.6 | 956 | 489.9 |

First version (two-block padded SHA-256 node, 67-byte message): 1,702 cycles per node hash, 962,000 tree 223.5 M.

**Against what it replaces / the alternatives, on 962,000 (BLOCK_962000.md):** SwiftSync ECMH 10.0 M (but sound only
at H); the old Utreexo forest path ~87 M (cheap appends; roots depend on history). Zutxo 141.6 M gross.

**Not yet tried:** traversal overhead (~950 cycles per node is more than one accelerated compression), raw
txid ‖ vout keys (no key hash: one-sided splits cost nothing, so a transaction's outputs need no extra nodes), input
reading by slice instead of serde.

---

# Coin log prototype (2026-11-02)

Code: `methods/guest/src/coinlog.rs` (shared), guest mode 7, host `zbench-host coinlog-test <trials>` and
`zbench-host coinlog <block file> cost/txcounts.txt <k> <seed>`; `coinlog_keys.py` extracts a block's appends and the
creation heights of the coins it spends.

**Structure.** Every coin that outlives its block is APPENDED, in birth order, to a forest of perfect Merkle trees
(utreexo-style stump, largest/oldest first). A spend flips the coin's leaf to spent(c) in place and re-hashes its path.
Nothing moves, nothing is deleted. Leaf flip and parents are one SHA-256 compression each (per-level IVs).

**Correctness:** `coinlog-test 300`: batched spends + appends == the log rebuilt from scratch, 300/300, logs of
1..3,000 leaves with pre-spent leaves; every root also checked against the perfect tree over the leaves.

**Model.** Log size at height h = 1.64 × cumulative transactions (`getchaintxstats`; 1.64 = the measured recent appends
per transaction, 7,670 per block over ~4,670 transactions). A spent coin sits at a random rank inside its creation
block's appends (spends of one creation block that land on the same position are dropped: 4–160 per block, so slightly
optimistic; real coins from one transaction are adjacent, which shares more, so slightly pessimistic). Ages are real,
from the block files' `coin_height`. Witness read excluded (0.6–2.0 MB per block).

| block | log size | spends / appends | median spend age | hashes per spend | log cycles M | key tree M | block today M | log adds |
|---|---|---|---|---|---|---|---|---|
| 970,000 | 2.38 B | 2,129 / 787 | 109 | 21.7 | 24.7 | 98.3 | 390.6 | +6.3% |
| 962,000 | 2.32 B | 2,533 / 1,782 | 65 | 22.8 | 31.4 | 141.6 | ~427 | +7.3% |
| 900,000 | 1.97 B | 3,645 / 2,571 | 79 | 17.1 | 34.0 | 199.7 | 280.4 | +12.1% |
| 850,000 | 1.71 B | 4,943 / 8,542 | 70 | 20.6 | 58.6 | 409.7 | 430.4 | +13.6% |
| 800,000 | 1.44 B | 3,361 / 10,692 | 6 | 16.1 | 34.8 | 428.2 | 423.3 | +8.2% |
| 750,000 | 1.24 B | 5,984 / 7,916 | 25 | 14.7 | 50.4 | 423.8 | 412.5 | +12.2% |
| 700,000 | 1.10 B | 6,130 / 5,977 | 42 | 16.0 | 54.6 | 371.2 | 423.4 | +12.9% |
| 650,000 | 0.94 B | 8,695 / 2,877 | 275 | 17.5 | 80.9 | 355.3 | 489.9 | +16.5% |

525 cycles per hash (vs 951 in the key-tree prototype: no tags, no per-node allocation). On 962,000: coin log 31.4 M,
old Utreexo forest path ~87 M, SwiftSync ECMH 10.0 M (sound only at H), key tree 141.6 M.
