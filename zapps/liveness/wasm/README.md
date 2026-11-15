# `zapp-liveness-wasm`: the coin-liveness zyncAPP in a browser

The liveness check of `zapp-liveness` (one library, `zapps/liveness`), compiled to WebAssembly: a page verifies a core
receipt and one outpoint's state after a height against the question it asks, on the device; nothing is uploaded. A
module of its own (docs/libzync/ZYNCAPPS.md §9), NOT part of the core verifier's `hazync-verify.wasm`, which it does
not change (until 15.11.2026 this was the `"liveness"` key of the core's `verifyFacts`). Raw ABI, no wasm-bindgen, as
the core's; glue: `zapp-liveness.js` (`loadLiveness`, `verifyLiveness`).

```sh
./build.sh                 # byte-reproducible: the core's flags (../../../scripts/wasm-build.sh), Rust 1.99.0, stage /tmp/zapp-liveness-wasm-build
node test-liveness.mjs     # 13 cases on the real mainnet 1..200 proof (8888d59c) and ../vectors
```

| | |
|---|---|
| size | 1,189,194 bytes raw, 341,737 gzipped |
| sha256 (this commit, `./build.sh`) | `64cf09f94205134ddb597d3abebebd724dd230882ebe867a73f81486bffbb25d` (Rust 1.99.0, 2026-11-15) |
| reproduces | only with the same Rust and at the default stage path, as the core's (swiftsync-verify-wasm/README.md) |
| tests | `test-liveness.mjs` 13/13, run by `../test.sh` in CI's zapps job |

No lineage text here (the core's optional last chunk): an image the built-in table does not list, with its own
handovers, is checked with the core verifier.
