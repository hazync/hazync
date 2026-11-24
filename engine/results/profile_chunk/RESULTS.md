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
