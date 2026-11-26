# GPU memory a block needs on the engine (A40, 25.11.2026)

Question: can a 4 GB card (GTX 1050 Ti) prove? One A40, ONE prover (`--workers 1`), segment cap 2 GiB (`HZ_SEG_MEM_GB=2`),
GPU memory sampled every 200 ms (nvidia-smi memory.used). keys-hz24, host 06217cae. ~20 min, ~$0.20.

| block | proved | true peak GPU memory |
|---|---|---|
| 130,000 (tiny) | 16.4 s (leaves 8.9 + aggregate 7.3), verified | **6,643 MiB** |
| 976,379 | 505.2 s (leaves 390.5 + aggregate 114.1), verified, journals identical | **6,699 MiB** |

- The memory is nearly all FIXED: the same ~6.6-6.7 GB for a 10-input block and a full one, and the 2 GiB segment cap
  barely matters. The trace (smi_130000t_w1_s2.log) shows a single jump of ~4 GB about 3.6 s in (2.5 GB -> 6.6 GB),
  then flat. OpenVM's pool preallocates nothing (VPMM_PAGES = 0) and grows in device-granularity pages (2 MB), so this
  is real demand, not rounding.
- So as built, a 4 GB card (GTX 1050 Ti) does NOT fit, whatever the segment cap. What the ~4 GB is: not yet confirmed;
  the likely candidate is a buffer sized for the largest stacked trace (2^24 rows, MAX_APP_LOG_STACKED_HEIGHT, which is
  in the keys' SystemParams), not for the actual one.
- ⚠ The `peak_mem` column in times.tsv here and in EVERY earlier run is wrong: run_tune compared the samples as text
  ("673" > "6643"); fixed in run_tune (numeric). The true peaks above are from the raw samples.

## The same with the app stacked height 2^22 (keys-s22, HZ_APP_LOG_STACKED_HEIGHT=22), and what the memory IS

Same pod type, same single prover and 2 GiB segments: 130,000 peak **6,567 MiB** (vs 6,643 at 2^24); 976,379
**6,840 MiB**, proved in 458 s (vs 505 s), verified; chunk and trie journals identical to RISC Zero's (CPU, s22.out).
**So the stacked height is NOT the floor.**

OpenVM's own GPU memory tracker (the "GPU mem:" lines, info level) names it: every segment proof begins with
**"set initial memory": +4.5 GiB** (to current 6.0 GiB), freed again in "generate mem proving ctxs" (-4.5 GiB). That
is the guest's WHOLE address space (2^29 bytes = 512 MiB, openvm-platform MEM_BITS) put on the device as a dense
image with its Merkle tree, ~9 bytes of GPU memory per guest byte, whatever the block touches. Everything else peaks
at ~3 GiB (prover.rs_code_matrix +1.2 GiB, WHIR, GKR). The lever is the guest address space: 2^27 (128 MiB; the guest
heap is 8-31 MB per proof) would make that ~1.1 GiB and the peak ~3 GiB. It is in the VM config (keys, image id) and
needs openvm-platform's MEM_BITS to match (a one-line patch), so a guest past it stops with "Maximum memory exceeded",
which #795's re-split recognises.

Also found: 976,379 at engine defaults (3 provers + the aggregate) with keys-s22 ran OUT OF GPU MEMORY in the
aggregate (requested 8.5 GB, 2.7 GB free; peak 42,983 MiB): the leaf workers now stay alive for segpar and keep their
pools. To fix: release them (or their pools) before the aggregate on a single GPU.

## Guest address space 2^27 (keys-m27: HZ_GUEST_MEM_BITS=27, openvm-platform MEM_BITS = 27)

Same pod type, single prover, 2 GiB segments; CPU journals identical first (m27.out). **"set initial memory" is now
+1.1 GiB (was +4.5).** 130,000: true peak **3,793 MiB** (was 6,643), 20 s, verified. 976,379: **5,028 MiB** (was
6,699), 444 s (was 505), verified; its largest step is now prover.rs_code_matrix (+2.3 GiB: the stacked trace's
Reed-Solomon encoding), which the stacked height sets (2^22 halved it in the keys-s22 run). Next: 2^27 AND 2^22 with
1 GiB segments. The defaults job (3 provers) again ran the aggregate out of memory: the kept leaf workers; fixed in
52a28fd2 (the workers the aggregate does not use stop before it).

## 2^27 + stacked 2^22 + 1 GiB segments (keys-ms), and the leaf batch (A40 / A6000, 25.11.2026 evening)

- keys-ms (HZ_GUEST_MEM_BITS=27 + HZ_APP_LOG_STACKED_HEIGHT=22), 1 GiB segments, one prover: 130,000 peak 5,136 MiB
  (tracker: 4.70 GiB in use), 976,379 peak 5,489 MiB in 793 s: WORSE than keys-m27 at 2 GiB. The tracker places the
  peak in agg_layer "leaf": many small segments make each leaf proof verify more of them (+2.3-2.4 GiB
  rs_code_matrix). Small segments are not the way down.
- The same keys at engine defaults (3 provers, A40): 976,379 in **132 s**, verified (the 52a28fd2 fix: the aggregate
  no longer runs out of memory). Faster than keys-hz24's 149-160 s on A40s.
- HZ_LEAF_CHILDREN=1 (keys-m27, 2 GiB, one prover, RTX A6000): 130,000 6,061 MiB, 976,379 6,099 MiB in 572 s, both
  tracker 5.60 GiB. FLAWED TEST: the run builds its own AggProver while the SDK still makes its default one, so two
  sets of aggregation keys are on the GPU (~+2.3 GB; the SDK only hands out its deferral setup through prover(), and
  the cached-key constructor that takes a tree config needs OpenVM's crate-private from_supported_deferral_pks).
  Even so: the full block now peaks no higher than the tiny one, and the +2.2 GiB step moved from the leaf layer to
  internal_for_leaf (3 children by default). Next: the SDK built with the tree config (one patched OpenVM line, or
  upstream), num_children_leaf 1 and num_children_internal 2.

## Proving-time aggregation tree (3edbde74; SDK builder + checked deferral prover), 25.11.2026 night

One prover, 2 GiB segments, every proof verified (the rebuilt deferral prover's config matched the app key's each time).

| keys | tree (leaf/internal) | card | block | wall | nvidia-smi peak | tracker max |
|---|---|---|---|---|---|---|
| m27 | 1/2 | A40 | 130,000 | 21 s | 3,817 MiB | 3.40 GiB |
| m27 | 1/2 | A40 | 976,379 | 666 s | 4,284 MiB | 3.60 GiB |
| m27 | **2/2** | A40 | 976,379 | **513 s** | **4,049 MiB** | - |
| m27 + stacked 2^22 | 2/2 | L4 | 976,379 | 604 s | 4,166 MiB | 3.60 GiB |
| (before: m27, default tree 4/3) | | A40 | 976,379 | | 5,0xx MiB | |

- The tree takes the full block from ~5.0 GB to ~4.05 GB. 2 per leaf beats 1 per leaf on both memory and time.
- Stacked 2^22 changes nothing: the peak step is not in the app proofs.
- What is left (L4 log, harvest of run_20261125_205749):
  - ~1.5-1.6 GiB stays resident through every phase and between chunks: the device proving keys of every circuit, uploaded once and kept for the run.
  - On top of that, the largest single step is rs_code_matrix in the leaf layer (1.2-1.6 GiB) and in internal_for_leaf (1.6 GiB), the recursion circuits' own fixed 100-bit presets (leaf 2^21 x blowup 4, internal 2^19 x blowup 8).
  - The app proofs peak lower (2.6 GiB resident at start, including the 1.1 GiB address space).
- Next lever for a 4 GB card: keep the proving keys on the host and upload only the active layer's (prover-side, inside OpenVM).

## App VM released before aggregation (f38eb2fa), RTX 4090, 25.11.2026 night

- CPU first (hazync-coord, keys-m27, tree 2/2): 130,000 chunk + trie IDENTICAL to RISC Zero, aggregate verified both
  children in-guest, final proof `verify: OK`.
- RTX 4090, tree 2/2, 2 GiB segments: 976,379 in **330 s**, verified, nvidia-smi peak **4,160 MiB**; 130,000 14 s,
  3,796 MiB.
- The release works: the late aggregation layers now start at 0.34-0.37 GiB resident (were 1.5-1.6). The peak did not
  move because it is now the app phase (L4 tracker: 3.2 GiB = app key 1.2 + the 2^27 address space image/tree 1.1 +
  encoding), and the memory pool keeps its high-water mark. Only log tails were harvested (essentials harvest), so the
  phase attribution of this run's peak is from the L4 run; the next memory run uses HARVEST_FULL=1.
- Next: 2^26 address space (halves the 1.1 GiB), if every guest input fits in 64 MiB (CPU check m26.sh).

## 2^26 address space (keys-m26) and the 1 GiB segment cap, RTX 4090, 25.11.2026 late

- CPU (m26.sh, hazync-coord): every chunk and trie input of 976379t, 364292t, 962000t and 965500 (46 inputs) runs
  in a 64 MiB address space, journals IDENTICAL to RISC Zero's. The aggregate guest is covered by the GPU run below.
- 4090, keys-m26, tree 2/2, 2 GiB segments, full log: 976,379 in 357 s, verified, nvidia-smi peak **4,168 MiB**
  (130,000: 15 s, 3,792 MiB). The address space halving did not move the peak.
- Full-log attribution (tracker): leaf now 1.4-1.8 GiB and internal_for_leaf 2.1-2.2 (the app VM release works). The
  peak, 3.6 GiB, is the AGGREGATE's app-segment proofs (154 segments): ~2.2-2.5 GiB live (app key 1.2 + address
  space + the segment's traces) + rs_code_matrix 1.0 GiB, and the logup GKR step +1.2 GiB.
- 1 GiB segments (same keys): chunk_00 has 565 segments and took 189.7 s (vs ~71 s at 2 GiB); tracker max 2.80 GiB,
  smi 3,374 MiB through chunk_00 and part of chunk_01. The run was stopped by gpu_block's balance guard (< $1) before
  the aggregate, which held the peak at 2 GiB, so the full-block peak at 1 GiB is NOT measured.

## Full block at 1 GiB segments (keys-m26, tree 2/2), RTX 4090, 25-26.11.2026

Launcher stopped by hand at setup (slow pod network: rustup 3 min, CUDA 14 min; the 20-min setup guard would have
killed the pod just before SETUP_OK) and the job run by hand; full output harvested.

- 976,379: **verified** (`verify: OK`), every chunk and trie journal IDENTICAL to RISC Zero's, nvidia-smi peak
  **3,765 MiB**, OpenVM tracker max **2.80 GiB**, wall **1,619 s** (vs 357 s at 2 GiB: 4.5x).
- Segment counts multiply ~6.7x, not 2x (chunks 520-580 each, trie 912, aggregate 1,029 vs 154 at 2 GiB), so the
  slowdown is per-segment overhead. A cap between 1 and 2 GiB is the next point to measure.
- smi minus tracker ≈ 0.9 GiB (CUDA context + VPMM pool slack).

## Full block at 1.5 GiB segments (HZ_SEG_MEM_MB=1536, 183637cd, keys-m26, tree 2/2), RTX 4090, 26.11.2026

- 976,379: verified, every journal IDENTICAL, **584 s**, nvidia-smi peak **3,943 MiB**, tracker max 3.00 GiB,
  pool max 2.90 GiB. Segments: chunks 147-166, trie 254, aggregate 306 (chunks ~70 s each, as at 2 GiB).

| segment cap | wall | tracker | smi peak |
|---|---|---|---|
| 2 GiB | 357 s | 3.6 GiB | 4,168 MiB |
| 1.5 GiB | 584 s | 3.0 GiB | 3,943 MiB |
| 1 GiB | 1,619 s | 2.8 GiB | 3,765 MiB |

- smi minus pool ≈ 1.0 GiB: 396 MiB CUDA context on the 4090 before any allocation (measured, first smi samples),
  ~0.5 GiB outside the tracker. Context size depends on the card (SM count); a 1050 Ti's is not measured.
