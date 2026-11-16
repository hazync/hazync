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
| size | 1,165,029 bytes raw, 331,195 gzipped |
| sha256 (this commit, `./build.sh`) | `f5a4e5c76f9bc446f76ecb3aa8e75382f1d6afff14376ba49813403efd36423e` (Rust 1.99.0 in the pinned container reproduce/wasm.Dockerfile, fixed stage path; laptop and hazync-coord, 16.11.2026; with the review fixes of 16.11.2026 (an --image pinned exactly, the image that verified reported); before them `cbf31ea2…`) |
| reproduces | only with the same Rust and at the default stage path, as the core's (swiftsync-verify-wasm/README.md) |
| tests | `test-assume.mjs` 11/11, run by `../test.sh` in CI's zapps job |

**Not for a mainnet snapshot.** The page passes the whole file in one frame: it is held twice (the page's copy and the
module's), each chunk's length is a u32 (4 GiB at most), and a 32-bit WebAssembly memory is 4 GiB in all, while
mainnet's snapshot is several GB. Use it for low heights and demonstrations; check a mainnet snapshot with the CLI or
the C ABI, which stream it (docs/libzync/ASSUME_ZAPPS.md §1).

The frame has no lineage chunk (the core's optional last one): an image the built-in table does not list, with its own
handovers, is checked with the core verifier.
