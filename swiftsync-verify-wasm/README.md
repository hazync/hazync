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
| size | 1,237,433 bytes raw, 358,169 gzipped |
| sha256 (this commit, `./build.sh`) | `bce00ae64aa1a950b3320244fd22c7bd308e71da66e809c55cdad20f4e4ea633` (with `verifyHeaders`, headers image `4d04d6bf`) |
| reproduces | only at the default stage path: a `SWIFTSYNC_WASM_STAGE` override gave other bytes (`2ea330c0…`, +235 bytes) with no path in the strings |
| vectors | 14/14 (`test-vectors.out`), all 14 in 0.5 s under Node 24 |
| browser | the page loads the module and shows the built-in image id in headless Chrome |

**What a browser cannot do:** step 7 of the proposal's §5 (the tip is on your own most-work chain) needs your
node, so the page prints the proven tip and chainwork for you to compare, as the CLI does without `--bitcoin-cli`.
WebAssembly has no clock, so the page passes the device's time for `time-too-new`.
