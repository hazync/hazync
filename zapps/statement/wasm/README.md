# `zapp-statement.wasm`: the Balance statement zyncAPP's checker in a browser

The statement check of `zapps/statement` (docs/libzync/STATEMENT_ZAPP.md) compiled to WebAssembly, on top of the core
verifier library, which verifies the receipt. Its own module: only the statement page loads it, with
`zapp-statement.js`; the core `hazync-verify.wasm` carries no statement code. Raw ABI, no wasm-bindgen.

```sh
./build.sh                       # byte-reproducible, as ../../../swiftsync-verify-wasm/build.sh: Rust 1.99.0, staged sources
node test-statement.mjs target/wasm32-unknown-unknown/release/zapp_statement_wasm.wasm <statement.json> <purpose> <nonce>
                                 # or ../test_statement.sh with STATEMENT_WASM set, which makes the statement
```

| | |
|---|---|
| size | 2,473,954 bytes raw, 1,519,905 gzipped |
| sha256 (this commit, `./build.sh`) | `81a7a1ef11e514b4b668a9c7ec6078b1d2b7604dfe0826c9ccbda63d6c25d89c` (Rust 1.99.0, default stage path, two builds, 15.11.2026) |
| tests | `test-statement.mjs` 8/8 against the real mainnet 1..200 proof (`8888d59c`) |
