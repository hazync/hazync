# Result: the v2 cost curve, block 170 to 976,000 (image 46666b6f): execute-mode cycles + real GPU proving times

2026-11-20. §1-§4: execute-only cycle counts on hazync-coord's CPU with the published v0.27.0 host (no proof made).
§5-§6: 11 of those blocks PROVED for real on one RTX PRO 6000 with a clean build of the same v0.27.0 source (the
published CUDA asset is broken, §5), and the cycles -> card-time rate fitted from them. §7: the whole-chain
PROJECTION, arithmetic shown. Commands, hashes and settings: [`README.md`](README.md). Everything in §1-§6 is
**MEASURED**; §7 is a projection. "A" = real mainnet state (prep's leaf bundle); "B" = the block is real, the
trie update and the aggregate run against a MODELLED state. Cycles: **user** = executed; **padded** = sum of 2^po2
over segments at po2 21, what a GPU prover pays for. Shipped chunking (2 chunks, cost-packed) throughout.

## 1. Checks (is each run the real one?)

- **(A) reproduces prep's state, 9/9.** The state each leaf execution commits (decoded from the dev-mode aggregate
  journal) equals prep's `state_<h>.json` in all 23 fields except `anchor_id`, at 170, 1,000, 100,000, 200,000,
  270,000, 280,000, 290,000, 300,000 and 400,000 (`raw/runA/leaf_<h>.log`, line `STATE_MATCH`). `anchor_id` differs
  by construction: a leaf is a base step on S(h-1). Every guest run committed (no refusal); the guest checked every
  witness against S(h-1).
- **Chunks do not depend on the state:** at 100,000, 200,000, 300,000 and 400,000 the A and B chunk cycles are
  identical to the cycle (e.g. 400,000: 276,375,345 in both).
- **The setup reproduces the published profile:** block 962,000 run exactly as `../profiles/run.sh` (repo fixture,
  ONE chunk, po2 20) gave chunk 352,092,950, trie 47,599,783, aggregate 66,434,430 cycles and aggregate journal
  `d2a9ee3d…0cfad`, all identical to `../profiles/RESULT.txt` (`raw/repro/agg.log`).
- **Profiles are the executions they are named after:** each profile's non-paging total equals the host's printed
  count to within 56 cycles (e.g. A400000 trie 65,506,572 vs 65,506,600).
- The fetched block 962,000 has transactions and prevouts identical to the repo fixture.

## 2. The curve (MEASURED; B rows: trie and aggregate MODELLED)

Block content from `getblockstats` (weight and outputs exclude the coinbase; inputs exclude the coinbase input).
Segments = chunks + trie + aggregate at po2 21. "trie+agg" = share of user cycles. Card s (R1) is the pre-GPU
projection per block (R1, §6); the measured rate R4 gives 2-7% more. CPU wall = this box (load 25-34), whole leaf run (A) or
hints + agg-exec-fake (B).

| height | | txs | inputs | outputs | weight WU | chunks M | trie M | aggregate M | **total user M** | padded M | segments | trie+agg | k cycles/input | card s (R1, projection) | CPU wall s |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 170 | A | 2 | 1 | 3 | 1,100 | 1.2 | 0.1 | 2.6 | **3.8** | 5.5 | 1+1+2 | 70% | – | 29 | 43.9 |
| 1,000 | A | 1 | 0 | 1 | 0 | 0.0 | 0.1 | 2.6 | **2.6** | 3.4 | 0+1+2 | 100% | – | 27 | 33.6 |
| 100,000 | A | 4 | 3 | 6 | 2,964 | 2.4 | 0.2 | 2.7 | **5.3** | 7.9 | 2+1+2 | 54% | – | 31 | 57.2 |
| 150,000 | B | 10 | 16 | 18 | 14,200 | 3.5 | 0.4 | 2.9 | **6.8** | 9.4 | 2+1+2 | 49% | – | 32 | 56.0 |
| 200,000 | A | 388 | 1,231 | 822 | 989,032 | 63.6 | 12.7 | 14.6 | **91.0** | 104.3 | 35+8+9 | 30% | 73.9 | 121 | 71.4 |
| 250,000 | B | 156 | 493 | 307 | 381,064 | 25.2 | 5.0 | 7.9 | **38.1** | 45.4 | 15+3+5 | 34% | 77.3 | 66 | 64.2 |
| 270,000 | A | 616 | 1,213 | 1,331 | 993,684 | 62.5 | 17.4 | 19.2 | **99.1** | 117.4 | 35+10+11 | 37% | 81.7 | 133 | 65.4 |
| 280,000 | A | 240 | 660 | 746 | 517,276 | 34.7 | 10.7 | 13.4 | **58.8** | 69.2 | 20+6+8 | 41% | 89.1 | 88 | 59.6 |
| 290,000 | A | 355 | 1,528 | 2,626 | 1,326,912 | 89.4 | 36.8 | 28.1 | **154.4** | 182.5 | 50+21+16 | 42% | 101.0 | 194 | 73.6 |
| 300,000 | A | 237 | 628 | 673 | 514,288 | 34.9 | 10.6 | 10.8 | **56.3** | 65.1 | 20+6+6 | 38% | 89.6 | 84 | 59.4 |
| 350,000 | B | 1,795 | 5,210 | 4,502 | 3,983,764 | 258.8 | 61.0 | 61.0 | **380.8** | 455.1 | 147+35+35 | 32% | 73.1 | 448 | 124.4 |
| 400,000 | A | 1,660 | 4,806 | 3,881 | 3,795,052 | 276.4 | 65.5 | 64.1 | **406.0** | 482.3 | 158+38+37 | 32% | 84.5 | 473 | 113.4 |
| 450,000 | B | 2,156 | 5,085 | 4,332 | 3,998,560 | 321.8 | 75.5 | 70.7 | **467.9** | 556.5 | 183+43+41 | 31% | 92.0 | 543 | 127.8 |
| 500,000 | B | 2,701 | 4,370 | 6,269 | 3,991,605 | 348.2 | 78.6 | 70.9 | **497.7** | 590.9 | 197+45+41 | 30% | 113.9 | 575 | 136.1 |
| 550,000 | B | 2,748 | 5,053 | 5,894 | 3,991,551 | 343.8 | 67.5 | 61.2 | **472.6** | 558.9 | 193+39+35 | 27% | 93.5 | 545 | 124.6 |
| 600,000 | B | 1,925 | 3,761 | 4,120 | 2,847,448 | 275.3 | 48.5 | 46.9 | **370.6** | 434.1 | 153+28+27 | 26% | 98.5 | 428 | 106.9 |
| 650,000 | B | 1,089 | 8,823 | 2,992 | 3,991,701 | 471.8 | 117.7 | 99.0 | **688.5** | 843.7 | 280+67+57 | 31% | 78.0 | 810 | 161.2 |
| 700,000 | B | 1,276 | 6,342 | 6,156 | 3,996,622 | 389.4 | 113.8 | 88.1 | **591.3** | 709.9 | 225+65+50 | 34% | 93.2 | 686 | 154.1 |
| 750,000 | B | 2,496 | 6,423 | 8,246 | 3,991,924 | 369.8 | 125.0 | 97.4 | **592.2** | 711.5 | 214+72+56 | 38% | 92.2 | 687 | 154.5 |
| 800,000 | B | 3,721 | 4,911 | 12,103 | 3,991,889 | 342.9 | 97.6 | 96.1 | **536.6** | 638.6 | 193+57+55 | 36% | 109.3 | 619 | 164.6 |
| 850,000 | B | 3,163 | 6,086 | 10,512 | 3,991,888 | 351.5 | 118.8 | 103.0 | **573.2** | 687.9 | 202+68+59 | 39% | 94.2 | 665 | 156.2 |
| 900,000 | B | 1,562 | 4,386 | 3,904 | 3,128,543 | 228.5 | 50.7 | 53.2 | **332.4** | 396.5 | 130+29+31 | 31% | 75.8 | 393 | 101.8 |
| 950,000 | B | 639 | 1,002 | 1,561 | 889,711 | 63.4 | 19.7 | 19.1 | **102.2** | 121.1 | 37+11+11 | 38% | 102.0 | 136 | 67.0 |
| 962,000 | B | 6,304 | 8,006 | 12,853 | 3,995,336 | 355.5 | 47.6 | 66.4 | **469.6** | 556.8 | 199+28+39 | 24% | 58.7 | 543 | 125.1 |
| 976,000 | B | 4,003 | 6,838 | 12,956 | 3,991,895 | 405.2 | 156.7 | 129.3 | **691.2** | 832.6 | 233+90+74 | 41% | 101.1 | 800 | 181.5 |

Dispersion (B), five consecutive blocks after 800,000: 597.5, 566.8, 627.1, 590.9, 560.9 M user (all ~3.99 M WU,
5,476-6,637 inputs); 800,000 itself 536.6 M. Full tables with segments and padded counts: `raw/table_a.tsv`,
`raw/table_b.tsv`. Peak RSS: 0.6-1.15 GB.

## 3. Real vs modelled state (A vs B at the same block)

| block | trie A / B (M) | aggregate A / B (M) | block total A / B (M) |
|---|---|---|---|
| 100,000 | 0.151 / 0.154 (-2%) | 2.742 / 2.695 (+1.7%) | 5.34 / 5.30 (+0.8%) |
| 200,000 | 12.68 / 10.46 (**+21.3%**) | 14.64 / 14.76 (-0.8%) | 90.95 / 88.85 (+2.4%) |
| 300,000 | 10.63 / 10.32 (+3.1%) | 10.78 / 10.67 (+1.1%) | 56.29 / 55.86 (+0.8%) |
| 400,000 | 65.51 / 61.66 (+6.2%) | 64.13 / 62.83 (+2.1%) | 406.01 / 400.87 (+1.3%) |

The model understates the trie by 3-21% and the aggregate by up to 2%; the block total by 0.8-2.4%. Applying the
400,000 ratio (trie+aggregate x1.041) to every B row moves the whole-chain total by +1.3% (§5, "B adj"). The model
also undersizes the coin log: k = 1.64 x cumulative transactions gives 184.5 M leaves before 400,000, prep's real
state has 286.7 M (1.55x; 1.13x at 200,000, 1.34x at 300,000), about 0.6 extra tree levels; the aggregate is +2.1%
there. Whether the trie error grows towards the tip is not measured (prep has not reached 450k).

## 4. Where the cycles go (MEASURED, `prof_split.py`, proved parts only = chunks + trie + aggregate)

Per category, % of non-paging cycles (`raw/decoded.txt`; 962,000 from `../profiles/RESULT.txt`, one chunk):

| | 200,000 (A) | 400,000 (A) | 700,000 (B) | 962,000 (B, 1 chunk) | 976,000 (B) |
|---|---|---|---|---|---|
| signatures (libsecp + batch ECDSA) | 27.7% | 34.7% | 30.3% | 21.8% | 24.4% |
| state (coin log, trie, ECMH, BIP30) | 27.4% | 30.3% | 32.2% | 23.4% | **38.9%** |
| hashing | 24.1% | 15.1% | 19.6% | 22.8% | 17.1% |
| script interpreter | 7.0% | 7.7% | 6.6% | 7.2% | 4.9% |
| tx parse / serialise | 3.4% | 3.0% | 3.0% | 8.8% | 4.6% |
| guest main / I/O | 6.5% | 5.9% | 5.5% | 9.7% | 6.9% |
| other | 3.9% | 3.4% | 2.7% | 6.3% | 3.2% |
| paging (% of profile total) | 11.5% | 14.5% | 15.4% | 18.0% | 15.6% |

(700,000 and 976,000: the state row includes the MODELLED trie and aggregate.) Not proved but run by every prover on
its CPU: the signature-hint collect runs execute the chunks once more with full libsecp verification (A400000: 407.0
+ 390.0 M vs 276.4 M proved) and the aggregate collect run 619.4 M at 400,000 (`raw/prof/A400000.{0,1,4}.pb`).

## 5. GPU proving, MEASURED (2026-11-20, one RTX PRO 6000, real proofs)

**Build provenance.** The published v0.27.0 Linux CUDA asset (`hazync-host-x86_64-linux-gnu-cuda`, sha256 `b42cf87d…`)
is **broken**: it segfaults on its first GPU step because the release was built from stale objects. These times are
therefore from a **clean build of the unchanged v0.27.0 source**: same image `46666b6f…`, host sha256
`df57ca2d14b062636a1c4437dad016550438f3a03e55625432db8a6dc264a187`, built on hazync-coord `/root/fix739/dist`, the
binary v0.27.1 is to ship. The coordinator ran it (my pod create was blocked by the permission system).

Run setup:
- RunPod, SECURE cloud, one NVIDIA RTX PRO 6000 Blackwell Server Edition (driver 595.112.07, 97,887 MiB); host CPU
  AMD EPYC 9555, 256 threads.
- `gpu/pod_run.sh`, with its download step replaced by a sha-checked push of that host. Not dev mode.
- A warm-up prove of leaf 170 first, then every block in its own process. Shipped 2 chunks, po2 21.
- All 18 steps exited 0 on the first try. Pod 16:48:22-17:38:28 UTC; **spent $2.22** ($25.86 -> $23.64 balance);
  terminated, 0 pods listed after (`raw/gpu/control.log`).

**Verification** (hazync-coord, Ubuntu 24.04 / glibc 2.39, the v0.27.0 release verifier
`hazync-verify-x86_64-linux-gnu`, sha256 `62bf0c7d…`, `--network mainnet --product`, `--expect <h>:<hash from the
archive node>`; `gpu/verify_all.sh`, `raw/gpu/verify_*.log`, `raw/gpu/verify_summary.txt`):

- **9/9 A leaves: `SWIFTSYNC_VERIFY_DEMO`** with `--anchor` = the leaf's own S(h-1). DEMO is the expected verdict
  from a given anchor: below the minimum chainwork, and before the pinned checkpoint at 970,000. In every case:
  - the receipt is a real proof (not a FakeReceipt), verified against image 46666b6f;
  - the block hash matches the node's (`--expect ok`);
  - the anchor equals prep's `state_<h-1>.json`, and the committed state equals prep's `state_<h>.json`
    (`STATE_MATCH`, all fields but `anchor_id`).
- **700,000 and 976,000 (B): `SWIFTSYNC_VERIFY_FAIL` on exactly one check, "anchor_id is genesis".** That fails by
  construction: `agg-chunks` proves on a synthesised anchor, which was not saved. Every other check passes: real
  proof, image, profile 0x71, one forest leaf per block, and `--expect` matches the block hash. `agg-chunks` itself
  verified both receipts on the pod (`succinct receipt VERIFIED`). The chunk receipts are its assumptions.
- Receipt sizes: leaves 227,618-231,698 B, B blocks 231,226 / 231,234 B. Chunk receipts (.hzk, not committed):
  2,095,298 + 1,342,698 B at 700,000 and 3,082,626 + 2,329,970 B at 976,000. Block receipt hashes are in
  `raw/gpu/receipts.sha256`.
- The proofs ran the cycles the execute runs counted: every leaf's `LZ_STATS` user total equals §2's to the cycle,
  and the B segment counts equal §2's (asserted in `gpu_fit.py`).

Per block and part (`gpu_fit.py`, `raw/gpu_fit.out`; seconds wall, segments at po2 21). A parts come from the host's
`times.tsv`: "CPU hints" is the signature-hint collect, "trie" includes the aggregate's collect run, and aggregate =
the rest. B rows are separate processes (`libzync-hints`, `prove-chunk` x2, `agg-chunks`, whose log gives the trie's
own seconds):

| block | | total s | CPU hints | chunks s (seg) | trie s (seg) | aggregate s (seg) | user M | padded M | s / padded Mcycle |
|---|---|---|---|---|---|---|---|---|---|
| 170 | A | 29.0 | 5.9 | 7.8 (1) | 6.3 (1) | 9.0 (2) | 3.8 | 5.5 | 5.27 |
| 1,000 | A | 20.7 | 0.0 | 0.0 (0) | 12.1 (1) | 8.6 (2) | 2.6 | 3.4 | 6.07 |
| 100,000 | A | 36.7 | 5.9 | 15.0 (2) | 6.4 (1) | 9.4 (2) | 5.3 | 7.9 | 4.67 |
| 200,000 | A | 129.4 | 8.8 | 77.4 (35) | 21.3 (8) | 21.9 (9) | 91.0 | 104.3 | 1.24 |
| 270,000 | A | 139.3 | 8.5 | 76.8 (35) | 27.1 (10) | 26.9 (11) | 99.1 | 117.4 | 1.19 |
| 280,000 | A | 95.2 | 7.6 | 48.5 (20) | 18.6 (6) | 20.5 (8) | 58.8 | 69.2 | 1.38 |
| 290,000 | A | 203.7 | 9.1 | 106.7 (50) | 51.1 (21) | 36.8 (16) | 154.4 | 182.5 | 1.12 |
| 300,000 | A | 89.8 | 7.2 | 47.1 (20) | 18.1 (6) | 17.4 (6) | 56.3 | 65.1 | 1.38 |
| 400,000 | A | 496.6 | 17.7 | 316.2 (158) | 86.7 (38) | 76.0 (37) | 406.0 | 482.3 | 1.03 |
| 700,000 | B | 738.4 | 33.1 | 451.7 (225) | 134.4 (65) | 119.2 (50) | 591.3 | 709.9 | 1.04 |
| 976,000 | B | 847.5 | 39.5 | 454.7 (233) | 183.5 (90) | 169.8 (74) | 691.2 | 832.6 | 1.02 |

The warm-up (leaf 170) took 29.1 s, the same as the measured 170, so there was no kernel JIT on this build. The 11
blocks together took 2,826 s = 0.785 card-hours. 700,000 and 976,000 prove their trie and aggregate against a
MODELLED state; their trie and aggregate seconds are a modelled state's.

## 6. The cycles -> card-time rate on 46666b6f (MEASURED fit)

Least-squares fits of block wall time against padded cycles (`gpu_fit.py`):

| fit | fixed s per block | s per padded Mcycle | residuals |
|---|---|---|---|
| **R4: all 11 blocks (the new headline rate)** | **23.77** | **0.9940** | -6.6..+9.0 s |
| small (A, up to 300,000) | 24.05 | 0.9929 | -6.7..+4.8 s |
| big (400k, 700k, 976k) | 12.77 | 1.0096 | -5.8..+8.9 s |
| per user Mcycle, all 11 | 22.27 | 1.1951 | -10.9..+9.4 s |

Per part, seconds per 2^21-cycle segment (blocks with ≥ 5 segments in the part):

| part | fixed s | s per segment | s per padded Mcycle |
|---|---|---|---|
| chunks | 9.34 | 1.939 | 0.924 |
| trie | 7.57 | 1.968 | 0.939 |
| aggregate | 1.46 | 2.260 | 1.078 |

The aggregate's segments are dearer because they resolve the chunk and trie receipts as assumptions.

- **Small and big blocks are on one line.** The slope is 0.993 vs 1.010 s per padded Mcycle; the cost is linear in
  padded cycles from 3.4 M to 833 M with ~24 s fixed per block. The fixed term (2 chunks, 4 proof processes) is the
  same as R1's 23.49 s (c14, 1 chunk).
- **The earlier rates**, all measured on c14 images before this run and none on 46666b6f:
  - **R1** = 23.49 s + 0.9326 s per padded Mcycle: a fit to `../results` run1 (8 real-state leaf blocks
    200k-300k, c14 `bbf82800`, 1 chunk, po2 21).
  - **R2** = 1.100 s per user Mcycle all-in (962,000 proved in 537 s, LIBZYNC.md §4).
  - **R3** = 1.604 s per user Mcycle all-in (`../results` run2, 5 full blocks 969,118-122, modelled state, a
    busier host).
  - R2 and R3 are applied per era as max(23.49 s, rate x cycles).
- **Against those rates**, on the measured 976,000 (832.6 padded / 691.2 user M):

| rate | 976,000 projected | vs measured 847.5 s |
|---|---|---|
| R1 (0.9326) | 800 s | -6% |
| R2 (1.100 s per user Mcycle) | 760 s | -10% |
| R3 (1.604 s per user Mcycle) | 1,109 s | +31% |
| **R4** | **851 s** | +0.4% |

  R1 was close, about 6% low; R3, from the busier run2 host, overstated big blocks by about 30%. The all-in user rate
  on big blocks here is 1.22-1.25 s per user Mcycle.
- Per block, at $2.49/h: a 2009-2012 block costs ~21-37 s ($0.014-0.025); a full modern block (370-690 M user
  cycles, 434-844 M padded) costs ~455-862 s, $0.31-0.60. The pod's effective rate in this run was $2.22 / 0.835 h = $2.66/h
  (deploy to terminate, idle minutes included): +7% on every dollar figure if that is the price.

## 7. Whole chain, blocks 1..976,724 (PROJECTION)

Interpolation (`project.py`, output `raw/projection.out`). Each sample stands for its era (boundaries at the
midpoints between sample heights). The curve uses A where it exists, B elsewhere. Three methods:

- **per-input (headline)**: every block costs base + (sample - base)/sample inputs x its own inputs. Base = 2.64 M
  user (block 1,000, real state, no input). The chain's input counts come from `getblockstats` on one block per
  1,000 (`raw/chainsample_1k.tsv`, 977 blocks, each standing for its 1,000). Over the 29 measured blocks with >100
  inputs, cycles correlate with inputs at r = 0.956 and with weight at r = 0.949, but with transactions at only 0.739.
- **pooled**: one fit over all measured blocks, 25.1 M + 71.5 k x inputs + 8.8 k x outputs (user), applied per
  1,000-block stratum. Its residuals are -21% to +65% per block, worst on small blocks and on 962,000 (+51%).
- **height**: per-block cycles linear in height between samples.
- per-tx (`../cost/estimate.py`'s method) is shown in `raw/projection.out` but not used. It misses the 800,000
  neighbours by -18% to +50% and inflates the 650,000 era (a consolidation block: 1,089 txs, 8,823 inputs).

| method | T user | T padded | **R4 (measured) card-h / $** | R1 card-h / $ | R2 card-h / $ | R3 card-h / $ |
|---|---|---|---|---|---|---|
| **per-input, as measured** | **331.8** | **396.3** | **115,884 / $288,551** | 109,048 / $271,530 | 101,879 / $253,679 | 148,185 / $368,980 |
| per-input, B adj (+4.1% trie+agg) | 336.1 | 400.7 | 117,088 / $291,548 | 110,177 / $274,342 | 103,189 / $256,940 | 150,094 / $373,735 |
| pooled | 313.8 | 375.6 | 110,161 / $274,302 | 103,679 / $258,161 | 96,445 / $240,148 | 140,261 / $349,249 |
| height | 322.2 | 385.0 | 112,747 / $280,740 | 106,105 / $264,202 | 99,156 / $246,899 | 144,120 / $358,860 |

Arithmetic (headline): 976,724 blocks x 23.77 s = 23.22 M s; plus 0.9940 x 396.3 M padded Mcycles = 393.9 M s; total
417.2 M s = 115,884 card-hours; x $2.49 = $288,551.

Where the cost sits (per-input, as measured, `raw/projection.out`). Blocks up to 175,000 come to 0.7% of the cycles,
but at ~24 s each their fixed cost is ~1,150 card-hours of the total. 175k-425k is 9.2% of the cycles, 425k-625k
26.6%, 625k-976k 63.5%. The single largest eras are 775k-875k (21.7% together).

**Uncertainty.**
- **Rate:** now measured on 46666b6f. The 11 residuals are within ±9 s (±2% on big blocks), and the slope is the same
  for small and big blocks. Untested: other card instances or hosts (one pod, one EPYC host), and how much proving
  several blocks at once on one card changes per-block time.
- **Interpolation method:** the three methods span 313.8-336.1 T user cycles, i.e. 110.2-117.1 k card-hours
  (-5% to +1%).
- **One sample per era:** on the 800,000 dispersion test, the per-input model of 800,000 overpredicts its five
  neighbours by +5.5% to +21.3% (800,000 is a costly-per-input block: 109 k cycles/input against 90-104 k for the
  neighbours), and the pooled fit misses by -8.9% to +0.9%. Read the era level as about ±20%. This is now the
  dominant uncertainty.
- **Modelled state:** about +1-2% on a block from the A/B comparison. It is unmeasured above 400,000, where B is all
  there is.
- Combined (a judgement, not a statistical interval): **~330 T user / ~396 T padded cycles, ~116 k RTX PRO 6000
  card-hours, ~$289k at $2.49/h; range ~$230k-$350k** (±20%). The earlier projection's band was $240k-$375k, headline
  $272k on R1. The measured rate moves the headline up 6% and removes R3's +36% tail; the remaining spread comes from
  sampling, not from the rate.
- Not included: joins for a backfill, ~1,700 card-hours / ~$4.3k (ESTIMATED, LIBZYNC.md §4), and the price
  (+7% at this run's effective $2.66/h).
- This agrees with the earlier c14 estimate of 335 T to 970,000 (`../cost/WHOLE_CHAIN_COST.md`, modelled state, per-tx).

## 8. Still open

1. **Real-state trie cycles above 400,000:** run the same `run_a.sh` on prep's leaves at 481,824, 550,000, 650,000,
   709,632, 800,000, 900,000, 962,000 and 967,600-974,000 as prep reaches them. Execute-only on a CPU is enough; the
   cycles convert with R4.
   481,824 is done (22.11.2026): [`RESULT_481824_real_state.md`](RESULT_481824_real_state.md), model error +0.26% on the block.
2. **More samples per era** for the ±20% term: CPU-only, about 3 min per block on hazync-coord.
3. **The published v0.27.0 CUDA asset is broken** (see §5). The rate here is for the clean build; v0.27.1 should be
   checked to have the same host hash, or re-timed.
4. **The CPU side of a prover:** hint collection and the aggregate collect run (§4) are inside these wall times on a
   256-thread EPYC host. A pod with a weaker CPU may be slower per block.
