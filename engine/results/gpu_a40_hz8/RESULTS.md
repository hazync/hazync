# keys-hz8 on one A40, engine defaults (24.11.2026)

Pod gv8zpysoebzvgq, 1x A40 (46,068 MiB), 15.6 min including the CUDA build. Same blocks, exports and flags as
gpu_a40_defaults (`prove-block` with nothing set: 3 provers, 11 GiB leaf segment cap, 24 GiB aggregate). What changed
since then is the guest's batch signature check (results/profile_chunk: copy-free msm, Straus for the two-point
fallbacks, in-place live-coin sum) and the leaf order (the whole trie is queued first).

| block | chunks | defaults (keys-hz4) | hz8 | change | leaves | aggregate | verify |
|---|---|---|---|---|---|---|---|
| 976,379 | 5 | 204 s | **172 s** | −16% | 154.4 → 127.5 s | 46.2 → 41.6 s | ok |
| 962,000 | 4 | 159 s | **144 s** | −9% | 120.8 → 106.7 s | 35.2 → 34.8 s | ok |
| 741,000 | 1 | 42 s | **32 s** | −24% | 27.9 → 20.2 s | 10.3 → 9.4 s | ok |

Every chunk and trie journal identical to RISC Zero's; each aggregate journal has RISC Zero's length and differs in its
32 image-id bytes. `times.tsv` is wall clock per job (includes process start), the per-proof lines are in <block>.txt.

The trie is now the long pole: on 976,379 it took 96.6 s (sharing the card with two chunk provers), against 26-57 s
per chunk; the leaf phase ends when the last chunk queued behind it finishes (124 s).
