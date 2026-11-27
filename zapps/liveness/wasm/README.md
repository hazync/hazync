# `zapp-liveness-wasm`: the coin-liveness zyncAPP in a browser

The liveness check of `zapp-liveness` (one library, `zapps/liveness`), compiled to WebAssembly: a page verifies a core
engine proof and one outpoint's state after a height against the question it asks, on the device; nothing is uploaded. A
module of its own (docs/libzync/ZYNCAPPS.md §9), NOT part of the core verifier's `hazync-verify.wasm`, which it does
not change (until 15.11.2026 this was the `"liveness"` key of the core's `verifyFacts`). Raw ABI, no wasm-bindgen, as
the core's; glue: `zapp-liveness.js` (`loadLiveness`, `verifyLiveness`).

```sh
./build.sh                 # byte-reproducible: the core's flags (../../../scripts/wasm-build.sh), Rust 1.99.0, stage /tmp/zapp-liveness-wasm-build
node test-liveness.mjs     # 30 cases on the engine chain's proof of mainnet 1..200 and ../vectors/engine
```

| | |
|---|---|
| size | 382,030 bytes raw, 153,106 gzipped (engine only; before the cutover 1,197,878 bytes raw, 346,406 gzipped) |
| sha256 (this commit, `./build.sh`) | `c79fedef1f8fd90d04e2a8be0bd5797cea97aaaceedc7af3923721ec2dcb3559` (engine/openvm, cutover C5: engine proofs only, the liveness module with no RISC Zero code; OpenVM v2.1.0 patched (engine/scripts/wasm_ovm.sh, tree digest 38eccd09) in every build; built in the published image, HAZYNC_WASM_REQUIRE_CLEAN=1 HAZYNC_WASM_REQUIRE_PUBLISHED=1, hazync-coord (a fresh clone of ba9ec0b8) and the laptop (twice), equal, 27.11.2026). Before it `4109f327b1b91c3ce688da3bd5c5c3e39f7e9df4099bd82846dcfb6a963522f7` (engine/openvm: the core verifier it links, with its engine changes (swiftsync-verify's `engine` feature, off in this build: the image record's optional key, the shared statement checks); built in the published image, HAZYNC_WASM_REQUIRE_CLEAN=1 HAZYNC_WASM_REQUIRE_PUBLISHED=1, hazync-coord (a fresh clone of 866cba06) and the laptop (twice), equal, 27.11.2026). Before it `38eb760e341165d8b822df0e4b32155b62060e77df0fcc13529bd14a802857a3` (feat/zapp-liveness on main after #762 strict facts; built in the published image, laptop). Before it `fbb5af9ebf4a8b20a713b19e0e60cd52cbfdc1e141ff115034b3e58213840576` (merged with main's #758: the core verifier it links decodes receipts strictly, and zapp-corecheck's `image_used` decodes through it; built in the published image ghcr.io/hazync/hazync-wasm-build@sha256:d46e4164…, laptop and hazync-coord (a fresh clone of github main plus this branch's commits as a git bundle), 18.11.2026). Before it `ce08ac1744c18c1d77a28b9f4a3c0456a3350577c46fcda397da172bad1f1297` (Rust 1.99.0 in the published image ghcr.io/hazync/hazync-wasm-build@sha256:d46e4164…, HAZYNC_WASM_REQUIRE_CLEAN=1 HAZYNC_WASM_REQUIRE_PUBLISHED=1, fixed stage path; laptop and a fresh clone on hazync-coord, 18.11.2026; with the review fix of 18.11.2026 (the facts JSON takes only `"liveness"`; `"expect"` refused in any form, null too); before it `f6594771…`, before that `42ac0589…`, before 16.11.2026's review fixes `232ea26d…`) |
| reproduces | only with the same Rust and at the default stage path, as the core's (swiftsync-verify-wasm/README.md) |
| tests | `test-liveness.mjs` 32/32, run by `../test.sh` in CI's zapps job |

No lineage text here (the core's optional last chunk): an image the built-in table does not list, with its own
handovers, is checked with the core verifier.
