# libzync: making the SwiftSync path sound (handover design, for decision)

**Status:** proposal, 2026-10-29. Nothing below is built yet.

## The gap today

- In agg-lite, every block proof computes a SwiftSync delta: created coins ADDED, spent coins SUBTRACTED, as an
  ECMH point. The delta is **logged and thrown away**: it is not committed in the chain state, and nothing ever
  compares it to a UTXO set.
- Agg-lite also does no Utreexo work, so the chain state's UTXO roots pass through unchanged.

So a proof of blocks 0..H says nothing about which coins exist, and a prover could spend coins that never
existed. SwiftSync's guarantee only appears once the running total is **checked** at a handover height H.

## Two ways to close it

### A. A separate handover pass

- Carry the ECMH running total in `ChainState`.
- At H, a separate program reads the whole UTXO set S_H, sorted (a set, no duplicates), and computes:
  - ECMH(S_H), asserted equal to the carried total;
  - fresh Utreexo roots over S_H, for the tip phase.
- **Cost:** every live coin once, about 180 M leaves.
  - Each needs a hash-to-curve plus Utreexo hashing: on the order of **0.5–1 T cycles per handover**.
  - That is roughly **150–300 PRO 6000 card-hours, or $400–750**, every time H moves.
- **Also needed:** an ordering convention, plus a split/merge protocol for the forest across chunks.

### B. SwiftSync proper, plus an append-only Utreexo of the survivors (recommended)

Ruben's SwiftSync hints file gives one bit per created output: is it still **unspent at H**? During the sweep of
blocks 0..H, each block proof does:

| event | today | under B |
|---|---|---|
| output created, bit = 0 (spent before H) | ECMH add | **ECMH add** |
| output created, bit = 1 (unspent at H) | ECMH add | **append to an append-only Utreexo forest** |
| input spends a coin | ECMH sub | **ECMH sub** |

- **The chain state carries:**
  - the ECMH total;
  - the append-only forest roots (the existing `utxo_roots` field, which agg-lite already passes through);
  - a running hash of the hint bits.
- **At H, the last block's proof asserts:**
  1. **ECMH total == identity.** Every coin added as "will be spent" was spent exactly once, and nothing else was
     spent. This is SwiftSync's own check.
  2. **The hint-bit hash == the hints file's hash.** It binds the proof to one published file.
- **Why that is sound:**
  - A coin marked unspent but actually spent: its spend subtracts a point that was never added, so check 1 fails.
  - A coin marked spent but actually unspent: it was added and never subtracted, so check 1 fails.
  - So the forest holds **exactly S_H**: every surviving coin, once, in creation order.
- **What it gives:**
  - **For the tip phase:** S_H as Utreexo roots, at no extra cost. The tip continues from those roots with
    ordinary Utreexo, deletes included. The tip bridge rebuilds the same forest deterministically from the chain
    plus the hints file.
  - **For Ruben:** a proof that a specific hints file is correct for the chain up to H. That is what he asked for:
    "who regenerates the hints file" becomes "anyone, and here is the proof". No-scripts (sweep 1) proofs are
    enough for him, since the accounting is the same.
- **Cost:**
  - Per block, one append (≈ 2 SHA-256 compressions amortised) replaces one ECMH add (a hash-to-curve plus a point
    add) for every surviving coin. So it is **cheaper** per block than today's accounting, and there is **no
    separate handover pass**.
  - The extra input is 1 bit per output.

### The catch with B

- The hints file for H must exist **before** the sweep of 0..H is proved. So epochs are proved after H is in the
  past, which is exactly the epoch model we discussed: SwiftSync up to H, then Utreexo at the tip.
- Moving H means re-proving the sweep, or proving a new epoch. Per-block H movement is not possible, as before.

## What I would build (on the libzync branch, laptop only)

1. **`ChainState`** gains the ECMH total (point plus identity flag) and the hint-bit hash. Agg-lite updates the
   append-only forest instead of passing `utxo_roots` through.
2. **Hint input:** each chunk receives its outputs' bits; the aggregate already sees every created output via the
   chunk journals.
3. **A `handover H` assertion** on the block at H: ECMH == identity and the bit hash equals the given file hash.
4. **Host:** generate the hints file from a node (spentness at H), and a negative test for each failure: wrong bit
   in either direction, a dropped spend, a wrong file hash.
5. **Measure** on a range of early blocks on the laptop (execute mode) and on 962,000.

## Decisions needed from you

- **A or B.** I recommend B.
- **Hints-file format.** Ruben's own spec if he has one, or 1 bit per output in block order (with tx/vout order
  defined) with a header naming H and the tip hash. Agreeing it with Ruben now avoids a second format later.
- **Epoch boundary for a first real run.** For example H = a recent height, then measure one epoch's sweep
  (scripts off) end to end.
