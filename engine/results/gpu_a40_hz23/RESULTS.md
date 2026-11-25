# hz23 on one A40 (25.11.2026)

keys-hz23 (branch at 8856563f): the hz8 guest plus the 25.11 guest changes (coin log and trie hash pairs compressed in
place, swiftsync x_for on one padded block, Core's CSHA256::Finalize padding in place). Same card class, same blocks,
same defaults (3 provers on the card) as results/gpu_a40_hz8. Wall time = run_tune's prove-block, `verify=0` = the
final proof verified.

| block | hz8 (24.11) | hz23 | change |
|---|---|---|---|
| 976,379 | 172 s | 149 s (leaves 108.2 s + aggregate 37.0 s) | −13% |
| 962,000 | 144 s | 129 s (leaves 96.3 s + aggregate 29.5 s) | −10% |
| 741,000 | 32 s | 35 s (leaves 22.6 s + aggregate 8.8 s) | +3 s (not explained; fixed costs dominate) |

Every chunk and trie journal IDENTICAL to RISC Zero's; each aggregate journal RISC Zero's except the image id (31 of
its 32 bytes differ). REPORT_*.txt per block, times.tsv, gpu_block.log. Pod: 1x A40, ~18 min.
