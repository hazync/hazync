# `swiftsync-verify-wasm`: the libzync chain-proof verifier in a browser

The same checks as `swiftsync-verify` (one library), compiled to WebAssembly, with a one-page front end. Released as
`hazync-verify.wasm` with this directory's `swiftsync-verify.js` as `hazync-verify.js` (serve the two together). It
checks core chain proofs only: the header-chain zyncAPP's `verifyHeaders` is its own module since 15.11.2026
(`../zapps/headers/wasm`, `zapp-headers.js`; docs/libzync/ZYNCAPPS.md §9). The proof and its facts are verified on the device; nothing
is uploaded. No wasm-bindgen: a raw ABI and ~60 lines of
hand-written glue (`swiftsync-verify.js`), so the `.wasm` is a plain cargo output anyone can rebuild and compare.

```sh
./build.sh                                   # byte-reproducible: installs and uses Rust 1.99.0 itself (HAZYNC_WASM_TOOLCHAIN), stages the sources at a fixed path
node test-vectors.mjs                        # the vectors of ../swiftsync-verify/vectors through the WASM
cp target/wasm32-unknown-unknown/release/swiftsync_verify_wasm.wasm . && python3 -m http.server   # then open index.html
```

| | |
|---|---|
| size | 1,238,180 bytes raw, 359,393 gzipped |
| sha256 (this commit, `./build.sh`) | `bc2c98593e850705d9a52eaa26a6eb55e96065191be6c80b9a4906305cdad69e` (v0.26.0's verifier with the verify_facts frame fix of 15.11.2026, main's Frame::take checked_add (6d459c15, 16.11.2026) and unknown fields refused in every fact type and unknown keys in the facts JSON (18.11.2026), WITHOUT the header-chain zyncAPP, now `../zapps/headers`; risc0 3.1.0; mainnet `46666b6f`, signet `01a80f17`, and `8888d59c` / `d4210dae` by id; Rust 1.99.0 in the pinned container `reproduce/wasm.Dockerfile`, laptop and hazync-coord). Before the unknown-field refusals the wasm pin was `aa33d1a9…`; before the checked_add `622ce430…`; with the zyncAPP still in it, the source before the checked_add was `f16b5b98…`; 2026-11-14's `12bbc9ae…` had the frame bug; v0.24.0's release asset was `446fdaeb…` |
| reproduces | on any x86_64 machine with docker, cargo >= 1.78 (it reads the v4 Cargo.lock), python3 and the network (no wasm32 Rust needed): `./build.sh` builds in the pinned container `reproduce/wasm.Dockerfile` (Rust 1.99.0, base image by digest, an Ubuntu archive snapshot) at the fixed stage path `/tmp/swiftsync-wasm-build`, so neither the host's toolchain nor its paths reach the bytes (18.11.2026: this commit's `bc2c9859…` the same on a laptop and on hazync-coord from a fresh clone; 16.11.2026: the core without the header-chain zyncAPP, the same `aa33d1a9…` on both). Before that the stage path DID reach the bytes, through cargo's `-C metadata` hash of the path dependencies (`swiftsync-verify`, `libzync`: outside the building workspace, so hashed by absolute path): another stage gave `7d79f68c…` (a laptop) and `05754e5f…` (hazync-web #57), so `SWIFTSYNC_WASM_STAGE` is refused now. The Rust version still matters: 1.99.0 gave v0.24.0's `446fdaeb…`, 1.97.1 `a3a85b7f…` |
| vectors | 11/11 (`test-vectors.out`; the 2 CLI-only cases of the manifest are skipped) |
| browser | the page loads the module and shows the built-in image id in headless Chrome |

**What a browser cannot do:** step 7 of the proposal's §5 (the tip is on your own most-work chain) needs your
node, so the page prints the proven tip and chainwork for you to compare, as the CLI does without `--bitcoin-cli`.
WebAssembly has no clock, so the page passes the device's time for `time-too-new`.
