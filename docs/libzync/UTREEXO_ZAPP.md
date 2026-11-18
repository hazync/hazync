# zyncAPP: proven Utreexo roots for assumeutreexo (design and prototype, #723)

Status (15.11.2026): **prototype** on `feat/zapp-utreexo`, CPU only. Executed (no dev mode) from genesis to H = 1,999,
10,000 and 100,000; the roots are byte-identical to Floresta's own code at every H tested; real CPU proofs at H = 2 and 3
(§8). Not reviewed, not reproducible-built, no published id. Utreexo stays out of the Hazync core; this zAPP sits on
top of it, like every zAPP.

Utreexo is Tadge Dryja's accumulator; Floresta (getfloresta/Floresta, formerly vinteumorg) is Davidson Souza's node
and rustreexo its library. This zAPP adds nothing to either. It is an additive product on a Hazync chain proof:
*"these are the roots and leaf count Floresta holds after block H"*, so a Floresta node can start from proven roots
instead of the `AssumeUtreexoValue` compiled into it.

## 1. What it proves and commits

Statement (`UtreexoStatement`, kind 'ZU' version 1, `zapps/utreexo/src/lib.rs`): *after block `height` with hash
`block_hash` of the chain that the core receipt `core_id` proves valid from genesis (scripts checked), Floresta's
accumulator has `leaves` leaves and these `roots`.*

```
UtreexoStatement {
  kind        u32        'ZU' 1
  self_id     [u32; 8]   the image every run of the chain was verified under (the checker requires its own id)
  network     u32        the profile's network bits
  height      u32        H
  to, runs    u32        the last block of this run (= H in the final run), the number of runs
  state_out   [u8; 32]   SHA-256 of the walk state after `to` (runs below H only)
  done        u32        1 in the final run, which checked the core receipt
  block_hash  [u8; 32]   block H, internal byte order
  leaves      u64        Stump.leaves: every leaf ever added (Floresta's `numleaves`)
  roots       [[u8;32]]  Stump.roots in rustreexo's order (largest tree first, one per set bit of `leaves`);
                         an Empty root as 32 zero bytes
  n_live      u64        non-Empty leaves (coins live at H, plus the two BIP30 leaves once H >= 91,812)
  core binding           core_id, lineage, anchor_id, core height T (H <= T), tip_hash, cum_work, max_time, profile
}
```

The Utreexo fields are exactly Floresta's `AssumeUtreexoValue` (block_hash, height, roots, leaves):
`zapp-utreexo verify --floresta` prints that struct as Floresta's chainparams source writes it (`acchashes![…]`), and
with an Empty root an explicit `vec![…, BitcoinNodeHash::Empty, …]` instead (`acchashes!` takes hex literals only).
The CLI's `--json` and the WASM's JSON write an Empty root as the string `"empty"` (rustreexo's Display). ⚠ 32 zero
bytes here MEANS `BitcoinNodeHash::Empty`. Loaded as the hash of zeros
(`BitcoinNodeHash::from([0u8; 32])` or `from_str("00…00")` give `Some(zeros)`), it is not Empty to rustreexo, and the
next add would give wrong roots. An Empty root can occur (a small tree whose leaves were all created before H and spent
by H); a consumer must map zeros (and `"empty"`) to `Empty`.

## 2. Compatibility with Floresta (read at Floresta `8399792e`, rustreexo 0.7.0 `b73cbac1`, the version Floresta pins)

**Leaf** (floresta-chain `src/pruned_utreexo/udata.rs:332` `get_leaf_hashes`, the same as `LeafData::_get_leaf_hashes`
at `udata.rs:45`, struct at `udata.rs:24`):

```
leaf = SHA-512/256( TAG ‖ TAG                          TAG = UTREEXO_TAG_V1, 64 bytes = SHA-512("UtreexoV1") (consensus.rs:62)
                  ‖ block hash of the creating block   32 bytes, internal order
                  ‖ txid ‖ vout u32 LE                 the outpoint
                  ‖ header code u32 LE                 height·2 + coinbase
                  ‖ TxOut )                            value u64 LE ‖ CompactSize(len) ‖ scriptPubKey
```

`TAG ‖ TAG` is one whole SHA-512 block, so its state is computed once (`utreexo_roots::tag_state`); a leaf then costs
one compression for a script of up to 29 bytes (P2PKH, P2WPKH), two for P2PK, P2TR, P2WSH.

**Which outputs are leaves** (`udata.rs:383` `get_block_adds`): every output in block order (transactions in order,
outputs in vout order) except (a) `Consensus::is_unspendable` (`consensus.rs:243`: OP_RETURN first byte, or a script over
10,000 bytes) and (b) an outpoint that any input of the same block spends. The genesis coinbase is never a leaf
(Floresta starts from an empty Stump at height 0). These are the coin log's rules 1 and 3 (UTREEXO_FORMAT.md §3), in the
same order, with one exception:

**BIP30.** The coinbases of 91,722 and 91,812 ARE leaves in Utreexo (the block hash in the leaf keeps them apart from
the recreations at 91,880 / 91,842), and Floresta refuses ever to delete them (`consensus.rs:75`, `:86`
`UNSPENDABLE_BIP30_UTXO_*`; `update_acc` at `:931` via `contains_unspendable_utxo` at `:958`). The coin log skips them.
So the zAPP adds each as a live leaf at its block; it is live at every H after.

**Deletion** (`process_proof`, `udata.rs:509`): each input of a non-coinbase transaction that does not spend an output
of the same block deletes the leaf of the coin it spends. In rustreexo positions do move on deletion: the sibling of a
deleted leaf takes its parent's place (`src/util/mod.rs:57`, `src/stump/mod.rs:559`; `src/proof/mod.rs:524`
`calculate_hashes_delete`). The hashes, though, are exactly those of a fold in which the deleted leaf becomes
**Empty** and Empty is the identity of the parent rule (`parent(Empty, x) = x`, `parent(x, Empty) = x`,
`parent(Empty, Empty) = Empty`; `src/proof/mod.rs:584-589`), because "the sibling takes the parent's place" gives the
same hash as `parent(Empty, x) = x`. The add
rule treats Empty the same way (`src/stump/mod.rs:518`), and parents are `SHA-512/256(left ‖ right)` with the older
subtree on the left (`src/node_hash/mod.rs:268`).

**Consequence (the design's key fact).** The roots after H are a function of (the sequence of every leaf ever added,
the set of those deleted by H) only: the same forest folded with each deleted leaf replaced by Empty. *When* a leaf was
deleted does not matter. Floresta relies on this itself: its SwiftSync mode builds the accumulator from hints by adding
spent outputs as Empty (`udata.rs:430` `get_block_adds_from_hints`), and its test
`mainnet_additions_with_implicit_deletion_out_of_order` (floresta-wire `src/p2p_wire/stump_updater.rs:406`) expects
the roots after mainnet block 175 that a real deletion gives (`utreexo-ref` below prints the same four roots by
`update_acc` with a MemForest proof). The 14.11.2026 note "roots are order-dependent, so replay every block's adds and
dels" holds for the adds; the dels need no replay.

**How Floresta loads assumeutreexo.** `AssumeUtreexoValue { block_hash, height, roots, leaves }`
(`src/pruned_utreexo/chainparams.rs:131`), one per network in `ChainParams::get_assume_utreexo` (`:147`): mainnet
height 939,969 (`000000000000000000009d36aae180d04aeac872adb14e22f65c8b6647a8bf79`), 18 roots, 3,066,490,760 leaves;
signet 296,870, 11 roots, 126,113,034 leaves; testnet, testnet4, regtest the empty genesis state. florestad uses it
unless `--no-assume-utreexo` (bin/florestad `src/cli.rs:132`); the node config takes
`assumeutreexo_value.or(<built-in>)` (floresta-node `src/florestad.rs:426-447`), but florestad always passes
`assumeutreexo_value: None` (bin/florestad `src/main.rs:90`), so a user cannot give other roots on the command line.
When the chain selector finds the value's block in the best chain it calls `mark_chain_as_assumed(Stump { leaves,
roots }, block_hash)` (floresta-wire `src/p2p_wire/node/chain_selector_ctx.rs:644-659`), and by default backfills
(validates every block in the background up to it).

**Checked** (`prover/zyncapps/utreexo-ref`: floresta-chain at `8399792e` as a git dependency, rustreexo `=0.7.0`):
per block it calls Floresta's `proof_util::get_block_adds` and `Consensus::update_acc` with a rustreexo `MemForest`
proof for the deletions, checks my leaf data hashes to Floresta's adds, the MemForest roots equal the Stump's, and at
every requested height the Empty fold above equals the Stump; at 91,842 / 91,880 the kept leaves hash to Floresta's
two `UNSPENDABLE_BIP30_UTXO` constants.
- Floresta's published mainnet value after block 169 (`stump_updater.rs:387`, 169 leaves, 4 roots `69482b79…`,
  `53c92fa0…`, `6096c842…`, `fd46029e…`): equal. After 175 (one real deletion, at 170): equal to the test at `:406`.
- Floresta's signet roots for heights 1..120 (floresta-wire `tests/test_data/roots.json`, used by `signet_roots` at
  `tests/utils/mod.rs:308`) from its own `blocks.json`: 120 of 120 equal.
- Mainnet 1..100,000 (hazync-coord's block files): every block passed the checks; Empty fold == Stump at 1,999, 10,000,
  20,000, 50,000, 91,722, 91,812, 91,842, 91,880, 99,999 and 100,000 (24.5 s). After 100,000: 197,456 leaves, 71,890 live
  (the hints zAPP's 71,888 coins unspent at 100,000 plus the two BIP30 leaves), 6 roots.
- Floresta's shipped mainnet (939,969) and signet (296,870) values are NOT reproduced here: that needs every block to
  939,969 (mainnet) or a synced signet node (hazync-coord's signet node is at 60,039).

## 3. The guest: a walk over the blocks, no Utreexo proofs

Because of §2, the guest never deletes. It is the hints zAPP's walk (HINTS_ZAPP.md §3, on `feat/hints-zapp`, PR #731, not yet on main) with a Utreexo fold beside the
coin-log fold (`libzync/src/utreexo_roots.rs`, `Deriver`, #[path]-included by the guest
`prover/zyncapps/methods/utreexo`). Per block 1..H, in block order:

1. hash the header; require it to follow the previous; parse the transactions without witness; rebuild the Merkle
   root (CVE-2012-2459 refused, the coinbase fixes the depth); mark same-block spends;
2. for each output that is not unspendable and not spent in its block:
   - a BIP30-overwritten coinbase (mainnet 91,722 / 91,812, matched by height and hash): add its Utreexo leaf (live);
   - otherwise rebuild its coin-log leaf (UTREEXO_FORMAT.md §2) from the output, the block's height, coinbase flag,
     tracked bit (host hint) and creation MTP (kept by the guest). The host says unspent at H (`0`) or gives the
     spender `(height, tx, vin)` with creation < height <= H; the coin-log leaf is the coin leaf or `spent(c, s)`
     (COINLOG.md §2). The Utreexo add is the coin's Floresta leaf if unspent at H, else Empty;
3. fold the coin-log forest, the Utreexo forest and the block forest (`leaf_u(hash, h, state_h, undo_h)`, UNDO.md §3;
   `state_h`, `undo_h` from the host, except `state_H`, which must equal the coin state the walk computed).

Then the block-forest leaves H+1..T, and the checks of §4. The frame per block is the hints zAPP's (HINTS_ZAPP.md
§10): `[L, k, s] ‖ block ‖ tracked ‖ status ‖ state ‖ undo`.

Memory: O(log n) per forest plus one block. Hashing: SHA-256 (accelerated) for txids, Merkle roots, coin-log and block
leaves; SHA-512/256 (software, 64-bit arithmetic on a 32-bit machine) only for live leaves and the Utreexo parents.

## 4. Soundness: why the host cannot change the roots

What the host supplies: the block bytes, the tracked positions, each coin's status at H, each block's `state_h` and
`undo_h`, the tail leaves, and between runs the walk state.

- **The blocks are the chain's.** Each block's header hash and height enter its block-forest leaf; the final run
  requires the walk's block forest (roots and leaf count) to equal the core journal's `block_roots` /
  `block_leaves`, which the core receipt proves. A different header anywhere changes a leaf. The transactions are bound
  to the header by the Merkle root (mutation and depth checks as `tx_in_block`), so the outputs, txids and inputs are
  the block's.
- **Spent / unspent at H is the chain's.** The coin-log leaves rebuilt from the claims fold to the coin state at H,
  which must equal `state_H`, the value block H's leaf commits (and so the core proof). A spent coin claimed unspent
  gives a coin leaf where the log has `spent(c, s)`; an unspent coin claimed spent, or a wrong spender, gives a
  `spent(c, s')` the log does not have. Every Utreexo leaf is decided by that same claim, so a wrong Empty or a wrong
  live leaf breaks the coin log at H. The tracked bit and the creation MTP are inside the coin leaf likewise.
- **The Utreexo leaf data is the block's.** Block hash, txid, vout, height, coinbase flag, value and script come from
  the bound block itself; nothing about a leaf comes from the host except its status.
- **Same-block spends and unspendable outputs** are decided by the guest from the block.
- **Runs.** A run without a previous run starts from the genesis state the guest builds itself. A later run verifies
  the previous run's receipt under `self_id` inside the zkVM, requires its `self_id`, network, H and `done = 0`, and
  takes the state bytes only if their SHA-256 is the previous run's `state_out` (the decoder also refuses any non-
  canonical encoding). The checker requires the final `self_id` to be the image it trusts, so every run in the chain
  is that image; a forged earlier run would have to verify under that id.
- **The core statement** is checked in the final run as every zyncAPP does (ZYNCAPPS.md §4): `env::verify(core_id,
  journal)`, `check_core_statement` (KIND_CHAIN, version >= 2, `self_id == core_id`, the network, scripts checked, coin
  log, the network's PINNED genesis anchor), a zero lineage and an image not in `PRE_UNDO_IMAGES` (so every block 1..T
  has the 0x05 leaf), H <= T. `core_id` is an input, never compiled in.

**Does the per-block undo hash make deletions sound?** Two answers.
1. In this design there are no deletions to make sound: a spend at or below H is the coin log's flipped leaf, and the
   coin log at H is bound by `state_H`. `undo_h` is only an opaque part of each block leaf here (as in the hints zAPP).
2. For the replay design the issue first sketched (each block deletes with a Utreexo inclusion proof from the host),
   the undo hash is **not needed** for soundness: a deletion is accepted only if the leaf, recomputed by the guest from
   the input's outpoint (from the bound block) and the spent coin's fields, opens against the current roots, which the
   guest itself built; a wrong field gives a leaf that is not in the forest. The guest must still take the set of
   deletions from the block's inputs (completeness), not from the host. The undo hash would be a second, redundant
   binding of the spent coins' fields (U0 omits the outpoint and the creating block's hash, so it could not replace
   the leaf check either).

**What the checker adds** (`zapps/utreexo`, `verify_utreexo_on`; the verdict on the decoded statement is the pure
function `verify_statement`): the **caller names the network** (`--network`, default mainnet, as every zAPP; the
WASM frame's network field), never the proof, and a proof of another network is FAIL (its network word and core
profile must be that network's); not a FakeReceipt, `RISC0_DEV_MODE` unset, the receipt verifies under the trusted
utreexo id, `done = 1` and `to = H`, `self_id` = that id, one root per set bit of `leaves`, zero lineage (one core
image since genesis: a handover lineage is refused, even the image's own), the core image, profile and anchor are
that network's trusted ones (swiftsync-verify's `Image` table, used as a library), `max_time <= now + 7200`; OK
additionally needs **block H from the node's own header chain** (`--expect`, or `--mempool` / `--bitcoin-cli` asked
for block H), the core tip too when T > H (`--expect-tip`; a tip that is given is compared also when T = H; with
`--mempool` / `--bitcoin-cli` block T is asked as well, must agree with a given `--expect-tip`, and the node's
chainwork at T is compared with the proven one), minimum work plus the checkpoint, and a **pinned utreexo image**:
the built-in id `27bea7da…` is a local build, not reproducible, so a proof under it is at most DEMO until it is pinned
in reproduce/LIBZYNC_IDS (`UTREEXO_IMAGE_PINNED`); otherwise DEMO. The receipt is decoded with a 16 MiB bincode
limit and no trailing bytes (the core verifier's own decode, swiftsync-verify `src/lib.rs:463`, has no limit; core
code, not changed here).

**The CLI's output.** Check lines and the verdict line on stdout, exit 0 OK, 2 DEMO, 1 FAIL or a usage error (an
unknown flag, a flag without its value, not exactly one receipt). With `--json` or `--floresta`, stdout carries only
that output (the checks go to stderr; `---` between the two when both are asked), and only on OK, or on DEMO with
`--allow-demo`; the JSON has `verdict`, `network` (the one checked), `identity_checked` and `image_pinned` (with
`image_label` when false), and the `--floresta` source starts with a comment line naming the network. The built-in
image id is printed with the label "local build, not reproducible, not published" until it is pinned.

## 5. Host and module layout

- `libzync/src/utreexo_roots.rs`: the derivation (guest and host); SHA-512/256 by blocks; the Utreexo fold; the
  resumable state (`encode` / `decode` / `digest`). Unit tests: rustreexo's parent vector, the tag-state leaf against a
  one-shot SHA-512/256 for scripts of 0-300 bytes, the Empty rules, the state round trip.
- `prover/zyncapps/methods/utreexo`: the guest. Its own image; it does not include `hints.rs`, so neither image moves
  the other (the block walk is a copy of the hints Deriver's).
- `prover/zyncapps/host` (`zyncapp`): `utreexo-prove <block dir> <H> <core receipt | state_T.json> <out> [--split N]
  [--execute] [--prof N] [--ref <utreexo-ref log>]` replays 1..T natively (the hints host's replay, copied into
  `utreexo_host.rs` so this branch stands on main), derives the roots natively in the same runs, compares them with
  `utreexo-ref`'s line for H, then executes or proves the runs; `UTREEXO_TAMPER=<case>` negative tests;
  `utreexo-count <block_h.json ...>` the per-block cost model counts.
- `zapps/utreexo` (this zAPP's own module, the same layout as every zAPP): the check library `zapp_utreexo` and the CLI
  `zapp-utreexo verify <receipt> [--network mainnet|signet] [--expect H:hash] [--mempool | --bitcoin-cli "<cmd>"] [--expect-tip T:hash] [--json] [--floresta] [--allow-demo]`, using
  swiftsync-verify as a library (no Utreexo code in the core verifier crates). `zapps/utreexo/wasm`: the same checks as
  WebAssembly (raw ABI, `zapp-utreexo.js` glue, `build.sh` byte-reproducible with Rust 1.99.0 at a fixed stage path,
  `test.mjs`). `zapps/utreexo/vectors`: the real receipts of §8. `zapps/utreexo/test.sh`: the library's tests, 27 CLI
  cases, the WASM build and its 12 cases, and the built WASM's sha256 against the line in `wasm/README.md`.
- `prover/zyncapps/utreexo-ref`: the reference (Floresta's code), test only.

## 6. Distribution (proposal)

One file per (network, H): the final run's succinct receipt (225 kB at H = 3, §8), plus the
`AssumeUtreexoValue` it proves, printed by `zapp-utreexo verify --floresta`. A node (or its packager) checks the receipt
with the CLI, the WASM or a future Floresta-side verifier, and compares block H with its own headers. Heights: any H <= the
core tip is valid, because each is proved; which ones to publish is the Floresta question Q2.

## 7. Measured (15.11.2026, hazync-coord, CPU, risc0 3.1.0 executor, no dev mode)

Execute only; the core receipt an unresolved assumption whose journal is the published host's (`46666b6f`) prep state at
T; every check of the guest run; roots compared with `utreexo-ref` (all byte-identical). Image `27bea7da…` (a local
build of this branch). `--prof` runs call `env::cycle_count()` around every Utreexo add; the unprofiled split runs give
the overhead: 100,000 in 4 runs 5,272,138,959 against the profiled single run's 5,306,655,984 (0.7%).

| H (T) | user cycles | padded | segments | execute | per block | live leaves | leaves |
|---|---|---|---|---|---|---|---|
| 1,999 (1,999) | 111,580,197 | 123,207,680 | 118 | 2.1 s | 55,818 | 1,929 | 2,036 |
| 1,999 (10,000) | 124,027,714 | 136,314,880 | 130 | 2.2 s | | 1,929 | 2,036 |
| 10,000 (10,000) | 549,337,980 | 605,552,640 | 578 | 10.4 s | 54,934 | 9,494 | 10,101 |
| 10,000, 4 runs of 2,500 | 548,330,749 | 604,045,312 | 579 | 4 x ~2.8 s | | 9,494 | 10,101 |
| 100,000 (100,000) | 5,306,655,984 | 5,953,814,528 | 5,678 | 99.9 s | 53,067 | 71,890 | 197,456 |
| 100,000, 4 runs of 25,000 | 5,272,138,959 | 5,912,395,776 | 5,639 | 25.4 + 17.4 + 23.2 + 33.6 s | | 71,890 | 197,456 |

**Where the cycles go** (profiled, cumulative at H):

| H | all | Utreexo part | share | leaf hashes | leaf SHA-512 blocks | parent hashes | Empty adds | cycles per SHA-512 compression |
|---|---|---|---|---|---|---|---|---|
| 10,000 | 536,512,936 | 421,272,437 | 78% | 9,494 | 18,951 | 9,485 | 607 | 14,815 |
| 100,000 | 5,306,557,640 | 2,961,755,697 | 56% | 71,890 | 124,431 | 71,884 | 125,566 | 15,087 |

- A SHA-512 compression costs about **15,000 cycles** in the guest (the Utreexo part divided by leaf blocks + parents;
  it includes the forest bookkeeping). A SHA-256 compression with risc0's accelerator is far cheaper, so Utreexo's
  choice of SHA-512/256 dominates this zAPP's own cost.
- An Empty add costs no hash. Parent hashes ≈ live leaves (71,884 for 71,890).
- The rest (100,000: 2,344,802,000 cycles, the block walk, coin log and block forest) is close to the hints zAPP's whole
  run at the same H (2,389,822,475, HINTS_ZAPP.md §10), as expected: the same walk.
- Real CPU proofs (H = 2, 3): §8.

**Modern blocks** (`zyncapp utreexo-count`, measured counts, not cycles): adds per block and leaf SHA-512 blocks if every
add were live: 741,000: 387 / 406; 900,000: 2,571 / 3,170; 930,000: 9,353 / 10,020; 950,000: 1,213 / 1,295; 962,000:
1,782 / 1,955 (12,853 outputs, 5,607 unspendable); 964,500: 765 / 913; 965,978: 5,668 / 6,473. So 1.05-1.23 leaf
compressions per add in these blocks (P2PK-era blocks: 2). The walk's cost on modern blocks is not measured here (nor
in the hints zAPP).

### ESTIMATE to the tip (not measured; from the numbers above)

- **This design's Utreexo part** ≈ live leaves at H × (≈1.2 leaf compressions + ≈1 parent) × 15,000 ≈ **33,000 cycles
  per live leaf**, i.e. ≈ 3.3 × 10^12 cycles per 100 million coins live at H. The live coin count at 939,969 was not
  measured here (hazync-coord's node has no coinstatsindex); with the order of 10^8 live coins this is a few times
  10^12 cycles. The other ~2.9 billion leaves Floresta counts at 939,969 (3,066,490,760 in all) are Empty and cost
  no SHA-512.
- **Plus the walk** over every block to H, which this zAPP shares with the hints zAPP; it is unmeasured for modern
  blocks and is likely the larger part at the tip. No total mainnet figure is claimed.
- **The replay design (deletions with Utreexo proofs), for comparison:** each deletion recomputes its path to a root
  twice (old and new), about 2 × 31 SHA-512 compressions at the tip's forest height before sharing within a block,
  ≈ 0.9 M cycles; with ≈ 2.9 × 10^9 deletions to 939,969 that is of the order of 10^15 cycles, plus one leaf hash and
  about one parent per add for all 3.07 × 10^9 adds (≈ 10^14). Two to three orders of magnitude above this design's
  Utreexo part, which is why it is not built.
- Serial execution ran at 53 M cycles/s; mainnet will need runs of bounded size proved in parallel (§3: the chain of
  runs is built and tested).

## 8. Tested (15.11.2026)

- Roots and leaf count == `utreexo-ref` (Floresta's `update_acc`) at H = 1,999, 10,000, 100,000; also with T = 10,000
  for H = 1,999; in one run and in four chained runs (10,000 and 100,000): the same roots.
- Refused by the guest (H = 10,000 unless stated; `zyncapp utreexo-prove ... --execute` with `UTREEXO_TAMPER`): a spent
  coin claimed unspent, an unspent coin claimed spent, a wrong spender, a wrong tracked bit (all: "the coin state at H
  is not the one the claimed spends imply"); a flipped coin state and a flipped undo hash (the block forest); a flipped
  nonce; a dropped and a swapped transaction (block 170); a core lineage, a pre-undo core image, another core id (no
  receipt to resolve), an assumevalid core; between runs, a flipped state byte and another image id for the previous
  run; with T = 10,000 for H = 1,999, a flipped leaf above H. 16 of 16 refused.
- Unit tests of `libzync::utreexo_roots` (4).
- **Real CPU proofs** (no dev mode, `ProverOpts::succinct`, `RAYON_NUM_THREADS=6`, the shared box; the core receipts are
  the published host's `46666b6f` proofs of mainnet 1..3 that the hints zAPP used): H = 3 in one run 149.5 s (225,514
  bytes); H = 3 in two runs (1..2, then 3, the second verifying the first in the zkVM) 75.0 + 167.4 s; H = 2 under
  T = 3 150.7 s. Roots: after 3, `5fcbe4f3…`, `1fb4d2cf…` (3 leaves), equal to `utreexo-ref`; one run and two runs give
  the same statement roots. Blocks 1..3 have no spends: the spend, same-block and BIP30 paths are covered by the
  executions above, not by a proof.
- **The checker** (`zapps/utreexo/test.sh`, 15.11.2026, review fixes): `cargo test` 3/3 (the Floresta source form with
  and without an Empty root; a hostile receipt length refused by the decode limit; the real receipts: DEMO below
  minimum work, refused for H or block hash not the node's, another image id, a wrong tip given when T = H, H = 2 with a
  wrong core tip); 14 CLI cases on 15.11.2026, 18 since 16.11.2026 with the chain-identity flags of zapps/common (exit codes, strict flags, a truncated receipt, `--json` silent on DEMO, `--allow-demo`
  JSON marked `demo`, `--floresta` alone on stdout); the WASM build (`wasm/`, sha256 `a0279e99…` then; `515e020a…` since 16.11.2026, libzync's `utreexo` feature; reproduced by a second
  build) 7/7 in node. Since 18.11.2026 (the review's network, image-pin, chainwork and trailing-bytes findings, and
  the cargo-mutants pass): `tests/verdict_paths.rs` builds statements that reach OK and every FAIL and DEMO branch of
  `verify_statement` on both networks; 27 CLI cases (`--network`, a mainnet proof checked as signet, a trailing byte,
  the node's chainwork at T with `--expect-tip`); the WASM `106feaf9…`, 12/12.

## 9. Questions for the Floresta developers

1. **Would you use proven roots?** Floresta now trusts a compiled-in `AssumeUtreexoValue` and backfills. Would a
   node, or your release process, check a receipt for that value (a Rust library call, a CLI before release, or
   WASM), and would it let you skip or defer the backfill?
2. **Which heights and how often?** The value you ship (939,969 now) at each release? Or the latest height on a
   schedule? Any H can be proved once the chain proof reaches it.
3. **Format pinning.** Is the leaf (`UTREEXO_TAG_V1`, SHA-512/256, the header code, TxOut serialisation) and the
   Stump's root order frozen for the life of `rustreexo 0.7` / Floresta's current release, or is a change planned
   (for example a new tag)? A change means a new image of this zAPP; I would pin a statement version to your version.
4. **Empty roots.** I commit an Empty root as 32 zero bytes (rustreexo's test encoding). Do you serialise or ship
   Empty roots anywhere (they cannot occur in `acchashes!`), and is that encoding acceptable?
5. **BIP30 leaves.** I add the coinbases of 91,722 and 91,812 as permanent live leaves, as `get_block_adds` does;
   please confirm that this is intended for every future version (utreexod does the same?).
6. **Custom value.** florestad passes `assumeutreexo_value: None`, so a user cannot pass proven roots without a
   rebuild. Would you accept a flag or config entry for them, gated on a proof check?
7. **SwiftSync.** Your SwiftSync mode builds the accumulator from hints with Empty adds. A proven hints file (the hints
   zAPP, HINTS_ZAPP.md) would let a node build the roots itself while syncing; proven roots let it skip that. Which of
   the two is more useful to you?
