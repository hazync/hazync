# Segment cap vs padding on the GPU's memory model, one A40 (24/25.11.2026)

Pod cq3p7fj4s0tgng (A40), keys-hz8, export 976379t, `scripts/pod/run_seg.sh`. The run was cut short at 15.3 min by
the pod watchdog (RunPod balance under $1): the sweep and four proofs finished, the aggregate part did not run.

**Sweep** (seg_sweep.tsv): `HZ_SEGMENTS=1 HZ_SEG_MEM_GB=<cap> hzengine execute` on the CUDA build, i.e. OpenVM's own
segmentation with the GPU prover's memory accounting; trace cells summed over segments as executed and padded to
powers of two. Padded main cells, billions:

| job | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 16 | 20 | 24 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| chunk_00 | 7.66 | 7.40 | 7.97 | 8.25 | 8.32 | 7.52 | 7.13 | 7.15 | 7.09 | 7.68 | 8.42 | 6.95 |
| chunk_01 | 7.73 | 7.67 | 8.16 | 8.18 | 8.31 | 8.26 | 7.06 | 7.40 | 7.71 | 8.04 | 8.48 | 7.05 |
| chunk_02 | 6.81 | 6.82 | 6.76 | 7.44 | 7.32 | 6.87 | 6.34 | 6.21 | 6.56 | 6.74 | 7.05 | 5.53 |
| chunk_03 | 8.06 | 7.91 | 8.62 | 8.33 | 8.15 | 7.75 | 7.44 | 7.94 | 8.05 | 7.67 | 8.06 | 7.22 |
| chunk_04 | 9.10 | 9.11 | 9.20 | 9.26 | 9.05 | 8.46 | 8.53 | 8.58 | 9.36 | 9.14 | 9.29 | 8.09 |
| trie | 18.86 | 18.93 | 16.89 | 16.90 | 17.35 | 17.27 | 18.16 | 18.37 | 18.26 | 15.52 | 15.95 | 16.50 |

Padding is not monotonic in the cap: a segment ends just before the crossing that would exceed the cap, so which
circuits sit just over a power of two moves with it (chunk_00: 7.09B at 14 GiB, 8.32B at 10 GiB).

**Time follows padded cells** (seg_prove.tsv, each proof alone on the card):

| proof | cap | segments | padded main cells | app proving | wall |
|---|---|---|---|---|---|
| chunk_00 | 10 | 10 | 8.32B | 22.4 s | 31.6 s |
| chunk_00 | 11 | 9 | 7.52B | 21.3 s | 30.0 s |
| chunk_00 | 14 | 7 | 7.09B | 19.9 s | 28.7 s |
| trie | 7 | 39 | 18.93B | 47.0 s | 60.9 s |

**What a per-job cap would buy** on these six leaves: against 11 GiB for every leaf (today's 3-prover default on a
48 GB card), the best cap per job within 6..11 GiB is −2.1% padded main cells and −10.1% padded interaction rows;
within 6..12 GiB −5.0% / −10.1%. Not built yet: it needs a per-request cap in `serve` and a GPU run to confirm.
