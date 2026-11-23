# ovm-agg — Hazync's guest `main.rs` (aggregate, mode 5) compiled for OpenVM

`assemble.sh <hazync checkout>` copies the RISC Zero guest (prover/methods/guest) to prover/methods/ovmagg in that
checkout, so every `#[path]` include (coinbase-smt, libzync) resolves unchanged, and applies the ONLY edits:
- `edit_main.py`: `mod xver` (cross-version handover: vendored old RISC Zero recursion circuits) -> a stub that panics
  if called (mainnet's coin-log lineage is empty, so the aggregate never calls it); main() reveals the journal hash.
- `batch_ecdsa.rs` replaced by `batch_ecdsa_ovm.rs` (same public items; OpenVM ECC instead of risc0 bigint2 blobs).
- Cargo.toml / build.rs / openvm.toml replaced (OpenVM deps; C/C++ built as bench/ovm-core does).
Compat crates under the names the guest imports, implemented on OpenVM:
- `risc0-zkvm`: guest::env (read / read_slice over the same byte stream, verify = NOT CHECKED (recorded only; recursion
  is OpenVM's own job), commit -> journal, log, exit) and serde (vendored from risc0-zkvm 3.1.0, Apache-2.0).
- `risc0-crypto`: the `Fq` / `BigInt` the live-coin hash (swiftsync.rs) uses, on OpenVM's modular extension.
- `sha2`: Sha256 / Digest / compress256 on OpenVM's SHA-256 extension.
