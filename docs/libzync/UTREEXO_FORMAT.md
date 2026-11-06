# libzync coin leaf and forest format

The byte-exact coin leaf (§2), which outputs become leaves (§3) and the forest hashing (§5) used by the coin log
(`COINLOG.md`) and, for `parent`, the block forest (`HANDOVER.md` §3). Normative sources:
`prover/methods/guest/verify_input.cpp` (`hz_leaf`, `coin_leaf`, `tx_out_leaves`), `prover/methods/guest/src/utreexo.rs`
(`parent`, `Stump::add`) and `prover/methods/guest/src/agg_lite.rs` (which outputs enter the log).

The file name is historical: it began as the format of design B's Utreexo-style forest of the coins unspent at a
hints file's height, retired in E2 (2026-11-05). Section numbers are kept because the frozen guest's comments cite
§2 and §2a. Design B's §1 (what that forest committed to), §4 (its order), §6 (its test vector) and §7 (differences
from utreexod / Floresta) are in the full original, `experimental/libzync-bench/retired-docs/UTREEXO_FORMAT.md`.

⚠ This is **not** the utreexod / Floresta format: other hashes and leaves, and no deletions.

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

(Design B added a rule 4, "still unspent at H", from a hints file; the coin log appends every output that
passes rules 1-3 and records its later spend in place: `COINLOG.md`.)

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
  by k, and `utxo_leaves` is a u64 (the coin log's forest, COINLOG.md; the block forest, HANDOVER.md §3, uses the
  same `parent`).

The tags 0x00 and 0x01 separate the leaf domain from the interior domain. Without them, a leaf whose script
is 7 bytes would have a 64-byte preimage (57 + 7), the same length as an interior node's (32 + 32).
