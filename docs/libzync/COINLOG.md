# libzync: the coin log

Status: built behind `HAZYNC_COINLOG=1` (profile bit 4, `0x10`); design B (SwiftSync forest + hints file) without it.
Normative source: `libzync/src/coinlog.rs` (shared by the host and the guest) and the coin-log block of
`prover/methods/guest/src/agg_lite.rs`. `scripts/libzync/coinlog_ref.py` is an independent implementation written from
this document, with its own SHA-256 compression function.

## 1. What it is

Every coin that outlives the block that creates it is **appended**, in birth order, to an append-only forest of
perfect Merkle trees. Spending a coin **flips** its leaf in place to a spent leaf that records who spent it. Nothing
is moved or deleted, so a coin's position never changes.

The commitment is therefore the complete coin history, and it is sound **at every block**: a spend opens the coin's
unspent leaf at its position against the current roots, so a coin that does not exist, or was already spent, cannot
be spent. There is no hints file and no closing height H. Peter Todd's TXO commitments are the same idea.

## 2. Hashes

All three are a single SHA-256 **compression** of one 64-byte block, starting from a tag-derived state instead of
SHA-256's IV. There is no padding and no length block. A state is 8 words, big-endian, as in FIPS 180-4; the output is
the 8 result words, big-endian.

```
IV_NODE[l] = SHA-256("libzync-coinlog-node" ‖ l as 1 byte)        l = 0..63
IV_SPENT   = SHA-256("libzync-coinlog-spent")

parent(l, L, R) = compress(IV_NODE[l], L ‖ R)                      the root of a tree of 2^l leaves, l ≥ 1
spent(c, s)     = compress(IV_SPENT,  c ‖ s)                       c: the coin's unspent leaf
s               = height u32 LE ‖ tx u32 LE ‖ vin u32 LE ‖ 20 zero bytes
```

- `height` is the spending block, `tx` the spending transaction's position in it (the coinbase is 0, so the first
  spending transaction is 1), `vin` the input index.
- The per-level IV binds a node to its height in the tree, so a path cannot be shortened or lengthened.
- An unspent leaf is the coin leaf of UTREEXO_FORMAT.md §2 (tag 0x00, a full SHA-256). A spent leaf is a compression
  output from another IV, so it never equals a coin leaf.

Test vectors (`coinlog.rs` test `refusals`, from `coinlog_ref.py`): with x = SHA-256("c"), y = SHA-256("d"),
`spent(x, s(974000, 5, 1)) = cdcc3dda…388993e`, `parent(3, x, y) = fcd6a122…6c29f14f`.

## 3. Shape and order

- The forest of n leaves has one perfect tree per set bit of n, the largest (oldest leaves) first. `roots[l]` is the
  tree of 2^l leaves, as in UTREEXO_FORMAT.md; `ChainState.utxo_roots` carries it with trailing empties removed.
- **Appended:** the outputs of UTREEXO_FORMAT.md §3 rules 1 to 3 (spendable, not one of the two BIP30-unspendable
  coinbases, not spent in the same block), **in block order**: transactions in block order, outputs in vout order.
  ⚠ Not sorted by (txid, vout) as design B's survivors are: the log's positions follow the chain, not Core's key order.
- **Flipped:** every input spending a coin created in an earlier block.
- A block first applies all its flips against the roots it starts from, then all its appends.

## 4. A block's witness

The aggregate reads, after its hint block: `[k, (position lo, hi) × k, sibling hashes × 8 words]`.

- `k` positions, one per spend of an earlier coin, in block order. The guest pairs each with the leaf the chunk
  committed for that input, sorts by position and requires strictly increasing positions below n.
- The siblings are the hash of every untouched subtree met while walking down from each touched tree's root to the
  spent positions, in the order `update` reads them; the positions alone fix the shape. Each touched tree's old root
  must equal the committed root (`OldRoot` otherwise); the witness must be used exactly (`Witness`).
- Refusals are logged as `agg-lite: REJECT coin log: <Order|PastEnd|OldRoot|Shape|Witness>`.

## 5. What the proof commits

| field | under the coin log |
|---|---|
| `utxo_roots`, `utxo_leaves` | the log's roots and the number of coins **ever appended** (live and spent) |
| `utxo_value` | the satoshis of the **live** coins: previous + this block's appends − this block's flips |
| `ss_set`, `ss_hints` | unused (0); a hints file is refused |
| `profile` bit 4 | set |
| `block_roots` | each block's leaf commits the coin state after it (HANDOVER.md §3), so a tip proof commits the log at every height |

## 6. What it lets a zyncAPP prove

From one verified proof at the tip, with a path from the host's copy of the log:

- **Coin C is unspent now:** C's leaf at its position against `utxo_roots`.
- **C was spent, by (height, tx, vin):** `spent(C, s)` at its position.
- **C's state at an earlier height h:** a block-forest path to h's leaf, the roots that hash to its `state`, and C's
  path against those roots (the host keeps or replays the log at h).
- **A coin's history in both directions:** backwards through each transaction's inputs (block forest + Merkle
  branch), forwards through the spender record.

## 7. Cost (block 962,000, execute mode, `zutxo/README.md`)

Aggregate 73.1 M against design B's 64.1 M; chunks +2.6 M (each transaction's output total); whole block about
+2.7%. The log witness replaces the hints file.

## 8. Host

The host keeps every node of the log (`coinlog::Store`) and writes `state_<h>.coinlog` (u64 count, then 32 bytes per
leaf) at the end of a range, or at every height with `HAZYNC_COINLOG_SAVE_EACH=1`. A run anchored on a receipt needs
that receipt height's log file. Mainnet's log is about 2.4 billion leaves; a production host needs a disk-backed store.
