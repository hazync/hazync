# The libzync paper: proposed outline (G3, to be written with the user)

A proposal to react to, not a draft: the sections, what each must establish, the evidence that exists for it (file
references), and the questions only the author can answer. Nothing here is prose for the paper yet.

**Working title:** proving every Bitcoin block from genesis with Bitcoin Core's own consensus code in a zkVM.
**Claim in one sentence (to agree):** a single succinct proof shows that the chain ending at a given tip is valid
under Bitcoin Core v28's rules from genesis, and commits a state from which coin, balance, history, absence and
transaction facts can be checked without trusting anyone; it costs about 8× less card time per block than running
the canonical guest (ESTIMATED for the frozen image), and blocks can be proved in any order and joined.

## 1. Introduction
- The problem: a new node either replays 15+ years of blocks or trusts someone (assumevalid, assumeutxo, a server).
- What a validity proof changes; why "Core's own code" rather than a re-implementation.
- Contributions (to agree): the coin log (accounting sound at every block); the counting trie in its own receipt;
  joins of independently proved blocks; facts a light client can check; a frozen, reproducible image.
- Evidence: LIBZYNC.md §1-2; SWIFTSYNC_PROOFS.md §2.

## 2. Background and related work
- risc0 / STARK recursion; SwiftSync (Ruben Somsen); Utreexo; assumeutxo; ZeroSync / earlier zk-Bitcoin efforts;
  Peter Todd's TXO commitments (COINLOG.md §1).
- ⚠ needs a literature pass with citations we check, not from memory.

## 3. What a proof states (the statement)
- `ChainState`, field by field; the genesis anchor; the verifier's 13 steps; DEMO vs OK; what is NOT claimed
  (chain identity is the verifier's step 7; minimum work and the checkpoint).
- Evidence: SWIFTSYNC_PROOFS.md §3, §5.

## 4. Design
1. Chunks and aggregate (Core's VerifyScript per chunk, batch signature check, the aggregate's block rules).
2. The coin log: append-only, flip-on-spend, sound at every block (and why design B's "true only at H" was retired).
3. The counting trie: what it holds, the separate receipt, the coin-key aliasing and how facts bind it.
4. The live-coin hash (instant IBD).
5. The block forest and the handover (image changes without breaking the chain).
6. Joins: anchored leaves, the link digest, a balanced tree.
- Evidence: COINLOG.md, ZYNCAPPS.md, HANDOVER.md, UTREEXO_FORMAT.md, ROADMAP H-B.

## 5. Soundness
- The argument: what each step relies on (risc0's recursion, SHA-256, ECMH collision resistance, Core's code as
  compiled), the host as an adversary, what the batch-signature check needs (weights from a hash over every R).
- Known gaps and where the trust sits (an external audit is not done; one zkVM).
- Evidence: SECURITY.md, CONSENSUS_RULES.md (63 rows; STRICTER rows), the 12 review rounds (LIBZYNC.md §5, ROADMAP H6).

## 6. Implementation and how much of Core is in the proof
- Which Core / libsecp sources are compiled unchanged, lifted, or patched (and why each patch preserves results).
- Evidence: LIBZYNC.md §6; CONSENSUS_RULES.md table legend; patches/ review (ROADMAP rounds 9-10).

## 7. Evaluation
- Correctness: rig vs Core 28.0 96/96; Core's script / tx vectors 22,666 / 22,666; mainnet 1..200 and signet
  1..2600 vs independent reference implementations; handover 16/16; crash safety (10 scenarios).
- Cost: cycles per block (c14: 440.6 M + the trie receipt); GPU: 537 s on one RTX PRO 6000 for block 962,000; the
  first real join tree (6.4 s / join); vs the canonical guest (8.6× MEASURED for design B on one A40; ~8.2×
  ESTIMATED for c14). ⚠ the GPU block's coin state is MODELLED, not mainnet history: say so in the text.
- Whole-chain cost: an ESTIMATE (~$114-119k A40, scripts checked, design B) — re-estimate on c14 before quoting.
- Evidence: GPU_RESULTS.md, RESULT_c13.txt, RESULT_c14.txt, core_vectors/RESULT_c10e12_variants.txt,
  cost/WHOLE_CHAIN_COST.md.

## 8. Applications (zyncAPPs) and limits
- Facts a wallet / exchange can check; the coin and header-chain zyncAPPs; what is design only.
- Limits: no mainnet proof from genesis yet; mainnet-scale prep not done; one zkVM; no external audit; not consensus.

## 9. Conclusion and availability
- The frozen image ids (from the reproducible container, H7), the repo, how to reproduce (LIBZYNC.md §8).

## Questions for the author (before writing)
1. Audience and venue: a research paper (which venue / arXiv), a technical report, or a long blog post? This sets
   length, related-work depth and tone.
2. Authorship and credit: names, affiliations, acknowledgements (Ruben, risc0, reviewers).
3. Do we wait for a real-state block (mainnet prep, ROADMAP H-B) before publishing, or publish with the modelled
   benchmark clearly labelled?
4. Which numbers to headline (card time vs cycles vs dollars), and whether to re-measure CORE on the same card first
   (a direct same-card ratio instead of the ~8.2× estimate).
5. How much of the design history (design B, the review rounds) belongs in the paper vs an appendix.
