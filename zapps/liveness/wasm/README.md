# `zapp-liveness-wasm`: the coin-liveness zyncAPP in a browser

The liveness check of `zapp-liveness` (one library, `zapps/liveness`), compiled to WebAssembly: a page verifies a core
receipt and one outpoint's state after a height against the question it asks, on the device; nothing is uploaded. A
module of its own (docs/libzync/ZYNCAPPS.md §9), NOT part of the core verifier's `hazync-verify.wasm`, which it does
not change (until 15.11.2026 this was the `"liveness"` key of the core's `verifyFacts`). Raw ABI, no wasm-bindgen, as
the core's; glue: `zapp-liveness.js` (`loadLiveness`, `verifyLiveness`).

```sh
./build.sh                 # byte-reproducible: the core's flags (../../../scripts/wasm-build.sh), Rust 1.99.0, stage /tmp/zapp-liveness-wasm-build
node test-liveness.mjs     # 31 cases on the real mainnet 1..200 proof (v0.25.0) and ../vectors
```

| | |
|---|---|
| size | 1,197,512 bytes raw, 345,921 gzipped |
| sha256 (this commit, `./build.sh`) | `f6594771a9bce4e6a1b73471f821d01246ebc4a94f013feb554bba4165372d3a` (Rust 1.99.0 in the published image ghcr.io/hazync/hazync-wasm-build@sha256:d46e4164…, HAZYNC_WASM_REQUIRE_CLEAN=1 HAZYNC_WASM_REQUIRE_PUBLISHED=1, fixed stage path; laptop and a fresh clone on hazync-coord, 18.11.2026; with the review fix of 18.11.2026 (the facts JSON takes only `"liveness"`); before it `42ac0589…`, before 16.11.2026's review fixes `232ea26d…`) |
| reproduces | only with the same Rust and at the default stage path, as the core's (swiftsync-verify-wasm/README.md) |
| tests | `test-liveness.mjs` 31/31, run by `../test.sh` in CI's zapps job |

No lineage text here (the core's optional last chunk): an image the built-in table does not list, with its own
handovers, is checked with the core verifier.
