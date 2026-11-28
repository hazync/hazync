//! Σ sᵢ·Pᵢ on secp256k1 for the batch signature check: signed-digit Pippenger on OpenVM's curve intrinsics.
//!
//! Why not openvm-ecc-guest's `msm`: on block 976,379 chunk 0 it took 31% of the chunk's instruction cells, two thirds
//! of them in `memcpy` moving 64-byte points between buckets, temporaries and return values, plus a fresh bucket vector
//! and a per-point digit extraction at every window (engine/results/profile_chunk). The curve intrinsics themselves
//! are ~2% of cells. This version computes every digit once, negates each point once, adds in place, and reuses one
//! bucket buffer.
//!
//! Exactness: the result is Σ sᵢ·Pᵢ for EVERY input, adversarial ones included, so the batch check's accept/reject
//! cannot differ from openvm's `msm`. Scalars are read as their stored little-endian bytes (as openvm's `msm` reads
//! them; callers assert them reduced).
//!
//! INVARIANTS (review R3-1 #1). OpenVM's `add_ne` leaves λ, and so the sum, UNCONSTRAINED when both x are equal, its
//! `double` leaves λ unconstrained on the (0,0) encoding of the identity, and `div` of 0 by 0 is unconstrained: a
//! prover could then choose the result and push the batch sum to the identity, a signature forgery. This file never
//! reaches those cases, and every change to it must keep that so:
//! - identity bases are dropped before any arithmetic (`live`), so every point below is non-identity;
//! - `add_into` is the ONLY addition: an empty accumulator takes a copy; equal x is tested first (canonical `eq_impl`)
//!   and becomes the identity (a = -p) or a doubling of a non-identity point (a = p); `add_ne` runs only on
//!   different x;
//! - every doubling (`double_assign_nonidentity`) acts on a non-identity point: `add_into`'s a = p case, and the
//!   accumulators only while their `filled` flag is set; secp256k1 has prime order, so 2a != O for a != O;
//! - the Straus tables hold (k+1)·P with k+1 < n, never the identity.
//! The `div` (ECDSA's u1 = e/s, u2 = r/s, batch_ecdsa_ovm.rs) only divides by an s that libsecp has already refused
//! when zero (secp-0201's verify, before the hook). msm-test checks this file against RustCrypto's k256 on inputs that
//! reach every guarded branch, and counts them (msm-test/build.rs).
use alloc::vec::Vec;
use k256::{Secp256k1Coord as Fq, Secp256k1Point as P, Secp256k1Scalar as Fr};
use openvm_algebra_guest::IntMod;
use openvm_ecc_guest::weierstrass::WeierstrassPoint;

/// Bit length of a little-endian integer.
fn bit_len(s: &[u8]) -> usize {
    for k in (0..s.len()).rev() {
        if s[k] != 0 {
            return 8 * k + (8 - s[k].leading_zeros() as usize);
        }
    }
    0
}

/// `c` bits (c <= 16) of the little-endian `s` from bit `at`; bits past the end are 0.
fn bits_at(s: &[u8], at: usize, c: usize) -> u32 {
    let k = at / 8;
    let mut v = 0u32;
    for j in 0..3 {
        if k + j < s.len() {
            v |= (s[k + j] as u32) << (8 * j);
        }
    }
    (v >> (at % 8)) & ((1u32 << c) - 1)
}

/// Windows a scalar of `b` bits needs at window size `c` (signed digits; see `msm`). 0 for a zero scalar.
fn windows(b: usize, c: usize) -> usize {
    if b == 0 { 0 } else { b / c + 1 }
}

/// Estimated cost in curve additions: one per nonzero digit, two per bucket per window (summation by parts), `c`
/// doublings per window, and the digit scan (a few instructions per point per window, ~1/8 of an addition).
fn cost(bits: &[usize], c: usize) -> usize {
    let w = bits.iter().map(|&b| windows(b, c)).max().unwrap_or(0);
    let digits: usize = bits.iter().map(|&b| windows(b, c)).sum();
    digits + w * ((1usize << c) + c) + w * bits.len() / 8
}

/// `a += p` for a NON-identity `p`, with `a`'s identity tracked by `filled` (false = identity). Complete: equal x
/// is a doubling or the identity, as in openvm's `add_assign_impl`. `double_assign_nonidentity` is sound here because
/// secp256k1's group has prime order (no point of order 2), so 2a != O for every a != O.
#[inline(always)]
fn add_into(a: &mut P, filled: &mut bool, p: &P) {
    if !*filled {
        *a = p.clone();
        *filled = true;
        return;
    }
    // SAFETY: the curve (and its coordinate field) are set up at the top of `msm`; a and p are non-identity and,
    // in the add_ne branch, have different x (so a != +-p).
    unsafe {
        if a.x().eq_impl::<false>(p.x()) {
            if a.y() + p.y() == <Fq as IntMod>::ZERO {
                *filled = false; // a = -p
            } else {
                a.double_assign_nonidentity::<false>(); // a = p
            }
        } else {
            a.add_ne_assign_nonidentity::<false>(p);
        }
    }
}

/// Signed digits of a scalar of `b` bits at window size `c`, low window first, into `out` (see `msm`).
fn digits(s: &[u8], b: usize, c: usize, mut out: impl FnMut(usize, i32)) {
    let half = 1i32 << (c - 1);
    let mut carry = 0i32;
    for w in 0..windows(b, c) {
        let mut d = bits_at(s, w * c, c) as i32 + carry;
        carry = 0;
        if d > half {
            d -= 1 << c;
            carry = 1;
        }
        out(w, d);
    }
    debug_assert!(carry == 0);
}

/// Up to this many points: Straus (interleaved windows, a table of multiples per point, all on the stack).
const STRAUS_MAX: usize = 4;
const STRAUS_C: usize = 4;

pub fn msm(coeffs: &[Fr], bases: &[P]) -> P {
    assert_eq!(coeffs.len(), bases.len(), "msm requires matching scalar/base lengths");
    <P as WeierstrassPoint>::set_up_once();
    let id = <P as WeierstrassPoint>::IDENTITY;
    // identity bases contribute nothing; every point below is non-identity
    let live: Vec<usize> = (0..bases.len()).filter(|&i| bases[i] != id).collect();
    let sc: Vec<&[u8]> = live.iter().map(|&i| coeffs[i].as_le_bytes_unchecked()).collect();
    let bits: Vec<usize> = sc.iter().map(|s| bit_len(s)).collect();
    let n = live.len();
    if n <= STRAUS_MAX {
        return straus(&live.iter().map(|&i| &bases[i]).collect::<Vec<_>>(), &sc, &bits);
    }
    // c <= 15: signed digits lie in (-2^(c-1), 2^(c-1)] and are stored as i16; at c = 16 a digit of +32768 wrapped
    // (review R1-3 #3)
    let c = (1..=15usize).min_by_key(|&c| cost(&bits, c)).unwrap();
    let nw = bits.iter().map(|&b| windows(b, c)).max().unwrap_or(0);
    if nw == 0 {
        return id;
    }

    // Signed digits, window-major: s = Σ_w d_w·2^(c·w) with d_w in [-(2^(c-1) - 1), 2^(c-1)]. A raw window plus the
    // carry is in [0, 2^c]; above 2^(c-1) it becomes d - 2^c and carries 1. The top window of a b-bit scalar holds at
    // most c-1 bits (b < c·(b/c + 1)), so raw + carry <= 2^(c-1) there and no carry is left over.
    let half = 1usize << (c - 1);
    let mut dig: Vec<i16> = alloc::vec![0; nw * n];
    let mut any_neg = false;
    for i in 0..n {
        digits(sc[i], bits[i], c, |w, d| {
            any_neg |= d < 0;
            dig[w * n + i] = d as i16;
        });
    }
    let pts: Vec<&P> = live.iter().map(|&i| &bases[i]).collect();
    let negs: Vec<P> = if any_neg { pts.iter().map(|p| -*p).collect() } else { Vec::new() };

    let mut buckets: Vec<P> = alloc::vec![id.clone(); half];
    let mut full: Vec<bool> = alloc::vec![false; half];
    let (mut acc, mut acc_f) = (id.clone(), false);
    for w in (0..nw).rev() {
        if w + 1 != nw && acc_f {
            for _ in 0..c {
                // SAFETY: acc is non-identity (acc_f) and the group has no point of order 2
                unsafe { acc.double_assign_nonidentity::<false>() };
            }
        }
        let row = &dig[w * n..(w + 1) * n];
        let mut top = 0usize; // buckets[top..] are all empty
        for i in 0..n {
            let d = row[i] as i32;
            if d != 0 {
                let (k, p) = if d > 0 { ((d - 1) as usize, pts[i]) } else { ((-d - 1) as usize, &negs[i]) };
                add_into(&mut buckets[k], &mut full[k], p);
                if k >= top {
                    top = k + 1;
                }
            }
        }
        // Σ_k (k+1)·buckets[k] by parts: running = Σ_{j>=k} buckets[j], window sum = Σ_k running_k.
        let (mut run, mut run_f) = (id.clone(), false);
        let (mut win, mut win_f) = (id.clone(), false);
        for k in (0..top).rev() {
            if full[k] {
                add_into(&mut run, &mut run_f, &buckets[k]);
                full[k] = false;
            }
            if run_f {
                add_into(&mut win, &mut win_f, &run);
            }
        }
        if win_f {
            add_into(&mut acc, &mut acc_f, &win);
        }
    }
    if acc_f { acc } else { id }
}

/// Σ sᵢ·Pᵢ for a few non-identity points: one shared accumulator, STRAUS_C-bit signed windows, a table of 1..8
/// multiples of each point and their negations.
fn straus(pts: &[&P], sc: &[&[u8]], bits: &[usize]) -> P {
    const H: usize = 1 << (STRAUS_C - 1);
    const MAXW: usize = 256 / STRAUS_C + 1;
    let id = <P as WeierstrassPoint>::IDENTITY;
    let n = pts.len();
    let nw = bits.iter().map(|&b| windows(b, STRAUS_C)).max().unwrap_or(0);
    let mut dig = [[0i8; MAXW]; STRAUS_MAX];
    let mut tab: [[P; H]; STRAUS_MAX] = core::array::from_fn(|_| core::array::from_fn(|_| id.clone()));
    let mut neg: [[P; H]; STRAUS_MAX] = core::array::from_fn(|_| core::array::from_fn(|_| id.clone()));
    // tab[i][k] = (k+1)·P_i; never the identity (k+1 < n, the group order), so the table is all non-identity
    let mut tab_f = [[false; H]; STRAUS_MAX];
    for i in 0..n {
        digits(sc[i], bits[i], STRAUS_C, |w, d| dig[i][w] = d as i8);
        let mut f = false;
        let mut t = id.clone();
        for k in 0..H {
            add_into(&mut t, &mut f, pts[i]);
            tab_f[i][k] = f;
            tab[i][k] = t.clone();
            neg[i][k] = -&t;
        }
    }
    let (mut acc, mut acc_f) = (id.clone(), false);
    for w in (0..nw).rev() {
        if acc_f {
            for _ in 0..STRAUS_C {
                // SAFETY: as in msm
                unsafe { acc.double_assign_nonidentity::<false>() };
            }
        }
        for i in 0..n {
            let d = dig[i][w] as i32;
            if d != 0 {
                let k = (d.unsigned_abs() - 1) as usize;
                if tab_f[i][k] {
                    add_into(&mut acc, &mut acc_f, if d > 0 { &tab[i][k] } else { &neg[i][k] });
                }
            }
        }
    }
    if acc_f { acc } else { id }
}
