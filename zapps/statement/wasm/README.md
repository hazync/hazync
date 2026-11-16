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
| size | 2,486,202 bytes raw, 1,522,902 gzipped |
| sha256 (this commit, `./build.sh`) | `510c205bae7f23fa043bbf5abc43805bbcbd06927570fc16c88d98b3ccb96f15` (Rust 1.99.0, default stage path, two builds equal, 16.11.2026, on a box whose builds reproduce the core's published `f16b5b98…` and the assume module's `d69abab3…`; the module no longer links zapps/common. The `f3ceb7cc…` published at e7edf0a8 is not what e7edf0a8 builds: that tree gives `a7a6d98e…` here; before the review fixes `01408a2c…`) |
| reproduces | only with the same Rust and at the default stage path, as the core's (swiftsync-verify-wasm/README.md) |
| tests | `test-statement.mjs` 15/15 against the real mainnet 1..200 proof (`8888d59c`), run by `../test.sh` in CI's zapps job |
