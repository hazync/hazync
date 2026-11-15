# zyncAPP: a proven SwiftSync hints file (design, #722)

Status: design APPROVED 2026-11-14; built on `feat/hints-zapp` (§10: what exists, measured, tested). Base: `feat/undo-commit`
(the U0 undo commitment, UNDO.md).

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
  format          u32       1 = hintsfile.md file version 0x00, the BIP test vectors' bit order (built);
                            2 = the same file with the hintsfile crate 0.2.0's bit order (built, for comparison)
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
byte string, even one that decodes to the same indices, is refused (the safe direction). The verifier decodes a file
only after its SHA-256 and length matched the receipt's, and every size is bounded by the bytes present before
anything is reserved (review H1: a 9-byte file naming H = 2^32 − 1 reserved ~103 GB and aborted).

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

**Alternative (profile bit 6).** The published images have it (46666b6f's profile is 113: bits 0, 4, 5, 6). With
`live_hash` (ECMH of the live coins) at the tip, step 3's spent leaves go: add each hinted-unspent coin to an ECMH and
require `J.live_hash`. Both are sound; the coin log is built because it works at any H ≤ T (`live_hash` only exists at
the tip) and needs no EC arithmetic.

## 4. Verifying the chain proof, and what a node checks

Inside: `env::verify(core_id, core_journal)`, then `facts::check_core_statement` (KIND_CHAIN, version ≥ 2,
`self_id == core_id`, the network, scripts checked, coin log) with the network's PINNED genesis anchor compiled into the
guest (so the statement itself says "from genesis"), then a zero lineage and a core image not in the core's own
`handover_pin::PRE_UNDO_IMAGES` (included, not copied).
`core_id` is an input, never compiled in (ZYNCAPPS.md §4).

The node (or the CLI for it) checks: the receipt verifies under the published hints image id (the `zyncapp-hints`
line of `reproduce/LIBZYNC_IDS`, rebuilt by `reproduce/libzync.Dockerfile`), not a FakeReceipt, `RISC0_DEV_MODE` unset
(refused inside `verify_hints`, so in every entry point); `core_id`, `lineage` and the whole `profile` are a published
core's; `anchor_id` the pinned
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

Three files per (network, H): `<net>-<H>.hints`, `<net>-<H>.undo`, `<net>-<H>.receipt` (succinct; no Groth16
wrap: no trusted setup, nodes verify with our library, CLI, WASM or C ABI). Served by hazync.org and anyone else: the proof decides, not the server.

- Host: `zyncapp hints-prove <block dir> <H> <core receipt> <out> [--format 1|2]` replays blocks 1..T natively from
  the block files (`fetch_block_rpc.py`), checks its block forest against the core state, writes the file and the undo
  file, runs the same derivation natively, then proves; the guest's statement must equal the native one.
- CLI: `zapp-hints [--expect <H>:<hash> | --mempool | --bitcoin-cli "<cmd>"] [--expect-tip <T>:<hash>] [--undo <file>] <receipt> <file>` (and
  `zyncapp hints-verify`, the same library call); exit 0 OK, 2 DEMO, 1 FAIL.
- WASM `verify_hints` / JS `verifyHints({receipt, file, undo, expect})` (the file is hashed on the device); C ABI
  `swiftsync_verify_hints` (zapps/hints/ffi, include/zapp_hints.h) for a node patch to call before `-utxohints`. One implementation,
  `zapp_hints::verify_hints`, in the zAPP's own module `zapps/hints` (CLI, `ffi/`, `wasm/` with `zapp-hints.js`; ZYNCAPPS.md §9), on the core verifier as a library: neither Core nor the core verifier changes.

## 8. Tests

- **Real file:** H = 1,999, 10,000, 100,000 byte-equal to `hints_ref.py` (an independent generator from hintsfile.md)
  in format 1 and to the `hintsfile` crate's writer in format 2; the BIP's `elias_fano.json` through both encoders.
  (`rustaceanrob/bitcoin` `hintsfile-v1` and #34004's `generatetxohints` need a node rolled back to H: not run on the
  shared node.)
- **Refused by the verifier:** a flipped bit, a truncated file, a non-canonical re-encoding of one block; a file for
  H−1 against a journal for H; `--expect` from another chain; a receipt of another image (a core receipt, a
  dev-built hints id); an untrusted `core_id`; a FakeReceipt or dev mode.
- **Refused by the guest:** a spent leaf claimed unspent, a wrong `s`, a wrong tracked bit, a dropped or duplicated
  transaction, a mutated Merkle list, a wrong `undo_h` or `state_h`, a chain without U0 from block 1.
- **Undo:** the undo file equals `libzync-undo-hashes 1 H`; `bspent`-format coins expanded to U0 match `undo_ref.py`.

## 9. Decisions and open questions

Decided by the author (2026-11-14): no Groth16 (Q7); BIP/crate numbering only, #34004 documented, not a format (Q6);
publish at heights we choose, any H is valid because each file is proved (Q4); Ruben's "undecided" was the refresh
cadence, full files or per-epoch deltas, not blocking (Q5); bit 6 is in the images, the coin log is used (Q8, §3);
mainnet first.

Asked of the maintainers, open: **Q1** the bit order (format 1 follows the BIP's vectors; a change is a new format id)
and step 4.2's index reset; **Q2** a block hash in the file header; **Q3** U0 as the input-hash file.

## 10. Built (2026-11-14)

**Code.** `libzync/src/hints.rs` (the derivation and the Elias-Fano encoder/strict decoder, shared by guest and host),
`prover/zyncapps/methods/hints` (the guest; published id `1894f12e…`, the `zyncapp-hints` line of `reproduce/LIBZYNC_IDS`),
`prover/zyncapps/host/src/hints_host.rs` + `hints-prove` / `hints-verify`, `zapps/hints/src/lib.rs` (the
verifier for CLI, WASM, C ABI), `scripts/libzync/hints_ref.py` (an independent generator written from hintsfile.md),
`prover/zyncapps/hints-crate-ref` (the `hintsfile` crate 0.2.0 as a second reference, test only).

**Input framing.** After the core receipt, journal, network flag, format, H, the genesis header and a profiling step:
per block one frame `[L, k, s] ‖ block (L bytes, header ‖ count ‖ transactions without witness) ‖ k tracked positions
‖ s status words (0 = unspent at H; height, tx, vin = its spender) ‖ state (8) ‖ undo (8)`; then `[T − H]` and the
block-forest leaves above H.

**Measured: CPU execute, no dev mode** (risc0 executor on hazync-coord; the core receipt an unresolved assumption whose
journal is the published host's `libzync-prep` state at T, the guest's every check run). User cycles:

| H (T) | user cycles | padded | segments | per block | per counted output | execute |
|---|---|---|---|---|---|---|
| 1,999 (1,999) | 27,422,130 | 30,932,992 | 30 | 13,718 | 13,442 | 0.8 s |
| 10,000 (10,000) | 135,017,419 | 152,043,520 | 145 | 13,502 | 13,343 | 3.4 s |
| 100,000 (100,000) | 2,389,822,475 | 2,724,724,736 | 2,599 | 23,898 | 9,044 | 52.8 s |

(The published image `1894f12e`. The local build before the review fixes measured within 0.1% of these, and H = 1,999
under T = 10,000 took 40,235,303.)

Per 1,000-block window (`--prof 1000`, cumulative `env::cycle_count()`): about 13.4-14.0 M while blocks hold about one
transaction (windows to 41,000); 22.3 M for the window ending 81,000 (1,856 transactions, 2,257 outputs, 2,224 inputs,
502 kB without witnesses). A leaf above H costs about 1,600 (12.8 M for 8,001). Two- and four-variable linear fits
over the 100 windows did not fit (relative error up to 0.27 and 0.54), so no per-input or per-byte rate is claimed;
modern blocks are not measured. No mainnet figure is given.

**Measured: real CPU proofs** (no dev mode, `ProverOpts::succinct`, `RAYON_NUM_THREADS=6`, a shared box). The core
receipt: the published host (`46666b6f`) proving blocks 1..3 from genesis, 1,061.5 / 980.3 / 943.3 s per block. The
hints receipt by the published image for H = 3 (T = 3): 182.0 s; H = 2 (T = 3, one leaf above H): 184.3 s; 225,202
bytes each (`zapps/hints/vectors`; CI verifies them, `zapps/hints/test.sh`). Blocks 1..3
have no spends: the paths with spends, same-block spends and the BIP30 blocks are covered by the executions above, not
by a proof. Proving the core further on CPU (~16 min a block here) was not attempted.

**Tested** (`scripts/libzync/hints_test.sh` at H = 1,999, 10,000 and 100,000: 26 of 26 each):
- the BIP's three Elias-Fano vectors; format 1 == `hints_ref.py` byte for byte (5,822 / 28,862 / 226,363 bytes);
  format 2 == the `hintsfile` crate's own writer on `hints_ref.py`'s indices, byte for byte, and the crate reads it back;
  counts and value equal (100,000: 264,251 indexed, 71,888 unspent, 499,990,000,000,000 sat, i.e. 5,000,000 BTC less
  the two overwritten 50 BTC coinbases) and equal the core state's `utxo_value`;
- refused by the guest: a spent coin claimed unspent, the reverse, a wrong spender, a wrong tracked bit, a flipped coin
  state, a flipped undo hash, a flipped nonce, the coinbase listed twice, the last transaction listed twice (block
  586, the same Merkle root: "mutated", CVE-2012-2459), a dropped and a swapped transaction, a trailing byte, a core
  lineage, a pre-undo core image, another core id, an unknown format, an assumevalid core, a flipped leaf above H;
- H = 1,999 under T = 10,000 gives the same file as under T = 1,999;
- the verifier on the real receipts (`hints_verify_test.sh` 11/11, `test-hints.mjs` 5/5, `test_hints.c` 4/4, unit
  test `hints::tests::real_receipts`): the proven file DEMO (3 blocks are below minimum work), refused: a flipped bit,
  a truncated file, a non-canonical re-encoding, the file for H = 2, a tampered undo file, another hash or height at H,
  a receipt of another image, an untrusted core image, `RISC0_DEV_MODE` set; H = 2 needs `--expect-tip` for T.

**Pinned** (review H2): `zyncapp-hints` `1894f12e…` from the reproducible build's steps on hazync-coord (the three
existing ids reproduced unchanged), with CI's `reproducible-image-id` as the second build; every tool trusts that id
(`swiftsync_verify::hints::HINTS_IMAGE_ID`; `zyncapp hints-verify` too).

**Not done.** A proof over a chain with spends; a cost for modern blocks; signet (the code takes `--signet`,
untested); `hints_test.sh` in CI (it needs block data and a prep state).
