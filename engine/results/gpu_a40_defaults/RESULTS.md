# The engine's defaults, no flags, one A40 (24.11.2026)

`hzengine prove-block --dir keys --export <x> --out <o>`, nothing else set, on keys-hz4 and the cost-sized exports
(the same inputs as gpu_a6000_tuning). No A6000 was free; the A40 also has 48 GB (nvidia-smi: 46,068 MiB), so the
defaults chose 3 provers per GPU and an 11 GiB segment cap for the leaf provers (round(45/3) - 4), 24 GiB for the
aggregate, which runs alone. Pod ~17 min.

| block | chunks | leaves (3 provers) | aggregate | total | GPU util | verify |
|---|---|---|---|---|---|---|
| 976,379 | 5 | 154.4 s | 46.2 s | 204 s | 74% | ok |
| 962,000 | 4 | 120.8 s | 35.2 s | 159 s | 77% | ok |
| 741,000 | 1 | 27.9 s | 10.3 s | 42 s | 45% | ok |

Every chunk and trie journal IDENTICAL to RISC Zero's; each aggregate journal has RISC Zero's length and differs in
32 bytes (the image id). For scale: 976,379 on an A40 with the first settings (16 chunks, one prover) took 409 s
(gpu_a40_block976379); on an A6000 the tuned run took 183 s (gpu_a6000_tuning). Different cards, so not a
like-for-like comparison with the A6000 numbers.

After this run: prove-block caps the provers at the number of leaf proofs (741,000 has two, and started three).
