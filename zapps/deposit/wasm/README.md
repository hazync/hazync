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
| size | 1,193,703 bytes raw, 340,904 gzipped |
| sha256 (this commit, `./build.sh`) | `b574ef4e442c8c03e5d300d4096ef0d563263505c11493a022906409ba22201b` (headers image `de54b94f`; Rust 1.99.0, default stage path, 16.11.2026, on a box whose build reproduces the core's `622ce430…`; `min_validated_depth` defaults to 1, the deposit's own block fully validated; before it `b2fd059f…`, the module no longer linking zapps/common) |
| reproduces | only with the same Rust and at the default stage path, as the core's (swiftsync-verify-wasm/README.md) |
| tests | `test-deposit.mjs` 14/14, run by `../test.sh` in CI's zapps job |
