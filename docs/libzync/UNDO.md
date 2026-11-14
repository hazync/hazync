# libzync: the per-block undo-data commitment

Status: in the v0.26.0 images (format U0, issue #729). Normative sources: `prover/methods/guest/src/handover_pin.rs`
(`undo_record_head`, `block_leaf_undo_preimage`, `PRE_UNDO_IMAGES`, `leaf_has_undo`) and `agg_lite.rs` (the chunks and
the aggregate). `scripts/libzync/undo_ref.py` and `scripts/libzync/block_forest_ref.py` are independent
implementations written from this document; `libzync/src/undo.rs` is the host and verifier side.

## 1. What it is

Each block's proof already checks its spends: every spent coin must be in the proven coin log and unspent, or created
earlier in the same block. The guest also holds every spent output, because it checks the scripts. Since v0.26.0 the
proof also **commits** that undo data: one hash per block over every spent coin, in the block's leaf of the block
forest (HANDOVER.md §3). A tip proof then commits the undo hash of every block it proved.

Consumers: a SwiftSync hints file (#722) needs each block's undo hash; split designs (header validation, per-block undo
commitments, UTXO updates, full validation) link their parts through it. The `ChainState` layout is unchanged.

## 2. The format (U0)

For each input of each non-coinbase transaction of the block, in block order (transactions in block order, inputs in
index order):

```
record_i   = code u32 LE ‖ amount u64 LE ‖ len(scriptPubKey) u32 LE ‖ scriptPubKey
code       = creation height * 2 + coinbase           (coinbase: the spent coin came from a coinbase; 0 or 1)
d_i        = SHA-256(record_i)
undo_hash  = SHA-256(d_1 ‖ d_2 ‖ … ‖ d_n)              (SHA-256 of the empty string for a block with no inputs)
```

- The fields are exactly a Core `Coin` (`nHeight`, `fCoinBase`, `out.nValue`, `out.scriptPubKey`) and the order is
  Core's `CBlockUndo` (one `CTxUndo` per non-coinbase transaction, one `Coin` per input), so a node computes a block's
  undo hash from its own `rev*.dat` without the chain proof. Scripts are uncompressed (Core's undo files compress them).
- A coin created and spent in the same block is included like any other input.
- Independent of how the prover splits the block into chunks: each chunk commits its inputs' digests `d_i`, and the
  aggregate hashes them in block order.
- `coinbase` is the coinbase flag only. The block files' `coin_is_coinbase` also carries the BIP30-tracked bit (bit 1),
  which is not part of the record.

## 3. The block leaf that commits it

```
leaf_u(hash, height, state, undo) = SHA-256( 0x05 ‖ block hash, internal order ‖ height u32 LE ‖ state ‖ undo_hash )   101 bytes
leaf(hash, height, state)         = SHA-256( 0x02 ‖ block hash, internal order ‖ height u32 LE ‖ state )                69 bytes
```

`state` is the coin-state digest after the block (HANDOVER.md §3). Tag 0x05 is new; the others in use are 0x00 coin
leaf, 0x01 interior node, 0x02 block leaf, 0x03 lineage step, 0x04 coin state, 0x10 / 0x11 the BIP30 SMT. The two leaf
formats differ in tag and length, so no opening in one format can match a leaf of the other.

**Which leaf a block has** is fixed by the image that proved it:

- **Block 0** (the genesis anchor, proved by no image) always has the 0x02 leaf. The per-network genesis anchor
  (`852c5c7e…` mainnet, `3b67ead0…` signet) is therefore unchanged.
- A block proved by an image in `PRE_UNDO_IMAGES` has the 0x02 leaf: v0.23.0-v0.25.0's mainnet `8888d59c` and signet
  `d4210dae`. Their receipts still verify (the verifier keeps them by id), and no block they proved carries an undo hash.
- Every other block has the 0x05 leaf. The v0.26.0 images have an empty lineage, so every block from 1 on.

The proving image of block `h` comes from the lineage: the first handover entry with `h < first_height`, else the
tip proof's own image (`handover_pin::image_at`). A future image that continues a v0.26.0 chain by a handover keeps
0x05 for the blocks below its `first_height` and gives the 0x05 leaf to its own blocks.

## 4. Who computes and who checks

| | where |
|---|---|
| per-input digest | the chunk (`ChunkCtx::record`), from the same prevout, height and coinbase flag as the spent coin's leaf, so the coin log check (or the block's created set, for an in-block spend) binds it |
| undo hash, leaf | the aggregate (`validate_block_lite`, `aggregate()`), logged as `undo: block <h> inputs <n> hash <hex>` |
| native copy | the host: `libzync-prep` (`prep::undo_of_block_json`) for every prepared state, and `libzync-prove-range`, which refuses a block whose guest undo hash is not its own and prints `undo: block <h> … (guest == host)` |
| history | `history.bin` records carry the undo hash (bit 31 of `n_appends`, libzync/src/history.rs), so `zync-path` / `zync-serve` rebuild the 0x05 leaves |
| verifiers | `swiftsync-verify`: `Report::undo_from` (the first height whose leaf has it, from the verified image and the lineage its proof commits), `block_in_chain(cs, undo_from, h, hash, state, undo, path)`, and the `block_undo` / `undo` fields of coin and transaction facts; the C ABI and WASM take the same facts |

## 5. What a consumer reads

A tip proof commits the undo hash of every block it proved, through the block forest. To read block `h`'s:

1. verify the tip proof (`swiftsync-verify`; it prints which blocks' leaves carry the commitment);
2. compute the undo hash yourself from block `h` and its spent coins (Core's undo data, or the block and the coins it
   spends), or take it from anyone;
3. open block `h`'s leaf with it: a block-forest path, the block hash, the coin state after it and the undo hash
   (`block_in_chain`, or a fact's `block_undo` / `undo` field). It opens exactly when that is the undo hash the proof
   committed.

The proof does not publish the undo data itself, only the hash: the data is the spent coins, which the block and the
chain already determine.

## 6. Blocks proved before the commitment (back-filling, not done)

Blocks proved by `8888d59c` have no undo hash in their leaves. It can be derived later by a zyncAPP (#722, #729): a
coin-log leaf commits the whole output (value, script, height, coinbase flag, creation MTP; UTREEXO_FORMAT.md §2) and
a spent leaf records its spender, so one tip proof, the block bytes and one coin-log path per input give a block's undo
data. The board re-proves from genesis in v0.26.0, so its new chain carries the commitment from block 1.

## 7. Measured (2026-11-14, #729)

Execute only (`agg-exec-fake`, `HAZYNC_CHUNKS=4`, no dev mode), the prototype `exp/undo-commit` (this format, the digests
committed by the chunks and hashed by the aggregate) against the `1ff5a129` image (an earlier v0.26.0 build, never released), 11 real mainnet blocks: **+0.35% to
+2.46% of a block's executed cycles** (+0.00% to +2.68% padded), most at 962,000 (8,006 inputs). The guest's hashes
equal `undo_ref.py` on 11 of 11. This implementation differs from the prototype in parsing each run's prevouts once
for both the trie and the undo records (the prototype parsed them twice) and in the leaf (the prototype folded the
hash into the 0x02 leaf's state, for measurement only).

CI (`scripts/libzync/ci/`): `blocks.sh` step 3 compares the guest's undo hash with `undo_ref.py` on the ten edge blocks
and 962,000, with a control; `chain.sh` step 1 on every block of mainnet 1..200 with inputs, and the block forest with
`block_forest_ref.py --undo-from 1`, with a control (the forest without undo hashes must not match); `signet.sh` the
signet forest the same way; the verifier's unit tests both leaf formats across a handover height.

## 8. The switch: carrying a running prep over (no restart from genesis)

A prep run made before v0.26.0 (`libzync-prep`, `libzync-board-prep`) has the right coin log, trie, BIP30 tree and
every ChainState field except one: `block_roots`, whose leaves lack the undo hash. `libzync-reforest` (the new host)
rebuilds the forest over the run's own states and rewrites it, without touching any store, so prep continues instead
of starting again.

```
libzync-reforest <out dir> <S> --bitcoin-cli "<cmd>" [--all] [--jobs N] [--bundles <leaf bundle dir>]
libzync-undo-hashes <lo> <hi> --bitcoin-cli "<cmd>" [--jobs N]      # each block's undo hash, from a node
```

1. **Stop prep at a save point** (`systemctl stop libzync-prep`). Its runner always resumes from the newest COMPLETE
   save point, `state_<S>.json` with `.coinlog` and `.trie` (every 1,000 blocks); take that S (`libzync-prep-run.sh`'s
   rule). Re-forest holds the out dir's `LOCK`, so it refuses to run while prep does.
2. **Re-forest** with the NEW host, HAZYNC_COINLOG_DIR / HAZYNC_TRIE_DIR unset:
   `host libzync-reforest $LZ_ROOT/out S --bitcoin-cli "bitcoin-cli -datadir=…" --all --bundles $LZ_ROOT/leaves`.
   - Phase 1 asks the node (`getblock <hash> 3`, unpruned, Core ≥ 25) for each block's spent coins and writes
     `out/reforest/undo.tsv` as it goes; a rerun resumes after its last line.
   - Phase 2 walks `state_1..S.json`: each must be the node's block at its height, and its `block_roots` the old forest
     over the states before it (or, on a rerun, the new one); anything else stops the command. It writes the new
     forest, stamped as the new prep stamps (`--all`: every state, else only `state_S.json`), each file replaced by a
     rename; `--bundles` gives each leaf bundle 2..S+1 its re-forested `prev` and names any above S+1 (remove them).
3. **Swap the binary** (`$LZ_ROOT/bin/host_lz` = the v0.26.0 host) and **start prep**: the runner continues from S
   with `HAZYNC_PREP_CONTINUE=1`, re-stamping on read, and every block from S+1 on gets the 0x05 leaf directly.
4. **Board-prep** cuts its window of bundles again after the board resets (or is re-forested the same way).

**Tested** (2026-11-14, hazync-coord, CPU, the node there; `accept.sh` of the PR):
- prep 1..2,000 by the released v0.25.0 host (`8888d59c`), re-forested by the new host: **2,000 of 2,000
  `state_<h>.json` byte-identical** to prep 1..2,000 by the new host, and the three leaf bundles cut on the way (171,
  1,500, 2,000) byte-identical to the new prep's. The coin-log and trie snapshots are untouched (same sha256).
- a rerun rewrites nothing (2,000 already in the new format); the control (block 170's undo hash replaced by SHA-256
  of nothing) leaves 1..169 identical and makes 170..2,000 differ (1,831 of 1,831);
- refused: a held `LOCK`, `HAZYNC_COINLOG_DIR` set, an S that is not a save point;
- the node path's undo hashes equal `undo_ref.py` on the block files of 130,000, 140,000, 741,000, 962,000, 965,500.

**Phase 1 rate** (`libzync-undo-hashes`, 8 at a time, the node on hazync-coord; MEASURED on these samples only):
946 blocks/s at 100,000-100,399; 54 at 300,000-300,199; 14.1 at 500,000-500,099; 14.0 at 590,000-590,099; 8.1 at
970,000-970,049. Phase 2 (reading and writing the states) ran 2,000 blocks in under a second of the 3 s total.
ESTIMATE from those samples, not measured: a pass to about 590,000 is a few hours, dominated by 300,000 onwards
(roughly 290,000 blocks at 14-50 blocks/s); more `--jobs` may help if the node's RPC has threads to spare.
