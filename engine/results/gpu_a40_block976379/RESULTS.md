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
The bench proved each part as ONE program with no recursion. Measured here (prove_block.log phase timings):
- A chunk proof is mostly app proving: chunk_01 (1.81B cost) = 4 segments, 12.3 s of segment proving + 0.5 s
  metered execution; the aggregation layers to one STARK are small (leaf 0.6 s, internal-for-leaf 0.3 s, two
  internal-recursive 0.35 s); about 4 s more per proof is host work (the journal-emitting execute pass, key
  transfer, memory partition): roughly 5 s fixed per proof, ~80 s over 16 chunks. GPU utilisation 50%.
- 7.2 ns/cost of segment proving here vs 5.9 on the A6000 (bench run 3): the A40 is the slower card (696 vs
  768 GB/s); not separated from other effects.
- In-guest verification of 17 proofs: aggregate 68.5 s (A40) vs 49.1 s without verification (bench, A6000): at
  most about 20 s, about 1.1 s per proof, part of it the slower card.
- The trie: 8.92B cost (the platform's sha2 already puts the update-list hash on the SHA extension, the bench's
  biggest trie lever); the RISC Zero-layout SHA shim is the remaining same-format lever (native SHA, in progress).

## Metered cost of each stream (CPU, `hzengine execute`, every journal byte-identical to RISC Zero's)
| stream | cost | instructions |
|---|---|---|
| 16 chunks | 28.68B (1.64-2.01B each) | 703M |
| trie | 8.92B | 156M |

Next levers, by size: fewer, larger chunks for throughput (the chunk count is the host's; ~5 s fixed per proof),
overlapping host work with GPU proving (utilisation 50%), native SHA, then the trie's new format (a commitment
change: needs a decision).
