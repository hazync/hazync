# `zapps/utreexo/wasm`: the Utreexo roots zAPP checker in a browser

The same checks as `zapp-utreexo verify` (one library, `zapp_utreexo::verify_utreexo`), compiled to WebAssembly with a
raw ABI and hand-written glue (`zapp-utreexo.js`), as `swiftsync-verify-wasm`. The receipt is checked on the device.

```sh
./build.sh          # byte-reproducible: scripts/wasm-build.sh, in the pinned container reproduce/wasm.Dockerfile
node test.mjs       # the real receipts of ../vectors through the WASM: 23 cases (9 on a tip receipt, 1 a nested composite)
```

| | |
|---|---|
| size | 1,118,429 bytes raw, 311,646 gzipped |
| sha256 (this commit, `./build.sh`) | `43d9fe4867f59eaa026a41f19bc83c120b537f55ece7e719834aae6deed60f35` (review of f99c301 L6: `decode_receipt` itself refuses a composite before bincode; SECOND_MACHINE; before it `fb282bc3…`: review H1: a composite receipt is refused before its journal is read, so a deeply nested one no longer traps; the same bytes from a laptop's local build of `reproduce/wasm.Dockerfile` and from the published image `ghcr.io/hazync/hazync-wasm-build@sha256:d46e4164…` on hazync-coord, a clean clone of `ea14217`; 23/23 in node on both; before it `71dd28bb…`: 18.11.2026: both utreexo images pinned (reproduce/LIBZYNC_IDS), so the built-in ids report `builtin-pinned`; the same bytes from a laptop's local build of `reproduce/wasm.Dockerfile` and from the published image `ghcr.io/hazync/hazync-wasm-build@sha256:d46e4164…` on hazync-coord, a clean clone of `15d99d2`; 22/22 in node on both; before it `42df910a…` (the same two-machine check, `ccedefe`): tip receipts too, UTREEXO_ZAPP.md §10: the crate feature `tip`, an optional base-image field at the frame's end, `tip_image_id()`; before it `d240082b…`, Rust 1.99.0 in the pinned container `reproduce/wasm.Dockerfile`, fixed stage path; hazync-coord, 18.11.2026, the same bytes from a local build of the Dockerfile and from the published image `ghcr.io/hazync/hazync-wasm-build@sha256:d46e4164…` (main's reproduce/wasm-image.digests); since the caller names the network (the frame's last field), trailing receipt bytes are refused and the unpinned built-in image caps the verdict at `demo`; the review fixes (`image_source`, a FAIL carries no statement) on top of `106feaf9…`; before it `515e020a…` (16.11.2026), since libzync's `utreexo_roots` sits behind the feature `utreexo` (so the core .wasm no longer carries it); before it `a0279e99…`; built-in utreexo image `27bea7da…`, a local build, not reproducible, not published) |
| tests | 23/23 (`node test.mjs`); `../test.sh` checks the built file's sha256 against this table (scripts/zapp-checks.sh) |


The JSON's `roots` write an Empty root as "empty" (in the journal: 32 zero bytes); load it as `BitcoinNodeHash::Empty`,
never as the hash 00…00.

The page names the network (`network`, `'mainnet'` by default, or `'signet'`): a proof of another network is `fail`;
the JSON's `network` is the one checked. `image_source` says where the utreexo image id comes from:
`"builtin-unpinned"` (with `image_label`) is the built-in one, not yet a pinned reproducible build, under which the
verdict is at most `demo`; `"caller"` is an id the page named (`imageHex`), whose code this checker does not know.

A browser has no node: the page must pass block H's hash (and the core tip's when T > H) from a source the user
trusts (`expectAt`, `expectTip`), else the verdict is at best `demo`.
