# Engine vs RISC Zero across block sizes, SAME card: one RTX A6000 (24.11.2026)

Pod uco2hoept59em5 (A6000, secure, $0.69/hr), 11:33-12:44 = 71 min ≈ $0.82. Engine = keys-hz4 (native SHA + fat LTO,
in-guest recursion: the aggregate verifies every chunk and trie proof). RISC Zero = the released v0.26.0 CUDA product
host (image 46666b6f), same blocks, same 16-chunk layout. Times are whole-block proving on one card (`times.tsv`).
Every engine proof verified; every engine journal identical to RISC Zero's except the 32-byte self_id.

| block | inputs | RISC Zero (16 chunks) | engine (16 chunks) | ratio | engine, chunked by size | ratio |
|---|---|---|---|---|---|---|
| 130,000 | 10 | 179 s | 56 s | 3.2x | 19 s (1 chunk) | 9.4x |
| 140,000 | 212 | 305 s | 90 s | 3.4x | 23 s (1 chunk) | 13x |
| 741,000 | 670 | 414 s | 110 s | 3.8x | 54 s (4 chunks) | 7.7x |
| 962,000 | 8,006 | 1,385 s | 254 s | 5.5x | | |
| 965,500 | 7,889 | | 339 s | | | |
| 976,379 | 8,130 | 1,658 s (run 2) | 289 s | 5.7x | | |

(130,000 has 10 inputs, so "16 chunks" is 10 there. RISC Zero was not re-run on 965,500/976,379; 976,379's 1,658 s
is the earlier A6000 measurement of the same host.) GPU utilisation averaged 48% over the engine runs.
