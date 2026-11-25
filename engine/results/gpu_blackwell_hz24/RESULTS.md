# hz24 on RTX PRO 6000 Blackwell, 1 and 4 cards (25.11.2026)

keys-hz24 (branch at 402e528d): hz23 plus the split trie verified by the aggregate (no combine proof). Block 976,379,
whole trie (export 976379t) and split at depth 2 into 4 parts (976379ts2). Engine defaults (provers per card by
memory). First OpenVM run on Blackwell: the CUDA build and every proof worked unchanged.

| setup | whole trie | split trie (4 parts) |
|---|---|---|
| 1x A40 (same pod for both) | 160 s (leaves 117.9 + agg 38.5) | 175 s (leaves 126.6 + agg 44.1) |
| 1x PRO 6000 Blackwell | 65 s (leaves 43.4 + agg 18.5) | 68 s (leaves 44.6 + agg 21.3) |
| 2x PRO 6000 Blackwell | | 51 s (leaves 29.4 + agg 19.2) |
| 4x PRO 6000 Blackwell | 48 s (leaves 28.5 + agg 16.4) | 49 s (leaves 26.7 + agg 19.0) |

(wall = run_tune's prove-block; leaves + aggregate from REPORT_*.) Every chunk, trie and part journal IDENTICAL to
RISC Zero's; every aggregate journal RISC Zero's except the 32-byte image id; the final proof verified each time.

On 4 cards the whole trie is no longer the long pole (24.2 s against chunks 12.8-19.4 s), and the split's parts
(10.9-14.3 s) leave the chunks (up to ~20 s) as the pole, while its aggregate costs ~2.6 s more (9 child proofs, the
combine's update): a tie. The aggregate (16-19 s, alone on one card) is now a third of the block.
