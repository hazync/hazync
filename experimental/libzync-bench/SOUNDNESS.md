# libzync soundness work (Phase 3), 2026-10-29

All of this ran on the laptop in execute mode, with no GPU spend. Status of each item:

| item | status |
|---|---|
| Negative tests: batch signature verification | ✅ done, all 6 behave as designed |
| BIP30 grandfathered blocks (91,842 / 91,880) on agg-lite | ✅ fixed and checked leaf for leaf |
| Coinbase-only blocks on agg-lite | ✅ **bug found and fixed** |
| SwiftSync handover check | ✅ **design B built and verified on blocks 1–200** (section 5) |

## 1. Batch verification: negative tests

**Setup:**
- Harness: `neg/tamper.py` + `neg/run.sh`, using the host command `exec-chunk <i>`, which executes one chunk and prints its
  batch verdict, `all_valid` and journal hash.
- Data: block 962,000, chunk 0 (405 hint records, kinds 0/1/3/4).

| test | tampering | batch | `all_valid` | journal |
|---|---|---|---|---|
| base | none | ok, 2 fallbacks | 1 | `e11c1595…` |
| T1 | flip a bit of a valid ECDSA hint's R (off-curve) | ok, 3 fallbacks | 1 | **same** |
| **T2** | valid ECDSA hint R → **−R**. Same x, so it passes the acceptance test | **FAILED** | **0** | rejected |
| **T3** | a genuinely FAILING check (CHECKMULTISIG key/sig mismatch) forged to flag 1 with an on-curve R where x(R) = r | **FAILED** | **0** | rejected |
| T4 | last 10 hints dropped | ok, 12 fallbacks | 1 | **same** |
| T5 | every flag 0 | ok, 405 fallbacks | 1 | **same** |
| T6 | Schnorr hint R → −R (odd y) | ok, 3 fallbacks | 1 | **same** |

- **The two real attacks (T2, T3) are caught by the batch equation.** These are hints built to pass every local check
  while being wrong.
  - In T3 the forged "true" changed how CHECKMULTISIG walked its keys, and every later hint fell out of step
    (397 fallbacks).
  - The chunk was still marked invalid.
- **Harmless tampering costs only speed:** the journal is byte-identical, so an untrusted hint provider cannot change
  what a chunk commits.

## 2. BIP30 grandfathered blocks on agg-lite

**Before:** agg-lite rejected 91,842 and 91,880 outright, because the full path's fix (a Utreexo delete of the superseded
coinbase coins) needs inclusion proofs agg-lite does not carry.

**Now:**
- **SwiftSync:** at those two block HASHES, the aggregate subtracts the superseded coin(s). They are recomputed from the
  block's own coinbase at the ORIGINAL height and creation MTP, both pinned as constants:
  - `BIP30_OLD_A = (91,812, 1,289,756,038)`;
  - `BIP30_OLD_B = (91,722, 1,289,721,830)`.
  The MTPs come from a node, as GetMedianTimePast of the block before.
- **Coinbase-txid SMT:** an overwrite claim is required exactly at those two blocks, the same rule as the full path. The
  host's block-JSON path now builds that witness (`smt_witness_standalone_at`).

**Checked, laptop, `host14` vs the round-13 guest:**

| block | round 13 | round 14 |
|---|---|---|
| 91,842 (BIP30, coinbase-only) | rejected | **valid**, ECMH adds=1 subs=1 |
| 91,880 (BIP30) | rejected | **valid**, ECMH adds=3 subs=2 |
| 91,812 / 91,841 (controls) | valid | valid |
| 962,000 regression | journal `98e2652d`, 326,652,314 cycles, ECMH `5e4a508f` | **identical**; aggregate 57.56 M cycles |

**Leaf for leaf:**
- 91,842 subtracts exactly the coinbase leaf 91,812 added: `e1f870b6…`.
- 91,880 subtracts exactly the one 91,722 added: `24697aea…`.

So the running total stays correct across both overwrites.

## 3. Bug found: coinbase-only blocks were rejected by agg-lite

**Cause:** with no non-coinbase transaction there are no chunk records, so the coverage check read `None => false`.
Every coinbase-only block failed, and that is most blocks before ~2011.

**Fix:** `None => true`. This is safe because the Merkle check binds the full txid list, so dropping every record of a block
that HAS transactions still fails there.

**Covered by:** 91,842 above.

## 5. SwiftSync design B, built and checked end to end

**Decision:** B, per the user, 2026-10-29. Design in `HANDOVER_DESIGN.md`.

**Hints format.** The published one, not a new one: **hintsfile v0**.
- Defined by the `hintsfile` crate 0.2.0 from 2140-dev/swiftsync, Robert Netzke's Rust SwiftSync node.
- Served at `utxohints.store`. The mainnet file there is 122.5 MB with stop height **970,000**.
- Layout:
  - magic `UTXO`, version 0, stop height (u32 LE);
  - then, per height 1..=H, an Elias–Fano list of the indices of outputs still unspent at H.
- Index rules:
  - outputs are counted per block, in tx/vout order, skipping OP_RETURN and scripts over 10,000 bytes;
  - the coinbase coins of 91,722 / 91,812 (BIP30) are skipped.
- That index order is exactly the guest's `output_leaves` order, so the files are consumed as published, unmodified.
  The host decodes them with the crate itself.

**Built:**
- **Guest, agg-lite:** each indexed output's hint bit routes it.
  - Bit 0 (spent by H) → ECMH total.
  - Bit 1 (unspent at H) → an **append-only Utreexo forest**, carried in `utxo_roots`.
  - A bit-1 coin spent in its own block → reject.
  - The bit count must equal the block's indexed outputs.
  - BIP30 now follows the file's convention: skip the old coinbases. The round-14 subtraction is gone.
- **Guest, `ChainState`:** carries `ss_set`/`ss_acc` (the ECMH total; it starts each block from the previous one,
  which the aggregate verifies recursively) and `ss_hints`, a running SHA-256 over (height, nbits, bits).
- **Host:**
  - `HAZYNC_SS_HINTS=<file>`;
  - `swiftsync-make-hints <H> <dir> <out>` writes a hintsfile v0 with the crate's own builder;
  - `swiftsync-sweep <H> <dir>` runs blocks 1..H in execute mode, feeding each aggregate the previous block's
    committed `ChainState`;
  - `HAZYNC_SS_FLIP=h:k` flips one bit (a test-only negative hook).

**Index alignment against the REAL published file.** Our file at H = 500 vs `utxohints.store`'s mainnet file at 970,000
(`swiftsync/efcheck.py`):
- every one of the 475 coins the published file marks unspent sits **at the same index** in ours (SUBSET_OK);
- the 28 differences are all coins unspent at 500 and spent later (e.g. block 170's change output).

**End to end, blocks 1–200, real chain-state carry.** METHOD_ID `76b98544…`.

| run | at H = 200 | forest | verdict |
|---|---|---|---|
| correct hints | **ss_set = 0: BALANCED** | **204 leaves = the 204 coins unspent at H** | ✅ SWEEP_OK; guest hint hash == host's |
| bit flipped: Satoshi's block-1 coin marked "spent" | ss_set = 1 | 203 | ❌ NOT BALANCED |
| bit flipped: block 9's coin (spent at 170) marked "unspent" | ss_set = 1 | 205 | ❌ NOT BALANCED |

Aggregate execution: 485.8 M cycles over the 200 blocks.

**What a verifier checks on the proof at H:**
- `height == H`;
- `ss_set == 0`;
- `ss_hints ==` the hash of the published file.

The forest roots then commit to exactly the UTXO set at H, ready for the tip phase.

**Longer sweep: blocks 1–500.** Same build. **BALANCED**, forest = **503 leaves = the 503 coins unspent at 500**,
hint hash == the host's. 1.22 B aggregate cycles.

**Checkable from the file alone (hint hash v2, round 16, METHOD_ID `038a6a31…`).**
- `ss_hints` now chains SHA-256 over (height, count, unspent indices): exactly what a hintsfile stores per height.
- `host swiftsync-file-hash <file>` computes the value from the `.hints` file and nothing else.
- Blocks 1–200 are BALANCED, and the proof's `ss_hints` = **`e33c66b3…` = `swiftsync-file-hash h200.hints`**.
- On the published mainnet file (`utxohints.store`, stop height 970,000): **`e215fe7e73298dd9dcd509de6c772f47704068485f187dc3fed158d793b1171d`**.
  That is the value a mainnet proof at 970,000 must commit for that file to be proven correct.

**For Ruben.** Given a Hazync proof at H, a client checks three things:
1. `ss_set == 0`;
2. `height == H`;
3. `ss_hints == swiftsync-file-hash(their .hints file)`.

If all three hold, the hints file is proven correct: no trust in whoever served it. The UTXO set at H is then the proof's
forest roots.

### Next for design B
- ✅ ~~Make `ss_hints` computable from the file alone~~: done (v2).
- **One real epoch on GPUs,** which needs credit. For example to 970,000, against the published file.
- **Laptop sweeps are slow,** ~14 s per block. Each aggregate execution loads the large guest image (ECMULT window 21
  tables). That costs nothing on a prover, but a fast execute path for sweeps would help testing.

## 4. Still open

- **Batch verification review, beyond these tests:**
  - an independent read of the transcript construction;
  - the sizing of the 128-bit weights;
  - the CHECKMULTISIG fallback path.
- **The new guest (METHOD_ID `8285c143…`) needs GPU proving** to confirm proving time is unchanged: chunk cycles are
  identical, so it should be.
