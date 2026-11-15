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
| size | 1,236,767 bytes raw, 358,689 gzipped |
| sha256 (this commit, `./build.sh`) | `622ce4306960ec5dcf4b47b2e2fe38e58eb2ebc516016a818b8715413bc98849` (v0.26.0's verifier with the verify_facts frame fix of 15.11.2026, and WITHOUT the header-chain zyncAPP, now `../zapps/headers`; risc0 3.1.0; mainnet `46666b6f`, signet `01a80f17`, and `8888d59c` / `d4210dae` by id; Rust 1.99.0, 2026-11-15). With the zyncAPP still in it, the same source was `f16b5b98…`; 2026-11-14's `12bbc9ae…` had the frame bug; v0.24.0's release asset was `446fdaeb…` |
| reproduces | **only with the same host Rust**: 1.99.0 gave v0.24.0's `446fdaeb…`, 1.97.1 `a3a85b7f…`. The checkout path does not matter (the sources are staged), so use `RUSTUP_TOOLCHAIN=1.99.0`; the repo does not pin it yet. Also only at the default stage path: a `SWIFTSYNC_WASM_STAGE` override gave other bytes (`2ea330c0…`, +235 bytes) with no path in the strings |
| vectors | 11/11 (`test-vectors.out`; the 2 CLI-only cases of the manifest are skipped) |
| browser | the page loads the module and shows the built-in image id in headless Chrome |

**What a browser cannot do:** step 7 of the proposal's §5 (the tip is on your own most-work chain) needs your
node, so the page prints the proven tip and chainwork for you to compare, as the CLI does without `--bitcoin-cli`.
WebAssembly has no clock, so the page passes the device's time for `time-too-new`.
