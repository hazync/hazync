# `zapp-deposit-wasm`: the deposit-proof zyncAPP in a browser

The deposit check of `zapp-deposit` (one library, `zapps/deposit`, on the header-chain zAPP `zapps/headers`),
compiled to WebAssembly: a page checks a deposit bundle against the caller's expectations (the txid, the output, the
depth), on the device; nothing is uploaded. A module of its own (docs/libzync/ZYNCAPPS.md §9), NOT part of the core
verifier's `hazync-verify.wasm` (until 15.11.2026 `verifyDeposit` was the core's). Raw ABI, no wasm-bindgen, as the
core's; glue: `zapp-deposit.js` (`loadDeposit`, `verifyDeposit`).

```sh
./build.sh                                                       # byte-reproducible: the core's flags (../../../scripts/wasm-build.sh), Rust 1.99.0, stage /tmp/zapp-deposit-wasm-build
node test-deposit.mjs ../target/release/zapp-deposit             # 10 cases on bundles `zapp-deposit build` makes from the real receipts
```

| | |
|---|---|
| size | 1,192,195 bytes raw, 340,387 gzipped |
| sha256 (this commit, `./build.sh`) | `91157776966e30781b731503be523e3d6c1e44a2862877d91d9b94eba67af48b` (headers image `de54b94f`; Rust 1.99.0, 2026-11-15) |
| reproduces | only with the same Rust and at the default stage path, as the core's (swiftsync-verify-wasm/README.md) |
| tests | `test-deposit.mjs` 10/10, run by `../test.sh` in CI's zapps job |
