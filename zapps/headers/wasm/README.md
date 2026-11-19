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
| size | 1,141,077 bytes raw, 322,157 gzipped |
| sha256 (this commit, `./build.sh`) | `bc0fa13a46028b047468834f56bebcc8a9ab3b23537dd54952e164334785f5b7` (fix/headers-include-strict on main's strict facts (#762): the include JSON and its /tx answer take only their documented keys, each once; an explicit null txid or tx is refused; built in the published image, laptop). Before it `8c6a87bf984d00f8aa9b22bc6f65feef77e2cb6c891966dc2eaf547aae6a09ad` (fix/triefact-strict: the core verifier it links refuses unknown fact fields and facts-JSON keys; headers source unchanged; built in the published image, HAZYNC_WASM_REQUIRE_CLEAN=1 HAZYNC_WASM_REQUIRE_PUBLISHED=1; laptop and hazync-coord (a fresh clone), 18.11.2026). Before it `b5f89b8d811fa7436d83c4966aa78d738d0cae270765eea075b2023ed5ed54ed` (fix/receipt-hardening: the reader decodes receipts with the core's strict decoder and journal guard and refuses under RISC0_DEV_MODE; built in the published image, laptop and CI's zapps job equal, 18.11.2026). Before it `89544a8e3fc3482eab519f054eb439f6244ecb398b38bbc6bfd5b37474296372` (headers image `de54b94f`; Rust 1.99.0 in the pinned container `reproduce/wasm.Dockerfile`, fixed stage path; laptop and hazync-coord, 16.11.2026; with the review fix of 16.11.2026 ("ok" and `included` need the tip compared with `expect`); before it `8eb3cfe9…`) |
| reproduces | only with the same Rust and at the default stage path, as the core's (swiftsync-verify-wasm/README.md) |
| tests | `test-headers.mjs` (WASM_HEADERS_OK), run by `../test.sh` in CI's zapps job |
