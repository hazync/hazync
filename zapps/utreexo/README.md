# `zapps/utreexo`: proven Utreexo roots (assumeutreexo) for Floresta

The Utreexo roots zyncAPP's own module (design, soundness, measurements: `docs/libzync/UTREEXO_ZAPP.md`).

| part | where |
|---|---|
| the check library `zapp_utreexo` and the CLI `zapp-utreexo` | here (`src/`), using `swiftsync-verify` as a library |
| the WASM build | `wasm/` |
| real receipts (CPU proofs, mainnet H = 2 under T = 3, H = 3 in one run and in two runs) | `vectors/` |
| the guest | `prover/zyncapps/methods/utreexo` (with the other zAPP guests) |
| the derivation shared by guest and host | `libzync/src/utreexo_roots.rs` |
| the prover (`zyncapp utreexo-prove`) | `prover/zyncapps/host` |
| the reference (Floresta's own code) | `prover/zyncapps/utreexo-ref` |

```sh
cargo test --release                        # the real receipts: DEMO (3 blocks are below minimum work), and refusals
cargo run --release --manifest-path cli/Cargo.toml -- verify vectors/mainnet-3.receipt --expect 3:0000000082b5015589a3fdf2d4baff403e6f0be035a5d9742c1cae6295464449 --floresta
```
