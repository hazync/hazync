# Cycles on REAL mainnet state: libzync c14 vs the original CORE guest (2026-11-07)

**What is measured.** Proven cycles (segments × 2^20, the padded cycles a prover pays for) and, for libzync, user
cycles, per block, from EXECUTION in dev mode (the session is really executed; no proof is made). GPU card time and
dollars are measured separately, on one card type (paper plan).

- **libzync** (image bbf82800, frozen c14): `libzync-prove-range h h` as a LEAF from prep's `leaf_<h>.bin`, cut by
  `libzync-prep` from genesis on the coordinator box (REAL coin log, trie and BIP30 tree at h-1; the guest checks every
  witness against that state). One chunk per block (`HAZYNC_CHUNKS=1`); signature hints collected first, as on a
  prover. `LZ_STATS` lines (prover/host/src/lz.rs).
- **CORE** (the canonical guest 37987b85, the published host `/usr/local/bin/hazync-host` on the coordinator box):
  `prove-range-bridge h` from the original bridge's `bundle_<h>.json`, dev mode; the segment count from its
  "executed, N segments at po2 20" line (it then stops at assembly, which dev mode cannot do). CORE bundles exist for
  1..50,000, 126,433..418,268 and 967,500..974,932 only, so the head-to-head is limited to those ranges.

| block | CORE segments | CORE proven Mcycles | libzync segments (chunk + trie + agg) | libzync proven Mcycles | libzync user Mcycles | ratio (proven) |
|---|---|---|---|---|---|---|
| 1,000 | 3 | 3.1 | (prep not there yet) | | | |
| 200,000 | 2,011 | 2,108.7 | 104 (72 + 15 + 17) | 108.0 | 88.3 | **19.5x** |
| 300,000 | 411 | 431.0 | (prep not there yet) | | | |
| 400,000 | 3,770 | 3,953.1 | (prep not there yet) | | | |
| 967,600 | 5,292 | 5,549.1 | (prep not there yet) | | | |
| 970,000 | 8,737 | 9,161.4 | (prep not there yet) | | | |
| 972,000 | 10,294 | 10,794.0 | (prep not there yet) | | | |
| 974,000 | 10,981 | 11,514.4 | (prep not there yet) | | | |

**Block 200,000 against the modelled survey** (`cost/survey_c14.out`, coin log and trie MODELLED around the block):
chunk 61.30 M user cycles (identical: a chunk does not depend on the state), aggregate 14.35 M real vs 14.47 M
modelled, trie update **12.68 M real vs 10.46 M modelled (+21%)**; the block 88.32 M real vs 86.22 M modelled
(+2.4%). The whole-chain estimate's trie was modelled and called likely understated: on this block it is, by 21% of
the trie and 2.4% of the block.

**Not yet:** libzync at 1,000 (not a leaf height of this prep run), 300,000 (at the end of the slice), 400,000 and the
recent blocks (after the prep run passes them); then the same blocks on one GPU card type for card time and dollars.
The ratio here is proven cycles, CORE's padded segments against libzync's; card time is what the paper leads with
after cycles.

## GPU run 1 (2026-11-07): same card, real state, 8 blocks

ONE RunPod RTX PRO 6000 Blackwell Server Edition (97 GB; host AMD EPYC 9355, load ~10 from other tenants), the same
pod for both: CORE = the v0.22.1 CUDA release binary (37987b85, checksum-verified), `prove-range-bridge h` from
`bundle_<h>.json`; libzync = c14 (bbf82800, the pinned guest), `libzync-prove-range h h` as a LEAF from prep's
real-state `leaf_<h>.bin`, one chunk, signature hints collected on the CPU first. Wall time per block includes every
step each needs (libzync: hint collection on the CPU, chunk + trie + aggregate proofs; CORE: segments + assembly).
Both release builds use 2^21-cycle segments. Cost of the run: $5.71 (setup, build, 16 proofs). Raw: `run1/`.

| block | CORE s | libzync s | card-time ratio | CORE Mcycles (proven) | libzync Mcycles (proven) | cycle ratio |
|---|---|---|---|---|---|---|
| 200,000 | 1,805 | 120 | 15.1x | 2,028 | 102 | 19.9x |
| 210,000 | 769 | 102 | 7.5x | 875 | 84 | 10.4x |
| 220,000 | 690 | 125 | 5.5x | 780 | 109 | 7.2x |
| 230,000 | 137 | 50 | 2.7x | 151 | 29 | 5.3x |
| 270,000 | 774 | 129 | 6.0x | 891 | 113 | 7.9x |
| 280,000 | 393 | 84 | 4.7x | 447 | 66 | 6.8x |
| 290,000 | 1,245 | 185 | 6.7x | 1,430 | 174 | 8.2x |
| 300,000 | 362 | 82 | 4.4x | 411 | 63 | 6.5x |
| **all 8** | **6,176** | **877** | **7.0x** | 7,013 | 738 | 9.5x |

Reading: libzync needs 5-20x fewer proven cycles per block (9.5x over the 8); card time is 2.7-15x (7.0x over the 8),
lower than the cycle ratio because libzync's fixed per-block work (CPU hint collection, the trie and aggregate proofs,
process start) weighs more on small blocks: 230,000 (151 M CORE cycles) is 2.7x, 200,000 (2,028 M) is 15.1x. These
are 2012-2014 blocks; the modern blocks (970k+) are batch 2, when prep reaches them.
