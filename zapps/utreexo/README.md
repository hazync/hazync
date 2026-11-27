# `zapps/utreexo`: proven Utreexo roots (assumeutreexo) for Floresta

The Utreexo roots zyncAPP's own module (design, soundness, measurements: `docs/libzync/UTREEXO_ZAPP.md`).

| part | where |
|---|---|
| the check library `zapp_utreexo` and the CLI `zapp-utreexo` | here (`src/`), using `swiftsync-verify` as a library |
| the WASM build | `wasm/` |
| engine (OpenVM) proofs: base 100 and a tip step 100 -> 150, on the engine chain's core proof of block 200 | `tests/data/engine/` (ids in `ids.txt`), `tests/engine.rs` |
| the guest | `prover/zyncapps/methods/utreexo` (with the other zAPP guests) |
| the derivation shared by guest and host | `libzync/src/utreexo_roots.rs` |
| the prover (`zyncapp utreexo-prove`) | `prover/zyncapps/host` |
| the reference (Floresta's own code) | `prover/zyncapps/utreexo-ref` |

```sh
cargo test --release --features tip         # the engine proofs: DEMO (200 blocks are below minimum work), and refusals
bash test.sh                                # the CLI and WASM cases
V=tests/data/engine
cargo run --release --manifest-path cli/Cargo.toml -- verify $V/base100.hze --image 340f6675bafcdd893c12e0dfae0391043cc7fd8919b526ff756ec13d5d76cc95 \
  --vk $V/base100.vk --vk-sha256 <the key file's pin> --core-image c7fe0eb85d2d4c10532742191b059a2adcc3e24e22aa81c1e632e2ee718d2fc9 \
  --expect 100:000000007bc154e0fa7ea32218a72fe2c1bb9f86cf8c9ebf9a715ed27fdb229a --floresta
```

The last line prints `ZAPP_UTREEXO_DEMO` (exit 2): block 100's roots on a chain below Core's minimum work. No engine
image is published yet (the Stage 4 freeze), so every call names the utreexo image, its key and the core image.
