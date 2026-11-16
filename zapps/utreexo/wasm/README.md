# `zapps/utreexo/wasm`: the Utreexo roots zAPP checker in a browser

The same checks as `zapp-utreexo verify` (one library, `zapp_utreexo::verify_utreexo`), compiled to WebAssembly with a
raw ABI and hand-written glue (`zapp-utreexo.js`), as `swiftsync-verify-wasm`. The receipt is checked on the device.

```sh
./build.sh          # byte-reproducible: scripts/wasm-build.sh, in the pinned container reproduce/wasm.Dockerfile
node test.mjs       # the real receipts of ../vectors through the WASM: 7 cases
```

| | |
|---|---|
| size | 1,100,680 bytes raw, 305,171 gzipped |
| sha256 (this commit, `./build.sh`) | `a0279e993347e74be14ff932127cc96b79934609d68185d8d8fc76739db82fc7` (Rust 1.99.0 in the pinned container `reproduce/wasm.Dockerfile`, fixed stage path; laptop and hazync-coord, 16.11.2026: the same bytes as the native builds of 15.11.2026 on hazync-coord; built-in utreexo image `27bea7da…`, a local build, not reproducible, not published) |
| tests | 7/7 (`node test.mjs`); `../test.sh` checks the built file's sha256 against this table (scripts/zapp-checks.sh) |


The JSON's `roots` write an Empty root as "empty" (in the journal: 32 zero bytes); load it as `BitcoinNodeHash::Empty`,
never as the hash 00…00.

A browser has no node: the page must pass block H's hash (and the core tip's when T > H) from a source the user
trusts (`expectAt`, `expectTip`), else the verdict is at best `demo`.
