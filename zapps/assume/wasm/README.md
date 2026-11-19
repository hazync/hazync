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
| size | 1,165,792 bytes raw, 331,972 gzipped |
| sha256 (this commit, `./build.sh`) | `582dc8fafd608752582029c87fd615529598875e28e744303847f96b3c686075` (feat/zapp-assume on main after #762 strict facts; built in the published image, laptop). Before it `5be95c43f6ca5d0add836b85792112bd80fd6078ef754165c733b50beaa289c8` (merged with main's #758: the core verifier it links decodes receipts strictly, and zapp-corecheck's `image_used` decodes through it; built in the published image `ghcr.io/hazync/hazync-wasm-build@sha256:d46e4164…`, laptop and hazync-coord (a fresh clone of github main plus this branch's commits as a git bundle), 18.11.2026). Before it `09704efb8ba5baef0e63a1acbb669effaa398f5149a98b11f1ba7124c2c3bc63` (Rust 1.99.0 in the pinned container reproduce/wasm.Dockerfile, the published image `ghcr.io/hazync/hazync-wasm-build@sha256:d46e4164…` (tag 27a0579f77c95178), fixed stage path; laptop and hazync-coord (a fresh clone of 30991669), 18.11.2026; with the header-run height fix of 18.11.2026 (`HeaderChain::tip_height` without an intermediate overflow, a run no chain has an error); before it `f5a4e5c7…`) |
| reproduces | only with the same Rust and at the default stage path, as the core's (swiftsync-verify-wasm/README.md) |
| tests | `test-assume.mjs` 11/11, run by `../test.sh` in CI's zapps job |

**Not for a mainnet snapshot.** The page passes the whole file in one frame: it is held twice (the page's copy and the
module's), each chunk's length is a u32 (4 GiB at most), and a 32-bit WebAssembly memory is 4 GiB in all, while
mainnet's snapshot is several GB. Use it for low heights and demonstrations; check a mainnet snapshot with the CLI or
the C ABI, which stream it (docs/libzync/ASSUME_ZAPPS.md §1).

The frame has no lineage chunk (the core's optional last one): an image the built-in table does not list, with its own
handovers, is checked with the core verifier.
