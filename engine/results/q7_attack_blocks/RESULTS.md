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
  either a larger OpenVM address space (pointer_max_bits > 29: whether that is sound in OpenVM's 31-bit-field range
  checks is UNKNOWN, a question for the OpenVM team; it would also double the per-segment GPU memory image per bit)
  or a reclaiming allocator (an image change, #794's own conclusion, which RISC Zero needs for tap_full too).
- light_only's end state: **identical to RISC Zero's in 22 of 23 fields; only self_id (the image id) differs**, when
  both start from the same anchor. Found on the way: blocks 1..125 of each attack directory are symbolic links into
  blocks/main, which the pod bundle left out, so on the pod the host could not replay the BIP30 tree from genesis and
  SEEDED a demonstration anchor (smt_root 80,125,... instead of the replayed 34,121,...). The like-for-like reference
  is RISC Zero (dev mode, the same host build) from that same seeded anchor (block 126 alone): its state_126.json
  equals the engine's except self_id. The out-of-memory results do not depend on the anchor (they are per input).
