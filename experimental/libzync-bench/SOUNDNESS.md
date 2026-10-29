# libzync soundness work (Phase 3), 2026-10-29

All of this ran on the laptop in execute mode, with no GPU spend. Status of each item:

| item | status |
|---|---|
| Negative tests: batch signature verification | ✅ done, all 6 behave as designed |
| BIP30 grandfathered blocks (91,842 / 91,880) on agg-lite | ✅ fixed and checked leaf for leaf |
| Coinbase-only blocks on agg-lite | ✅ **bug found and fixed** |
| SwiftSync handover check | 📝 design ready for decision: `HANDOVER_DESIGN.md` |

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

## 4. Still open

- **The SwiftSync handover check:** without it the SwiftSync path proves nothing about which coins exist. The design is
  in `HANDOVER_DESIGN.md`, recommending SwiftSync proper plus an append-only Utreexo of the survivors. It needs a
  decision on the approach and on the hints-file format.
- **Batch verification review, beyond these tests:**
  - an independent read of the transcript construction;
  - the sizing of the 128-bit weights;
  - the CHECKMULTISIG fallback path.
- **The new guest (METHOD_ID `8285c143…`) needs GPU proving** to confirm proving time is unchanged: chunk cycles are
  identical, so it should be.
