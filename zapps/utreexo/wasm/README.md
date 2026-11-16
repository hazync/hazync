# `zapps/utreexo/wasm`: the Utreexo roots zAPP checker in a browser

The same checks as `zapp-utreexo verify` (one library, `zapp_utreexo::verify_utreexo`), compiled to WebAssembly with a
raw ABI and hand-written glue (`zapp-utreexo.js`), as `swiftsync-verify-wasm`. The receipt is checked on the device.

```sh
./build.sh          # byte-reproducible: scripts/wasm-build.sh, in the pinned container reproduce/wasm.Dockerfile
node test.mjs       # the real receipts of ../vectors through the WASM: 7 cases
```

| | |
|---|---|
| size | 1,101,030 bytes raw, 305,051 gzipped |
| sha256 (this commit, `./build.sh`) | `515e020a9bb6d01df9d7c5f03b50f28a8e0e5cf3240cccaa3576d16115f6596a` (Rust 1.99.0 in the pinned container `reproduce/wasm.Dockerfile`, fixed stage path; laptop and hazync-coord, 16.11.2026; since libzync's `utreexo_roots` sits behind the feature `utreexo` (so the core .wasm no longer carries it); before it `a0279e99…`; built-in utreexo image `27bea7da…`, a local build, not reproducible, not published) |
| tests | 7/7 (`node test.mjs`); `../test.sh` checks the built file's sha256 against this table (scripts/zapp-checks.sh) |


The JSON's `roots` write an Empty root as "empty" (in the journal: 32 zero bytes); load it as `BitcoinNodeHash::Empty`,
never as the hash 00…00.

A browser has no node: the page must pass block H's hash (and the core tip's when T > H) from a source the user
trusts (`expectAt`, `expectTip`), else the verdict is at best `demo`.
