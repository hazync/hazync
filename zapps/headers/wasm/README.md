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
| size | 1,139,635 bytes raw, 321,541 gzipped |
| sha256 (this commit, `./build.sh`) | `9217726d436912bf98d4caf2de27264f996ca614b44f3f73e45b517119620caa` (headers image `de54b94f`; Rust 1.99.0 in the published image ghcr.io/hazync/hazync-wasm-build@sha256:d46e4164…, HAZYNC_WASM_REQUIRE_CLEAN=1 HAZYNC_WASM_REQUIRE_PUBLISHED=1, fixed stage path; laptop and a fresh clone on hazync-coord, 18.11.2026; with the review fix of 18.11.2026 (the include JSON and its /tx answer take only their documented keys, each once); before it `89544a8e…`, before 16.11.2026's `8eb3cfe9…`) |
| reproduces | only with the same Rust and at the default stage path, as the core's (swiftsync-verify-wasm/README.md) |
| tests | `test-headers.mjs` (WASM_HEADERS_OK), run by `../test.sh` in CI's zapps job |
