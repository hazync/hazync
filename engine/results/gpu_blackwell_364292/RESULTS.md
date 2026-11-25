# Block 364,292 on one RTX PRO 6000 Blackwell; segpar first try (25.11.2026)

keys-hz24 guest, host at 2f592af0 (segpar). 2x RTX PRO 6000 Blackwell pod, ~21 min.

**364,292** (5,569 inputs; legacy signature hashing over one huge transaction): export364292t, 16 chunks + trie, on
ONE card (`364292t@1`): **201 s wall, block 198.0 s = leaves 181.1 s + aggregate 15.8 s**, final proof verified;
every chunk and trie journal IDENTICAL to RISC Zero's (also on CPU first: chunk costs 12.49B falling linearly to
1.39B, ~109B in all, trie 0.78B). The chunks are badly unbalanced: the export's default cost model predicted 15.8B
for the whole block and does not price the legacy sighash, whose input k hashes the inputs after it (cost falls with
k). Chunk 0 is the long pole. Next: the fitted model (HAZYNC_COST_MODEL=fit, which has that term).

**segpar, first try** (976,379, both cards, the aggregate's 8 segments split 0..5 on a worker, 5..8 here): block 62.7 s,
aggregate 19.2 s, "prove (segpar)" 17.9 s: no gain visible against the 1-card pod's 18.5 s aggregate (other
machine). The same-pod control (`976379t~`, no segpar) did not run: bash expanded the `~` (fixed in bbb38923). Phase
timings added (bbb38923) before the next pod.

Correction: on ONE card chunk 0 is not the pole (it proved in 48.5 s); the 181 s leaf phase is the 16 chunks sharing
the card, so total work decides it there. Balance matters with several cards.

**The fitted cost model** (export364292tf: HAZYNC_COST_MODEL=fit, 16 chunks; CPU): chunk costs 7.28B .. 5.66B
(default model 12.49B .. 1.39B), the same ~109B in all; every chunk and trie journal IDENTICAL to RISC Zero's.
