# R1-3: batch ECDSA / MSM (reviewer report, 27.11.2026, as given)

Read-only. Answer: YES, one concrete Schnorr forgery and one weakness letting a determined prover pass an invalid
ECDSA signature; both regressions against the RISC Zero product guest. The header (batch_ecdsa_ovm.rs:1-3) claims "SAME
transcript, SAME acceptance tests": neither holds.

1. HIGH (high confidence). A hinted R = (0,0) becomes the point at infinity, so a forged BIP340 signature passes.
   batch_ecdsa_ovm.rs:71-73, accept_r :39. OpenVM v2.1.0 `WeierstrassPoint::from_xy` returns Some(IDENTITY) when
   x == 0 and y == 0 (extensions/ecc/guest/src/weierstrass.rs:108-117). accept_r for kind 1 only asks x(R) == rx and
   y even: with rx = 0 both hold, the item is deferred; in finish, -identity is dropped by the MSM's identity filter
   (msm_ovm.rs:248), leaving s.G - e.P = O. Scenario: spend your own taproot output with (r = 0^32, s = e.d mod n),
   e = H(0^32 || P || m); libsecp computes s.G - e.P = infinity and rejects; a malicious prover sends the hint
   {valid=1, x=0, y=0} and the chunk proves valid. Kinds 3/4: open in principle (needs P + t.G = O, t a tagged hash:
   infeasible). The product is safe: batch_ecdsa.rs checks y^2 == x^3 + 7 by hand. Fix: from_xy_nonidentity, or reject
   the identity explicitly.
2. MEDIUM-HIGH (medium-high on the flaw, medium on attack cost). R.y is not in the transcript, so for ECDSA (kind 0) the
   prover picks +/-R after the weights are fixed. batch_ecdsa_ovm.rs:74-76 (raw[128..] holds R.x only), :87. The
   product hashes the whole Deferred incl. R.y (192 bytes per item); the engine 160 bytes. For kind 0 both R and -R pass
   accept_r, so the seed and every z_i are fixed independently of a free +/- per item. Scenario: N own valid ECDSA
   signatures plus one invalid one with chosen R0; flipping item i adds 2.z_i.tau_i; forging = subset-sum mod n;
   Wagner's k-tree with a few thousand items, ~2^40 work, far below the intended 2^-128. Kinds 1, 3, 4 fix y by parity.
   Fix: hash y(R) (or its parity) into raw, i.e. the product's layout.
3. LOW (high that it exists, low that it is reached). msm_ovm.rs:265/270: digits stored as i16; at window c = 16 a
   digit can be +32768 -> -32768, wrong result. The cost model picks c = 16 only near a million points; if reached, a
   valid batch is rejected (liveness), never a forgery. Fix: cap c at 15 or use i32.
4. LOW / design note. main.rs:2090 sets all_valid = false when the batch fails; agg_lite.rs:543 fails the block. A
   prover can force it (e.g. the -R hint of 2). Fine unless something treats all_valid = false as proof a block is
   invalid: document it as "not proven", not "invalid".

Checked and sound: weights drawn after every equation is fixed (seed = SHA-256 over domain tag, n, every tuple and kind,
in finish(); hints installed once before any script; z_i 128-bit); host-forced verdicts (missing / short /
non-canonical / off-curve / accept_r-failing hint returns 0 and libsecp's own code runs; the no-hint path computes
u1.G + u2.Q itself, infinity invalid); truncation (n counts the guest's own deferred calls and is in the transcript);
libsecp hooks (ECDSA after parse and r, s != 0; div_unsafe safe; x mod n == r matches libsecp's r / r+n; Schnorr rx < p,
s overflow rejected before the hook, e negated, even y enforced; tweak overflow, x-only >= p, parity not 0/1 skip the
hook; low-S unchanged; 8x32 limbs on riscv32); lift_x (hz_fe_sqrt canonical input, OpenVM's hinted sqrt proving
non-residues too, then fe_set_b32_limit and libsecp's parity fix); values (scalars assert_reduced before every MSM, Q
from libsecp; MSM edge cases: identity bases filtered, equal / opposite points, zero scalars; doubling without identity
check safe, prime order; signed-digit carries, 3-byte reads for c <= 16, Straus tables without identity).

"Findings 1 and 2 should be fixed before this path is trusted."
