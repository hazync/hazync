# `zapp-headers-wasm`: the header-chain zyncAPP in a browser

The header-chain check of `zapp-headers` (one library, `zapps/headers`), compiled to WebAssembly: a page checks a
proof of the mainnet header chain from genesis, and that a block (by its MMR path) and a transaction in it (by its
merkle branch) are on that chain, on the device; nothing is uploaded. A module of its own (docs/libzync/ZYNCAPPS.md
§9), NOT part of the core verifier's `hazync-verify.wasm` (until 15.11.2026 `verifyHeaders` was the core's). Raw ABI,
no wasm-bindgen, as the core's; glue: `zapp-headers.js` (`loadHeaders`, `verifyHeaders`, `headersImageId`).

```sh
./build.sh               # byte-reproducible: the core's flags (../../../scripts/wasm-build.sh), Rust 1.99.0, stage /tmp/zapp-headers-wasm-build
node test-headers.mjs    # the real chained receipts and MMR / transaction inclusion cases (experimental/libzync-bench/headers)
```

| | |
|---|---|
| size | 1,120,635 bytes raw, 314,537 gzipped |
| sha256 (this commit, `./build.sh`) | `8eb3cfe9025841ee6eb0d6b798e5441b13928106ac1fecd4b05024e08af4e9f5` (headers image `de54b94f`; Rust 1.99.0, default stage path, reproduced 16.11.2026 on a box whose builds reproduce the core's published `f16b5b98…`; the library no longer links zapps/common; before it `8adc91dc…`) |
| reproduces | only with the same Rust and at the default stage path, as the core's (swiftsync-verify-wasm/README.md) |
| tests | `test-headers.mjs` (WASM_HEADERS_OK), run by `../test.sh` in CI's zapps job |
