# 969,122 like for like, segpar re-measured: 4x RTX PRO 6000 Blackwell SE (26.11.2026)

Pod tz427i5mke6391 (secure, ~$6.76/hr for 4), 01:51-02:00, $1.11 in all. keys-m26 (2^26 address space), host at the
branch head (app VM released before aggregation, segpar with reused leaf workers), engine defaults (`run_tune.sh`
bare job names), HARVEST_FULL. Every final proof verified.

| block | leaves | aggregate | prove-block | note |
|---|---|---|---|---|
| **969,122** (9,047 inputs, 5 chunks + trie) | 20.6 s | 13.0 s | **33.9 s** (job 36 s) | RISC Zero's rehearsal block |
| 976,379 (segpar) | 23.8 s | 14.5 s | 38.6 s (job 41 s) | |
| 976,379 (HZ_SEGPAR=0, same pod) | 22.8 s | 19.4 s | 42.4 s (job 45 s) | |
| 130,000 (10 inputs) | 3.6 s | 3.5 s | 7.2 s (job 9 s) | |

- 969,122's export (build box, tip969.sh): hints 35 s (16 threads), every chunk and trie journal IDENTICAL to RISC
  Zero's on the engine (CPU).
- segpar with the leaf workers reused (37a02971) is now a gain: the aggregate 19.4 -> 14.5 s (-25%), the block
  42.4 -> 38.6 s, same pod.
- RISC Zero's 191 s on 969,122 (libzync 9c2123e rehearsal, 4x PRO 6000) is detect to verified receipt, its hints
  26.5 s on the pod. The engine's proving part on the same block and card count: 33.9 s. Detection, staging and the
  live pipeline are not in this number.
