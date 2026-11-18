# `zapp-deposit-wasm`: the deposit-proof zyncAPP in a browser

The deposit check of `zapp-deposit` (one library, `zapps/deposit`, on the header-chain zAPP `zapps/headers`),
compiled to WebAssembly: a page checks a deposit bundle against the caller's expectations (the txid, the output, the
depth), on the device; nothing is uploaded. A module of its own (docs/libzync/ZYNCAPPS.md §9), NOT part of the core
verifier's `hazync-verify.wasm` (until 15.11.2026 `verifyDeposit` was the core's). Raw ABI, no wasm-bindgen, as the
core's; glue: `zapp-deposit.js` (`loadDeposit`, `verifyDeposit`).

```sh
./build.sh                                                       # byte-reproducible: the core's flags (../../../scripts/wasm-build.sh), Rust 1.99.0, stage /tmp/zapp-deposit-wasm-build
node test-deposit.mjs ../target/release/zapp-deposit             # 24 cases on bundles `zapp-deposit build` makes from the real receipts
```

| | |
|---|---|
| size | 1,197,088 bytes raw, 342,787 gzipped |
| sha256 (this commit, `./build.sh`) | `5f1e1fb14509dd5f96ba243a3d5d513bc9b6ea1958e4d2609e2fba1340b59738` (headers image `de54b94f`; Rust 1.99.0 in the pinned container `reproduce/wasm.Dockerfile`, fixed stage path; laptop and hazync-coord (a fresh checkout of 4c21a877), 18.11.2026; with the review fixes of 18.11.2026 (unknown or null ask keys and unknown bundle fields refused, one opening per block, the outputs bound by libzync's parser); before the null-key fix wasm `0484b509…`, before them all wasm `5017298f…`, before 16.11.2026 wasm `b574ef4e…`) |
| reproduces | only with the same Rust and at the default stage path, as the core's (swiftsync-verify-wasm/README.md) |
| tests | `test-deposit.mjs` 24/24, run by `../test.sh` in CI's zapps job |
