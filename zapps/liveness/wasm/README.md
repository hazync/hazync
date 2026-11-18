# `zapp-liveness-wasm`: the coin-liveness zyncAPP in a browser

The liveness check of `zapp-liveness` (one library, `zapps/liveness`), compiled to WebAssembly: a page verifies a core
receipt and one outpoint's state after a height against the question it asks, on the device; nothing is uploaded. A
module of its own (docs/libzync/ZYNCAPPS.md §9), NOT part of the core verifier's `hazync-verify.wasm`, which it does
not change (until 15.11.2026 this was the `"liveness"` key of the core's `verifyFacts`). Raw ABI, no wasm-bindgen, as
the core's; glue: `zapp-liveness.js` (`loadLiveness`, `verifyLiveness`).

```sh
./build.sh                 # byte-reproducible: the core's flags (../../../scripts/wasm-build.sh), Rust 1.99.0, stage /tmp/zapp-liveness-wasm-build
node test-liveness.mjs     # 32 cases on the real mainnet 1..200 proof (v0.25.0) and ../vectors
```

| | |
|---|---|
| size | 1,197,810 bytes raw, 346,235 gzipped |
| sha256 (this commit, `./build.sh`) | `fbb5af9ebf4a8b20a713b19e0e60cd52cbfdc1e141ff115034b3e58213840576` (merged with main's #758: the core verifier it links decodes receipts strictly, and zapp-corecheck's `image_used` decodes through it; built in the published image ghcr.io/hazync/hazync-wasm-build@sha256:d46e4164…, laptop and hazync-coord (a fresh clone of github main plus this branch's commits as a git bundle), 18.11.2026). Before it `ce08ac1744c18c1d77a28b9f4a3c0456a3350577c46fcda397da172bad1f1297` (Rust 1.99.0 in the published image ghcr.io/hazync/hazync-wasm-build@sha256:d46e4164…, HAZYNC_WASM_REQUIRE_CLEAN=1 HAZYNC_WASM_REQUIRE_PUBLISHED=1, fixed stage path; laptop and a fresh clone on hazync-coord, 18.11.2026; with the review fix of 18.11.2026 (the facts JSON takes only `"liveness"`; `"expect"` refused in any form, null too); before it `f6594771…`, before that `42ac0589…`, before 16.11.2026's review fixes `232ea26d…`) |
| reproduces | only with the same Rust and at the default stage path, as the core's (swiftsync-verify-wasm/README.md) |
| tests | `test-liveness.mjs` 32/32, run by `../test.sh` in CI's zapps job |

No lineage text here (the core's optional last chunk): an image the built-in table does not list, with its own
handovers, is checked with the core verifier.
