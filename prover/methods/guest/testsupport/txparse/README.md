# txparse: the Rust transaction parsers against Core's deserialiser

Review round 2, R2-3 #2 (engine/reviews/round2/r2_3_chunk_trie.md): the trie keys, output script hashes and undo
records come from Rust parsers in `agg_lite.rs`, separate from Core's C++ parse of the same bytes. This test runs both
on one corpus and compares. Nothing here is compiled into any guest.

Run: `prover/methods/guest/test-txparse.sh` (about 20 s on the laptop with a Core clone given in `HAZYNC_CORE_SRC`;
it fetches a shallow v28.0 clone otherwise).

| file | what it is |
|---|---|
| `harness.cpp` | Core's side: Core v28.0's `UnserializeTransaction` (with and without witness) and `std::vector<CTxOut>` reader, txid / wtxid by re-serialisation, and the guest's raw-byte txid code (`verify_input.cpp` `HAZYNC_RAW_TXID`, extracted verbatim) |
| `rust/` | the Rust side; `build.rs` copies `compact_size`, `tx_io`, `prevout_fields` (agg_lite.rs) and the hints / utreexo readers out of the shipped sources, so the test runs the shipped text; `libzync::facts::parse_tx` is called directly |
| `gen_corpus.py` | the corpus, deterministic: the repo's real mainnet fixtures, Core's tx_valid / tx_invalid, and mutations |
| `compare.py` | the rules (what counts as a mismatch, what is stricter by design) |

## Result, 28.11.2026 (laptop, engine/openvm)

Corpus: 23,311 distinct real mainnet transactions and coinbases (prover/block_*.json, prover/testdata/boundaries/*.json;
9,069 also in stripped form), 23,295 prevouts blobs, 213 Core v28.0 tx_valid / tx_invalid transactions, 83,723
mutations; 139,620 lines.

- **0 mismatches.** On every input Core accepts (42,382 transactions with witness; 23,595 prevouts blobs) `tx_io`
  gives Core's outpoints and output scripts and `prevout_fields` Core's amounts and scripts; the guest's raw-byte
  txid / wtxid equals Core's re-serialised one on all 40,128 fully consumed inputs; `facts::parse_tx` and the hints /
  utreexo readers accept nothing Core refuses and read every Core-accepted input as Core does, apart from the forms
  they refuse by design (no inputs: 212 for facts, 32,357 for hints and utreexo, which take the stripped form only).
- `tx_io` reads 14,494 inputs that Core refuses (it does not read witnesses or the lock time). Not a split: every
  call in the guest follows a Core parse of the same buffer (`tx_vin_count` before it in `ChunkCtx::record`,
  `tx_out_leaves_mv` / `hz_indicated_height` before it for the coinbase), and that parse traps on refusal: no proof.
- Controls: `tx_io` without the marker skip gives 12,537 mismatches (caught). A compact-size reader that takes
  non-canonical encodings gives none, which measures the claim above: the strict reader is defence in depth there.

Not covered: the guest is ILP32 and this runs LP64 (see test-txparse.sh); parsing only, no script semantics.
