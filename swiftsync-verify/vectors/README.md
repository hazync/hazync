# Test vectors for the libzync chain-proof verifier

⚠ **Refusals only, until the first GPU proofs of the frozen image.** Every receipt here is from the retired round-4
design-B signet image (`c3501ac9…`): its journal predates ChainState version 2 and its image kept no coin log, so every
case must FAIL. E2 (11-05) removed the SwiftSync hints file from every proof; the hints-file cases went with it. OK and
DEMO cases come back with receipts of the product image (`experimental/libzync-bench/gpu/COINLOG_GPU_PLAN.md`).

`manifest.json` lists 8 cases: verifier arguments (in `swiftsync-verify` syntax), the exit code a correct verifier must
give (0 OK, 2 DEMO, 1 FAIL), and what each one breaks. `cli_only` cases test the CLI's own argument handling (a
hints-file or `-` argument is refused, not ignored); the C and WASM harnesses skip them. It also lists reference
values a second implementation should reproduce: genesis `anchor_id`s per network, the empty coinbase-SMT root, and
the Utreexo roots at mainnet height 200.

```sh
python3 run_vectors.py ../target/release/swiftsync-verify     # or any verifier taking the same arguments
```

`run_vectors.out` is the result for `swiftsync-verify` at this commit: 8/8.

`fake_block_2501.rcpt` is a dev-mode FakeReceipt. `bad_bundle_header.sspf` and `r4_2600.sspf` are version-1 bundles
(they carried a hints-file hash), refused by today's version-2 reader. `signet2600.hints` is kept only as the argument
the CLI must refuse.
