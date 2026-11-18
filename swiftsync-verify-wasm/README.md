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
| size | 1,237,972 bytes raw, 359,338 gzipped |
| sha256 (this commit, `./build.sh`) | `3f699baec899b705fe07637e75b950c32e8db74eff08b18a6e4aca5e4d9781aa` (main's core with fix/receipt-hardening: the strict receipt decoder and journal guard of fuzz round 11-17 and their review fixes, the header-chain reader's share of them now in `../zapps/headers`; built in the published image from reproduce/wasm-image.digests, laptop, 18.11.2026; a second machine to confirm). Before it `aa33d1a945e382962066a63de4b5701e07143d9a8a51b620b69ff75fa6083617` (v0.26.0's verifier with the verify_facts frame fix of 15.11.2026 and main's Frame::take checked_add (6d459c15, 16.11.2026), WITHOUT the header-chain zyncAPP, now `../zapps/headers`; risc0 3.1.0; mainnet `46666b6f`, signet `01a80f17`, and `8888d59c` / `d4210dae` by id; Rust 1.99.0 in the pinned container `reproduce/wasm.Dockerfile`, laptop and hazync-coord). Before the checked_add `622ce430…`; with the zyncAPP still in it, the source before the checked_add was `f16b5b98…`; 2026-11-14's `12bbc9ae…` had the frame bug; v0.24.0's release asset was `446fdaeb…` |
| reproduces | on any x86_64 machine with docker, cargo >= 1.78 (it reads the v4 Cargo.lock), python3 and the network (no wasm32 Rust needed): `./build.sh` builds in the pinned container `reproduce/wasm.Dockerfile` (Rust 1.99.0, base image by digest, an Ubuntu archive snapshot) at the fixed stage path `/tmp/swiftsync-wasm-build`, so neither the host's toolchain nor its paths reach the bytes (16.11.2026: this branch's core, without the header-chain zyncAPP, the same `aa33d1a9…` on a laptop and on hazync-coord). Before that the stage path DID reach the bytes, through cargo's `-C metadata` hash of the path dependencies (`swiftsync-verify`, `libzync`: outside the building workspace, so hashed by absolute path): another stage gave `7d79f68c…` (a laptop) and `05754e5f…` (hazync-web #57), so `SWIFTSYNC_WASM_STAGE` is refused now. The Rust version still matters: 1.99.0 gave v0.24.0's `446fdaeb…`, 1.97.1 `a3a85b7f…` |
| vectors | 14/14 (`test-vectors.out`; the 2 CLI-only cases of the manifest are skipped) |
| browser | the page loads the module and shows the built-in image id in headless Chrome |

**What a browser cannot do:** step 7 of the proposal's §5 (the tip is on your own most-work chain) needs your
node, so the page prints the proven tip and chainwork for you to compare, as the CLI does without `--bitcoin-cli`.
WebAssembly has no clock, so the page passes the device's time for `time-too-new`.
