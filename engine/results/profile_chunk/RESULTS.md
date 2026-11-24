# Where a chunk's proving goes (24.11.2026)

Block 976,379 chunk 0, keys-hz4 (native SHA + fat LTO), CPU proof with `hzengine --features profile`
(HZ_METRICS): trace cells by circuit (cells_chunk00_976379.txt). Function-level spans were not recorded (the
executable carries no function bounds); per-circuit cells are what the prover pays for.

| circuit | main trace cells | what it is |
|---|---|---|
| Poseidon2 periphery | 39% | hashing the memory pages each segment touches (memory Merkle), at every segment |
| RV32 load/store | 20% | ordinary memory instructions |
| RV32 ALU | 16% | ordinary arithmetic |
| SHA-256 block hasher | 5% | the SHA extension |
| shifts, branches, memory Merkle, boundary, program, secp256k1 field ops | the rest | |

Segments are cut by a 15 GiB per-segment memory cap (OpenVM default, `segmentation_max_memory`); chunk 0 is 7
segments. `HZ_SEG_MEM_GB` (prover-side, not in keys or commitments; image unchanged) gives 7 / 4 / 3 segments at
15 / 24 / 32 GiB. Levers: larger segments on 48 GB cards; a smaller memory footprint (the guest's bump allocator
never frees, so every allocation touches new pages).

## Allocator (24.11.2026, CPU, block 976,379, keys-hz4 = bump allocator)

| run | instructions | metered cost | cost / instruction | segments at 11 GiB (CPU metering) |
|---|---|---|---|---|
| chunk 0 | 101.2M | 4.25B | 42 | 21 |
| chunk 4 | 101.3M | 4.30B | 42 | 22 |
| trie | 88.6M | 6.31B | 71 | 34 |

A reusing allocator (OpenVM's `heap-embedded-alloc`, keys-hz5, image exe 6fc68157...) is refused by the guest's own
first check: the per-transaction caches in verify_input.cpp and Core's sighash cache (libzync/0101) are keyed by
buffer address, sound only while no address is handed out twice. So the memory lever is closed unless those caches
change, which would need a soundness review of its own. Not pursued.

(Segment counts here come from CPU metering. They don't match the GPU prover's counts for the same cap (chunk 0 was
8 segments on the A40), so compare them only with each other.)
