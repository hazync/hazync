# `zapps/utreexo/wasm`: the Utreexo roots zAPP checker in a browser

The same checks as `zapp-utreexo verify` (one library, `zapp_utreexo::verify_utreexo`), compiled to WebAssembly with a
raw ABI and hand-written glue (`zapp-utreexo.js`), as `swiftsync-verify-wasm`. The receipt is checked on the device.

```sh
./build.sh          # byte-reproducible: Rust 1.99.0 (HAZYNC_WASM_TOOLCHAIN), sources staged at a fixed path
node test.mjs       # the real receipts of ../vectors through the WASM: 6 cases
```

| | |
|---|---|
| size | 1,099,714 bytes raw, 304,950 gzipped |
| sha256 (`./build.sh`, Rust 1.99.0, hazync-coord, 15.11.2026, two builds equal) | `484709071646ec2a898b14a9a64e4feb2c567dba90b9a850b87fba88c7eb5170` (built-in utreexo image `27bea7da…`, a prototype build, not published) |
| tests | 6/6 (`node test.mjs`) |

A browser has no node: the page must pass block H's hash (and the core tip's when T > H) from a source the user
trusts (`expectAt`, `expectTip`), else the verdict is at best `demo`.
