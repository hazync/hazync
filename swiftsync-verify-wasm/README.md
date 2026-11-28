# `swiftsync-verify-wasm`: the libzync chain-proof verifier in a browser

The same checks as `swiftsync-verify` (one library), compiled to WebAssembly, with a one-page front end. Released as
`hazync-verify.wasm` with this directory's `swiftsync-verify.js` as `hazync-verify.js` (serve the two together). It
checks core chain proofs only: the header-chain zyncAPP's `verifyHeaders` is its own module since 15.11.2026
(`../zapps/headers/wasm`, `zapp-headers.js`; docs/libzync/ZYNCAPPS.md §9). It checks engine (OpenVM) proofs. No engine image is published yet (the Stage 4 freeze), so a caller gives the image id, its verifying key file
and the key's pin (`verify({ ..., imageHex, vk, pin })`); the RISC Zero chain's proofs are checked with v0.27.1's
verifier. The proof and its facts are verified on the device; nothing is uploaded. No wasm-bindgen: a raw ABI and ~60 lines of
hand-written glue (`swiftsync-verify.js`), so the `.wasm` is a plain cargo output anyone can rebuild and compare.

```sh
./build.sh                                   # byte-reproducible: installs and uses Rust 1.99.0 itself (HAZYNC_WASM_TOOLCHAIN), stages the sources at a fixed path
node test-vectors.mjs                        # the vectors of ../swiftsync-verify/vectors through the WASM
node test-engine-frame.mjs target/wasm32-unknown-unknown/release/swiftsync_verify_wasm.wasm ../swiftsync-verify/tests/data/engine200
node test-facts-frame.mjs                    # verify_facts' frame on the engine chain's proof of 1..200
cp target/wasm32-unknown-unknown/release/swiftsync_verify_wasm.wasm . && python3 -m http.server   # then open index.html
```

| | |
|---|---|
| size | 423,181 bytes raw, 167,223 gzipped (engine only; before the cutover 1,241,166 bytes raw, 360,495 gzipped) |
| sha256 (this commit, `./build.sh`) | `4cbc884d39898121146b0a50564a4b354d0323e94e84dcc65366ae4b819e445c`: engine proofs only (cutover C5), OpenVM v2.1.0 patched (engine/scripts/wasm_ovm.sh, tree digest 38eccd09); built in the published image with HAZYNC_WASM_REQUIRE_CLEAN=1 HAZYNC_WASM_REQUIRE_PUBLISHED=1 on the laptop at e053e1f8 (27.11.2026) and hazync-coord, a fresh clone of a297fe2e (28.11.2026), equal: round 2's verifier changes (engine/reviews/round2/fixes.md). Before it `1f6ebed9b95eb1754398e6e767da7bd8f5ce857fcee709f45656a41d99064bee` (the same build at the cutover: hazync-coord, a fresh clone of ba9ec0b8, and the laptop twice, equal, 27.11.2026). Earlier hashes: this file's git history |
| released asset | every release up to v0.27.1 shipped the RISC Zero verifier (v0.27.0's `hazync-verify.wasm`: `0c8cf026…`); this module ships with the next release. Each release's hash is in its signed `SHA256SUMS.txt`, and `./build.sh` on a checkout of its tag reproduces it |
| reproduces | on any x86_64 machine with docker, cargo >= 1.78 (it reads the v4 Cargo.lock), python3 and the network (no wasm32 Rust needed): `./build.sh` builds in the pinned container `reproduce/wasm.Dockerfile` (Rust 1.99.0, base image by digest, an Ubuntu archive snapshot) at the fixed stage path `/tmp/swiftsync-wasm-build`, so neither the host's toolchain nor its paths reach the bytes. The Rust version does: a new toolchain pin means a new hash |
| vectors | 14/14 (`test-vectors.out`; the 2 CLI-only cases of the manifest are skipped) |
| engine frame / facts frame | `test-engine-frame.mjs`: block 200 DEMO and its four refusals; `test-facts-frame.mjs`: 7/7 (27.11.2026) |
| browser | checked 27.11.2026 in headless Chromium: block 200's engine proof with its image id, key and pin gives DEMO, a wrong pin FAIL; 390 px wide at phone width; no console errors (engine/CUTOVER.md) |

**What a browser cannot do:** step 7 of the proposal's §5 (the tip is on your own most-work chain) needs your
node, so the page prints the proven tip and chainwork for you to compare, as the CLI does without `--bitcoin-cli`.
WebAssembly has no clock, so the page passes the device's time for `time-too-new`.
