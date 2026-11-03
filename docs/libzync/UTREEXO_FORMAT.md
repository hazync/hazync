# libzync UTXO accumulator: format specification

Status: describes the code at branch `libzync`. Normative sources are `prover/methods/guest/verify_input.cpp`
(`hz_leaf`, `coin_leaf`, `tx_out_leaves`), `prover/methods/guest/src/utreexo.rs` (`parent`, `Stump::add`) and
`prover/methods/guest/src/agg_lite.rs` (which outputs enter the forest). An independent implementation written
from this document is `scripts/libzync/utreexo_ref.py`; §6 says how the two are checked against each other.

⚠ This is **not** the utreexod / Floresta format (§7).

## 1. What it commits to

At SwiftSync stop height H a libzync chain proof commits `ChainState.utxo_roots` and `ChainState.utxo_leaves`:
an append-only Utreexo forest whose leaves are **exactly the coins unspent at H**, one leaf per coin, by
height and, within a block, by (txid, vout) (§4). One mainnet exception: for H from 91,722 to 91,879, Core's UTXO set still holds the coinbase
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
              ‖ flags                     1 byte: bit 0 created by a coinbase, bit 1 tracked by the BIP30 tree (22 live coins on mainnet)
              ‖ creation_mtp )            u32 little-endian
```

The preimage is `58 + len(scriptPubKey)` bytes.

- **`creation_mtp`** is the median-time-past of the creating block's **parent**: the median of the 11 timestamps
  before the creating block, or of all of them when fewer than 11 exist. BIP68 time locks measure from it.
  Median means sort ascending and take element `n / 2`, as Core's `GetMedianTimePast` does.
- A plain SHA-256, not double SHA-256 and not a BIP340 tagged hash.
- In `verify_input.cpp`, `tx_out_leaves` takes this value as `creation_mtp`.

### 2a. Flag bit 1: tracked by the BIP30 tree

Bits 2-7 of `flags` are zero. Bit 1 is set on every output of a **tracked** transaction, the only transactions that
can ever repeat a txid (coinbase-smt/src/bip30.rs):

1. **A coinbase is tracked** if either
   - it is mined at height h below the network's BIP34 height (mainnet 227,931; signet 1; regtest 110, as the test
     chain runs it with `-testactivationheight=bip34@110`) and **indicates** a height Y with h < Y < 100,000,000; or
   - its txid repeats a tracked coinbase whose outputs are all spent (the BIP30 tree holds it as REPEATABLE).
2. **A coinbase indicates Y** when its first input's scriptSig begins with exactly the bytes of Core's `CScript() << Y`:
   OP_1..OP_16 (0x51..0x60) for 1..16, otherwise a push of 1-5 bytes holding Y little-endian in minimal CScriptNum form
   (a 0x00 byte appended when the top bit is set). Any other prefix indicates nothing (Y = 0).
3. **A non-coinbase transaction is tracked** if it has at least one input and every input spends a coin with bit 1.

On mainnet that is 173 transactions to height 940,000; signet (BIP34 from height 1) has none.

**The BIP30 tree counts tracked outputs only** (libzync images, since 11-03). Every coinbase txid is checked and
inserted as before, but a spend updates the tree only when the spent coin has bit 1. A repeat of an untracked coinbase
is refused whether its slot reads live or SPENT, so its count never decided anything, and a double spend is refused
by the coin commitment. Block 185,956 (190 coinbase coins spent): aggregate 294 M -> 8.3 M cycles. The guest learns bit 1
from the BIP30 tree's proofs and the chunk records (agg_lite.rs); `scripts/libzync/utreexo_ref.py` (`Tracked`)
implements this text.

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

Leaves are added block by block, in height order. Within a block, the block's surviving coins are sorted by
**(txid, vout)**: the txid compared as 32 bytes in internal byte order (as in the outpoint), then `vout` as a number.

This is NOT quite Bitcoin Core's coin-key order: Core's chainstate key is `'C' ‖ txid ‖ VARINT(vout)`, compared as
bytes, and its VARINT does not sort numerically past 16,511 (16,512 encodes as `808000`, before 16,511's `ff7f`).
`dumptxoutset` writes in Core's order, so a reader must **re-sort each block's coins by (txid, vout as a number)**;
the two orders differ only when one transaction keeps more than 16,512 unspent outputs. A Core snapshot carries
each coin's height, so it gives the forest order with no block data: a node that has only synced headers can check a
snapshot against a proof (`utxo_snapshot_check.py` re-sorts, §7).

The hints file's bits stay in block order (transactions in block order, outputs in `vout` order, skipped outputs
taking no position); only the forest is sorted. Within one block no two transactions share a txid (the guest refuses
a block where they do), so the order is total.

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

Mainnet, H = 200, hints file `h200.hints` (hint-hash `e33c66b3…`): 204 leaves, `utxo_value` 1,000,000,000,000 sat.

```
roots[2] = ac7ca14570ba4972538dfe1587a44d87fc1e7a6d2f24f9ffef0a113afa5c189d
roots[3] = 7ced478f2fdebea9ce3293a479b092618073fe5c89045085ec312b7854ea9a9a
roots[6] = 94ed3b428abfc24c6261ac76dfab9cad07ca35bc28a39bd9109a5288a6886e51
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
| roots depend on | the UTXO set at H (order: height, then txid, vout) | the whole add/delete history since genesis |

The last row is why converting is not a re-hash. Matching utreexod would mean proving every spend's deletion.
Measured: 14,530 cycles per SHA-512/256 parent in the guest, against 929 for ours. That puts the total at an
ESTIMATED ~6× a current block's cycles (`experimental/libzync-bench/README.md` §7).

A node holding the UTXO set at H can rebuild this forest and compare it with the proof.
`scripts/libzync/utxo_snapshot_check.py <dumptxoutset file> --bitcoin-cli "<command>" [--state state.json]` does
it from a standard Core v28 `dumptxoutset` file: the snapshot gives the coins and, with their heights, the order
(§4); the user's own node gives the one thing the snapshot lacks, each coin's creation MTP, from the parent
header's `mediantime`. Headers are enough: no block data is read. `scripts/libzync/test_utxo_snapshot.py` checks it end to end without a node, by
writing the UTXO set from the block files in Core's snapshot encoding and answering the RPC calls from the same
files: it matches the guest's committed forest at signet 2,600 (293 coins) and mainnet 200 (204 coins, which
exercises the uncompressed-key script compression), and a one-satoshi change to one coin is a mismatch. It has not
yet been run on a snapshot written by Core itself.
