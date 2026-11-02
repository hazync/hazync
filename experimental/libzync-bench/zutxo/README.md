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
