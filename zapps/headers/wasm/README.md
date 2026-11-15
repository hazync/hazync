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
| size | 1,120,493 bytes raw, 314,624 gzipped |
| sha256 (this commit, `./build.sh`) | `9f1526185fde428963245c8251fa54d6211a9b4da1e81428f0cdeaa1c554bc87` (headers image `de54b94f`; Rust 1.99.0, built on hazync-coord, 2026-11-15) |
| reproduces | only with the same Rust and at the default stage path, as the core's (swiftsync-verify-wasm/README.md) |
| tests | `test-headers.mjs` (WASM_HEADERS_OK), run by `../test.sh` in CI's zapps job |
