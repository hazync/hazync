# R2-6: zAPP guests: zeroconf, headers (hints, utreexo briefly) (reviewer report, 27.11.2026, as given)

At 6459eef0, read only. Proof wiring sound in every guest looked at: no way found to make the reader accept a T that
spends a spent coin, a coin from another tip, a failing script, an immature coinbase, or a coin breaking a locktime /
sequence rule, nor a header chain with bad proof of work or retarget. The weak points are what the zeroconf statement
leaves unbounded.

1. MEDIUM (high). Zeroconf has no per-transaction weight or sigops limit, so it accepts a T no block can include.
   ChunkCtx::record (prover/methods/guest/src/agg_lite.rs:242) refuses only raw_tx.len() > MAX_BLOCK_WEIGHT (bytes,
   not weight); Core's CheckTransaction bounds only stripped size x 4; the 4M weight and 80k sigop-cost limits are
   block-level, in the aggregate (agg_lite.rs ~905, total_weight). The zeroconf guest commits weight and sigops
   unchecked (prover/zyncapps/methods/zeroconf/src/main.rs:120, 136-152); the reader never checks them
   (zapps/zeroconf/src/lib.rs:267-297); the CLI only prints them. Scenario: T of ~1 MB stripped + 3 MB witness (weight
   ~7M, under 4M bytes), or ~1,000 bare 20-key CHECKMULTISIG outputs (sigop cost > 80,000): proved, the merchant gets
   OK, T never relays or confirms, the payer's coins stay unspent: no double-spend race needed. Fix: assert
   weight <= 4,000,000 and sigops <= 80,000 (or the policy 400k / 16k).
2. LOW-MEDIUM (high). The guest doc overclaims "as Core's mempool acceptance would" (zeroconf/src/main.rs:11-12): only
   consensus rules run, with block_script_flags(height, 0); no policy (standardness, min relay fee, dust,
   MAX_STANDARD_TX_WEIGHT, RBF): a zero-fee or non-standard T gets OK but will not propagate. Fix: add policy checks, or
   say "consensus-valid for block tip+1".
3. LOW (high). The reader binds the txid only, never the wtxid (lib.rs:270-271): the proof shows some witness for this
   txid is valid, not that the merchant's bytes are; limited (a valid variant needs the keys). Fix: optional wtxid
   compared with z.wtxid.
4. LOW / INFO (high). Tip freshness: expect_tip is an identity comparison (lib.rs:284-287): OK means "the tip you
   passed", not "your current tip"; an older block of the merchant's own chain passes and misses later spends: the CLI
   and docs should require the current tip. Genesis anchor: the guest's anchor check is a no-op (main.rs:95 passes
   &j.anchor_id as the expected anchor); soundness rests on the reader's genesis check (lib.rs:278-282), present;
   hints and utreexo pin it in the guest, zeroconf should too. Network: not committed, but bound via core_id + profile.

Checked and sound. Zeroconf: both journals through env::verify(core_id, ...); check_core_statement requires KIND_CHAIN,
self_id == core_id, network bits, no assumevalid, a coin-log chain; the reader requires core_id = the trusted image,
profile, lineage, anchor, time-too-new, min work, checkpoint, OK needs the tip comparison; engine env::verify (image id
= sha256(exe||vm), public values = sha256(journal), dry runs cannot finalize, bench tagged); the chunk is tied to
height tip+1 and MTP(tip), so IsFinalTx, BIP113, BIP68 at tip+1 as Core; the coin leaf commits outpoint, value,
script, height, coinbase/tracked flag, creation MTP (verify_input.cpp:94-129); script results and batch finish fold
into all_valid, collect mode falls back to libsecp; coin log (position-bound paths, peak / root shape checks, spent
leaves on a separate IV, coins spent in their creating block never appended; the fixture's coin spent at 105 refused);
the record must be T's first run and cover every input; an unconfirmed-parent input (flag bit 1) refused; the parse
layout matches agg_lite.rs's writer with a trailing-words check. Headers (prover/zyncapps/methods/headers/src/main.rs):
prev-hash link; nBits bounds (negative, overflow, zero, pow limit); high-hash; retarget on the epoch-first time with
Core's off-by-one, the [T/4, 4T] clamp, the pow-limit cap, arith_uint256 wrapping; no min-difficulty (mainnet only);
time-too-old over the last 11; BIP34/66/65 version heights; GetBlockProof; MMR leaf index = height with peak-count
asserts; a run from genesis starts from the one empty state and Core's genesis hash; runs chain by input digest; reader:
time-too-new, min chain work, gated() puts the tip check in front of OK (CLI, WASM), tx inclusion depth-pinned by the
coinbase (CVE-2012-2459). hints / utreexo / utreexo-tip (surface): each verifies the core receipt in-guest with pinned
genesis anchors and zero lineage; readers check self_id, base_id, core_id, base_core_id.

Not covered: hz_tx_rules / CheckTxInputs internals (call sites trusted); OpenVM's in-guest deferral binding of the
child key; hints and utreexo Deriver logic; host / prover code; assume and deposit zAPPs.
