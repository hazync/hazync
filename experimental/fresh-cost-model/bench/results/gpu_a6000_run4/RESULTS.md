# Run 4: Hazync's aggregate and trie update on OpenVM, PROVED on one RTX A6000 (24.11.2026)

Pod 4prxuku4g241ol, $0.69/hr, 23:51-00:08 UTC = 16.5 min, about $0.19. Every proof verified (`cargo openvm verify`).
Executables re-metered on the build box before upload (run4_prep.sh): outputs and costs as below.

| proof | what | OpenVM cost | app proof | one STARK | output check |
|---|---|---|---|---|---|
| aggregate (`ovm-agg`) | Hazync guest main.rs mode 5: coin log, block rules, live-coin hash, BIP30, block forest | 7.62B | **45.6 s** | **49.1 s** | ChainState journal SHA-256 95ab76d0…43221 = RISC Zero's, same input |
| trie, same format | mode 17 + ovm_iv, zerocopy, ovm_sha (bench/trie2) | 6.69B | **34.4 s** | **36.1 s** | journal 74b08f52… = native libzync |
| trie, new format | + ovm_nat (node hashes native state words) | 5.23B | **27.2 s** | | journal b7d58aea… = native (new-format trieprep) |
| trie, 1/16 part | same-format exe, bucket 0 (773 of 12,206 updates) | 0.43B | **4.1 s** | | cefed2d1… = native for that part |

STARK proof size 630,764 B. GPU utilisation mean 69%.

Same block, same card, Hazync v0.26.0 product (run 2): trie update 211.8 s; aggregate 274 s (486 s step incl. the trie).
=> aggregate 274 -> 49 s (5.6x), trie 212 -> 36 s (5.9x) same format / 27 s (7.8x) new format.
