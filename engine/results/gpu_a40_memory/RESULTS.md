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
