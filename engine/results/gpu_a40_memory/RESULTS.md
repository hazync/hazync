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
