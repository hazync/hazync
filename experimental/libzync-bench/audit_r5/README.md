# Round 5 audit: test outputs (2026-10-31)

Regenerated in round 7 on the round-7 images (mainnet `a50c3bac…`, signet `5e552ce9…`) with
`HR=<round-6 signet host> run_r5.sh`: the receipt checks (A-G) need a verifier of the receipt's image `c3501ac9…`,
and the verifier code is unchanged since round 6. Rounds 5 and 6 ran the same suite on the round-4 images.

- `test_r5_signet.out`: the round-5 verifier fixes on the signet receipt `signet/gpu_r4/block_2600.rcpt`.
  DEMO with the real clock (exit 2); `HAZYNC_VERIFY_NOW` set, even to a passing time, is a failed check (exit 1);
  an unparsable value too; missing or garbage receipt / hints file give `LIBZYNC_VERIFY_FAIL` exit 1 (not 101);
  `verify-chain` refuses a libzync build; `swiftsync-file-hash` exits 1 on a file with a trailing byte.
- `test_r5_regress.out` (`test_r5_regress.sh` = round 4's `test_r4.sh` into a fresh directory): block 962,000
  valid at 64,309,178 aggregate cycles; `is_base = 2` refused; mainnet 1..200 and signet 1..2,600 sweeps
  BALANCED with `REF_MATCH`; a flipped hint bit fails; BIP325 check. The expected per-block collect-mode panic
  lines (`main.rs:2093`) are filtered out.
- Also run (outputs not kept): `libzync-prove-range 2501 2501` in dev mode from the 2,500 sweep state with
  `max_time` removed, blocks 1..2,500 present: the replay path filled `max_time` = 1599659036, and the resulting
  `anchor_effective.json` equals the sweep's state exactly (`smt_root` replayed and checked).
- `scripts/libzync/genesis_anchor_id.py` against one-block sweeps from genesis: `ANCHOR_MATCH` on mainnet
  (`6178a96d…`) and signet (`91ccd466…`).
