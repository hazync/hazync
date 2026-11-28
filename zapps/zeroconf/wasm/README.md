# `zapp-zeroconf-wasm`: the zero-confirmation zyncAPP in a browser

The zeroconf check of `zapp-zeroconf` (one library, `zapps/zeroconf`), compiled to WebAssembly: a page checks a proof
that a transaction is consensus-valid for the block after a proven tip, every coin it spends unspent there, on the
device; nothing is uploaded. A module of its own (docs/libzync/ZYNCAPPS.md §9), NOT part of the core verifier's
`hazync-verify.wasm`, which it does not change. Raw ABI, no wasm-bindgen, as the core's; glue: `zapp-zeroconf.js`
(`loadZeroconf`, `verifyZeroconf`; `zeroconfImageId` returns "" until an engine zeroconf image is published).

The same inputs as the CLI `zapp-zeroconf` apart from its node and explorer lookups: the proof, the txid, the wtxid of
the bytes you hold (optional), your CURRENT tip from your own header chain (`expectTip`, without it at most DEMO), an
anchor state (optional, then at most DEMO), mainnet or signet, the zeroconf image id, the core image id, the zeroconf
image's key file and its pin. Consensus-valid for the next block: not final, not relay policy (../README.md).

```sh
./build.sh                                   # byte-reproducible: the core's flags (../../../scripts/wasm-build.sh), Rust 1.99.0, stage /tmp/zapp-zeroconf-wasm-build
node test-zeroconf.mjs ../tests/data/engine  # 23 cases on the zAPP's real engine proof (as ../test.sh)
```

The test proof is a regtest proof (tip 110, real children), and a release module has no regtest image (the tests-only
`regtest-test` feature is refused in any WASM graph), so the test asks it as mainnet: the real STARK, the journal, the
statement and every policy check run in the module, and the only check that fails is the network (the core profile's
regtest bit); each refusal case must add its own failing check. The OK and DEMO verdicts on that proof are tested
natively (../test.sh: the library and the CLI).

| | |
|---|---|
| size | 334,454 bytes raw, 133,615 gzipped |
| sha256 (this commit, `./build.sh`) | `144a764dbbe369580d4dac03c4ae2b0358ad714c1bbab2de0d9bbe22e02f8b23` (engine/openvm, 28.11.2026: the first build; OpenVM v2.1.0 patched (engine/scripts/wasm_ovm.sh) as every module; built in the published image, the laptop. The second-machine check (hazync-coord, a fresh clone) is PENDING) |
| reproduces | only with the same Rust and at the default stage path, as the core's (swiftsync-verify-wasm/README.md) |
| tests | `test-zeroconf.mjs` 23/23 (the real proof verified in the module in about 60 ms on the laptop under Node), run by `../test.sh` |
