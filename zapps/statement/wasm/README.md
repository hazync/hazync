# `zapp-statement.wasm`: the Balance statement zyncAPP's checker in a browser

The statement check of `zapps/statement` (docs/libzync/STATEMENT_ZAPP.md) compiled to WebAssembly, on top of the core
verifier library, which verifies the receipt. Its own module: only the statement page loads it, with
`zapp-statement.js`; the core `hazync-verify.wasm` carries no statement code. Raw ABI, no wasm-bindgen.

```sh
./build.sh                       # byte-reproducible: the core's flags (../../../scripts/wasm-build.sh), Rust 1.99.0, stage /tmp/zapp-statement-wasm-build
node test-statement.mjs target/wasm32-unknown-unknown/release/zapp_statement_wasm.wasm <statement.json> <purpose> <nonce>
                                 # or ../test.sh (CI's zapps job), which builds it and makes the statement
```

| | |
|---|---|
| size | 2,485,794 bytes raw, 1,522,758 gzipped |
| sha256 (this commit, `./build.sh`) | `5a284b26d11d1b6c2e9c95243dc63c8c816bc4530324a979225af291b64e61be` (Rust 1.99.0, clang 18.1.3 in `reproduce/wasm.Dockerfile`, default stage path; the same on a laptop and on hazync-coord, 16.11.2026. This module compiles C (secp256k1-sys, through bitcoin), so `scripts/wasm-build.sh` builds it in that pinned container: with the host's clang it was `510c205b…` under clang 14.0.0 and `5a284b26…` under 18.1.3) |
| reproduces | through `./build.sh` on any machine with docker (it builds in `reproduce/wasm.Dockerfile`: Rust 1.99.0, clang 18.1.3 from a pinned Ubuntu snapshot), at the default stage path |
| tests | `test-statement.mjs` 15/15 against the real mainnet 1..200 proof (`8888d59c`), run by `../test.sh` in CI's zapps job |
