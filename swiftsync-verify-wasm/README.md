# `swiftsync-verify-wasm`: the libzync chain-proof verifier in a browser

The same checks as `swiftsync-verify` (one library), compiled to WebAssembly, with a one-page front end. The proof
and its facts are verified on the device; nothing is uploaded. No wasm-bindgen: a raw ABI and ~60 lines of
hand-written glue (`swiftsync-verify.js`), so the `.wasm` is a plain cargo output anyone can rebuild and compare.

```sh
./build.sh                                   # byte-reproducible: stages the sources at a fixed path
node test-vectors.mjs                        # the vectors of ../swiftsync-verify/vectors through the WASM
cp target/wasm32-unknown-unknown/release/swiftsync_verify_wasm.wasm . && python3 -m http.server   # then open index.html
```

| | |
|---|---|
| size | 1,225,120 bytes raw, 354,277 gzipped |
| sha256 (`./build.sh`, 2026-11-10, main at #695) | `235ab8278a6f227b7a0f9a2aca84af449e07c8a51ed3b4aa83a89c00035e0aa3`; the #695 branch on 11-09 and main on 11-10 gave the same bytes |
| vectors | 14/14 (`test-vectors.out`), all 14 in 0.5 s under Node 24 |
| browser | the page loads the module and shows the built-in image id in headless Chrome |

**What a browser cannot do:** step 7 of the proposal's §5 (the tip is on your own most-work chain) needs your
node, so the page prints the proven tip and chainwork for you to compare, as the CLI does without `--bitcoin-cli`.
WebAssembly has no clock, so the page passes the device's time for `time-too-new`.
