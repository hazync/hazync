# libzync-bench — what a proving-native signature library buys on RISC Zero

**Measured 2026-10-28, laptop, r0vm 3.0.5 executor (execute only, no proving).** Branch `libzync`.

⛔ EXPERIMENT. Nothing here is consensus code or on any proving path. Batch verification is the
design in `docs/history/MSM_BATCH_VERIFY.md` §3, implemented here to measure it; it has not been
reviewed and would need to be before any use.

The question: with signatures at ~77% of a modern block's cycles, how much can a library built for
the zkVM (instead of libsecp) remove — and is the old "batch verification is a step too far for one
card" verdict still right at the batch sizes a redesigned pipeline could use?

## How to run

```
cargo build --release
./target/release/zbench-host ops 2000 50          # unit costs
./target/release/zbench-host msm 14031 2 9,10,11  # MSM; variant 0 unsigned, 1 signed, 2 raw
./target/release/zbench-host batch 7015 11 1 0    # batch vs single; last arg 1|2 = negative control
./target/release/zbench-host sethash 3000         # multiset hashes
```

Every section is bracketed by `env::cycle_count()` in the guest, so set-up (input decode, point
generation) is excluded. Correctness is checked, not assumed:

- the MSM result is compared against an **independent k256 computation on the host** at every size;
- every valid batch must pass, and two **negative controls** must fail — a flipped bit in one `s`
  with the honest R hint (caught by the MSM) and with an R re-derived from the bad `s` (caught by
  `x(R) == r`). Both rejected at n = 438 and at n = 50,000.

⚠ Cycles here are guest user cycles. Proving cost follows padded segment cycles, which ran
1.09x (batch, n = 7,015) to 1.57x (bare MSM, 50k points) higher, from paging and po2 rounding. And
`docs/BUILDS.md` §1 found cycles→GPU seconds optimistic by 14–36%. Treat ratios as the result.

## 1 · Unit costs (cycles)

| op | cycles | | op | cycles |
|---|---|---|---|---|
| Fq / Fr mul | 87 | | EC add (affine, coprocessor) | 268 |
| Fq add | 77 | | EC double | 248 |
| Fq / Fr inverse | 96 | | SHA-256, 32 B / 64 B message | 487 / 1,158 |
| sqrt by exponentiation | 37,595 | | sqrt as a checked hint | 347 |
| **ECDSA verify, `risc0-crypto` single** | **83,763** | | (Hazync GHOST ecmult term: 79,971) | |

**A new library alone buys nothing per signature.** risc0's own `risc0-crypto` verify costs the
same as GHOST's bigint2 ecmult. The win has to come from changing the algorithm.

## 2 · MSM: software overhead was 40% of it

The tree's existing `msm.rs` (via `risc0-crypto` point types) ran at **~420 cycles per point op**
against 268 in a tight loop. The profile showed **memcpy 35%** — `risc0-crypto` returns 72-byte
points by value — and PageIn 15%. `msm_raw` calls the coprocessor directly on `[u32; 16]` points
with ping-pong buffers, so a bucket update copies nothing:

| n = 2,000, best window | cycles / point | cycles / point op |
|---|---|---|
| `msm` (unsigned, as in the main tree) | 15,075 | 418 |
| `msm_signed` (half the buckets) | 14,161 | 434 |
| **`msm_raw`** | **9,094** | **279** |

`msm_raw` scales (256-bit scalars, best window):

| points | window | cycles / point |
|---|---|---|
| 877 | 8 | 10,395 |
| 2,000 | 9 | 9,094 |
| 14,031 | 12 | 7,041 |
| 50,000 | 13 | 5,984 |
| 200,000 | 15 | 5,340 |

The cost per point op stays at 270–280 cycles at every size: that is the coprocessor's own price,
and the floor for this design. Gains flatten past ~50k points.

## 3 · Batch ECDSA verification, real signatures

| signatures | window | single (cycles/sig) | **batch (cycles/sig)** | **speed-up** |
|---|---|---|---|---|
| 438 (one chunk today) | 8 | 83,428 | 24,232 | **3.44x** |
| 2,000 | 10 | 83,278 | 20,449 | 4.07x |
| 7,015 (block 962,000) | 11 | 83,281 | 18,494 | **4.50x** |
| 50,000 (a signature pool) | 13 | — | 16,194 | ~5.1x |

Where the 16,194 at n = 50,000 goes: **MSM 9,221 · per-signature prep 4,305 · transcript 2,668.**
Past ~10k signatures the MSM stops being the problem; the fixed per-signature work is 43% of it.

⏰ Not yet measured, cheap to try:
- **Transcript.** It currently hashes all 224 bytes of every signature. Q and the sighash are
  already fixed by the block the guest has hashed, so `H(block commitment ‖ R_i, r_i, s_i)` should
  bind the same thing at roughly one SHA block per signature (derived: ~−2k/sig).
- **Prep.** ~4.3k for an inverse, three muls, two on-curve checks and a SHA. Byte-order conversions
  and 72-byte `Affine` copies are most of it, as they were in the MSM. Derived: ~2k.
- **Schnorr (BIP340)** batches natively in the same MSM, with R lifted from x by a checked y hint
  (347 cycles). Only 191 of block 962,000's 7,015 signatures, so it was not measured separately.

## 4 · Multiset hashes for a SwiftSync-style aggregate (cycles per element)

| scheme | cycles | note |
|---|---|---|
| SHA-256 only (floor) | 1,355 | |
| salted SHA-256, summed mod 2^256 | 1,262 | SwiftSync's original; ⛔ needs a SECRET salt, which a public proof cannot keep |
| **ECMH on secp256k1** | **3,045** | try-and-increment, y and every skipped counter's non-residue proof hinted and checked; 2.01 tries/element |
| MuHash3072-style | 9,840 | SHA expansion + one 4096-bit coprocessor modmul (Core's MuHash uses ChaCha20, not SHA) |

**ECMH is the one to use:** about 2.2x the cost of hashing the element at all, and no salt needed.
Each coin is added once and removed once, ~6k cycles over its life. That is the same order as the
accumulator's ~2–3% share of today's chunk cycles: SwiftSync's gain is structural (no inclusion
proofs in the witness, no serial spine, any block in any order), not a per-block cycle cut.

## 5 · What it means for block 962,000 (DERIVED from the numbers above)

| | cycles | vs CORE |
|---|---|---|
| CORE (live) | 3.36 G | 1x |
| GHOST (measured, `docs/BUILDS.md`) | 1.20 G | 2.8x |
| GHOST + whole-block batch verify at 18.5k/sig, replacing ~80k+/sig | ~0.77 G | ~4.4x |
| … + cheaper transcript and prep (~12k/sig) | ~0.72 G | ~4.7x |

What is left after that is not signatures. The rest of the block (SHA and sighash, parsing,
memcpy, paging) becomes the next target. The cap on the chunk side was ~5.2x with signatures free.

**This changes `MSM_BATCH_VERIFY.md` §6's verdict on its own terms.** That verdict was "4.3x on the
term at chunk size, one card, not worth it". Measured:
- At chunk size (n = 438) the whole verify is **3.44x** cheaper, which is close to that estimate.
- At block size it is **4.50x**, and the §6 reopen condition, "a materially larger chunk size", is
  exactly what a signature pool gives.
- The 40% software overhead found in `msm.rs` applied to that estimate too.

## 6 · Other zkVMs (desk research 2026-10-28, NOT measured)

I found no published head-to-head of current zkVMs on secp256k1. Cycle counts cannot be compared
across VMs, because each one leaves its precompile work out of "cycles" in a different way. Only
proving seconds per signature on the same GPU would settle it.

**Shortlist, if we ever test one:**
- **SP1 Hypercube.** secp256k1 add, double and decompress precompiles, a patched `secp256k1` C crate,
  an open CUDA prover. Needs a ≥24 GB card (4090/5090). Independent figure: ~89k executor cycles
  per recovery, with ~760 precompile calls per signature not counted in that.
- **ZisK.** Native GLV ECDSA with hints, the cheapest self-reported Ethproofs cost (4×5090), an open
  GPU prover. Still alpha and in audit.
- **OpenVM 2.x.** An ECC extension with MSM, an open GPU prover, audited.

**Not shortlisted:** Airbender (no SHA-256 accelerator; its final proof needs ~150 GB RAM), Pico
(GPU prover not open), Jolt (alpha). RISC Zero's latest stable is 3.0.6 (5.0.0-rc.1 in pre-release).

**What this changes here:** the batch protocol above is plain guest code, so it carries to any of
these VMs. A fair comparison needs one rented 5090 for about an hour, proving the same 7,015
signatures single and batched on each VM. That cannot be done on this laptop: 8 GB RAM and no GPU.

## 7 · Utreexo hashing: ours vs the utreexod/Floresta format (measured 2026-10-30)

`zbench-host utreexo 2000 167,179,200,300`, laptop executor. Every chain is recomputed natively on
the host and must match (it did).

| hash | input | cycles |
|---|---|---|
| **ours** parent: SHA-256(0x01 ‖ L ‖ R), accelerated | 65 B | **929** |
| **standard** parent: SHA-512/256(L ‖ R), software | 64 B (1 block) | **14,530** |
| standard leaf: SHA-512/256(tag ‖ tag ‖ leaf data) | 167 / 179 / 200 B (2 blocks) | 29,302 / 29,298 / 29,294 |
| standard leaf | 300 B (3 blocks) | 43,016 |

**One SHA-512 compression costs ~14.5 k cycles, 15.6× an accelerated SHA-256 parent.** risc0 has no
SHA-512 accelerator; the hash is 64-bit arithmetic emulated on rv32.

**The hash is the smaller problem.** A Utreexo forest's roots depend on its add/delete HISTORY
(a deletion moves the sibling up), not only on the set it holds. utreexod's roots at H are the
result of every spend since genesis. Our design-B forest appends the coins that survive to H, in
creation order, and never deletes. So the two cannot agree even with the same hash and leaf format.
Matching utreexod means proving each spend's deletion against the full forest: an inclusion proof
plus the new roots, ~2 × (27 − log2 inputs) parent hashes per spent coin at ~170 M leaves.

ESTIMATE, not measured, for an illustrative block of 5,000 inputs and 6,000 outputs:
- each input: leaf 29.3 k + ~30 parents × 14.5 k ≈ 465 k cycles → ~2.3 B;
- each output: leaf 29.3 k + ~1 parent 14.5 k ≈ 44 k cycles → ~0.26 B;
- **~2.6 B cycles of Utreexo work, about 6× a whole current libzync block (~415–500 M).**
