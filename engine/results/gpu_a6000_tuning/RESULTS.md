# Tuning on one RTX A6000: chunk sizing x provers per GPU x segment size (24.11.2026)

Pod 40ix20g0j9wi01, 13:40-14:20 = 40 min ≈ $0.46. keys-hz4. Every completed run: proof verified, every chunk/trie
journal identical to RISC Zero's. `t` = chunks sized by predicted cost (HAZYNC_CHUNK_TARGET=900000000).

| block | chunks | provers on the GPU | segment cap | block time | GPU util |
|---|---|---|---|---|---|
| 976,379 | 16 | 1 | 15 GiB | 289 s (multiblock run) | 48% |
| 976,379 | 16 | 2 | 15 GiB | 245 s | 59% |
| 976,379 | 5 | 1 | 15 GiB | 240 s | 49% |
| 976,379 | 5 | 1 | 24 GiB | 221 s | 46% |
| 976,379 | 5 | 1 | 32 GiB | FAILED: prover refused (LogupZerocheck nonzero root sum) | |
| 976,379 | 5 | 2 | 15 GiB | 194 s | 66% |
| 976,379 | 5 | 2 | 20 GiB | 191 s | 65% |
| **976,379** | **5** | **3** | **12 GiB** | **183 s** | **70%** |
| 962,000 | 4 | 1 | 24 GiB | 180 s (was 254 s) | 45% |
| 741,000 | 1 | 1 | 24 GiB | 39 s (was 110 s at 16 chunks) | 35% |

976,379: RISC Zero 1,658 s -> engine 183 s = **9.1x** on the same card. 962,000: RISC Zero 1,385 s -> 180 s (one prover,
not the best setting) = 7.7x. 741,000: 414 s -> 39 s = 10.6x.

The 32 GiB failure is the prover refusing a segment (not a bad proof); 24 GiB is the largest that worked. Peak-memory
figures from nvidia-smi sampling were unreliable and are omitted.
