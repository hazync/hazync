# libzync UTXO accumulator: format specification

Status: describes the code at branch `libzync`. Normative sources are `prover/methods/guest/verify_input.cpp`
(`hz_leaf`, `coin_leaf`, `tx_out_leaves`), `prover/methods/guest/src/utreexo.rs` (`parent`, `Stump::add`) and
`prover/methods/guest/src/agg_lite.rs` (which outputs enter the forest). An independent implementation written
from this document is `scripts/libzync/utreexo_ref.py`; §6 says how the two are checked against each other.

⚠ This is **not** the utreexod / Floresta format (§7).

## 1. What it commits to

At SwiftSync stop height H a libzync chain proof commits `ChainState.utxo_roots` and `ChainState.utxo_leaves`:
an append-only Utreexo forest whose leaves are **exactly the coins unspent at H**, one leaf per coin, in
creation order. One mainnet exception: for H from 91,722 to 91,879, Core's UTXO set still holds the coinbase
outputs of 91,722 and/or 91,812 until they are overwritten; this forest never holds them (§3.2), matching the
hints-file numbering. Nothing is ever deleted from it. Coins the hints file marks spent go to the ECMH total instead,
which must be the identity at H (`ss_set == 0`).

## 2. Leaf

```
leaf = SHA-256( 0x00                      tag, 1 byte
              ‖ txid                      32 bytes, internal byte order (as in the outpoint)
              ‖ vout                      u32 little-endian
              ‖ value                     u64 little-endian, satoshis
              ‖ len(scriptPubKey)         u32 little-endian
              ‖ scriptPubKey              len bytes
              ‖ height                    u32 little-endian, the creating block's height
              ‖ coinbase                  1 byte: 0x01 if created by a coinbase transaction, else 0x00
              ‖ creation_mtp )            u32 little-endian
```

The preimage is `58 + len(scriptPubKey)` bytes.

- **`creation_mtp`** is the median-time-past of the creating block's **parent**: the median of the 11 timestamps
  before the creating block, or of all of them when fewer than 11 exist. BIP68 time locks measure from it.
  Median means sort ascending and take element `n / 2`, as Core's `GetMedianTimePast` does.
- A plain SHA-256, not double SHA-256 and not a BIP340 tagged hash.
- In `verify_input.cpp`, `tx_out_leaves` takes this value as `creation_mtp`.

## 3. Which outputs are leaves

An output of a block at height h becomes a leaf if and only if all of these hold:

1. **It is spendable:** the scriptPubKey does not start with `OP_RETURN` (0x6a) and is at most 10,000 bytes.
   This is Core's `CScript::IsUnspendable`.
2. **It is not one of Core's BIP30-unspendable coinbases.** On mainnet, the coinbase outputs of blocks 91,722
   (`…4dd08e`) and 91,812 (`…306f2f`) are skipped, matched by height *and* block hash. Their outpoints enter the
   set when blocks 91,880 and 91,842 recreate them. This is the result of Core's overwrite, with no deletion.
3. **It is not spent in the same block.**
4. **It is still unspent at H.** The proof learns this from the hints file and enforces it through the ECMH
   total.

## 4. Order

Leaves are added in creation order:
- blocks in height order;
- within a block, transactions in block order (coinbase first);
- within a transaction, outputs in `vout` order.

Skipped outputs take no position.

## 5. Forest

```
parent(L, R) = SHA-256( 0x01 ‖ L ‖ R )                      65-byte preimage
```

- **`roots[k]`** is the root of the perfect tree of 2^k leaves, or none.
- **Adding a leaf `x`:** set `k = 0`, `node = x`. While `roots[k]` exists:
  - `node = parent(roots[k], node)`;
  - clear `roots[k]`;
  - `k += 1`.

  Then set `roots[k] = node` and `num_leaves += 1`. The older subtree is always the left child.
- After `n` leaves, `roots[k]` exists exactly when bit k of `n` is set.
- **Normalisation:** trailing empty entries are dropped. Two states are equal when their normalised root lists
  and their `num_leaves` are equal.
- **Serialisation:** in the committed `ChainState`, `utxo_roots` is a list of optional 32-byte values indexed
  by k, and `utxo_leaves` is a u64.

The tags 0x00 and 0x01 separate the leaf domain from the interior domain. Without them, a leaf whose script
is 7 bytes would have a 64-byte preimage (57 + 7), the same length as an interior node's (32 + 32).

## 6. Test vector and how it is checked

Mainnet, H = 200, hints file `h200.hints` (hint-hash `e33c66b3…`): 204 leaves.

```
roots[2] = ac7ca14570ba4972538dfe1587a44d87fc1e7a6d2f24f9ffef0a113afa5c189d
roots[3] = 7ced478f2fdebea9ce3293a479b092618073fe5c89045085ec312b7854ea9a9a
roots[6] = 2f34b66ed7d65fc1ba5c22ddcfdb0d8cc1838d33e8295f64a1429e7461aec8a9
roots[7] = 339752865cc905092ec186b4dae9902e4951bd182855b61d80003867f4cbb452
(roots[0], [1], [4], [5] empty)
```

The two implementations check each other.
- `utreexo_ref.py` computes the vector from the block files alone, following every spend, with no hints file. It
  computes each block's creation MTP from the header timestamps (genesis's from its hash), requires every block to
  link to the previous one by hash, and cross-checks each prevout's height, coinbase flag and creation MTP, and the
  files' precomputed timestamp windows, against what it derived.
- The guest's sweep (`swiftsync-sweep 200` with `HAZYNC_SWEEP_SAVE`) must commit the same state:
  `utreexo_ref.py <dir> 200 state_200.json` prints `REF_MATCH`.

## 7. Differences from utreexod / Floresta

| | libzync | utreexod / Floresta (rustreexo) |
|---|---|---|
| parent hash | SHA-256(0x01 ‖ L ‖ R) | SHA-512/256(L ‖ R) |
| leaf | §2, includes creation MTP | tagged SHA-512/256 of block hash, outpoint, header code, amount, script |
| deletions | none: the forest holds the survivors to H | every spend deletes, so a sibling moves up |
| roots depend on | the UTXO set at H and creation order | the whole add/delete history since genesis |

The last row is why converting is not a re-hash. Matching utreexod would mean proving every spend's deletion.
Measured: 14,530 cycles per SHA-512/256 parent in the guest, against 929 for ours. That puts the total at an
ESTIMATED ~6× a current block's cycles (`experimental/libzync-bench/README.md` §7).

A node holding the UTXO set at H can rebuild this forest and compare it with the proof.
`scripts/libzync/utxo_snapshot_check.py <dumptxoutset file> --bitcoin-cli "<command>" [--state state.json]` does
it from a standard Core v28 `dumptxoutset` file: the snapshot gives the coins, and the user's own node gives what
the snapshot lacks (each coin's creation MTP, from the parent header's `mediantime`, and its creation order, from
the block's transaction list). `scripts/libzync/test_utxo_snapshot.py` checks it end to end without a node, by
writing the UTXO set from the block files in Core's snapshot encoding and answering the RPC calls from the same
files: it matches the guest's committed forest at signet 2,600 (293 coins) and mainnet 200 (204 coins, which
exercises the uncompressed-key script compression), and a one-satoshi change to one coin is a mismatch. It has not
yet been run on a snapshot written by Core itself.
