# Test vectors for a SwiftSync-hints proof verifier

`manifest.json` lists 14 cases: verifier arguments (in `swiftsync-verify` syntax) and the exit code a correct
verifier must give (0 OK, 2 DEMO, 1 FAIL), with a description of what each one breaks. It also lists reference
values a second implementation should reproduce: `ss_hints` of three hints files, genesis `anchor_id`s per image,
the empty coinbase-SMT root, and the Utreexo roots at mainnet height 200.

```sh
python3 run_vectors.py ../target/release/swiftsync-verify     # or any verifier taking the same arguments
```

`run_vectors.out` is the result for `swiftsync-verify` at this commit: 14/14.

**What is real and what is made.** `r4_block_2600.rcpt` and `r4_block_2530.rcpt` are real GPU proofs from the
round-4 signet image (`c3501ac9…`), starting from a supplied state at 2,500 (`r4_anchor_2500.json`), so the best
verdict any case can reach is DEMO: no genesis-anchored receipt exists yet. `fake_block_2501.rcpt` is a dev-mode
FakeReceipt. The `bad_*` files are made from the real ones by one change each (see the manifest).
