# engine/guest: the core guest on OpenVM

The engine's core guest is ASSEMBLED from the product guest's sources (prover/methods/guest, libzync, coinbase-smt)
by `assemble.sh` on the base `base.sh` makes (Core v28.0 and libsecp256k1 v0.5.1 with the patches listed in
docs/libzync/CONSENSUS_RULES.md "Patches on consensus paths"), with the engine's own pieces:

| file | what |
|---|---|
| `batch_ecdsa_ovm.rs` | the batch signature check (hints, the transcript, the weighted sum) on OpenVM's secp256k1 |
| `msm_ovm.rs` | Σ sᵢ·Pᵢ for that check (Straus for n <= 4, signed-digit Pippenger above) |
| `secp-0201-openvm-batch-liftx.patch`, `secp-0202-sha-openvm-native.patch` | libsecp256k1: the batch hooks, the hinted square root, SHA-256 on OpenVM |
| `core-0201-sha-openvm-native.patch`, `core-0202-sha-finalize-in-place.patch` | Core: SHA-256 on OpenVM's native state layout |
| `edits.py`, `coinlog_ovm.py` | source edits applied at assembly (each asserted to match exactly once) |
| `xver_ovm.rs` | the cross-version handover check (engine/HANDOVER.md) |
| `msm-test/`, `batch-test/` | host tests of msm_ovm.rs and batch_ecdsa_ovm.rs |

## Invariants of the signature arithmetic (review R3-1 #1, 28.11.2026)

OpenVM's curve and field chips have degenerate cases where the result is NOT constrained: `add_ne` with equal x
(λ free, so the sum is free), `double` on the (0,0) encoding of the identity, `div` of 0 by 0. A guest that reached one
would let a prover choose the result and push the batch sum to the identity: a signature forgery. The guest never
reaches them, and every change to these files must keep it so (the full list is the INVARIANTS block at the top of
`msm_ovm.rs`):

- identity bases are dropped before any arithmetic;
- one addition routine, `add_into`, tests equal x first and turns it into the identity (a = -p) or a doubling of a
  non-identity point (a = p); `add_ne` runs only on different x;
- every doubling acts on a non-identity point (prime order: no point of order 2);
- the batch's `div` divides only by an s that libsecp refused when zero, before the hook;
- R must be a non-identity curve point (`from_xy_nonidentity`: review R1-3 #1) and both its coordinates enter the
  transcript (R1-3 #2).

`msm-test` checks `msm_ovm.rs` against openvm-ecc-guest's `msm` and against an independent implementation
(RustCrypto's k256 0.13.4) on inputs that reach every guarded branch: repeated keys, repeated R, R = ±Q, equal-x pairs,
scalars 0 / 1 / n-1, identity bases, Straus and Pippenger sizes, honest batches (sum = O) and forged ones (sum != O).
Its build.rs makes a counting copy of `msm_ovm.rs` (markers asserted to occur exactly once) and the test requires every
counter to be non-zero, so it shows the branches were reached, not only that the answers agreed:

```sh
cd engine/guest/msm-test && cargo test --release -- --nocapture
```

28.11.2026: 710 cases, all equal to RustCrypto's k256; branch counts 151,647 / 55,952 / 16,765 / 345,114 / 57,185 /
40,188 / 36 / 375 / 265 / 266 (build.rs numbering). Controls: with the equal-x test removed the run fails (OpenVM's
off-zkvm add_ne panics on equal x); with the a = -p test removed it fails on the comparison with RustCrypto's k256.
