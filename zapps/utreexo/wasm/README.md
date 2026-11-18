# `zapps/utreexo/wasm`: the Utreexo roots zAPP checker in a browser

The same checks as `zapp-utreexo verify` (one library, `zapp_utreexo::verify_utreexo`), compiled to WebAssembly with a
raw ABI and hand-written glue (`zapp-utreexo.js`), as `swiftsync-verify-wasm`. The receipt is checked on the device.

```sh
./build.sh          # byte-reproducible: scripts/wasm-build.sh, in the pinned container reproduce/wasm.Dockerfile
node test.mjs       # the real receipts of ../vectors through the WASM: 12 cases
```

| | |
|---|---|
| size | 1,101,732 bytes raw, 305,318 gzipped |
| sha256 (this commit, `./build.sh`) | `106feaf96e555bae8d7137c22f6f0c0345c561c7ff6265794f164bda2865712b` (Rust 1.99.0 in the pinned container `reproduce/wasm.Dockerfile`, fixed stage path; hazync-coord, 18.11.2026, the same bytes from a local build of the Dockerfile and from the published image `ghcr.io/hazync/hazync-wasm-build@sha256:d46e4164…` (main's reproduce/wasm-image.digests); since the caller names the network (the frame's last field), trailing receipt bytes are refused and the unpinned built-in image caps the verdict at `demo`; before it `515e020a…` (16.11.2026), since libzync's `utreexo_roots` sits behind the feature `utreexo` (so the core .wasm no longer carries it); before it `a0279e99…`; built-in utreexo image `27bea7da…`, a local build, not reproducible, not published) |
| tests | 12/12 (`node test.mjs`); `../test.sh` checks the built file's sha256 against this table (scripts/zapp-checks.sh) |


The JSON's `roots` write an Empty root as "empty" (in the journal: 32 zero bytes); load it as `BitcoinNodeHash::Empty`,
never as the hash 00…00.

The page names the network (`network`, `'mainnet'` by default, or `'signet'`): a proof of another network is `fail`;
the JSON's `network` is the one checked. `image_pinned: false` (with `image_label`) marks the built-in utreexo image,
not yet a pinned reproducible build: under it the verdict is at most `demo`.

A browser has no node: the page must pass block H's hash (and the core tip's when T > H) from a source the user
trusts (`expectAt`, `expectTip`), else the verdict is at best `demo`.
