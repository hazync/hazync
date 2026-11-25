# 4x RTX PRO 6000 Blackwell: segpar control, 364,292 balanced vs not (25.11.2026)

keys-hz24 guest; host 831780a0 (segpar with phase timings). One pod, same machine for every row; every final proof
verified; every chunk / trie journal IDENTICAL to RISC Zero's (17/17 for both 364,292 exports). ~16 min of 4 cards
(~$4.2; the full harvest crawled at ~1 MB/min, so the REPORTs were fetched directly and the pod terminated by hand).

| job | leaves | aggregate | block (prove-block) | wall |
|---|---|---|---|---|
| 976,379, segpar (aggregate's 8 segments over 4 GPUs) | 32.3 s | 14.3 s | 46.9 s | 50 s |
| 976,379, no segpar (same pod) | 25.5 s | 16.8 s | 42.6 s | 45 s |
| 364,292 fitted cost model (16 even chunks, 5.7-7.3B) | 82.5 s | 12.4 s | 95.6 s | 98 s |
| 364,292 default model (16 chunks 12.5B falling to 1.4B) | 63.1 s | 12.5 s | 76.3 s | 79 s |

**segpar:** the aggregate's app proof went from serial segments to 8.2 s (stark prover 0.26 s, instance 0.52, metered
0.97, own run done 5.1, workers done 5.2 / 7.7 / 8.2, aggregation to 9.0, deferral to 10.9, wrap 11.0, deferral
Merkle 12.1, prove 12.3 s): aggregate -2.5 s. But the leaves were 6.8 s SLOWER: the segment workers start with the
block and load their keys during the leaf phase. Next: start them when the leaves end.

**364,292:** the "balanced" 16 chunks lost by 19 s. 4 cards x 3 provers = 12 slots: 16 equal chunks run as a first
wave of 12 and a second of 4 (chunk_14 done at 77 s), while the default model's chunks fall in size from the first, so
the largest start first and the small ones fill the gaps (all done by 59 s). Balance needs the chunk count matched to
the prover slots (12 here), or the largest first.
