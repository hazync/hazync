//! Σ sᵢ·Pᵢ on secp256k1 for the batch signature check: signed-digit Pippenger on OpenVM's curve intrinsics.
//!
//! Why not openvm-ecc-guest's `msm`: on block 976,379 chunk 0 it took 31% of the chunk's instruction cells, two thirds
//! of them in `memcpy` moving 64-byte points between buckets, temporaries and return values, plus a fresh bucket vector
//! and a per-point digit extraction at every window (engine/results/profile_chunk). The curve intrinsics themselves
//! are ~2% of cells. This version computes every digit once, negates each point once, adds in place, and reuses one
//! bucket buffer.
//!
//! Exactness: every addition and doubling is a complete one (`add_assign_impl`, `double_assign_impl`: identity,
//! equal and opposite operands handled), so the result is Σ sᵢ·Pᵢ for EVERY input, adversarial ones included; the
//! batch check's accept/reject cannot differ from openvm's `msm`. Scalars are read as their stored little-endian
//! bytes (as openvm's `msm` reads them; callers assert them reduced).
use alloc::vec::Vec;
use k256::{Secp256k1Point as P, Secp256k1Scalar as Fr};
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

pub fn msm(coeffs: &[Fr], bases: &[P]) -> P {
    assert_eq!(coeffs.len(), bases.len(), "msm requires matching scalar/base lengths");
    <P as WeierstrassPoint>::set_up_once();
    let n = bases.len();
    let sc: Vec<&[u8]> = coeffs.iter().map(|s| s.as_le_bytes_unchecked()).collect();
    let bits: Vec<usize> = sc.iter().map(|s| bit_len(s)).collect();
    let c = (1..=16usize).min_by_key(|&c| cost(&bits, c)).unwrap();
    let nw = bits.iter().map(|&b| windows(b, c)).max().unwrap_or(0);
    if nw == 0 {
        return <P as WeierstrassPoint>::IDENTITY;
    }

    // Signed digits, window-major: s = Σ_w d_w·2^(c·w) with d_w in [-(2^(c-1) - 1), 2^(c-1)]. A raw window plus the
    // carry is in [0, 2^c]; above 2^(c-1) it becomes d - 2^c and carries 1. The top window of a b-bit scalar holds at
    // most c-1 bits (b < c·(b/c + 1)), so raw + carry <= 2^(c-1) there and no carry is left over.
    let half = 1i32 << (c - 1);
    let mut dig: Vec<i16> = alloc::vec![0; nw * n];
    let mut any_neg = false;
    for i in 0..n {
        let mut carry = 0i32;
        for w in 0..windows(bits[i], c) {
            let mut d = bits_at(sc[i], w * c, c) as i32 + carry;
            carry = 0;
            if d > half {
                d -= 1 << c;
                carry = 1;
            }
            any_neg |= d < 0;
            dig[w * n + i] = d as i16;
        }
        debug_assert!(carry == 0);
    }
    let negs: Vec<P> = if any_neg { bases.iter().map(|p| -p).collect() } else { Vec::new() };

    let mut buckets: Vec<P> = alloc::vec![<P as WeierstrassPoint>::IDENTITY; half as usize];
    let mut acc = <P as WeierstrassPoint>::IDENTITY;
    for w in (0..nw).rev() {
        if w + 1 != nw {
            for _ in 0..c {
                acc.double_assign_impl::<false>();
            }
        }
        let row = &dig[w * n..(w + 1) * n];
        let mut top = 0usize; // buckets[..top] may be non-identity
        for i in 0..n {
            let d = row[i] as i32;
            if d > 0 {
                let k = (d - 1) as usize;
                buckets[k].add_assign_impl::<false>(&bases[i]);
                if k >= top { top = k + 1; }
            } else if d < 0 {
                let k = (-d - 1) as usize;
                buckets[k].add_assign_impl::<false>(&negs[i]);
                if k >= top { top = k + 1; }
            }
        }
        // Σ_k (k+1)·buckets[k] by parts: running = Σ_{j>=k} buckets[j], window sum = Σ_k running_k.
        let mut run = <P as WeierstrassPoint>::IDENTITY;
        let mut win = <P as WeierstrassPoint>::IDENTITY;
        for k in (0..top).rev() {
            run.add_assign_impl::<false>(&buckets[k]);
            win.add_assign_impl::<false>(&run);
            buckets[k] = <P as WeierstrassPoint>::IDENTITY;
        }
        acc.add_assign_impl::<false>(&win);
    }
    acc
}
