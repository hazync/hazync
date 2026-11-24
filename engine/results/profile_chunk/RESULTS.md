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

## Function-level profile and the batch check's msm (24.11.2026, CPU, block 976,379, 5-chunk export)

The earlier runs had no function names because the guest executable was transpiled by a non-profiling `hzengine`,
which drops the ELF's function bounds. Transpiling with the `--features profile` build (and `GUEST_SYMBOLS_PATH` set)
keeps them; the image is unchanged (same exe commit). `scripts/prof_names.py <metrics>.folded <guest.syms>` names the
functions. Chunk 0 on keys-hz6 (functions_chunk00_hz6.txt): 91.0M instructions; by instruction cells (cells_used,
which carry the function stack):

| inclusive | function |
|---|---|
| 43.6% | Core's `verify_inputs_batch_once` (scripts, sighash, libsecp parsing) |
| 40.1% | `batch_ecdsa::finish` (the batch signature equation) |
| 30.8% | of which openvm-ecc-guest's generic `msm` |
| 37.9% | `memcpy`, self (most of it under the msm, moving 64-byte points) |

The curve circuit itself (FieldExpression, EcAddNe) is about 2% of cells: the msm's cost was RISC-V glue. keys-hz7
replaces it with `guest/msm_ovm.rs`: signed-digit Pippenger with every digit computed once, each point negated once,
in-place complete additions (`add_assign_impl`), one reused bucket buffer, window size from a cost estimate. Host
test (`guest/msm-test`) checks it against openvm's `msm` on random inputs and edge cases (zero/one/n-1 scalars,
identity, equal and opposite bases, exact cancellation); a broken carry fails it. `finish` also merges keys through a
sorted index instead of a BTreeMap. Same batch equation, same transcript, same result for every input.

| chunk | hz6 cost | hz7 cost | change | hz6 instr. | hz7 instr. | journal vs RISC Zero |
|---|---|---|---|---|---|---|
| 0 | 4.248B | 3.384B | −20.3% | 101.2M | 78.0M | identical (both) |
| 1 | 4.401B | 3.477B | −21.0% | 103.4M | 78.6M | identical |
| 2 | 3.741B | 2.820B | −24.6% | 88.7M | 64.2M | identical |
| 3 | 4.216B | 3.415B | −19.0% | 99.4M | 78.2M | identical |
| 4 | 4.307B | 3.898B | −9.5% | 101.4M | 90.5M | identical |
| sum | 20.91B | 16.99B | **−18.7%** | | | |

(msm_hz6_vs_hz7.txt. Metered cost, not GPU time; the GPU run is pending.)

### Second pass (keys-hz8) and the aggregate (24.11.2026)

After hz7 the chunk profile (instruction cells) still had the msm at 17% inclusive, a third of it in the 36 two-point
fallback checks (each allocating zeroed digit and bucket vectors). hz8: identity bases dropped once, bucket emptiness
tracked by a flag (a bucket add is one x comparison and one add intrinsic instead of five equality intrinsics and the
add), Straus on the stack for up to 4 points, and limbs/bytes reinterpreted instead of rebuilt in `hazync_batch_defer`.
The host test covers both paths; breaking the cancellation, the doubling or Straus's negation fails it.

| chunk | hz6 | hz7 | hz8 | hz8 vs hz6 | journal |
|---|---|---|---|---|---|
| 0 | 4.248B | 3.384B | 3.235B | −23.9% | identical |
| 1 | 4.401B | 3.477B | 3.338B | −24.2% | identical |
| 2 | 3.741B | 2.820B | 2.770B | −26.0% | identical |
| 3 | 4.216B | 3.415B | 3.389B | −19.6% | identical |
| 4 | 4.307B | 3.898B | 3.877B | −10.0% | identical |
| sum | 20.91B | 16.99B | 16.61B | **−20.6%** | |

The aggregate, measured on metering-only images (`bench-noverify`: same code, `env::verify` accepts without a
proof, so no child proofs are needed; its own image id): **5.36B** with hz7's batch module, **5.11B** with hz8's (−4.7%:
`Acc`, the live-coin hash's running sum, now adds in place and writes its limbs only when read). Both journals are
RISC Zero's except the 32 image-id bytes. The aggregate is now the largest single proof of the block and runs alone
after the leaves (hz8_and_agg.txt).
