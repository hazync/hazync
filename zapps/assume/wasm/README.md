# `zapp-assume-wasm`: the assumevalid and assumeutxo zyncAPPs in a browser

The checks of `zapp-assume` (one library, `zapps/assume`), compiled to WebAssembly: a page checks the block it would
give a node as `-assumevalid`, and a Core `dumptxoutset` snapshot, against a verified core proof, on the device;
nothing is uploaded. A module of its own (docs/libzync/ZYNCAPPS.md §9), NOT part of the core verifier's
`hazync-verify.wasm` (until 15.11.2026 `verifyAssume` was the core's). Raw ABI, no wasm-bindgen, as the core's; glue:
`zapp-assume.js` (`loadAssume`, `verifyAssume`).

```sh
./build.sh             # byte-reproducible: the core's flags (../../../scripts/wasm-build.sh), Rust 1.99.0, stage /tmp/zapp-assume-wasm-build
node test-assume.mjs   # 11 cases on the real mainnet 1..200 proof and Core v31.1.0's dumptxoutset files (../vectors)
```

| | |
|---|---|
| size | 1,158,643 bytes raw, 328,398 gzipped |
| sha256 (this commit, `./build.sh`) | `5be89ffcbef1df9072d753fb4db794d06d1a377476b7c0f0036209d85e358e93` (Rust 1.99.0, default stage path, built on hazync-coord, 2026-11-15) |
| reproduces | only with the same Rust and at the default stage path, as the core's (swiftsync-verify-wasm/README.md) |
| tests | `test-assume.mjs` 11/11, run by `../test.sh` in CI's zapps job |

The frame has no lineage chunk (the core's optional last one): an image the built-in table does not list, with its own
handovers, is checked with the core verifier.
