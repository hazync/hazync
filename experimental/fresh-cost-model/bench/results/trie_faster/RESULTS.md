# Making Hazync's counting-trie update faster on OpenVM — CPU executor, block 976,379 (24.11.2026)

Workload: Hazync guest mode 17 (the per-block trie update), block 976,379's real update list (5,134 marks, 7,072
inserts, built with libzync's own `coin_key` / `spender`), the old trie modelled at 2.6e9 addresses as Hazync's
measurement path does (`trie_model::synth`, sub 2, counts) -> 1,308,817-word witness. RISC Zero (Hazync v0.26.0
product, same block, A6000) PROVED this in 211.8 s. Nothing here is proved on a GPU: OpenVM metered cost only.
Code: `bench/trie2/` (guest `ovm-trie`, input tool `trieprep`, `run_variant.sh`, `libzync-fresh-trie.patch` against
libzync at main 261f0a59, raw logs `variants{1,2,3}.log`).

## Where the work is (ablations)

- 324,278 SHA-256 compressions for 12,206 updates. ~7,000 distinct scripts touched, each ~44 compressions: ~18 unshared
  top-trie levels hashed twice (old + new), the address leaf (2 + 2) and its coin sub-trie. A binary SHA-256 Merkle
  trie takes 2 children per compression, so wider nodes do not reduce the count; the old path must be recomputed to
  check the witness. The count is near its floor for this structure.
- `decodeonly` (inputs read and parsed, update-list digest, journal; no tree walk): **88.4M of 231.6M instructions**.
  Almost all of it is `ttree::list_digest`, which hashes ~1.24 MB with RustCrypto `sha2` in SOFTWARE (~19k compressions).
- `nohash` / `floor`: replacing the SHA intrinsic with an XOR cuts cost by ~2.9-3.3B: the SHA extension is ~9-10k cost
  units per compression (27% of the base variant's cost, 55% of the best one's); the rest is plain RV32 tree logic.

## Results

| variant | format | instructions | OpenVM cost | compressions | journal SHA-256 |
|---|---|---|---|---|---|
| original port (run of 23.11, shim with byte-array staging) | same | 268.2M | 13.59B | 324,278 | 74b08f52… = native |
| base (cleaner shim: word-staged block) | same | 231.6M | 12.12B | 324,278 | 74b08f52… = native |
| zero-copy input words | same | 205.5M | 11.07B | 324,278 | 74b08f52… |
| native-IV SHA call (`ovm_iv`: no input-state swaps) | same | 200.1M | 10.77B | 324,278 | 74b08f52… |
| native IV + zero-copy | same | 174.0M | 9.72B | 324,278 | 74b08f52… |
| accelerated `list_digest` only (`ovm_sha`) | same | 156.4M | 9.09B | 324,278 | 74b08f52… |
| **native IV + zero-copy + accelerated digest** | **same** | **98.9M** | **6.69B** | 324,278 | **74b08f52… = native** |
| + fat LTO / 1 codegen unit (env) | same | 98.9M | 6.69B | | identical numbers: the env override did not reach the guest build, or is already the effective profile — no gain shown |
| **native-word hashing (`ovm_nat`) + zero-copy + accelerated digest** | **NEW** | **64.5M** | **5.23B** | 324,278 | b7d58aea… = native (new-format trieprep) |
| floor: `ovm_nat` levers with SHA replaced by XOR (ablation) | — | 56.4M | 2.35B | (324,278 calls) | wrong on purpose |
| 16-way split by script-hash prefix, part 0 (773 updates), same-format levers | same | 6.32M | 0.429B | 20,851 | cefed2d1… = native for that part |
| 16-way split, part 7 (801 updates) | same | 6.14M | 0.414B | 19,895 | 4e768c93… = native for that part |

16 × ~0.42B ≈ 6.8B vs 6.69B for the whole update (+2%): the work splits linearly.

## What each one is

**Same format (drop-in; every root, journal, witness and zyncAPP fact unchanged):**
1. `ovm_sha` — `ttree::list_digest` computes the same SHA-256 on OpenVM's SHA-256 extension instead of software
   `sha2` (`sha256_ovm` in the patch). Biggest single lever.
2. `ovm_iv` — on OpenVM, `coinlog` passes the IVs as native SHA state words, so the compression shim swaps only its
   8 output words (to the digest bytes `W` stores), not its 8 input words too.
3. `zerocopy` — the guest reads the input words in place instead of rebuilding a `Vec<u32>`.
Together: **13.59B -> 6.69B (2.03x)**, journal identical.

**New format (`ovm_nat`): a node hashes its children's SHA-256 STATE WORDS as little-endian bytes**, so no byte swap is
needed anywhere on OpenVM. Same structure, same security (a fixed bijection of each 32-byte child), same proof sizes;
every root changes, so it is a new trie format: host (`coinlog::compress_w` non-zkvm path, in the patch), guest,
witnesses and the zyncAPP verifiers would all move together. **6.69B -> 5.23B (another 1.28x; 2.6x vs the original
port).** It is RISC Zero's convention (digest bytes as words) that forced the swaps; the swap-free form is OpenVM's.

**Partitioned update (same format, for LATENCY not cost):** the top 4 levels are full at this population, so the update
splits into 16 independent parts by the first nibble of the script hash, each provable on its own card, plus a tiny
combine (hash 16 subtree roots: ~30 compressions). Measured: one part ≈ 0.42B, i.e. ~1/16 of the whole. The combine
step and per-part subtree-root journals are NOT built; the measured parts prove old->new for the whole root with only
their slice of updates, which costs the same as the slice's subtree plus 4 one-sided levels.

## What I looked at and did not pursue (and why, from the numbers above)

- Wider-arity nodes: SHA-256 compresses exactly 64 bytes = 2 children, so a k-ary node costs k/2 compressions per
  log2 k levels — the same count; it only changes witness layout. No compression saving to measure.
- An append-only MMR / Merkle-ised sorted list of coin events: cheap appends, but "balance of script S" and "S has no
  other coin" need a keyed index — that is the trie. It gives up the per-address query the trie exists for.
- Batching several blocks into one trie update: shares a few more top levels; at ~7,000 scripts/block the shared depth
  grows by ~log2(N) levels out of ~31, i.e. ≈ 2 compressions per script saved for N = 6 (~5%), and tip answers lag.
- Polynomial / Verkle-style commitments or a non-SHA hash: no OpenVM guest accelerator to make them cheap, and they
  add trust surface; SHA-256 is what OpenVM accelerates.

## Recommendation

1. Take the three same-format levers now: **2.0x** on the trie with no format change (journal-identical).
2. If the OpenVM migration happens (a new image re-anchored from genesis anyway), adopt the native-word hash
   convention at the same time: **2.6x** total vs the first port, for free, since every root is new at a re-anchor.
3. For tip latency, split the trie update 16 ways by script-hash prefix (measured: linear), a few seconds per part.
4. The structure itself (binary SHA-256 trie, ~44 compressions per touched script) is near its floor; what remains is
   ~2.35B of the 5.23B (floor ablation) in plain tree-walk logic: further gains are in the walk's instruction count,
   not in the trie design.

Rough GPU equivalent (NOT measured; the Core guest proved at 5.9 ns per cost unit on an A6000): original 13.59B ≈ 80 s,
same-format 6.69B ≈ 40 s, new format 5.23B ≈ 31 s, one 16th ≈ 2.5 s — against RISC Zero's measured 211.8 s.
