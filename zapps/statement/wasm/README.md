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
| size | 2,473,562 bytes raw, 1,518,554 gzipped |
| sha256 (this commit, `./build.sh`) | `01408a2c3bcd621261e45ef7bd9fde083f635fb5c64eee89780413a324cfd27a` (Rust 1.99.0, default stage path, built on hazync-coord, 15.11.2026; with the frame's overflow-safe take; the first module build was `81a7a1ef…`) |
| reproduces | only with the same Rust and at the default stage path, as the core's (swiftsync-verify-wasm/README.md) |
| tests | `test-statement.mjs` 8/8 against the real mainnet 1..200 proof (`8888d59c`), run by `../test.sh` in CI's zapps job |
