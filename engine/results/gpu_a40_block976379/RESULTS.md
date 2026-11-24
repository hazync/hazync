# Block 976,379 on the engine, one NVIDIA A40, in-guest recursion (24.11.2026)

Pod e7vlerdagapues (A40 48 GB, secure, $0.59/hr), 18 min incl. setup, ≈ $0.18. `hzengine prove-block` over the product
host's `engine-export` of the block (16 chunks, HAZYNC_PROFILE_LAYOUT=cost), engine guest = the product guest's source
(engine/guest), keys `keys-hz2`. No A6000 was free on secure cloud; the A40 is the same GA102 chip with less memory
bandwidth (696 vs 768 GB/s), so an A6000 should be a little faster.

| part | proved (full STARK each) | journal vs RISC Zero (same stream) |
|---|---|---|
| 16 chunks (scripts + signatures) | 285 s total, 14.9-20.4 s each | all 16 byte-identical |
| trie update (mode 17, product format) | 54.1 s | byte-identical |
| aggregate, verifying all 17 proofs in-guest | 68.5 s | differs in exactly 32 bytes: `self_id` (the engine image id) |
| **block** | **409 s ≈ $0.067** | final proof verifies (`hzengine verify`: STARK, public values = sha256(journal), image) |

RISC Zero, same block, the product (v0.26.0) on one A6000: 1,658 s ≈ $0.32 (`gpu_a6000_run2`).
=> **4.1x less GPU time, 4.8x less cost per block**, with every proof of the block checked inside the final one.

GPU utilisation averaged 50%.

## Against the bench port's 224 s (A6000, experiment/fresh-cost-model REVIEW.md)
The bench proved each part as ONE program with no recursion. What the real engine adds, measured here:
- Every chunk is its own proof wrapped to one STARK (the form verify-stark checks): about 7 s fixed per proof
  (chunk_01: 1.81B cost ≈ 10.7 s of app proving at the bench's 5.9 ns/cost, 18.0 s measured). 16 chunks cost
  about 110 s of wrapping. Fewer chunks trade latency for throughput; the chunk count is a host setting.
- In-guest verification of 17 proofs: aggregate 68.5 s (A40) vs 49.1 s without verification (bench, A6000): at most about 20 s, about 1.1 s per proof, part of it the slower card.
- The trie is the product's mode-17 code (13.6B cost); the bench's 36 s used the faster trie variant
  (experiment/fresh-trie), not yet ported into the engine.
