# Round 3: guest-side fixes (28.11.2026)

Commit 5cb6724e. Assessed first: cause, every path, risks.

## R3-2 L-1: the oversize-transaction guard ran after Core had parsed the transaction

- Cause: the chunk loop (prover/methods/guest/src/main.rs) read the transaction, ran verify_inputs_batch_once on it,
  and only then called ChunkCtx::record, which held the 4,000,000-byte guard. Its comment claimed "BEFORE any FFI call".
- Paths: ChunkCtx::record has one caller (the chunk loop). The aggregate's coinbase is guarded before any use
  (agg_lite.rs, the assert ahead of tx_out_leaves_mv). The zeroconf guest reads no transaction bytes itself.
- Risk of the fix: none for valid blocks (nothing over 4,000,000 bytes is in one). An oversize transaction now stops
  the run (no proof) after logging `core-reject: bad-blk-length`, instead of committing all_valid = 0. The aggregate
  only accepts all_valid = 1, so the chain statement is unchanged. The rig's oversize case (1.1 MB) is below the bound.
- Fix: the check right after `tx_len` is read; record's guard kept as a second line, its comment corrected.

## R3-3 L5 (the guest comment)

agg_lite.rs's header said the coins were "bound by the SwiftSync total"; they are bound by the coin log. Corrected.

## R3-1 #1: unconstrained degenerate cases of add_ne / double / div

- The guards were already there (equal x tested first in add_into, doubling only of non-identity points, identity
  bases dropped, s != 0 before the hook). Now written down as INVARIANTS (msm_ovm.rs, batch_ecdsa_ovm.rs at the div,
  engine/guest/README.md), and tested so a change that breaks one fails:
- msm-test/src/r3_reference.rs: msm_ovm.rs against RustCrypto's k256 0.13.4 (no code shared with OpenVM's curve
  arithmetic) on repeated keys, repeated R, R = ±Q, equal-x pairs, scalars 0 / 1 / n-1 / 2, identity bases, sizes
  0..200 (Straus and Pippenger), honest batch equations (sum = O) and forged ones (sum != O): 710 cases, all equal.
- msm-test/build.rs writes a counting copy of msm_ovm.rs (each marker asserted to occur exactly once); the test
  requires every counter > 0. Counts: 151,647 / 55,952 / 16,765 / 345,114 / 57,185 / 40,188 / 36 / 375 / 265 / 266.
- Controls (a scratch copy of msm_ovm.rs, restored and compared after): the equal-x test removed: the run fails
  (OpenVM's off-zkvm add_ne panics on equal x); the a = -p test removed: fails "msm_ovm differs from RustCrypto's k256".
- batch-test (R1-3's a, b, c) still passes.

## R3-2 I-6: the allocator-no-reuse startup check on the engine

Confirmed, no change: the engine guest's default features include libzync (engine/guest/Cargo.toml.in), so
c_heap::startup_check() runs at start (main.rs:2404); the guest links OpenVM's default bump allocator (`openvm` with
`std` only; `heap-embedded-alloc` is not enabled anywhere in engine/); engine/results/profile_chunk records that a build
with the reusing allocator (keys-hz5) was refused by this check.

## The new core image and the honest chunks

- reproduce/engine-ids.sh (laptop container, IMAGES=core, --rev 5cb6724e): core 755f5c5af4688606ac17e516161d870077ef40fdfe39f104c79f253845119c77,
  exe 0ee7ff3e87cdcf30f5723741f421db21649f3009532bbe069e3a71547cd6dc2a, vm 00e00077... (unchanged), vmexe c0301de5...,
  sdk.pk f0155abe / agg.vk 15d415ec / openvm.toml 75b745db (keygen unchanged).
- hazync-coord, the canonical layout in a private mount namespace (kit at /root/engine-kit, OPENVM_BUILD_LOCKED=1): the
  same image id.
- Honest chunks (engine/reviews/round1/r1_3_run_chunks.py on that image; r3_run_chunks.out): 91,880, 185,956,
  481,824, 976,379 (Taproot) and 364,292: 27 of 27 journals byte-identical to the previous image's and to RISC Zero's.
- Also run: prover/methods/guest/test-txparse.sh (TXPARSE OK, 0 mismatches, controls as before), test-cshims.sh (ok).
