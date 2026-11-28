# zapps/assume fuzz targets

`fuzz_targets/` holds the libFuzzer targets; `tests/replay.rs` replays the committed corpus (`corpus/<target>/`) on
stable Rust (`cargo test --release --manifest-path fuzz/Cargo.toml`). A crash input a round finds is kept under
`artifacts/<target>/` until its fix lands, then moves to the corpus.

**Retired 28.11.2026 (review R3-5 L6):** `artifacts/assume_core_receipt/crash-poseidon2-unreduced-word-at-225087.rcpt`
and `leak-1ef043b6d9171d0e2e2f6f90c4c431d3c5c16498`, both RISC Zero receipt inputs committed with the targets (a21c02ef, 18.11.2026). Since the
cutover (27.11.2026) the verifier takes engine (OpenVM) proof files only and refuses a RISC Zero receipt by its first
bytes, before any decoder sees it, so neither input reaches the code they crashed. They stay in git history.
