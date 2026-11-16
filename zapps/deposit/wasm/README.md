# `zapp-deposit-wasm`: the deposit-proof zyncAPP in a browser

The deposit check of `zapp-deposit` (one library, `zapps/deposit`, on the header-chain zAPP `zapps/headers`),
compiled to WebAssembly: a page checks a deposit bundle against the caller's expectations (the txid, the output, the
depth), on the device; nothing is uploaded. A module of its own (docs/libzync/ZYNCAPPS.md §9), NOT part of the core
verifier's `hazync-verify.wasm` (until 15.11.2026 `verifyDeposit` was the core's). Raw ABI, no wasm-bindgen, as the
core's; glue: `zapp-deposit.js` (`loadDeposit`, `verifyDeposit`).

```sh
./build.sh                                                       # byte-reproducible: the core's flags (../../../scripts/wasm-build.sh), Rust 1.99.0, stage /tmp/zapp-deposit-wasm-build
node test-deposit.mjs ../target/release/zapp-deposit             # 14 cases on bundles `zapp-deposit build` makes from the real receipts
```

| | |
|---|---|
| size | 1,193,024 bytes raw, 340,580 gzipped |
| sha256 (this commit, `./build.sh`) | `0085c412a147ab91236fc19601335dce4edf3f7f72d13c8450514fe0fddbc444` (headers image `de54b94f`; Rust 1.99.0, default stage path, 16.11.2026, on a box whose builds reproduce the headers module's `8eb3cfe9…`; the module no longer links zapps/common; before it `82437c4c…`) |
| reproduces | only with the same Rust and at the default stage path, as the core's (swiftsync-verify-wasm/README.md) |
| tests | `test-deposit.mjs` 14/14, run by `../test.sh` in CI's zapps job |
