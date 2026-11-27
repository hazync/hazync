# Round 2: assessment of every finding (27.11.2026)

Six reports (r2_1 .. r2_6). Summary: no forgery of the core chain statement found; OpenVM's own guarantees hold in its
source (R2-1); the block / join logic (R2-2), the chunk / trie binding (R2-3), the batch signatures after round 1's
fixes (R2-4) and the seams end to end (R2-5) are sound. One real defect in a zAPP (R2-6 #1, confirmed by me).

## Confirmed, to fix

| # | finding | checked | decision |
|---|---|---|---|
| R2-6 #1 | zeroconf accepts a T no block can include: weight and sigop cost committed but never bounded (the 4M / 80k limits are block-level) | YES: prover/zyncapps/methods/zeroconf/src/main.rs commits weight, sigops (lines 120-151) with no bound; zapps/zeroconf/src/lib.rs never reads them | reader FAILs weight > 4,000,000 or sigop cost > 80,000 (the values come from the proven chunk record, so the reader can bound them; no image change) |

## To fix (not exploitable)

| # | finding | decision |
|---|---|---|
| R2-6 #2 | zeroconf guest doc claims "as Core's mempool acceptance would"; only consensus rules run | say "consensus-valid for the block after the tip; no relay policy" in the reader, CLI and docs; the guest comment with its next rebuild (same line count) |
| R2-6 #3 | the reader binds the txid, not the wtxid | optional --wtxid compared with z.wtxid |
| R2-6 #4 | OK means "after the tip you passed", not your current tip; the guest's anchor check is a no-op (also R2-5 #2) | docs: give your current tip; the guest pins the genesis anchor like hints / utreexo (zeroconf image rebuild) |
| R2-4 #1 | the liftx patch reads r->y uninitialised on the non-square path (C UB) | zero r->y in that branch (core image) |
| R2-4 #2 | hz_fe_sqrt returns "not a square" on a non-canonical input (unreachable: the caller normalises) | panic instead (core image) |
| R2-5 #1 | verify_stark_bytes (no pin) and Image::with_engine_key (self-pin) are public | verify_stark_bytes test-only; with_engine_key takes the caller's pin and checks it; corecheck with_key(None) test-only |
| R2-1 #2 | verify_full_pinned trusts the key it is given (correctly only reached via verify_full_bytes) | rename / doc it as "the key is already authenticated"; keep verify_full_bytes the entry point |
| R2-2 L1 | a base step accepts any anchor state: soundness rests on the reader pinning anchor_id | docs (VERIFIER.md): every consumer must check anchor_id against the pinned genesis anchor |

## Noted, no change

R2-2 L2 (a join cannot cross a handover: liveness), L3 (checked by me: harmless), I1 / R2-5 #3 (rests on OpenVM's
deferral circuit: R2-1 checked it in source); R2-3 #1 (trie key ~2^112 collision: refusal or drift, not validity), #2
(Rust parsers match Core; a differential test is worth adding), #3 (txlog off: re-review before enabling), #4; R2-4 #3
(forced "not proven", as R1-3 #4).

## For round 3

OpenVM circuit-level checks every report assumed (add_ne / double / exp_bytes constraints, the hook verifier sub-AIR,
the count bus); the guest's consensus internals (hz_tx_rules, CheckTxInputs, BIP68 / MTP, the coin log's shape proof,
the BIP30 SMT, ttree top_pass / sub_pass); the hints and utreexo Deriver logic; the assume and deposit zAPPs; a
differential test of the Rust tx parsers against Core's.

## Status (27.11.2026, fixes.md)

| # | state |
|---|---|
| R2-6 #1 | FIXED: the reader FAILs weight > 4,000,000 or sigop cost > 80,000; tested at and over each limit |
| R2-6 #2 | FIXED in the reader, CLI and README; the guest's comment waits for the zeroconf image's next rebuild |
| R2-6 #3 | FIXED: --wtxid / Asked.wtxid |
| R2-6 #4 | docs FIXED ("pass your current tip"); the guest's genesis-anchor pin waits for the zeroconf image's next rebuild |
| R2-4 #1, #2 | FIXED (core image e3729469; 27/27 honest chunks unchanged) |
| R2-5 #1 | FIXED: with_pinned_engine_key in every shipped path; with_engine_key_unchecked tests only; with_key needs a pin; verify_stark_bytes removed |
| R2-1 #2 | FIXED: verify_full_trusted_key |
| R2-2 L1 | FIXED: VERIFIER.md |
