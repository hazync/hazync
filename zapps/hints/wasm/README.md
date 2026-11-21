# `zapp-hints-wasm`: the SwiftSync hints zyncAPP in a browser

The hints check of `zapp-hints` (one library, `zapps/hints`), compiled to WebAssembly: a page checks a downloaded
hints file against its proof on the device; nothing is uploaded. A module of its own (docs/libzync/ZYNCAPPS.md §9),
NOT part of the core verifier's `hazync-verify.wasm`, which it does not change. Raw ABI, no wasm-bindgen, as the
core's; glue: `zapp-hints.js` (`loadHints`, `verifyHints`, `hintsImageId`).

```sh
./build.sh                                                          # byte-reproducible: the core's flags (../../../scripts/wasm-build.sh), Rust 1.99.0, stage /tmp/zapp-hints-wasm-build
node test-hints.mjs ../vectors 0000000082b5015589a3fdf2d4baff403e6f0be035a5d9742c1cae6295464449   # 6 cases on the real H = 3 receipt
```

| | |
|---|---|
| size | 1,104,880 bytes raw, 307,685 gzipped |
| sha256 (this commit, `./build.sh`) | `1c7f050c4401a303e826654dea1a6bb42d2f71d94069e74fddff8565feacb584` (fix/zapp-parsers-strict: the built-in hints image is the new pin `11c72c44` (strict compact sizes in the walk); hints source otherwise unchanged; built in the published image, HAZYNC_WASM_REQUIRE_CLEAN=1 HAZYNC_WASM_REQUIRE_PUBLISHED=1, hazync-coord (a fresh clone of fa044d02), 21.11.2026). Before it `906676442ce95e34cedbca36c65ccafe6f3e64067d3f39e37e4f3ed2bc3023e7` (fix/triefact-strict: the core verifier it links refuses unknown fact fields and facts-JSON keys; hints source unchanged; built in the published image, HAZYNC_WASM_REQUIRE_CLEAN=1 HAZYNC_WASM_REQUIRE_PUBLISHED=1; laptop and hazync-coord (a fresh clone), 18.11.2026). Before it `348548a0e8250f442905b530f3951565bfcb9daa6dcd97df716e725e247fffc7` (fix/receipt-hardening: the core verifier it links decodes receipts strictly; hints source unchanged; built in the published image, laptop and CI's zapps job equal, 18.11.2026). Before it `11d43ced5e63508d5138b82ba7123a191838b731aebb1607e79aa96977e2fc5a` (hints image `1894f12e`; Rust 1.99.0 in the published build image (reproduce/wasm-image.digests), fixed stage path; laptop, 17.11.2026; on swiftsync-verify without its `headers` module, moved to `zapps/headers` (#724). The same hints source on swiftsync-verify with that module gave `33cf56a4…` (16.11.2026, laptop and hazync-coord); before the review fix of 16.11.2026 (a core tip given is compared also when T = H) `7c9c7107…`) |
| reproduces | only with the same Rust and at the default stage path, as the core's (swiftsync-verify-wasm/README.md) |
| tests | `test-hints.mjs` 6/6, run by `../test.sh` in CI's zapps job |
