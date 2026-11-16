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
| size | 1,120,683 bytes raw, 314,561 gzipped |
| sha256 (this commit, `./build.sh`) | `89544a8e3fc3482eab519f054eb439f6244ecb398b38bbc6bfd5b37474296372` (headers image `de54b94f`; Rust 1.99.0 in the pinned container `reproduce/wasm.Dockerfile`, fixed stage path; laptop and hazync-coord, 16.11.2026; with the review fix of 16.11.2026 ("ok" and `included` need the tip compared with `expect`); before it `8eb3cfe9…`) |
| reproduces | only with the same Rust and at the default stage path, as the core's (swiftsync-verify-wasm/README.md) |
| tests | `test-headers.mjs` (WASM_HEADERS_OK), run by `../test.sh` in CI's zapps job |
