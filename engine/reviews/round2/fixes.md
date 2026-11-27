# Round 2 fixes (27.11.2026)

Each fix against ASSESSMENT.md's tables, with the test that shows it.

## R2-6 #1-#4: zeroconf (reader and CLI; no image change)

- The guest commits T's weight (Core's GetTransactionWeight, weight units) and sigop cost (GetTransactionSigOpCost,
  cost units: legacy x 4 plus P2SH and witness), both from the proven chunk record (agg_lite.rs, core_tx_rules.cpp:151-152).
  The reader (zapps/zeroconf/src/lib.rs, new `check_statement`) now FAILs a weight outside 0..=4,000,000 or a sigop
  cost outside 0..=80,000 (Core's MAX_BLOCK_WEIGHT, MAX_BLOCK_SIGOPS_COST: above either, no block can include T). Core's
  relay limits (400,000 weight, 16,000 sigop cost) are reported as a check line, not enforced.
- Test (tests/engine.rs `statement_limits_and_wtxid`): the REAL proof's statement with one field changed, through the
  reader's statement check (a real proof of a T above a block's limits is impractical to make; the reader is where the
  bound belongs, and it is what changed). At 4,000,000 / 80,000: passes; 4,000,001, 80,001, -1: FAIL; above the relay
  limits but inside a block's: DEMO with "will not relay". Before the fix the reader read neither field (lib.rs at
  0382bb3f), so every over-limit case passed: shown by reading, not run (the old reader has no statement-only entry).
- `--wtxid` (CLI) / `Asked { txid, wtxid }` (library, `verify_zeroconf_asked`): another wtxid FAILs. `verify_zeroconf`
  keeps its signature.
- Wording: "consensus-valid for the block after the tip; not relay policy; not final; pass your CURRENT tip" in the
  library doc, the CLI's help and verdict lines, and zapps/zeroconf/README.md.
- Tests: lib 2 passed; test.sh 15/15 (2 new: T with its wtxid, T with another wtxid).
- Left for the zeroconf image's next rebuild (guest-side, not done: no zeroconf rebuild was needed): pin the genesis
  anchor in the guest (prover/zyncapps/methods/zeroconf/src/main.rs:95 passes the journal's own anchor_id), and its
  doc comment's "as Core's mempool acceptance would" (lines 11-12).

## R2-4 #1, #2: the liftx hook and hz_fe_sqrt (core image)

- engine/guest/secp-0201-openvm-batch-liftx.patch: on the non-square path `r->y` is set to 0 (it was left
  uninitialised and then read by normalize_var / is_odd). Same line count; the patch applies to pristine
  libsecp256k1 v0.5.1 (git apply --check).
- engine/guest/edits.py's hz_fe_sqrt: a non-canonical input panics instead of returning "not a square" (unreachable: the
  caller normalises; a false "not a square" would flip a legacy `<pk> CHECKSIG NOT`).
- New core image (reproduce/engine-ids.sh, laptop, commit e053e1f8): id e3729469cd8222cce0c60b881b8ce3feb12c895209630bf7ca0cc988a25d4024,
  exe e6d76a5f..., vm 00e00077... (unchanged: the keys depend only on the VM config), vmexe 07a1152e....
- Honest chunks: every chunk of 91,880, 185,956, 481,824, 976,379 (Taproot) and 364,292 executed on the new image
  (engine/reviews/round1/r1_3_run_chunks.py, output r2_run_chunks.out): 27 of 27 journals byte-identical to the old
  image's and to RISC Zero's.

## R2-5 #1, R2-1 #2: verifier APIs

- swiftsync-verify: `Image::with_pinned_engine_key(bytes, pin)` refuses a key whose SHA-256 is not the caller's pin; the
  CLI, the C ABI, the core wasm and corecheck use it. The self-pinning constructor is renamed
  `with_engine_key_unchecked` (documented: only bytes the caller authenticated, or tests); every remaining caller is a
  test, a fuzz crate, or a #[cfg(test)] module.
- zapps/corecheck::with_key requires a pin (`&[u8; 32]`; no unpinned form); its test callers pass the fixture key's own
  computed pin.
- engine/verify-core: verify_stark_bytes (no pin, no image, no journal check; no caller) removed; verify_full_pinned
  renamed verify_full_trusted_key and documented as trusting its key; verify_full_bytes stays the entry point.
- Test (swiftsync-verify tests/engine.rs `the_pinned_constructor_refuses_another_key`): the fixture key under its pin
  verifies; one changed key byte under the same pin, and the key under another pin, are refused.
- Also fixed: swiftsync-verify tests/cli_trie_fact.rs had failed since R1-6 #1 (it gave --vk without --vk-sha256).

## R2-2 L1: docs

- engine/VERIFIER.md "Every consumer checks the anchor": a base step accepts any anchor state; every consumer compares
  anchor_id with the pinned genesis anchor (or a state it trusts: at most DEMO).

## Wasm modules (re-pinned; laptop build at e053e1f8, second machine PENDING: hazync-coord was compiling for another
## agent's cost measurement)

| module | sha256 | raw | gzipped |
|---|---|---|---|
| core | 4cbc884d39898121... | 423,181 | 167,223 |
| assume | ee33676495fa0810... | 350,201 | 139,445 |
| deposit | 69a3dbec32a20c56... | 389,072 | 154,414 |
| headers | c1d551e44245ca85... | 320,963 | 127,394 |
| hints | ee444a7a50e34253... | 285,537 | 114,068 |
| liveness | 004a7aa76c99faf8... | 382,030 | 153,101 |
| statement | 682505b51d6aebec... | 1,680,473 | 1,335,959 |
| utreexo | d4cb8904951124e5... | 299,440 | 118,150 |
