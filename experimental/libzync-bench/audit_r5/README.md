# Round 5 suite: test outputs (regenerated 2026-10-31, round 8)

Run on the round-8 images (mainnet `6fcb65af…`, signet `4b91b085…`), both built from a clean guest target, with
`HR=<signet host of image c3501ac9…> run_r5.sh`. The receipt checks (A-G) need a verifier of the receipt's image, and
`test_r5_signet.sh` refuses any other; the verifier code is unchanged since round 6.

- `test_r5_signet.out`: the round-5 verifier fixes on the signet receipt `signet/gpu_r4/block_2600.rcpt`.
  DEMO with the real clock (exit 2); `HAZYNC_VERIFY_NOW` set, even to a passing time, is a failed check (exit 1);
  an unparsable value too; an empty one is unset (DEMO); missing or garbage receipt / hints file / argument give
  `LIBZYNC_VERIFY_FAIL` exit 1 (not 101); a closed stdout does not abort; `verify-chain` refuses a libzync build;
  `swiftsync-file-hash` exits 1 on a file with a trailing byte. Section H: `libzync-prove-range 2501 2501` in dev
  mode from the 2,500 sweep state with `max_time` removed, blocks 1..2,500 present, fills `max_time` = 1599659036
  and its `anchor_effective.json` equals the sweep's state exactly.
- `test_r5_regress.out` (`test_r5_regress.sh` = round 4's `test_r4.sh` into a fresh directory): block 962,000
  valid at 64,309,642 aggregate cycles; `is_base = 2` refused; mainnet 1..200 and signet 1..2,600 sweeps BALANCED
  with `REF_MATCH`; a flipped hint bit fails; BIP325 check. The expected per-block collect-mode panic lines
  (`main.rs:2093`) are filtered out.
- `genesis_anchor.out`: `scripts/libzync/genesis_anchor_id.py` against one-block sweeps from genesis on the round-8
  images: `ANCHOR_MATCH` on both networks.
