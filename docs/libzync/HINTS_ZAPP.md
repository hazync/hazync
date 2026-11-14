# zyncAPP: a proven SwiftSync hints file (design, #722)

Status: DESIGN, not built. For the author's approval. Base: `feat/undo-commit` (the U0 undo commitment, UNDO.md).

SwiftSync is Ruben Somsen's protocol; the hints file and block-undo formats are Robert Netzke's
(`2140-dev/swiftsync-bips`: `swiftsync.md`, `hintsfile.md`, `block-undo.md`, `test_vectors/`; the `hintsfile` crate).
This zyncAPP adds nothing to SwiftSync. It is an additive product on top of a Hazync chain proof: "this hints file for
height H is exactly right", so a node can check a downloaded file before it spends a sync on it. A wrong file is
already safe in SwiftSync (the final aggregate check fails and the node reindexes); the proof only saves the wasted
sync. The node's fallback stays.

## 1. What it proves and commits

Statement: *the file whose SHA-256 is `hints_sha256` is the hints file, in format `format`, for height H of the chain
that the core receipt `core_id` proves valid from genesis (scripts checked).*

```
HintsStatement {
  kind            u32       'ZH' statement, version 1
  format          u32       1 = hintsfile.md file version 0x00 (numbering §2, bit order: open question Q1)
  height          u32       H (the file header's height)
  block_hash      [u8;32]   block at H, internal byte order
  hints_sha256    [u8;32]   SHA-256 of the exact file bytes
  hints_len       u64
  n_counted       u64       outputs given an index, 1..H
  n_unspent       u64       indices listed (= live coins at H); their total value equals utxo_value at H (asserted)
  undo_from       u32       1 (the chain carries the U0 commitment from block 1; refused otherwise)
  undo_sha256     [u8;32]   SHA-256(undo_1 ‖ … ‖ undo_H), the per-block U0 undo hashes (§5)
  core binding    as ZYNCAPPS.md §4 step 5: core_id, lineage, anchor_id, core height T (H ≤ T), tip_hash,
                  cum_work, max_time, profile
}
```

The file format has no version of ours inside it; `format` is the zAPP's own. A change to numbering, encoding or
header is a new `format` value (and a new image), never a silent change. [AUTHOR: confirm the format with the
SwiftSync BIP maintainers (Robert Netzke, 2140-dev/swiftsync-bips)]

## 2. The numbering (hintsfile.md = the crate; Ruben: "the crate is most up to date")

File: `55 54 58 4f` ‖ version `00` ‖ H u32 LE ‖ one Elias-Fano list per block 1..H (genesis has none). A list is
`CompactSize n` ‖ (if n > 0) `CompactSize m` ‖ L ‖ H-bits, holding the block's **indices of outputs unspent at H**.

The index runs over the whole block (transactions in order, outputs in vout order) and counts every output except
the *unspendable* ones: script over 10,000 bytes, script starting `OP_RETURN`, and every output of the two
BIP30-overwritten coinbases (91,722 `…4dd08e`, 91,812 `…306f2f`, matched by height and hash). Outputs spent in their
own block **are counted** (always "spent"; hintsfile.md Rationale keeps them to simplify clients).

**Core PR #34004 differs** (`generatetxohints`): one bit for **every** output, unspendable ones and the BIP30
coinbases included; a per-block bitmap (`u32 bit count` ‖ bits LSB first), not Elias-Fano; a header table of
(height u32, file offset u64) per block. Its bits for 91,722 / 91,812 come from `HaveCoin` on outpoints shared with
91,880 / 91,842, so they can read 1; its reader skips them. Converting #34004 to hintsfile: drop the unspendable
positions and the two coinbases, renumber. The zAPP does not emit #34004 (could be a later `format`).

**Discrepancies found in the current references** (2026-11-14), to settle before `format` 1 is frozen:
- **Bit order.** The crate (0.2.0 and master `7dfc93ec`) fills each byte from bit 0. The BIP's `elias_fano.json`
  vectors fill each byte from bit 7 (each low value still least significant bit first): re-encoding both vectors
  matches only that order. hintsfile.md's prose ("most significant bit ordering") matches neither reading exactly.
- **Construction text.** hintsfile.md step 4.2 "write the current index … and go back to step 3" resets the index to
  0 after each unspent output, contradicting "monotonically increasing indices" and the crate. Read as a typo.
- The crate master now refuses malformed lists (`m == u32::MAX`, `n > m+1`, wrong high-bit count); 0.2.0 panics.

The zAPP writes the canonical encoding (minimal CompactSize, zero padding bits); a node compares hashes, so any other
byte string, even one that decodes to the same indices, is refused (the safe direction).

## 3. Deriving the hints soundly

**Mapping to the coin log.** Coin-log appends (COINLOG.md §3) are the counted outputs *minus* those spent in their
own block, in the same block order. Per output of block h ≤ H:

| output | hint index | coin-log leaf | hint |
|---|---|---|---|
| unspendable, or a BIP30-overwritten coinbase | none | none | — |
| spent in its own block | next index | none | spent |
| any other | next index | next position p | unspent iff leaf p is unspent at H |

Genesis: neither counts it. 91,842 / 91,880: counted and logged like any coinbase.

**The coin log alone is not enough.** It does not say where, between log positions, the same-block-spent outputs sit,
nor how many there are, and a spent leaf's preimage has no position in its block. A witness that only claimed those
would let a host shift indices. So the zAPP reads every block's transactions without witnesses (non-witness bytes are
all txids, outputs and prevouts need) and derives everything except spent-ness itself:

1. Verify the core receipt; check the statement (§4).
2. For h = 1..H: hash the 80-byte header to the block hash; rebuild the Merkle root from the transactions' txids and
   require the header's. Refuse a mutated list (CVE-2012-2459: duplicate last pair) and bind the tree depth by the
   coinbase as `tx_in_block` does (64-byte transactions; ZYNCAPPS.md §8). Mark same-block spends from the inputs.
3. Walk the outputs as in the table. For log position p rebuild its leaf: the coin leaf (UTREEXO_FORMAT.md §2) from the
   output, h, coinbase flag, the creation MTP (median of the previous 11 header times, kept by the guest) and the
   tracked bit (host hint per transaction; a wrong bit breaks the roots). The host says spent or unspent; if spent it
   gives `s` = (height, tx, vin) and the leaf is `spent(c, s)`. A spent leaf cannot pass as unspent, nor the reverse:
   both are rebuilt from preimages (the round-11 trap of ZYNCAPPS.md §8).
4. Fold the leaves into the coin log forest; at the end require the roots and leaf count to equal those of the state
   after H. Emit each block's Elias-Fano list and stream the file bytes into SHA-256.
5. Rebuild the block forest 0..T: leaf_u(hash_h, h, state_h, undo_h) for h ≤ H (state_h, undo_h from the host;
   state_H by its preimage, which gives the roots in step 4), leaf hashes from the host for H < h ≤ T. Require
   `J.block_roots`, `J.block_leaves`. This binds every header to the proven chain and every undo_h to the proof.

No scripts, signatures or UTXO lookups are re-checked (Ruben: "you do not need to prove script validity"); the core
proof already did. Memory is O(log n) roots plus one block.

**Alternative (if the image has profile bit 6).** With `live_hash` (ECMH of the live coins) at the tip, step 3's
spent leaves go: add each hinted-unspent coin to an ECMH and require `J.live_hash`. Only at H = T. Cheaper per spent
output, dearer per unspent (an EC addition each). Choose by measurement (§6).

## 4. Verifying the chain proof, and what a node checks

Inside: `env::verify(core_id, core_journal)`, then `facts::check_core_statement` (KIND_CHAIN, version ≥ 2,
`self_id == core_id`, the network, scripts checked, coin log, genesis anchor), as the `coin` and `zeroconf` guests do.
`core_id` is an input, never compiled in (ZYNCAPPS.md §4).

The node (or the CLI for it) checks: the receipt verifies under the published hints image id (rebuilt by
`reproduce/`), not a FakeReceipt, no dev mode; `core_id` and `lineage` are a published core; `anchor_id` the pinned
genesis anchor; `cum_work` ≥ `nMinimumChainWork`; `max_time` ≤ now + 7200; **`block_hash` is its own header chain's
block at H** (a SwiftSync node syncs headers first, so it has it); `format` is one it reads; SHA-256 of the file
= `hints_sha256`. The file's own height field is inside the hashed bytes.

## 5. Undo hashes (block-undo.md)

A fully validating SwiftSync node fetches each block's spent coins (`getbspent` / `bspent`) from untrusted peers;
swiftsync.md (griefing) recommends "commit to the hashes of the block inputs to a file the client has access to". The
undo file is that: `undo_1 ‖ … ‖ undo_H` (32 bytes each), committed by `undo_sha256`, each U0 (UNDO.md §2).

A `bspent` message expands losslessly to U0: the BIP's height code is U0's `code` (height·2 + coinbase); expand the
reconstructable script (prefixes 0x01-0x08, 0x00 = raw) and `DecompressAmount` the CompactSize amount; order by input
index (coinbase excluded, as in U0); hash. With `cutoff` > 0 the node first merges its cached young coins at their
input indices. A wrong message is then refused per block, not at the end of the sync.

The zAPP outputs nothing in the BIP's wire format: that encoding is not unique (any script may also be sent under
prefix 0x00), so its hash would depend on the encoder. U0 is canonical and is Core's own undo data.

## 6. Cost: what will be measured (no estimate yet)

CPU execute only, risc0 executor, no dev mode: total cycles and segments for H = 1,999 (demo), 10,000 and 100,000
(covers the BIP30 blocks), and `env::cycle_count()` at block boundaries logged by a test build, so cycles per block,
per output, per log leaf and per non-witness byte are measured separately. Modern blocks via a test-only entry that
starts from injected forests at a sampled height (windows near 500,000, 800,000, 962,000; not a provable mode). Also:
serial execute wall time (it bounds mainnet; proving is parallel per segment), host witness size and build time, and
one real GPU proof at H = 1,999. The mainnet figure is then an extrapolation, labelled so, from the named samples.
If serial execute is too slow, a later version proves block ranges in parallel and folds them (the file hash would
then be over per-range digests); not in v1.

## 7. Distribution and entry points

Three files per (network, H): `<net>-<H>.hints`, `<net>-<H>.undo`, `<net>-<H>.receipt` (succinct; Groth16 wrap
optional, Q7). Served by hazync.org and anyone else: the proof decides, not the server.

- Host: `zync-hints <out> <H> --bitcoin-cli "<cmd>"` builds the native file, the undo file and the witness (block
  data from the node, spender records from `history.bin`, states from prep), then proves; the guest's bytes must
  equal the native file's.
- CLI: `swiftsync-verify --hints <file> --hints-receipt <r> [--undo <file>] --expect <H>:<hash>`; prints the statement,
  non-zero exit on any failure.
- WASM `verify_hints(receipt, sha256, expect)` (the page hashes the ~100 MB file streaming); C ABI
  `swiftsync_verify_hints` for a node patch to call before `-utxohints`. Core itself is unchanged.

## 8. Tests

- **Real file:** mainnet H = 1,999 from the reference generator (`rustaceanrob/bitcoin` `hintsfile-v1`, and the
  crate) byte-equal to the zAPP's, and a #34004 file converted (§2) giving the same indices; BIP `elias_fano.json`
  through the guest encoder. Then H = 100,000 (BIP30 blocks, first same-block spends, OP_RETURN).
- **Refused by the verifier:** a flipped bit, a truncated file, a non-canonical re-encoding of one block; a file for
  H−1 against a journal for H; `--expect` from another chain; a receipt of another image (a core receipt, a
  dev-built hints id); an untrusted `core_id`; a FakeReceipt or dev mode.
- **Refused by the guest:** a spent leaf claimed unspent, a wrong `s`, a wrong tracked bit, a dropped or duplicated
  transaction, a mutated Merkle list, a wrong `undo_h` or `state_h`, a chain without U0 from block 1.
- **Undo:** the undo file equals `libzync-undo-hashes 1 H`; `bspent`-format coins expanded to U0 match `undo_ref.py`.

## 9. Open questions

- **Q1** (Robert Netzke) Bit order: the crate or the BIP vectors? And step 4.2's index reset, a typo?
- **Q2** (Netzke / Ruben) Should the file header carry the block hash at H? Today a file alone fits any fork.
- **Q3** Is U0 acceptable as the "hashes of the block inputs" file, or should the per-block hash be over the BIP's
  `Coin` (with its input index)?
- **Q4** Which H do providers publish (a fixed step, Core's assumevalid height)? One receipt per H.
- **Q5** (Ruben) His answer "5: Undecided": the question it answered is unknown. Author to fill in.
- **Q6** #34004's numbering as a second `format`, or hintsfile only?
- **Q7** Verifying in a C++ node: Groth16 wrap plus a small BN254 verifier, or the external CLI?
- **Q8** Is profile bit 6 (`live_hash`) in the current images (§3 alternative)? Signet first or mainnet first?
