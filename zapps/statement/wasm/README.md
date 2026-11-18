# `zapp-statement.wasm`: the Balance statement zyncAPP's checker in a browser

The statement check of `zapps/statement` (docs/libzync/STATEMENT_ZAPP.md) compiled to WebAssembly, on top of the core
verifier library, which verifies the receipt. Its own module: only the statement page loads it, with
`zapp-statement.js`; the core `hazync-verify.wasm` carries no statement code. Raw ABI, no wasm-bindgen.

```sh
./build.sh                       # byte-reproducible: the core's flags (../../../scripts/wasm-build.sh), Rust 1.99.0, stage /tmp/zapp-statement-wasm-build
node test-statement.mjs target/wasm32-unknown-unknown/release/zapp_statement_wasm.wasm <statement.json> <purpose> <nonce>
                                 # or ../test.sh (CI's zapps job), which builds it and makes the statement
```

| | |
|---|---|
| size | 2,497,541 bytes raw, 1,528,343 gzipped |
| sha256 (this commit, `./build.sh`) | `ac9d93fcdef6f01c670e8ca534887ed60785ab6f2eaae5665f08f7e6b3f4784a` (review 11-18, second pass: the result names the image that verified the receipt, errors that echo a signature escaped, unknown JSON fields refused; one checkout on the laptop, 18.11.2026, in reproduce/wasm.Dockerfile built locally: this branch's reproduce/wasm-image.digests has no line for its tag). Before it `5b5e00cbbe71d73732264a71b2e114571bac000ffe0374b62fe228f6ac31663e` (canonical hex in a statement's facts, fuzz round 11-17, and strict signature decoding, review 11-18; two checkouts on hazync-coord agree, 18.11.2026), before that `b14ce3a0c1620a7e86cd4a7f534e273d4453e57de25d0170abbfb40088badaa8` (Rust 1.99.0 and clang 18.1.3 in the pinned container `reproduce/wasm.Dockerfile` (this module compiles C: secp256k1-sys), fixed stage path; laptop and hazync-coord, 16.11.2026; with the review fixes of 16.11.2026 (the image resolved by zapps/corecheck, an --image pinned exactly); before them `5a284b26…`, and with a host clang 14 `510c205b…`) |
| reproduces | through `./build.sh` on any machine with docker (it builds in `reproduce/wasm.Dockerfile`: Rust 1.99.0, clang 18.1.3 from a pinned Ubuntu snapshot), at the default stage path |
| tests | `test-statement.mjs` 18/18 against the real mainnet 1..200 proof (`8888d59c`), run by `../test.sh` in CI's zapps job |
