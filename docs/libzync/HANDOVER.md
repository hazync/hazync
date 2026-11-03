# libzync: image handover and the block forest

Status: merged into `libzync`. Normative sources are `prover/methods/guest/src/handover_pin.rs`, `src/lineage/` and
`aggregate()` in `prover/methods/guest/src/main.rs`. `scripts/libzync/block_forest_ref.py` is an independent
implementation written from this document.

Both features change `ChainState`, so they go in before the image freeze.

## 1. Why

**Handover.** `self_id` pins one image for a whole chain. Without a way to continue, any new image means proving the
chain again from genesis. A soft fork would cost a full re-proof.

**Block forest.** `ChainState` says nothing about history except `tip_hash`, and its coin forest holds only coins
unspent at H. A proof could not show that an old block, an `OP_RETURN` commitment or a spent transaction was ever in
the chain.

## 2. New fields

Added to `ChainState` after `profile`, in this order:

| field | meaning |
|---|---|
| `block_roots` | the roots of the block forest, `Vec<Option<[u8; 32]>>`, as `utxo_roots`, with trailing empties removed |
| `block_leaves` | number of leaves; always `height + 1` |
| `lineage` | 32 zero bytes for a chain that has been one image since genesis; otherwise the digest of §5 |

### Layout version and appending fields (2026-11-02)

`ChainState` also gained `version` (right after `kind`; `1`, and `2` since block leaves carry the coin state) and `utxo_value` (after `utxo_leaves`: the
satoshis held by the coin forest). From the image freeze on, the layout only grows at the end:

- A field is never reordered, removed or changed in type. A new one is appended, and `version` goes up by one.
- risc0's serde reads fields in order and ignores trailing words, so a reader built for version n (a verifier, a
  zyncAPP) decodes the prefix it knows from any later journal. A zyncAPP therefore keeps working across a handover
  that adds fields, and a handover is the window in which a zyncAPP that needs a new core fact gets it.
- The reverse does not hold: an image of version n+1 cannot decode a version-n journal as its own struct (the new
  fields are missing). At a handover that adds fields, step 2 below decodes the predecessor with the version-n
  layout and fills the new fields from it explicitly.

## 3. Block forest

An append-only Utreexo forest with one leaf per block, genesis first, so leaf `i` is the block at height `i`.

```
leaf(hash, height, state) = SHA-256( 0x02 ‖ block hash, internal byte order ‖ height u32 LE ‖ state )   69 bytes
state                     = SHA-256( 0x04 ‖ utxo_leaves u64 LE ‖ utxo_value u64 LE ‖ present utxo_roots,
                                     the highest level first )
parent(L, R)              = SHA-256( 0x01 ‖ L ‖ R )                                     as UTREEXO_FORMAT.md §5
```

- `state` is the coin commitment **after** the block: the `utxo_roots`, `utxo_leaves` and `utxo_value` the block's
  own proof commits. Since version 2 (2026-11-02). The leaf count fixes which roots are present.
- The genesis state holds one leaf, for block 0: `block_roots = [Some(leaf(genesis hash, 0, SHA-256(0x04 ‖ 0 ‖ 0)))]`,
  `block_leaves = 1`.
- Every aggregate requires `prev.block_leaves == prev.height + 1`, appends `leaf(this block, height, state)` and
  commits the result. The block hash is the one the guest computed from the header it validated.
- **Why the state is in the leaf.** A proof at the tip then commits the coin state at every earlier height. With the
  coin log (COINLOG.md), whose commitment is the full coin history at every block, a zyncAPP shows "coin C was
  unspent (or spent, and by whom) at height h" from one tip proof: a block-forest path to h's leaf, the roots that
  hash to its `state`, and a coin path against them. Under design B the commitment is the survivor forest, so the
  state means something only in a proof closed at H.
- Trees run largest first: the forest of `n` leaves has a tree of height `h` for each set bit `h` of `n`, the highest
  bit holding the earliest leaves.
- Height is in the leaf so a path cannot be moved to another position. Tag 0x02 separates it from coin leaves (0x00),
  interior nodes (0x01), lineage steps (0x03), coin-state digests (0x04) and the BIP30 SMT's leaves (0x10) and nodes
  (0x11). The coin log uses its own IVs, not tags.

**Checking a block.** Given a verified state, a height, a block hash, its coin state and a path of sibling hashes, locate the leaf's
tree, require the path length to equal that tree's height, fold the leaf up the path and compare with `block_roots[h]`.
This is `block_in_chain` in `swiftsync-verify`. It needs no node and no proving: the path comes from anyone, since the
root in the proof decides. From the block hash, a Merkle branch against the header reaches any transaction.

**What it does not say.** A leaf proves a block is in this chain. It does not say an output is unspent, which is the coin
forest's job. A proof at height H also says nothing after H.

## 4. Handover

### When it may be used

| Reason for the new image | Handover? |
|---|---|
| A consensus rule that starts at height H, such as a soft fork | **Yes** |
| A cheaper prover, with the same statement | **Yes** |
| **A soundness bug in the old image** | **NO.** The old proof may attest something false, and continuing inherits that. Re-prove from before the bug could matter, normally genesis. |

A handover takes the old image's claims on trust. The new image re-checks nothing before `first_height`, so it is
only as sound as every image in its lineage.

A soft fork's image must ship **before** the rule activates. An old image keeps accepting blocks under the old rules,
so a proof from it after activation states the weaker rules.

### How it works

An image's source lists what it continues, in `prover/methods/guest/src/lineage/<variant>.rs`, one file per image
variant (`mainnet`, `signet`, `regtest`, each also `_coinlog`; an assumevalid build shares its network's file):

```rust
pub const LINEAGE: &[crate::handover_pin::Handover] = &[crate::handover_pin::Handover { image: OLD_ID, first_height: 974_000 }];
```

Only the variant's own file is compiled into it, so pinning one variant's predecessor does not change another
variant's image id (review 11-03: one shared list did). The list is compiled in, so the new image's id commits to it. It is source, not an environment variable or a flag, so a
rebuild gets the same list. `first_height` is the first block the new image proves, and the last entry is the direct
predecessor.

The aggregate's `is_base` takes a third value. `2` = handover:

1. `env::verify(pinned image, prev_journal)`. The id comes from the source, never from input, so `self_id` cannot widen it.
2. `prev.self_id` must be the pinned image.
3. `prev.profile` must equal this image's. A chain cannot gain a claim at a handover: an assumevalid predecessor cannot
   become a scripts-checked chain, and networks cannot mix.
4. The block's height must equal `first_height`.
5. `prev.lineage` must be the digest of the list's earlier entries, so the list cannot misstate history.

Everything else in the state is carried as in a recursive step: coins, work, SwiftSync totals, the BIP30 tree and
`anchor_id`. The step commits this image's `self_id`, and every later step is an ordinary recursive one. An image with
an empty list refuses `is_base = 2`.

⛔ **A new image that changes `ChainState` must decode the predecessor's format at step 2.** The guest marks the line.

### Genesis anchor

The genesis state now has `self_id` = 32 zero bytes. It used to hold the image id, which would make `anchor_id` differ
for each image and fail every handed-over chain at the genesis check. The base step never checks that field, and the
final `self_id` check is unchanged, so nothing about which image proved the chain is lost. `anchor_id` is now one value
per network for all images.

The genesis journal is in the layout of the network's FIRST image (ChainState version 2, kind `0xC4A1_0012`), and
`anchor_id` is carried unchanged through every handover. A later image that appends fields must therefore not recompute
the genesis anchor in its own layout: verifiers PIN it per network (`swiftsync-verify` `Image::genesis_anchor_hex`,
host `pinned_genesis_anchor`): mainnet `852c5c7e…390167`, signet `3b67ead0…938827`.

A handover sends the predecessor receipt's own journal bytes to the guest, never a re-encoding of the decoded state,
and needs every block file before it (the BIP30 tree is replayed, never seeded). ⚠ The host replays the tree under
ITS OWN image's update rule. A new image that changes that rule (as 11-03's tracked-only counting did) cannot take a
handover from an image with the old rule until the host replays each range under its own image's rule.

## 5. Lineage digest

```
L_0     = 32 zero bytes
L_(i+1) = SHA-256( 0x03 ‖ L_i ‖ image, as 8 u32 LE ‖ first_height u32 LE )     69-byte preimage
```

`image, as 8 u32 LE` is the image id's 32 bytes exactly as risc0 prints the id in hex.

Each handover sets `lineage` to the digest of the image's whole pinned list. No other step changes it.

A proof by an image carries one of two values: zero, because that image also proved the chain from genesis, or the
digest of its list. The verifier refuses any other value, and prints which one it saw.

## 6. What a verifier checks

Added to the steps in SWIFTSYNC_PROOFS.md §5:

- `block_leaves == height + 1`, and `block_roots` has a root exactly at the set bits of `block_leaves`.
- `lineage` is zero, or the digest of the image's stated list. `swiftsync-verify --handover <image>:<height>` states an
  entry, oldest first.
- The image id pinned is still the **newest** one. It commits to its whole lineage, so a verifier needs nothing else to
  accept the chain. Deciding to trust that lineage is a decision about every image in it.

## 7. Limits

- Proofs made before this format do not decode. The old real receipts in `swiftsync-verify/vectors` are retired until
  a real proof exists on this format.
- An anchored demonstration run has no real block history. The host fills placeholder roots of the right size and warns.
  Such a proof can only ever be DEMO, and its forest does not commit to real history.
- An image can only name a predecessor whose final id is known. Publishing an id means freezing that image's source.

## 8. Tested and measured (2026-11-02)

**Regtest handover, `experimental/libzync-bench/handover/run.sh`, dev mode** (rules execute, nothing proved; output in
`run_regtest.out`). Image O (`e9b6ca6e…`, empty lineage) proves blocks 1–100 from genesis; image N (`e7f2e7bc…`, O
pinned at 101) continues 101–110. 15/15 cases as expected:
- every tampered handover is refused by its own guest check: `self_id` ("previous proof is not the predecessor's"),
  `profile`, `lineage`, block forest ("block forest 100 leaves at height 100"), `kind`, height ("height 100 != pinned
  101"); a receipt claiming the wrong image, and no receipt at all, both stop at the guest's `env::verify` of O
  ("no receipt found to resolve assumption");
- O refuses `is_base = 2` ("this image has no predecessor"); the host refuses a receipt at the wrong height;
- the control: block 101 is a handover step, 102–110 are ordinary recursion; `anchor_id` is unchanged across the
  handover; `lineage` equals `block_forest_ref.py`'s digest; the block forest at 100 and at 110 equals the reference
  over the real block hashes, and block 7 checks against both.

**Cost on mainnet block 962,000** (`ab_962000.sh`, execute mode, 16 chunks, logs in `ab_962000/`). Baseline built from
`bf8e607` and the handover branch built in the same worktree path:

| | bf8e607 | handover | difference |
|---|---|---|---|
| chunks, sum | 361,473,807 | 361,474,446 | +639 |
| aggregate | 64,535,363 | 64,609,027 | **+73,664 (+0.114%)** |
| whole block | 426,009,170 | 426,083,473 | **+74,303 (+0.017%)** |

- The chunk code is unchanged; its 4–135 cycles per chunk come with the different ELF.
- At 962,000 the append merges **nothing** (962,000 leaves, an even number), so this is a forest step at its cheapest:
  one leaf hash plus the larger state to decode and commit (12 block roots after the block). A block whose append
  merges pays one parent hash per merge, one per block on average and about 20 at most. Not measured separately.

**On libzync `6a339bd` (rebased, `experimental/libzync-bench/handover/merged_6a339bd.out`).** The merged guest was
re-tested from scratch: handover suite 15/15; rig on Core 28.0 95 cases, 94 agree, 1 deferred (as libzync); gate
16 chunk journals identical to round 8, exception blocks 170,060 and 692,261 valid; SwiftSync regression mainnet
1..200 and signet 1..2600 BALANCED with the reference forest, flip refused, BIP325 check OK; the sweep states' block
forests equal `block_forest_ref.py` over the real block hashes.

The merged aggregate on 962,000 is 64,609,137 cycles against libzync's 63,021,004 (same build path), **+1,588,133**.
That is **not** the forest. Profiles of the four builds (bf8e607, handover on bf8e607, libzync, merged) show it is
rustc's inlining of agg-lite: libzync's new reject-reason logging stopped `validate_block_lite` being inlined into
the aggregate, which happens to run ~1.08 M cheaper (plus ~0.43 M in `swiftsync`); the larger aggregate of the
handover build tips it back to inlined. Merged against handover-on-bf8e607 differs by 110 cycles. The forest's own
frames (state serde, `Stump`, journal write) are ~70–95 k, as measured above.

⏰ **Lever, not applied:** `#[inline(never)]` on `validate_block_lite` (and the `swiftsync` step) should recover the
~1.5 M. It is a guest change, so every result above would need re-running on the new image.
