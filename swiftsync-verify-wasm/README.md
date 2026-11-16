# `swiftsync-verify-wasm`: the libzync chain-proof verifier in a browser

The same checks as `swiftsync-verify` (one library), compiled to WebAssembly, with a one-page front end. Released as
`hazync-verify.wasm` with this directory's `swiftsync-verify.js` as `hazync-verify.js` (serve the two together); it
also checks the header-chain zyncAPP (`verifyHeaders`). The proof and its facts are verified on the device; nothing
is uploaded. No wasm-bindgen: a raw ABI and ~60 lines of
hand-written glue (`swiftsync-verify.js`), so the `.wasm` is a plain cargo output anyone can rebuild and compare.

```sh
./build.sh                                   # byte-reproducible: installs and uses Rust 1.99.0 itself (HAZYNC_WASM_TOOLCHAIN), stages the sources at a fixed path
node test-vectors.mjs                        # the vectors of ../swiftsync-verify/vectors through the WASM
cp target/wasm32-unknown-unknown/release/swiftsync_verify_wasm.wasm . && python3 -m http.server   # then open index.html
```

| | |
|---|---|
| size | 1,249,260 bytes raw, 362,951 gzipped |
| sha256 (this commit, `./build.sh`) | `996f1ed661c2b68d4e5bcc6b56f53a5fbfdf910ef59e7ba57bd9a436f19081a9` (main after v0.26.0, with #735's Frame::take checked_add (6d459c15); risc0 3.1.0; mainnet `46666b6f`, signet `01a80f17`, headers `2db34e12`, and `8888d59c` / `d4210dae` by id; Rust 1.99.0 in the pinned container, laptop and hazync-coord equal, 16.11.2026). v0.26.0's released asset is `f16b5b98…`, built from 797b777e before #735's Frame::take fix; 2026-11-14's `12bbc9ae…` had the frame bug; v0.24.0's release asset was `446fdaeb…` |
| reproduces | on any machine with docker: `./build.sh` builds in the pinned container `reproduce/wasm.Dockerfile` (Rust 1.99.0, base image by digest, an Ubuntu archive snapshot) at the fixed stage path `/tmp/swiftsync-wasm-build`, so neither the host's toolchain nor its paths reach the bytes (16.11.2026: the same `996f1ed6…` on a laptop and on hazync-coord from this commit; before it, v0.26.0's source gave the same `f16b5b98…` on both, from two checkouts at different paths). Before that the stage path DID reach the bytes, through cargo's `-C metadata` hash of the path dependencies (`swiftsync-verify`, `libzync`: outside the building workspace, so hashed by absolute path): another stage gave `7d79f68c…` (a laptop) and `05754e5f…` (hazync-web #57), so `SWIFTSYNC_WASM_STAGE` is refused now. The Rust version still matters: 1.99.0 gave v0.24.0's `446fdaeb…`, 1.97.1 `a3a85b7f…` |
| vectors | 11/11 (`test-vectors.out`; the 2 CLI-only cases of the manifest are skipped) |
| browser | the page loads the module and shows the built-in image id in headless Chrome |

**What a browser cannot do:** step 7 of the proposal's §5 (the tip is on your own most-work chain) needs your
node, so the page prints the proven tip and chainwork for you to compare, as the CLI does without `--bitcoin-cli`.
WebAssembly has no clock, so the page passes the device's time for `time-too-new`.
