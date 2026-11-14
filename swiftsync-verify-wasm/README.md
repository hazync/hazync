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
| size | 1,248,921 bytes raw, 362,580 gzipped |
| sha256 (this commit, `./build.sh`) | `1d0d88681222843d5c9d9dc236a7802477d640750c5b97acabe1cf771e51b509` (v0.26.0: the undo-data commitment and the 11-14 review's trie-fact check; risc0 3.1.0; mainnet `46666b6f`, signet `c3a68e2d`, headers `2db34e12`, and `8888d59c` / `d4210dae` by id; one build, Rust 1.99.0, 2026-11-14). v0.24.0's release asset was `446fdaeb…` |
| reproduces | **only with the same host Rust**: 1.99.0 gave v0.24.0's `446fdaeb…`, 1.97.1 `a3a85b7f…`. The checkout path does not matter (the sources are staged), so use `RUSTUP_TOOLCHAIN=1.99.0`; the repo does not pin it yet. Also only at the default stage path: a `SWIFTSYNC_WASM_STAGE` override gave other bytes (`2ea330c0…`, +235 bytes) with no path in the strings |
| vectors | 11/11 (`test-vectors.out`; the 2 CLI-only cases of the manifest are skipped) |
| browser | the page loads the module and shows the built-in image id in headless Chrome |

**What a browser cannot do:** step 7 of the proposal's §5 (the tip is on your own most-work chain) needs your
node, so the page prints the proven tip and chainwork for you to compare, as the CLI does without `--bitcoin-cli`.
WebAssembly has no clock, so the page passes the device's time for `time-too-new`.
