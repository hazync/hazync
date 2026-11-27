# Test vectors for the libzync chain-proof verifier

Engine (OpenVM) proofs only since the cutover (27.11.2026, engine/CUTOVER.md). The RISC Zero receipts that were here
(the 8888d59c proof of mainnet 1..200 and its malformed copies, the round-4 design-B receipts, a FakeReceipt) went with
the RISC Zero front; v0.27.1's verifier and its vectors check them.

`mainnet_1_200_engine.hze` is the engine chain's real proof of mainnet blocks 1..200 (`../tests/data/engine200`), with
its image id (`.image_id`) and verifying key (`.vk`). No engine image is published yet (the Stage 4 freeze), so every
case gives both, and the key is pinned as given. A correct verifier says DEMO: valid, below Core's minimum chainwork.
`make_vectors.py <verifier>` writes every other file here from it: one-change copies (a proof bit, a journal bit, a byte
appended, the file cut, a proof length past the end, a key bit), a version-2 bundle written by the verifier's own
`bundle` command, that bundle with its stop height changed, and the bundle marked version 1. `signet2600.hints` is kept
only as the argument the CLI must refuse.

`manifest.json` lists 16 cases: verifier arguments (in `swiftsync-verify` syntax), the exit code a correct verifier must
give (0 OK, 2 DEMO, 1 FAIL), and what each one breaks. `cli_only` cases test the CLI's own argument handling (a
hints-file or `-` argument is refused, not ignored); the C and WASM harnesses skip them. It also lists reference values
a second implementation should reproduce: genesis `anchor_id`s per network, the empty coinbase-SMT root, and the coin
forest at mainnet height 200.

```sh
python3 run_vectors.py ../target/release/swiftsync-verify     # or any verifier taking the same arguments
```

`run_vectors.out` is the result for `swiftsync-verify` at this commit: 16/16.
