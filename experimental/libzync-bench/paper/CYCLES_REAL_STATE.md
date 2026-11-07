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
