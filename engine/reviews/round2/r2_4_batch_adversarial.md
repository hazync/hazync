# R2-4: batch signatures, adversarial second pass (reviewer report, 27.11.2026, as given)

At 6459eef0 (after R1-3's fixes), read only. No way found to get an invalid ECDSA, BIP340 or BIP341 tweak check
accepted; every idea in the brief fails. Three low items, none a forgery.

1. LOW (high that it is UB, high that it is harmless today). Uninitialised read on the "not a square" path:
   secp-0201-openvm-batch-liftx.patch:48-54: when hz_fe_sqrt returns 0, r->y is never written, but
   secp256k1_fe_normalize_var(&r->y) and is_odd still read it. The function returns 0 and callers discard r, so the
   verdict is right; still C undefined behaviour a compiler could use. Fix: zero r->y in the ret == 0 branch.
2. LOW (high that it is unreachable). hz_fe_sqrt (appended by edits.py) turns a non-canonical input into "not a
   square" (from_be_bytes(a) else { return 0 }): would make a valid pubkey look invalid, which in legacy script (NULLFAIL
   not consensus) would wrongly accept `<pk> CHECKSIG NOT` spent with a valid signature. Unreachable: the caller
   normalises first (patch line 47). Recommend panic / unreachable instead of return 0.
3. LOW. A prover can force all_valid = false for a valid block (any valid=1 hint with a wrong R makes finish() fail;
   main.rs:2090, agg_lite.rs:543): same as the RISC Zero product; matters only if an "invalid" proof is taken as evidence.

Checked and sound: non-canonical encodings (OpenVM's is_eq chip forces both operands below the modulus,
algebra/circuit/src/modular_chip/README.md, is_eq.rs:203-251, so every ==/!= on Fq/Fr, point PartialEq, as_le_bytes,
assert_reduced is canonical-sound; hint R.x/R.y from_le_bytes rejects >= p; transcript bytes via as_le_bytes; MSM scalars
asserted reduced (lines 67, 122) before as_le_bytes_unchecked; reduce_le_bytes ends with assert_reduced); R choices
(identity / off-curve refused by from_xy_nonidentity's y^2 = x^3 + 7 via is_eq, (0,0) fails; wrong-sign R for ECDSA only
breaks the batch: R.y in the transcript, the R1-3 #2 fix holds); scalars (0 or n cannot arrive: libsecp scalars reduced,
r, s = 0 rejected before the hook; kind 2: s != 0 so div_unsafe unique mod n, result asserted reduced); Q from
from_xy_unchecked always from libsecp-parsed keys (never identity / off-curve); pubkey lift (OpenVM's hinted sqrt, both
answers checked: sqrt^2 == a or == a.non_qr, non_qr by Euler's criterion; 0 bypasses the hint; a dishonest hint loops,
no proof); parity from normalised limbs / reduced bytes everywhere; fallback paths (missing / valid != 1 hint: the direct
u1.G + u2.Q check matches libsecp for all four kinds incl. ECDSA R.x == r or r+n when r+n < p, Schnorr and tweak
infinity reject; any other mismatch returns 0 and libsecp decides; NEXT advances before every early return); batch
equation and weights (transcript binds n, u1, u2, Q, R x and y, every kind; z distinct 128-bit; for a non-zero error
vector the equation holds with probability <= 2^-128 per hash query, 2^128 < n; mixing kinds changes only acceptance
tests; keys merge on canonical x||y; the sha2 shim's buffering, padding, alignment correct); MSM exactness (add_into
handles equal x via canonical is_eq; doubling never on the identity, no order-2 point; signed-digit ranges and no-carry
top window for c = 4 Straus and c <= 15 Pippenger, i16 fits; bits_at reads 24 bits >= 7 + 15; bucket reset, top
tracking, identity bases dropped); C hooks (placements keep libsecp's earlier rejections: zero r/s, r_x >= p, s >= n,
tweak overflow, parity not 0/1, tweaked x >= p fall through; hz_fe_limbs normalises before serialising); mode switches
(sig_collect only switches batching off; modes without install have empty hints so every check is direct; install resets
state; finish() gates all_valid before lite.commit, main.rs:2090-2098); the host hint writer (libzync/src/host.rs:63)
untrusted by design.

Not verified: OpenVM's add_ne / double chip constraints and exp_bytes (the non-QR check) at circuit level; assumed sound
congruence checks mod p.
