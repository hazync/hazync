# Split trie proofs on one A40 (24.11.2026)

keys-hz6 (image exe 04dc2137..., the guest with modes 19/20), block 976,379, defaults (3 provers, 11 GiB segments,
24 GiB for the aggregate). No multi-GPU pod was free (4x and 2x A6000/A40/L40S tried for 15 min each).

| export | leaf proofs | leaves | aggregate | total (prove-block) | verify |
|---|---|---|---|---|---|
| 5 chunks + trie | 6 | 156.2 s | 43.6 s | 200.3 s | ok |
| 5 chunks + 4 trie parts + combine | 10 | 170.6 s | 44.5 s | 215.7 s | ok |
| 11 chunks + trie | 12 | 176.0 s | 47.9 s | 224.9 s | ok |
| 11 chunks + 4 trie parts + combine | 16 | 190.3 s | 48.0 s | 239.2 s | ok |

Every chunk, part and whole-trie journal is IDENTICAL to RISC Zero's. The combine's journal is RISC Zero's whole-update
journal (old root, new root, list digest) plus the image id. Each split aggregate's journal is its export's RISC Zero
(unsplit) aggregate journal except the 32-byte image id. (The split and unsplit exports came from two product-host
builds whose METHOD_IDs differ, and that id reaches the anchor state, so their aggregate journals differ from each
other. Each matches its own RISC Zero run.)

Proving times, with 3 provers sharing the card: whole trie 58.8–77.7 s; each part 30.5–34.0 s; the combine 37.6–38.1 s.
Metered cost (CPU): whole trie 6.31B, parts 1.49 / 1.49 / 1.74 / 1.65B, combine 1.33B.

**Result: on one GPU the split is slower** (+15 s): the same trie work plus a combine almost as big as a part. Nor
would it help on several GPUs as built, because the combine waits for the parts: 33 s + 38 s against 59–78 s for the
whole trie. The combine's cost is memory. It re-reads and decodes the block's whole update list (1.3 MB) and hashes
it twice, work the aggregate already does on the same list. Next design: no combine proof. The aggregate verifies the
parts itself, on the list it already holds.
