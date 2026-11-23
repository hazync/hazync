# Run 2: sighash hashing, wrapping to one proof, and Hazync's own guest — RTX A6000 (23.11.2026)

Pod oszsfwovbpila1, $0.69/hr, 16:22-17:56 UTC = 1.57 h, about $1.08. Every proof verified.

## Sighash hashing (mode 4: every supported input's final sighash preimage, hashed and checked)

| block | preimages | SHA-256 compressions | SP1 core | OpenVM app |
|---|---|---|---|---|
| 976,379 (tip) | 7,959 | 55,035 | 14.7 s | 11.0 s |
| 510,500 (worst legacy block sampled) | 6,650 | 2,283,480 | 396.8 s | 164.5 s |
| per compression (510,500) | | | 174 us | 72 us |

SP1 and OpenVM (CPU executor) agree on the 976,379 digest. Excludes BIP143/341 per-tx midstates (small).

## Wrapping to ONE proof

| workload | SP1 core | SP1 compressed | OpenVM app | OpenVM stark |
|---|---|---|---|---|
| ECDSA x7,060, 976,379 | 399.2 s (run 1) | 489.3 s (+23%) | 130.5 s (run 1) | 139.6 s (+7%) |
| sighash, 510,500 | 396.8 s | 563.9 s (+42%) | 164.5 s | 182.2 s (+11%) |

SP1 compressed proof 1,272,601 bytes (bincode of the whole proof object).

## Hazync's own guest, same block, same card

Release v0.26.0 CUDA host (image 46666b6f, product). v0.26.1's host aborted on chunk 0 ("context is destroyed",
the known stale-sppark release-build defect, fixed in v0.27.1) — log kept in hz_v0261_abort/.
Coin log / trie around the block MODELLED (cl_synth_around), as in the 962,000 runs: real block cycles, not history.

| step | secs |
|---|---|
| hints + execute check (CPU) | 242 |
| 16 chunks, serial, one card | 1,172 |
| aggregate (trie update first), 53 segments | 486 |
| **block, one card** | **1,658** (succinct receipt VERIFIED) |

GPU utilisation mean 58% over the run.
