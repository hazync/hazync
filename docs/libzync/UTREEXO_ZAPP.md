# zyncAPP: proven Utreexo roots for assumeutreexo (design and prototype, #723)

Status: released in v0.27.0 (#744, 20.11.2026) as an **early** zAPP, CPU only. Executed (no dev mode) from genesis to H = 1,999,
10,000 and 100,000; the roots are byte-identical to Floresta's own code at every H tested; real CPU proofs at H = 2 and 3
(§8). Reviewed 15.11.2026 (no soundness break found); images pinned in `reproduce/LIBZYNC_IDS` from the reproducible
container build: base `00b2d1e8`, tip mode (§10) `5ef26dfe` since 21.11.2026 (the block walks read compact sizes as
Core does, with libzync::facts' reader; two agreeing builds of `cffa2cbb`); before them `27bea7da` / `ab6f7a26`. Nothing deployed. Utreexo stays out of the Hazync core; this zAPP sits on
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
function `judge_verified_statement`): the **caller names the network** (`--network`, default mainnet, as every zAPP; the
WASM frame's network field), never the proof, and a proof of another network is FAIL (its network word and core
profile must be that network's); not a FakeReceipt, `RISC0_DEV_MODE` unset, the receipt verifies under the trusted
utreexo id, `done = 1` and `to = H`, `self_id` = that id, one root per set bit of `leaves`, zero lineage (one core
image since genesis: a handover lineage is refused, even the image's own), the core image, profile and anchor are
that network's trusted ones (swiftsync-verify's `Image` table, used as a library), `max_time <= now + 7200`; OK
additionally needs **block H from the node's own header chain** (`--expect`, or `--mempool` / `--bitcoin-cli` asked
for block H), the core tip too when T > H (`--expect-tip`; a tip that is given is compared also when T = H; with
`--mempool` / `--bitcoin-cli` block T is asked as well, must agree with a given `--expect-tip`, and the node's
chainwork at T is compared with the proven one), minimum work plus the checkpoint, and a **pinned utreexo image**:
the built-in id `00b2d1e8…` (since 21.11.2026; `27bea7da…` from 18.11.2026) is **pinned** in reproduce/LIBZYNC_IDS (`zyncapp-utreexo`, the
reproducible container's build, §10.8; until then a local build, and a proof under an unpinned built-in id is at most
DEMO) (`UTREEXO_IMAGE_PINNED`; an id the caller names with `--image` may reach OK, and the checks
and the OK line say the checker does not know its code); otherwise DEMO. A FAIL report carries no statement. The receipt is decoded with a 16 MiB bincode
limit and no trailing bytes (the core verifier's own decode, swiftsync-verify `src/lib.rs:463`, has no limit; core
code, not changed here).

**The CLI's output.** Check lines and the verdict line on stdout, exit 0 OK, 2 DEMO, 1 FAIL or a usage error (an
unknown flag, a flag without its value, not exactly one receipt). With `--json` or `--floresta`, stdout carries only
that output (the checks go to stderr; `---` between the two when both are asked), and only on OK, or on DEMO with
`--allow-demo`; the JSON has `verdict`, `network` (the one checked), `identity_checked` and `image_source`
(`"builtin-unpinned"` with `image_label`, `"builtin-pinned"`, or `"caller"` for an id named with `--image`, whose code
the checker does not know; the OK line says so too), and the `--floresta` source starts with a comment line naming the network. The built-in
image id was printed with the label "local build, not reproducible, not published" until it was pinned (18.11.2026; now "pinned in reproduce/LIBZYNC_IDS, reproducible container build").

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
  `test.mjs`). `zapps/utreexo/vectors`: the real receipts of §8. `zapps/utreexo/test.sh`: the library's tests, 28 CLI
  cases, the WASM build and its 13 cases, and the built WASM's sha256 against the line in `wasm/README.md`.
- `prover/zyncapps/utreexo-ref`: the reference (Floresta's code), test only.

## 6. Distribution (proposal)

One file per (network, H): the final run's succinct receipt (225 kB at H = 3, §8), plus the
`AssumeUtreexoValue` it proves, printed by `zapp-utreexo verify --floresta`. A node (or its packager) checks the receipt
with the CLI, the WASM or a future Floresta-side verifier, and compares block H with its own headers. Heights: any H <= the
core tip is valid, because each is proved; which ones to publish was §9's question 2; Floresta's answer (the latest, a few blocks below the tip) is §10.

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
  `judge_verified_statement` on both networks; 28 CLI cases (`--network`, a mainnet proof checked as signet, a trailing byte,
  the node's chainwork at T with `--expect-tip`); the WASM `d240082b…`, 13/13.

## 9. Questions for the Floresta developers

Asked in getfloresta/Floresta discussion #1472 (16.11.2026) and **answered** there the same day; the answers and how
this design follows them are in §10.11. Kept here as asked.

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

## 10. Tip mode: the newest roots, a few blocks below the tip (design and prototype, 18.11.2026)

Floresta's answer to the questions above (getfloresta/Floresta discussion #1472, Davidson Souza, 16.11.2026): **the
latest height, or a few blocks below the tip for a reorg margin, is what they want**; no static per-release commitment
("proofs are easy to check and not trusted. If someone is serving them, just download the latest proof and try
ingesting them"); no change planned to the leaf format or the root order in rustreexo 0.7; Empty roots are implicit for
them and they parse ours after the check; the BIP30 leaves stay permanent (utreexod the same); proven **roots** over a
hints file. This section is the design and the built prototype for that, released in v0.27.0 with the base. Status: CPU only, low
heights, nothing deployed; reviewed before the merge (§10.9).

### 10.1 Why the walk of §3 cannot follow the tip, and what does

The walk adds every coin spent by H as Empty. "Spent by H" depends on H: moving H from P to P+1 turns every leaf whose
coin block P+1 spends from live to Empty, wherever it sits in the forest, so the walk's state after block k < P is not
reusable for a later H. A walk to every new H is a walk from genesis (§7: of the order of 10^12 cycles or more at the
tip). So tip mode has two parts:

- **The base**: one walk of §3 to some height B (any B ≤ the core proof's height), proved once. Its roots are exactly
  Floresta's after B (§2).
- **Steps**: each one starts from the roots a previous receipt proves (the base, or the previous step) and applies
  blocks P+1..H **as a Floresta node does**: `Stump::modify(adds, dels, proof)` of rustreexo 0.7.0 (the crate itself,
  as a guest dependency), with the adds of floresta-chain `get_block_adds` and the deletions of its `process_proof`,
  one block at a time. This is the "replay design" of §4 / §7, but only for the blocks after the base: at the tip, a
  step covers the blocks since the last one.

A step's receipt verifies the previous receipt inside the zkVM, so the **newest receipt alone** proves the whole chain
(base walk + every step); it is one succinct receipt of constant size, whatever the number of steps. A new step
**replaces** the last as the thing served; the last few are kept for reorgs (§10.6).

### 10.2 The step statement ('ZU' version 2)

```
TipStatement {
  kind          u32        'ZU' 2 (0x5A550002)
  self_id       [u32; 8]   the tip image every step of the chain was verified under (the checker requires its own id)
  base_id       [u32; 8]   the base image (§3) the chain starts from (the checker requires the trusted one)
  network       u32
  base_height   u32        B, the base's H
  base_core_id  [u32; 8]   the core image the base's walk rested on (its coin-state claims are bound by it)
  steps         u32        steps since the base (1 for a step from the base)
  from          u32        P, the height this step started from
  height        u32        H
  block_hash    [u8; 32]   block H, internal byte order
  leaves        u64        Stump.leaves        } Floresta's AssumeUtreexoValue,
  roots         [[u8;32]]  Stump.roots          } the same encoding as §1 (an Empty root = 32 zero bytes)
  core binding             core_id, lineage, anchor_id, core height T (H <= T), tip_hash, cum_work, max_time, profile:
                           THIS step's core receipt
}
```

`n_live` (§1) is not tracked in tip mode: the Stump does not know it, and nothing Floresta loads needs it.

### 10.3 The step guest (`prover/zyncapps/methods/utreexo-tip`, shared code `src/step.rs`)

Input: `self_id`, `base_id`, `core_id`, the core journal, the network, H, the previous receipt's kind and journal; per
block P+1..H a frame `[L, S, Q]` ‖ the block without witness (L bytes) ‖ the spent coins (S bytes) ‖ the batch proof
of its deletions (Q bytes); then the T − H headers from H+1 to the core tip. Per block:

1. hash the header; it must follow the previous block (the first: the previous statement's `block_hash`); parse the
   transactions without witness; rebuild the Merkle root (CVE-2012-2459 refused; only transaction 0 a coinbase);
2. **adds** (`get_block_adds`): every output in block order that is not `is_unspendable` (OP_RETURN, or a script over
   10,000 bytes) and whose outpoint no input of the block spends (every input, as Floresta's `get_input_outpoints`),
   each as Floresta's leaf (§2, `utreexo_roots::utreexo_leaf`, the same function the base uses);
3. **deletions** (`process_proof`): every input of a non-coinbase transaction, in input order, except one whose txid
   is a transaction of this block with vout below its output count; an outpoint twice is refused. Each deleted leaf
   is rebuilt from the input's outpoint (from the bound block) and the host's data of the spent coin: the creating
   block's hash, the header code (height·2 + coinbase), value and script. Floresta's `process_proof` gets the same
   data from its peers (CompactLeafData) and the block hash from its own header chain by height. A leaf equal to
   either of Floresta's `UNSPENDABLE_BIP30_UTXO_*` is refused, as `update_acc` refuses it;
4. `Stump::modify(adds, dels, proof)`: rustreexo recomputes the old roots from the deleted leaves and the proof and
   refuses the proof unless every one is a current root, then folds the adds in. The guest runs this code unchanged.

After H: `env::verify(core_id, journal)`, `check_core_statement` (as the base: KIND_CHAIN, version >= 2, the
network's pinned genesis anchor, scripts checked, no assumevalid), H <= T, and the T − H headers must chain from block
H to the core journal's `tip_hash`.

The previous receipt: kind 1 (a base): `env::verify(base_id, journal)`, a final run (`done = 1`, `to = H`), its
`self_id = base_id`, the same network; kind 2 (a step): `env::verify(self_id, journal)`, its `self_id` and `base_id`
equal to this step's, the same network. `base_height` and `base_core_id` are carried forward from the base.

### 10.4 Soundness: why the host cannot change the roots

What the host supplies: the blocks, the spent coins' data, the batch proofs, the link headers, the previous receipt
and the core receipt.

- **The blocks are the chain's.** Block P is the previous statement's (bound by its own proof). Each block follows the
  previous by hash, and the link headers continue that hash chain to the core proof's tip T, which the core receipt
  proves to be the tip of a valid chain from genesis (scripts checked, the network's anchor). A different block at any
  height between P and T would need a SHA-256d collision. The transactions are bound to their header by the Merkle
  root. So blocks P+1..H are the core-proven chain's, and so is block P (an ancestor of H on it). This is the same
  binding the header-chain zAPP gives (a hash chain to a proven tip); I bind to the core receipt's `tip_hash` directly
  rather than through the header MMR (one fewer receipt, and the core receipt is needed anyway for validity).
- **Adds and the set of deletions are the block's.** The guest takes them from the bound block, never from the host:
  the host cannot drop a deletion (a missing spent-coin record fails the parse, an extra one is refused), add one,
  or reorder adds.
- **Each deleted leaf is the coin really spent.** The leaf is rebuilt from the input's outpoint and the host's coin
  data; rustreexo's `modify` accepts it only if it opens against the roots the guest holds. A wrong value, script,
  height, coinbase flag or creating block gives a leaf that is not in the accumulator (a SHA-512/256 collision
  otherwise). Outpoints are unique among live leaves except the two BIP30 coinbases, whose leaves are refused.
- **The starting roots are Floresta's after P.** For the base, §4. For a step, by induction: the previous step was
  verified in the zkVM under this same image (`self_id`, which the checker pins).
- **Earlier steps' core receipts need no check of their own** (review L7). Each step commits only its OWN `core_id`,
  and the checker pins only the newest. That is enough: step k linked its blocks P_k+1..H_k to its core tip T_k, and
  step k+1 starts at block H_k by hash and links on to the newest core tip, which the checker pins. So every block any
  earlier step applied is a hash ancestor of a block on the pinned core chain, i.e. on that chain (a SHA-256d
  collision otherwise), whatever image an earlier core receipt was under. An earlier core receipt contributes nothing
  else to the roots: the deletions are checked against the accumulator, not against the core state.
- **The base's trust carries.** `base_id` and `base_core_id` are committed by every step; the checker requires both to
  be trusted (the base's spent/unspent claims rest on its core receipt).
- **Core handovers.** A step needs no undo commitment, so, unlike the base (§4: zero lineage), a step accepts the core
  verifier's own lineage rule (zero, or exactly the core image's pinned lineage). A chain of steps can continue across
  a core image upgrade; the base must still have been walked under a trusted core image (open: a base under the
  previous image after a handover: ours to decide, with the image table of the core verifier).
- Everything else (which images to trust, minimum work, the clock, that block H is the node's) is the checker's (§10.7).

### 10.5 Which H, and where the witness comes from

`hazync-utreexo-follow` (coordinator/deploy/utreexo/) takes **H = min(node tip − MARGIN, T, P + MAX_STEP)**, MARGIN = 6
by default, T the core receipt's height (`zyncapp core-info`), MAX_STEP = 2,016 (a long catch-up takes several runs).
⚠ **Today T bounds H, not the margin.** The genesis-anchored core proof (the spine) was at **3,836** on 18.11.2026
(MEASURED, hazync-proof `spine.json`), so the tip mode can follow the core proof but cannot be near Bitcoin's tip
until the core proof is. Nothing here needs the core proof to be at the tip; the same code serves "T − 0" while the
spine climbs and "tip − 6" once it is there.

**The witness** (spent coins and batch proofs per block) is the hard part at the tip, not the proof. The prototype's
host (`zyncapp utreexo-tip-prove`, `prover/zyncapps/host/src/utreexo_tip_host.rs`) replays Floresta's rules natively
from genesis with rustreexo's `MemForest` (every live leaf in memory) and a map of live coins, and caches both at the
last proven height (`--cache`, `HZUTIP01`). It runs every block through the same `step.rs` and the same
`Stump::modify` the guest runs, and compares the result with the previous statement, before proving. That is fine for
tests and low heights; it is **not** a mainnet-tip source (the forest of ~10^8 live leaves in memory: not measured
here). The production source is either (a) a Utreexo bridge node: utreexod with its proof index serves exactly this
per block (the same udata Floresta itself syncs from: leaf data + batch proof), or (b) a disk-backed forest kept by
Hazync's tip prep run. Neither is built (§10.5.1; a note for a later follow-up in #1472, §10.11). Because the guest checks every proof, the source is never
trusted, so (a) needs no trust in the bridge.

#### 10.5.1 The production witness source (DESIGNED, NOT BUILT)

⚠ Nothing in this subsection exists. The host's `Native` (MemForest + live-coin map) stays the test source; the
production host would replace only the part that yields, per block, the spent coins' data and the batch proof. Its
output format is the one the guest already reads (step.rs `put_spent`, `encode_proof`), so neither the guest nor the
image changes.

| option | how | cost and risk |
|---|---|---|
| **A. a Utreexo bridge node (preferred)** | utreexod run with its Utreexo proof index next to the proving host. Per block it holds the deleted leaves' data and the batch proof against the accumulator before that block, the same "udata" Floresta receives from its peers. The host converts it (leaf data + block hash by height from our own node → `put_spent`; targets + hashes → `encode_proof`) and, as now, runs `Stump::modify` natively before proving | an extra node to run and keep in sync (its disk for the index: not measured here). Nothing is trusted: a wrong proof fails natively and in the guest. Risk: a format or behaviour difference in the bridge's proofs (positions after deletions) is caught at once, natively. See §10.11 |
| B. Hazync's own disk forest | a disk-backed rustreexo forest (or a position-indexed node store) updated by the tip prep run, with an outpoint → (position, coin data) index | no third-party node, but a new store of the order of the live UTXO set plus internal nodes (not measured), its own crash/reorg handling (REORG.md P3 style undo) and a second Utreexo implementation to keep equal to rustreexo |

Either way the follower changes only in where `--cache`'s replay comes from; reorg handling stays as §10.6 (the
source must be rewound to the fall-back height: a bridge does that itself; a disk forest needs undo records).

### 10.6 What is served, where, and reorgs

**Served** (proposal; not deployed): a folder like the headers zAPP's (`/headers-live/`), e.g.
`https://hazync.org/utreexo-live/`, rsynced from the proving box with a key pinned to `rrsync -wo`:

| file | what |
|---|---|
| `latest.json` | `network`, `height` H, `block_hash`, `leaves`, `roots` (hex, an Empty root as "empty"), `core_height` T, `mode` ("tip" or "base"), `steps`, `base_height`, `receipt` (file name), `sha256`, `bytes`, `verdict_here` (the follower's own check: ok or demo), `proved_at`, `prove_seconds`. Written last (rename) |
| `latest.rcpt` | the newest receipt (the stable name to poll) |
| `utreexo-<H>.rcpt` | the same receipt under an immutable name; the last 3 kept |

Floresta polls `latest.json` (it is a hint, not trusted), downloads the receipt by name, checks its sha256 and then the
receipt itself (§10.7). Every value in the JSON is also in the receipt's journal; a node must take them from there.

**Replacing.** A step is published only after `zapp-utreexo verify` says OK with the node's own hashes at H and at T
(DEMO is accepted only with `ALLOW_DEMO=1`, as long as every proof is DEMO: below minimum work).
Each proven height is a state directory `state/<H>-<hash>/` (`rcpt`, `meta`); `latest` and `base` are symlinks to one,
switched by a single rename (review L2). The last `KEEP` = 12 states below `latest` are kept for reorgs; a failed step, a step that
does not verify, or a wrong ELF leaves the published folder untouched.

**Reorgs** (REORG.md §8, §10; ZAPP_FOLLOWER.md §7):

| case | what happens |
|---|---|
| depth < MARGIN | nothing: H trails the node's tip by MARGIN, so no published height is touched. At the tip this covers every ordinary reorg (depth 1-2) |
| the node's block at the published H changed | the follower falls back to the newest kept state (then the base) whose block is still the node's, moves the states, published receipts and cache above it to `orphaned/` (kept, not deleted) and steps forward on the node's branch. `latest.rcpt`/`latest.json` keep naming the old receipt until the new one verifies. A node RPC failure, or a node whose tip is below the proven height (still syncing), is an error, never a reorg (review M1) |
| the core proof's tip T is not the node's block T | the follower stops for this run ("wait for the next core proof"): the core receipt names an orphaned tip, so no step can link to it. REORG P4 sets the spine back to a kept head on the new branch; the next run uses it |
| below the base | stop with an error: a new base must be walked and installed (`--set-base`) |
| receipts already served about orphaned blocks | nothing to retract: each is a true statement about a valid chain whose block H it names. A Floresta node that compares block H with its own header chain (§10.7) refuses it, as it must |

**Known limits of the follower (documented, not fixed):** the two-pass upload (receipts, then `latest.json`) is
ordered but not atomic across files: a reader can briefly see a new receipt next to the previous `latest.json`, which
still names a receipt that is there. A fall-back drops the forest cache (a failed step keeps the one for P), so the
test-scale witness source replays from genesis (and, with `PRUNE_BLOCKS=1`, fetches every body again); the production
source (§10.5.1) would not. One run at a time (`flock`); a crash between a state's rename and the `latest` switch
leaves an unused state directory, which the retention removes.

### 10.7 What a Floresta node checks

`zapp_utreexo::verify_utreexo_tip_on` (Rust library; the CLI `zapp-utreexo verify` routes a tip receipt there by its
journal's kind word):

1. the receipt decodes (16 MiB limit, no trailing bytes), is a real succinct or Groth16 receipt (not a FakeReceipt,
   `RISC0_DEV_MODE` unset), and **verifies under the pinned tip image id**;
2. kind 'ZU' 2; `self_id` = that id (every step was this image); `base_id` = the pinned base image id; `steps >= 1`,
   `base_height <= from < height`; one root per set bit of `leaves`; the network the CALLER names;
3. `core_id` and `base_core_id` = the network's trusted core image; the core profile and the genesis anchor are that
   image's; the core lineage zero or the image's own; `max_time` not more than 2 h ahead of the node's clock;
4. **block H's hash = the node's own header chain's** at H (`--expect`, or `--bitcoin-cli` / `--mempool`), and the core
   tip T (> H) the node's too (`--expect-tip`); Core's minimum work and checkpoint at T;
5. the node's own freshness rule (Floresta's: H a few blocks below its tip; not the checker's business);
6. then load: `Stump { leaves, roots }` with every 32-zero-byte root as `BitcoinNodeHash::Empty` (§1), and
   `mark_chain_as_assumed(stump, block_hash)` as for a compiled-in `AssumeUtreexoValue` (§2).

OK needs 1-4 and both images pinned (both are, since 18.11.2026; an unpinned built-in id caps the verdict at DEMO). `--json` adds `mode: "tip"`, `steps`, `from`, `base_height`, `base_image`; `--floresta`
prints the same `AssumeUtreexoValue` source as for a base.

### 10.8 Measured (18.11.2026, hazync-coord, CPU, risc0 3.1.0 executor, no dev mode)

Images, **pinned in reproduce/LIBZYNC_IDS** (18.11.2026): `reproduce/libzync-ids.sh` now also prints the utreexo base
and tip lines (`--zyncapps-only` skips the core images, whose build the zyncAPP build does not read). Two builds in the
reproducible container agree byte for byte: hazync-coord (the deps stage of `reproduce/libzync.Dockerfile`, a clean
copy of `ccedefe`) and hazync-proof (its `hazync-libzync-repro` image, toolchains 1.99.1 + risc0 v1.94.1, a clean
clone of `ccedefe`, an empty target). Both print the published headers line `2db34e12…` / `5bd8011d…` as the control,
`zyncapp-utreexo 27bea7da…` (ELF `c7bbe197…`) and `zyncapp-utreexo-tip ab6f7a26…` (ELF `dd9d09bd…`): the same ids
as the local builds of 15.11 and 18.11, so **no id changed** and no vector or proof had to be redone. (A third build
on a laptop was abandoned: the deps stage's toolchain download crawled at ~7 kB/s.) CI's `reproducible-image-id` job
now compares every `zyncapp-*` line. Executed with the previous statement and the core state as unresolved
assumptions (the published host's prep states at 10,000 and 100,000), `--prof 1000` (`env::cycle_count()` around the
block rules and around `modify`). A chain: the base walk to 1,999, then steps to 10,000, 20,000, 50,000, 91,722,
91,812, 91,842, 91,880 and 100,000, each step starting from the previous step's journal. Columns from the guest's own
counters over the step's blocks (the link headers to the core tip excluded, listed apart):

| step | blocks | adds | dels | proof hashes | parse + leaves | modify | (parse + modify) per block | link headers (cycles) |
|---|---|---|---|---|---|---|---|---|
| 1,999 → 10,000 | 8,001 | 8,065 | 500 | 2,351 | 302,350,171 | 292,312,162 | 74,324 | 0 |
| 10,000 → 20,000 | 10,000 | 10,058 | 529 | 2,393 | 374,198,799 | 365,199,346 | 73,940 | 80,000 (161,823,170) |
| 20,000 → 50,000 | 30,000 | 30,752 | 8,244 | 34,678 | 1,339,543,699 | 2,115,662,799 | 115,174 | 50,000 (101,179,851) |
| 50,000 → 91,722 | 41,722 | 99,370 | 77,506 | 379,089 | 4,443,054,055 | 14,431,764,839 | 452,395 | 8,278 |
| 91,880 → 100,000 | 8,120 | 46,850 | 38,151 | 140,997 | 2,026,101,516 | 5,552,670,202 | 933,346 | 0 |

(The three short steps over the BIP30 heights: 90, 30 and 38 blocks, 51.7 M, 11.4 M and 51.2 M cycles.) A link header
costs about **2,020 cycles** (161,823,170 / 80,000: the cycles after the last block, so the core checks are in it too). Totals with the link: 602,320,636 user cycles for 2,000..10,000
(14.3 s), 7,651,274,090 for 91,881..100,000 (141.1 s, 8,054 segments).

- **Same roots as Floresta's code**: every step's roots and leaf count are byte-identical to `utreexo-ref`
  (floresta-chain `update_acc`) at 10,000, 20,000, 50,000, 91,722, 91,812, 91,842, 91,880 and 100,000
  (`UTREEXO_REF_MATCH`), across the BIP30 heights.
- **Same roots as the walk**: the base walk straight to 10,000 and base(1,999) + step(10,000) commit the same roots:
  the two designs (Empty adds vs real deletions) give one accumulator, as §2 says they must.
- **Where the cycles go**: `modify` per deletion, with the adds' parent hashes in it, 145,543 cycles at
  91,880..100,000 (5,552,670,202 / 38,151; the forest has ~200,000 leaves there, trees up to 2^17); parse + leaf
  hashes ~23,800 cycles per add-or-delete there (2,026,101,516 / 85,001; each leaf is about 2 SHA-512 compressions:
  99,767 leaf blocks for 85,001 leaves).

**ESTIMATE at the tip (not measured).** Deletions per block, MEASURED with `utreexo-count` (§7: inputs − same-block
spends): 900,000: 3,694; 930,000: 7,078; 950,000: 835; 962,000: 2,542; 964,500: 2,105; 965,978: 5,688. A deletion's
path is ~31 levels at the tip instead of ~17 here, so ~1.8x the 145,543 → **~260,000 cycles per deletion** if batching
saved nothing more than it does here (it saves more in big blocks: shared paths). Then a modern block is
dels × 260,000 + (adds + dels) × 23,800: 930,000 ≈ 7,078 × 260 k + 16,431 × 23.8 k ≈ **2.2 × 10^9 cycles**; 962,000 ≈
2,542 × 260 k + 4,324 × 23.8 k ≈ **0.76 × 10^9**. The block parse of a modern block is not separated here (blocks to
100,000 are small). At ~6 blocks an hour that is of the order of 10^10 cycles an hour (its proving time is not measured), against
~10^12 or more for one walk from genesis (§7): this is why tip mode steps rather than re-walks.

### 10.9 Tested (18.11.2026)

- `zapps/utreexo/tip-exec-tests.sh` (executed on hazync-coord, the numbers above): the chain base → 8 steps, each
  equal to Floresta's `update_acc`; walk(10,000) == walk(1,999) + step(10,000); then a control (the untampered step
  1,999 → 9,000 under the core state at 10,000 passes) and **21 refusals by the guest, each asserted with its reason**
  (re-run 18.11.2026 at `ea14217`, after the review): from the base, 17 on 1,999 → 9,000: a spent coin's value,
  creating block hash, height, coinbase flag, a proof hash, a proof target, an empty proof ("the deletions do not open
  in the accumulator"); the spent coins of a block dropped ("truncated") or doubled ("more records than deletions"); two
  transactions swapped ("Merkle root"); the first block's prev field ("header does not follow the previous block"); a
  middle link header ("does not reach the core tip"); the LAST link header ("the link from block H does not end at the
  core tip"); a link header dropped ("want T - H"); the previous journal changed, the previous kind swapped, another
  core id ("no receipt found to resolve assumption"); from a STEP (prev kind 2), 4 on 10,000 → 15,000 under the core
  state at 100,000 (a spent value, the first prev field, the previous journal, the kind). A tamper with nothing to
  tamper (dropping a link header when H = T) is a host error, never a pass.
- `zapps/utreexo` `tests/tip_verdict_paths.rs` (feature `tip`): OK reachable on both networks from a built statement;
  each of 14 checks fails alone; DEMO below minimum work, with T > H and no tip named, with block H not named, with the
  unpinned base image; the `AssumeUtreexoValue` source of a tip statement.
- **A real CPU proof** (no dev mode, `ProverOpts::succinct`, `RAYON_NUM_THREADS=6`, hazync-coord under a load average
  of ~37 from other work): the step 3 → 103 from the base receipt `vectors/mainnet-3.receipt` (H = 3), resting on the
  core spine proof 1..3,836 (`46666b6f`, copied from hazync-proof on 18.11.2026, sha256 `96ab09fc…`, no longer kept
  anywhere), so 100 blocks plus 3,733 link headers: **4,945.9 s**, 226,138 bytes (sha256 `4e0c2bde…`, under the tip
  image `ab6f7a26`). **Re-proved on 22.11.2026** under the strict-parser images (base `00b2d1e8`, tip `5ef26dfe`, the
  container build of `cffa2cbb`, same settings) from the re-proved base receipt, on the core spine 1..4,790
  (`46666b6f`, hazync-proof's `/var/lib/hazync-lz/spine/heads/spine_4790.bin` of 20.11.2026, sha256 `8a8e1b00…`, tip
  `00000000133…8eb5`; kept as `vectors/core-spine-4790.receipt` so the vector can be re-proved again): 100 blocks plus
  4,687 link headers, **5,180.9 s**, 226,138 bytes (`vectors/mainnet-tip-103.receipt`, sha256 `a2e37240…`). The
  roots equal the native replay's; `zapp-utreexo verify` gives DEMO (below minimum work; both images were also unpinned then) with
  every check ok. That time is a loaded shared box, not a benchmark. The blocks 4..103 have no spends: deletions are
  covered by the executions above, not by a proof (a proof of the step 103 → 180, with the first spends, was started
  and stopped for time).
- `zapps/utreexo/test.sh`: 38 CLI cases, 11 of them on the tip receipt (DEMO with block 103 and the core tip
  named; FAIL for another block 103, another core tip, another tip image, another base image (the failed check is
  the base's), signet, a trailing byte, `--base-image` on a base receipt; the `--json` fields `mode`, `steps`, `from`,
  `base_height`, `n_live: null`; `--floresta`); the library's tests with `--features tip`; the WASM sha. All pass.
- **The follower** (`coordinator/deploy/utreexo/test/test_follow.sh`, hazync-coord, real prover and checker, a fake
  bitcoin-cli over the mainnet block files): the base install (a wrong hash refused), the first step 3 → 103 PROVED
  for real and verified (DEMO, accepted with `ALLOW_DEMO=1`), "nothing to do" when H does not move, and the choice of H
  (`DRY_RUN`: node tip 3,840 → 3,834 by the margin; 4,000 → 3,836 by the core tip; the step cap). That run then
  **failed to publish**: the checker's JSON was pasted into a Python string and its escaped quotes broke it (fixed:
  it goes in through the environment). On a copy of that run's state the fixed script published `latest.json`,
  `latest.rcpt` = `utreexo-103.rcpt` (re-verified, DEMO), and then refused, leaving the folder untouched, a "prover"
  that returns another receipt and a wrong ELF; stopped on a reorg below the base; and on a reorg at 103 fell back to
  the base and chose H = 53 (`DRY_RUN`). **Then end to end with the fix** (18.11.2026, `test_follow.sh` at `15d99d2`, three real CPU proofs on hazync-coord,
~80 min each): **31 of 31 checks passed** (`UTREEXO_FOLLOW_TESTS_OK`): the base install, the step 3 → 103 published
and verified, no new height, the choice of H (margin, core tip, step cap), a step from a step 103 → 180 (the first
spends, at 170) published with the old receipt kept, a lying prover and a wrong ELF refused with the folder
untouched, a reorg at 180 falling back to 103 and stepping to 175 (`steps` 2), a reorg below the base stopping.
- **The WASM checks tip receipts** (`zapps/utreexo/wasm`, feature `tip`): the journal's kind word picks the check, an
  optional last frame field names another base image, `tip_image_id()`; the JSON adds `mode`, `steps`, `from`,
  `base_height`, `base_image`. `node test.mjs` 22/22 (9 new on the tip receipt: DEMO with its fields, FAIL for another
  block, core tip, tip image, base image, network, a base image on a base receipt). The .wasm is byte-reproduced on two
  machines: a laptop's local build of `reproduce/wasm.Dockerfile` and hazync-coord's build in the published image
  `ghcr.io/hazync/hazync-wasm-build@sha256:d46e4164…` (a clean clone); sha256 in `wasm/README.md`.
- The follower, end to end with the fix: 31/31, see the follower bullet above (at `15d99d2`). **Re-run after both
  reviews** (19.11.2026, `f08896a`, the `zyncapp` and `zapp-utreexo` built from `2d7352d`, which differs only in shell
  and docs; images unchanged, `27bea7da` / `ab6f7a26`): **31/31 again** with three real CPU proofs, the follower on its
  own block directory, the shared one checked intact (100,000 files) afterwards.
- **The follower's logic** (`coordinator/deploy/utreexo/test/test_follow_logic.sh`, stub prover and checker, seconds,
  after the review of `b127747`): 30 checks: an RPC failure and a node below the proven height are errors with nothing
  moved (M1); a reorg moves the orphaned state, published receipt and cache aside and steps on (M2); retention keeps
  the most recently published receipts and never the current one (M2); a republish checks the receipt's own core tip
  (L1); `latest` is a symlink to a state holding both files (L2); `latest.json` reaches the target, uploaded after the
  receipts (L3); only the step's bodies are fetched and the link is headers (`--headers`), `PRUNE_BLOCKS` (L4).
  Controls: the script with M1's check removed fails 4 checks, with M2's move removed 2. The run also found a real
  bug, fixed: `missing=$(… | head -1)` died of SIGPIPE under `pipefail` (exit 141) whenever more than one block file
  was missing.
  After the re-review of `f99c301` (42 checks): a state that cannot be written (a failing `cp`, disk full) is never
  installed and the next run recovers (bash clears errexit inside `$(…)`: the script now sets `inherit_errexit`, and
  `new_state` checks every step and verifies the copy); a step that fails after the prove drops the host's forest
  cache (saved at H while `latest` stays at P), so the next run replays from genesis instead of wedging under
  `PRUNE_BLOCKS`; a fall-back moves the old branch's block files above the fork aside, so the new branch's bodies are
  fetched; a `getblockhash` failure is logged (log and die write to stderr, never into a captured value); `latest` is
  switched before anything is moved aside, and directories are synced; `--set-base` honours `BASE_ID`. Controls: the
  script with the old `new_state` fails 4 checks, without the cache drop 2, without the block-file move 2. ⚠ The block-file move made the real-proof
  test unsafe as it stood: it pointed the follower's `BLOCKS` at the shared mainnet block directory, so its reorg case
  started moving another project's files. The run was stopped within two minutes, before any file moved (all 100,000
  checked present); the test now gives the follower its own directory of hard links and a copying fetch, and the
  script and README say `BLOCKS` must be private.
  After the review of `0d9683a` (shell logic only, no proof rerun needed): cached block files are never trusted because
  they exist: before each prove, every cached file in the step's range is compared with the node's hash at its height,
  and a mismatch (an old branch's file left by an interrupted fall-back, a failed step before a reorg, or a reorg below
  the base) moves to `orphaned/` and is fetched again; `--set-base` checks the files above the new base the same way.
  The host writes the cache at H to `work/forest.next` (`--cache-out`), which replaces `forest.bin` only after `latest`
  points at H, so a failed or killed run keeps a cache that is still valid for P (the earlier trap dropped it on any
  failure). `checker` checks every substitution itself (errexit does not apply in `x=$(f) || die`). `BLOCKS` must
  carry the marker `--init` writes (the follower refuses to run without it; `--init` refuses a non-empty unmarked
  directory); `PUBLISH` must be a dedicated target (README). A fall-back publishes the fall-back state at once, so
  `latest.json` never names the orphaned receipt while the new branch is proved. The logic test has a case and a
  control for each.
  After the delta review of `7806f4b`: a fall-back whose kept receipt's own core tip was orphaned too (a reorg deeper
  than that receipt's link) cannot be checked against the node, so it is logged and not published and the run steps
  on, instead of dying before the step; `--init` never overwrites a marker naming another follower; the forest cache
  is named by the height and hash it is for (`forest-<H>-<hash>.bin`), so only the cache of exactly (P, hash) lets a
  step start at P+1 (a kill between the `latest` switch and the cache's rename now means a replay from block 1, not a
  wedge under `PRUNE_BLOCKS`); `check_blocks` lists the files first and checks that listing's status, the node call
  never reads the loop's stdin (`</dev/null`, for an ssh or docker `BITCOIN_CLI`), and `--set-base` validates
  `getblockcount`. The logic test is at 71 checks, with a control for each.
- **Review H1**: `journal_kind` decoded the receipt before `receipt_kind` refused composites; a composite nested
  300,000 deep (6 MB) overflowed the CLI's stack (exit 134). Now refused first: a library test (it aborts with a stack
  overflow without the fix), a CLI case in `test.sh` and a WASM case in `test.mjs`, each generating the file.
- **BIP30**: `utreexo_tip_host::tests::a_deletion_of_a_bip30_leaf_is_refused` builds a block spending output 0 of the
  coinbases of 91,722 and 91,812 (their real bytes) with the overwritten coins' data: the rebuilt leaves are exactly
  Floresta's two UNSPENDABLE constants and both are refused; the recreated coins' data is not.

### 10.10 Layout (tip mode)

- `prover/zyncapps/methods/utreexo-tip`: the step guest, its own image (rustreexo `=0.7.0`, `default-features = false`,
  as a guest dependency; the base image `27bea7da` and the core image `46666b6f` do not move: the base guest's files are
  untouched, and the new guest has its own Cargo.lock). `src/step.rs` holds the block rules and is #[path]-included by
  the host.
- `prover/zyncapps/host` (`zyncapp`): `utreexo-tip-prove <block dir> <previous receipt | .journal> <H> <core receipt |
  state_T.json> <out> [--execute] [--prof N] [--cache <file>] [--ref <utreexo-ref log>] [--elf] [--base-id]`
  (`UTREEXO_TIP_TAMPER=<case>` negative tests), `core-info <core receipt | state JSON>`; `utreexo-prove --execute` now
  also writes the final run's journal (`<out>/<net>-<H>.journal`) so a step can start from an executed base.
- `zapps/utreexo`: `src/tip.rs` behind the crate feature `tip` (the CLI and the WASM enable it); `tests/tip_verdict_paths.rs`; `tip-exec-tests.sh` (the executed chain and the guest's
  refusals, §10.9).
- `coordinator/deploy/utreexo/hazync-utreexo-follow` and `test/test_follow.sh` (real CPU proofs against a fake
  bitcoin-cli over mainnet block files). No unit, timer, nginx alias or publish key yet: nothing is deployed.

### 10.11 What Floresta answered, and what is left for a later follow-up

The questions of §9 were asked in getfloresta/Floresta discussion #1472 and answered there by Davidson Souza on
16.11.2026 (our reply the same day). This design follows the answers; nothing here is a new question to them.

| §9 question | the answer in #1472 | where this design follows it |
|---|---|---|
| 1, 2: check a proof? which heights? | yes, it needs no change to their stack ("we already implemented it with things like ZKP in mind"); the **latest height, or a few blocks below the tip for a reorg margin**; no static commitment: "just download the latest proof and try ingesting them" | §10.5 (H = min(tip − 6, core height)), §10.6 (`latest.json` + `latest.rcpt`, the newest replaces the last), §10.7 (check, then ingest) |
| 3: format pinning | no change planned to the leaf format or the root order | the step guest runs rustreexo 0.7.0 itself and the leaf of §2 (§10.3) |
| 4: Empty roots as 32 zero bytes | they do not encode Empty roots (implicit data), and will parse ours into rustreexo after the check | unchanged (§1, §10.7 step 6) |
| 5: the BIP30 coinbases | 91,722 and 91,812 stay unspendable forever; utreexod does the same | the step refuses deleting either leaf (§10.3) |
| 6: custom roots | "IIUC this already exists" | §10.7 step 6 names `mark_chain_as_assumed`; how a florestad user sets it is theirs (§2 found only the library-side `assumeutreexo_value`) |
| 7: roots or hints | both have uses, **roots preferred** ("it skips a lot of work") | tip mode proves roots; the hints zAPP is separate |

**Not covered by the answers — for a later follow-up in #1472, if needed** (nobody contacts Floresta from this work;
these are notes for whoever writes the next reply there):

- **The witness source for deletions** (§10.5.1): we plan to read utreexod's proof index (the udata Floresta itself
  syncs from), or keep our own disk forest. Their view on which is more dependable would help, but neither choice
  changes what they check.
- **The receipt they ingest**: one succinct receipt (226,138 bytes at H = 103, §10.9) plus `latest.json`, or a Groth16
  wrap (much smaller, not measured here, needs the Groth16 verifier); and whether they want it over HTTP from
  `/utreexo-live/` or another channel.
- **The compiled-in ids**: a node pins three image ids (the core, the utreexo base, the utreexo tip;
  reproduce/LIBZYNC_IDS). Whether they would carry that triple in their releases or take it as configuration.
- **The margin**: 6 blocks is our reading of "a few blocks"; trivially changed (`MARGIN`).
