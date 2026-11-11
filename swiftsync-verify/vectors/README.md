# Test vectors for the libzync chain-proof verifier

`mainnet_1_200_8888d59c.rcpt` is a real GPU proof of mainnet blocks 1..200 under the frozen product image
`8888d59c` (risc0 3.1.0, proved 2026-11-11 on one RTX PRO 6000): a correct verifier says DEMO (valid, below Core's
minimum chainwork) and refuses it as signet, against `863c9ce3` (the same guest on risc0 3.0.5), and with one bit
flipped in its seal (`mainnet_1_200_8888d59c_flipped.rcpt`). The `r4_*` receipts are from the retired round-4
design-B signet image (`c3501ac9…`): its journal predates ChainState version 2 and its image kept no coin log, so
every one must FAIL. E2 (11-05) removed the SwiftSync hints file from every proof; the hints-file cases went with it.

`manifest.json` lists 12 cases: verifier arguments (in `swiftsync-verify` syntax), the exit code a correct verifier must
give (0 OK, 2 DEMO, 1 FAIL), and what each one breaks. `cli_only` cases test the CLI's own argument handling (a
hints-file or `-` argument is refused, not ignored); the C and WASM harnesses skip them. It also lists reference
values a second implementation should reproduce: genesis `anchor_id`s per network, the empty coinbase-SMT root, and
the Utreexo roots at mainnet height 200.

```sh
python3 run_vectors.py ../target/release/swiftsync-verify     # or any verifier taking the same arguments
```

`run_vectors.out` is the result for `swiftsync-verify` at this commit: 12/12 (the WASM harness, which skips the 2 `cli_only` cases: 10/10, `../../swiftsync-verify-wasm/test-vectors.out`).

`fake_block_2501.rcpt` is a dev-mode FakeReceipt. `bad_bundle_header.sspf` and `r4_2600.sspf` are version-1 bundles
(they carried a hints-file hash), refused by today's version-2 reader. `signet2600.hints` is kept only as the argument
the CLI must refuse.
