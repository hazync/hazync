# The #794 attack blocks on the engine (26.11.2026, regtest, one A40)

Regtest engine images built with the fixed engine guest build.rs (586c1203: HAZYNC_REGTEST for the C half; the first
regtest build ran Core's mainnet params and the guest's own check refused it, 144 vs 2016): keys-rt (2^26 address
space, exe edcb8d0a) and keys-rt29 (OpenVM's default 2^29, exe 979f816f). Product host of the engine branch built
regtest in the glibc-2.35 container (rtc). pod/run_rig.sh, pod 728rss4avwkcsa. RISC Zero's results: /root/oomfix/runs
(23.11, dev mode, an older host).

| block | RISC Zero (23.11) | engine 2^26 (64 MiB) | engine 2^29 (512 MiB) |
|---|---|---|---|
| mem_k1140 (201 x CHECKSIG over a 9.8 kB scriptCode per input) | proved, 7 chunks | 1,008+ re-splits (pieces of ~27 inputs) before stopped | 36 pieces; 2 proved, 768 / 778 s each; largest 41.2B cost (976,379 whole: 25.8B); stopped (~2.6 h of A40 leaves) |
| light_only | proved | chunks fine, **aggregate out of memory** ("Maximum memory exceeded"; unsplittable) | **proved** (564 s), end state equal to RISC Zero's except the image id (see below) |
| tap_small (block 127: one big tapscript input) | proved | **single input out of memory** (#794) | **single input out of memory** (#794) |
| tap_full (block 127) | single input out of memory | single input out of memory | single input out of memory |

- The #794 re-split works on the engine as on RISC Zero: a chunk that runs out of guest memory is split and run again
  (OpenVM's "Maximum memory exceeded" recognised).
- **2^26 is too small for adversarial blocks**: it loses light_only (the aggregate cannot be split) and tap_small,
  both of which RISC Zero proves, and blows mem_k1140 up to ~1,000 pieces. Fine for a 4 GB-card test image, not for
  production with the current never-freeing allocator.
- **At 2^29 the engine still loses tap_small**, which RISC Zero proves. The guest memory limits, from source: the
  product guest's risc0-zkvm-platform 2.2.3 has GUEST_MAX_MEM = 0xC000_0000 (**3 GiB**); OpenVM v2.1.0's default
  MemoryConfig is 1 << 29 cells (**512 MiB**, openvm-platform MEM_BITS = 29). Parity on the worst single input needs
  the guest to use less memory: OpenVM's spec bounds pointer_max_bits by F::bits() - 2 = 29 for BabyBear
  (specs/openvm/isa.mdx), so 512 MiB is its ceiling. A reclaiming allocator (an image change, #794's own conclusion,
  which RISC Zero needs for tap_full too) or less copying per input.
- light_only's end state: **identical to RISC Zero's in 22 of 23 fields; only self_id (the image id) differs**, when
  both start from the same anchor. Found on the way: blocks 1..125 of each attack directory are symbolic links into
  blocks/main, which the pod bundle left out, so on the pod the host could not replay the BIP30 tree from genesis and
  SEEDED a demonstration anchor (smt_root 80,125,... instead of the replayed 34,121,...). The like-for-like reference
  is RISC Zero (dev mode, the same host build) from that same seeded anchor (block 126 alone): its state_126.json
  equals the engine's except self_id. The out-of-memory results do not depend on the anchor (they are per input).

## FINAL image: C-heap recycling + node format 2, 2^29 (26.11.2026 afternoon; roadmap 1.2 / 1.3)

Regtest image keys-rtch (exe 09dd8758, 2^29), the regtest product host at the same commit (built in the glibc-2.35
container), anchors regenerated in node format 2 (anchors2: blocks 1..125 replayed from genesis, profile 0xf9). One A40
(pod bt9qe3dkzq545o, pod/run_rig.sh). RISC Zero references: the SAME host build in dev mode from the same anchors (r0ref2).

| block | engine (final image) | RISC Zero (same host, C heap) | end state vs RISC Zero |
|---|---|---|---|
| light_only | **PROVED** (603 s) | ok | identical except self_id |
| tap_small (block 127: one big tapscript input) | **PROVED** (185 s) | ok | identical except self_id |
| tap_full (block 127) | **PROVED** (1,177 s) | ok | identical except self_id |
| mem_k1140 | to run on several GPUs (36 pieces of up to 41B each at 512 MiB, results above) | | |

So with #794's C-heap recycling the engine proves, at OpenVM's 512 MiB ceiling, every attack block it was tested on,
including the two that failed before (tap_small on the engine; tap_full on BOTH provers). self_id is the image id
(each prover's own), the only field that may differ.
