//! Pippenger MSM over the bigint2 coprocessor, with operation counters.
//!
//! `msm` is a copy of prover/methods/guest/src/msm.rs (unsigned digits, 2^w - 1 buckets).
//! `msm_signed` recodes digits into [-2^(w-1), 2^(w-1)] so it needs half the buckets, and skips
//! the bucket reduction for a window in which no bucket was touched.
//!
//! The counters count only additions that reach the coprocessor (neither operand the identity),
//! because adds against the identity are a branch and a copy, not a point op.

use risc0_crypto::curves::secp256k1::{Affine, Fr};

static mut ADDS: u64 = 0;
static mut DBLS: u64 = 0;

pub fn reset_counts() {
    unsafe {
        ADDS = 0;
        DBLS = 0;
    }
}
pub fn counts() -> (u64, u64) {
    unsafe { (ADDS, DBLS) }
}

#[inline(always)]
fn add(dst: &mut Affine, a: &Affine, b: &Affine) {
    if !a.is_identity() && !b.is_identity() {
        unsafe { ADDS += 1 };
    }
    dst.add_into(a, b);
}
#[inline(always)]
fn dbl(p: &Affine) -> Affine {
    if !p.is_identity() {
        unsafe { DBLS += 1 };
    }
    p.double()
}

fn be_bytes(s: &Fr) -> [u8; 32] {
    let mut b = [0u8; 32];
    s.to_bigint().write_be_bytes(&mut b);
    b
}

#[inline]
fn bit_be(b: &[u8; 32], i: usize) -> u32 {
    ((b[31 - (i >> 3)] >> (i & 7)) & 1) as u32
}

fn digit(b: &[u8; 32], lo: usize, window: usize) -> usize {
    let mut d = 0usize;
    for k in 0..window {
        if lo + k < 256 {
            d |= (bit_be(b, lo + k) as usize) << k;
        }
    }
    d
}

/// Unsigned-digit Pippenger, as in the main tree.
pub fn msm(points: &[Affine], scalars: &[Fr], window: usize) -> Affine {
    assert_eq!(points.len(), scalars.len());
    let bytes: Vec<[u8; 32]> = scalars.iter().map(be_bytes).collect();
    let nb = 1usize << window;
    let mut acc = Affine::IDENTITY;
    let mut lo = (256 / window) * window;
    let mut first = true;
    loop {
        if !first {
            for _ in 0..window {
                acc = dbl(&acc);
            }
        }
        let mut buckets = vec![Affine::IDENTITY; nb];
        for (p, b) in points.iter().zip(bytes.iter()) {
            let d = digit(b, lo, window);
            if d != 0 {
                let cur = buckets[d];
                add(&mut buckets[d], &cur, p);
            }
        }
        let mut running = Affine::IDENTITY;
        let mut total = Affine::IDENTITY;
        for j in (1..nb).rev() {
            let r = running;
            add(&mut running, &r, &buckets[j]);
            let t = total;
            add(&mut total, &t, &running);
        }
        let a = acc;
        add(&mut acc, &a, &total);
        first = false;
        if lo == 0 {
            break;
        }
        lo -= window;
    }
    acc
}

/// `window` bits of a little-endian 8x32 scalar starting at bit `lo` (bits past 255 read as 0).
#[inline(always)]
fn digit_limbs(l: &[u32; 8], lo: usize, window: usize) -> u32 {
    let (i, sh) = (lo >> 5, lo & 31);
    if i >= 8 {
        return 0;
    }
    let mut v = (l[i] as u64) >> sh;
    if sh + window > 32 && i + 1 < 8 {
        v |= (l[i + 1] as u64) << (32 - sh);
    }
    (v as u32) & ((1u32 << window) - 1)
}

#[inline(always)]
fn add_assign(dst: &mut Affine, p: &Affine) {
    if !dst.is_identity() && !p.is_identity() {
        unsafe { ADDS += 1 };
    }
    *dst += p;
}

/// Signed-digit Pippenger: digits in [-2^(w-1), 2^(w-1)], 2^(w-1) buckets. Digits come straight
/// from the scalar's u32 limbs, and buckets are updated by value (`+=`), which risc0-crypto notes
/// is cheaper on R0VM than `add_into`'s out-parameter.
pub fn msm_signed(points: &[Affine], scalars: &[Fr], window: usize) -> Affine {
    assert_eq!(points.len(), scalars.len());
    assert!((2..=20).contains(&window));
    let nwin = 256 / window + 1;
    let half = 1i32 << (window - 1);
    let full = 1i32 << window;
    let mut digits = vec![0i32; points.len() * nwin];
    let mut top_used = 0usize;
    for (i, s) in scalars.iter().enumerate() {
        let l = &s.as_bigint().0;
        let mut carry = 0i32;
        let row = &mut digits[i * nwin..(i + 1) * nwin];
        for (wi, slot) in row.iter_mut().enumerate() {
            let mut d = digit_limbs(l, wi * window, window) as i32 + carry;
            if d > half {
                d -= full;
                carry = 1;
            } else {
                carry = 0;
            }
            *slot = d;
            if d != 0 && wi + 1 > top_used {
                top_used = wi + 1;
            }
        }
        assert_eq!(carry, 0, "carry out of the top window");
    }
    let negs: Vec<Affine> = points.iter().map(|p| -p).collect();

    let nb = half as usize;
    let mut acc = Affine::IDENTITY;
    let mut buckets = vec![Affine::IDENTITY; nb + 1];
    for wi in (0..top_used).rev() {
        for _ in 0..window {
            acc = dbl(&acc);
        }
        for b in buckets.iter_mut() {
            *b = Affine::IDENTITY;
        }
        let mut touched = false;
        for i in 0..points.len() {
            let d = digits[i * nwin + wi];
            if d > 0 {
                add_assign(&mut buckets[d as usize], &points[i]);
                touched = true;
            } else if d < 0 {
                add_assign(&mut buckets[(-d) as usize], &negs[i]);
                touched = true;
            }
        }
        if !touched {
            continue;
        }
        let mut running = Affine::IDENTITY;
        let mut total = Affine::IDENTITY;
        for j in (1..=nb).rev() {
            add_assign(&mut running, &buckets[j]);
            add_assign(&mut total, &running);
        }
        add_assign(&mut acc, &total);
    }
    acc
}

// ---------------------------------------------------------------------------------------------
// Raw coprocessor path: points as [u32; 16] (x then y, little-endian limbs), every add writes
// into the OTHER half of a ping-pong pair, so nothing is ever copied after a bucket's first touch.
// The two circuit blobs are copied verbatim from risc0-bigint2 1.4.14 (Apache-2.0).

#[repr(C, align(4))]
struct Aligned<T: ?Sized>(T);
static ADD_BLOB: &Aligned<[u8; 1596]> = &Aligned(*include_bytes!("ec_add_256.blob"));
static DBL_BLOB: &Aligned<[u8; 1680]> = &Aligned(*include_bytes!("ec_double_256.blob"));
static CURVE: [u32; 24] = [
    0xFFFFFC2F, 0xFFFFFFFE, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF,
    0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0,
];

pub type Pt = [u32; 16];

#[inline(always)]
fn raw_add(a: &Pt, b: &Pt, out: *mut Pt) {
    unsafe {
        ADDS += 1;
        risc0_bigint2::ffi::sys_bigint2_4(ADD_BLOB.0.as_ptr(), a.as_ptr(), b.as_ptr(), CURVE.as_ptr(), out.cast());
    }
}
#[inline(always)]
fn raw_dbl(a: &Pt, out: *mut Pt) {
    unsafe {
        DBLS += 1;
        risc0_bigint2::ffi::sys_bigint2_3(DBL_BLOB.0.as_ptr(), a.as_ptr(), CURVE.as_ptr(), out.cast());
    }
}

/// A point that may be the identity, stored as a ping-pong pair.
#[derive(Clone, Copy)]
pub struct Acc {
    slot: [Pt; 2],
    /// 0 = identity, 1 = value in slot[0], 2 = value in slot[1].
    st: u8,
}
impl Acc {
    pub const ID: Acc = Acc { slot: [[0; 16]; 2], st: 0 };
    #[inline(always)]
    pub fn get(&self) -> Option<&Pt> {
        match self.st {
            0 => None,
            s => Some(&self.slot[(s - 1) as usize]),
        }
    }
    /// self += p. Equal-x inputs (P+P or P-P) leave the chord rule, so they take a slow path;
    /// with honest random data that never happens, but a library must not trust that.
    #[inline(always)]
    pub fn add(&mut self, p: &Pt) {
        match self.st {
            0 => {
                self.slot[0] = *p;
                self.st = 1;
            }
            s => {
                let cur = (s - 1) as usize;
                let nxt = 1 - cur;
                if self.slot[cur][..8] == p[..8] {
                    if self.slot[cur][8..] == p[8..] {
                        let (a, b) = self.slot.split_at_mut(1);
                        if cur == 0 { raw_dbl(&a[0], &mut b[0]) } else { raw_dbl(&b[0], &mut a[0]) }
                        self.st = nxt as u8 + 1;
                    } else {
                        self.st = 0;
                    }
                    return;
                }
                let (a, b) = self.slot.split_at_mut(1);
                if cur == 0 { raw_add(&a[0], p, &mut b[0]) } else { raw_add(&b[0], p, &mut a[0]) }
                self.st = nxt as u8 + 1;
            }
        }
    }
    pub fn add_acc(&mut self, o: &Acc) {
        if let Some(p) = o.get() {
            self.add(p);
        }
    }
    pub fn dbl(&mut self) {
        if let Some(_) = self.get() {
            let cur = (self.st - 1) as usize;
            let (a, b) = self.slot.split_at_mut(1);
            if cur == 0 { raw_dbl(&a[0], &mut b[0]) } else { raw_dbl(&b[0], &mut a[0]) }
            self.st = (1 - cur) as u8 + 1;
        }
    }
}

pub fn to_pt(p: &Affine) -> Pt {
    let (x, y) = p.xy().expect("identity has no coordinates");
    let mut o = [0u32; 16];
    o[..8].copy_from_slice(&x.as_bigint().0);
    o[8..].copy_from_slice(&y.as_bigint().0);
    o
}

/// p - y for a canonical y (negation without the field type).
pub fn neg_pt(p: &Pt) -> Pt {
    const P: [u32; 8] = [0xFFFFFC2F, 0xFFFFFFFE, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF];
    let mut o = *p;
    let mut borrow = 0i64;
    for k in 0..8 {
        let v = P[k] as i64 - p[8 + k] as i64 - borrow;
        o[8 + k] = v as u32;
        borrow = (v < 0) as i64;
    }
    o
}

/// Signed-digit Pippenger on raw points. Same digit recoding as `msm_signed`.
pub fn msm_raw(points: &[Pt], scalars: &[Fr], window: usize) -> Acc {
    assert_eq!(points.len(), scalars.len());
    assert!((2..=20).contains(&window));
    let nwin = 256 / window + 1;
    let half = 1i32 << (window - 1);
    let full = 1i32 << window;
    let mut digits = vec![0i32; points.len() * nwin];
    let mut top_used = 0usize;
    for (i, s) in scalars.iter().enumerate() {
        let l = &s.as_bigint().0;
        let mut carry = 0i32;
        let row = &mut digits[i * nwin..(i + 1) * nwin];
        for (wi, slot) in row.iter_mut().enumerate() {
            let mut d = digit_limbs(l, wi * window, window) as i32 + carry;
            if d > half {
                d -= full;
                carry = 1;
            } else {
                carry = 0;
            }
            *slot = d;
            if d != 0 && wi + 1 > top_used {
                top_used = wi + 1;
            }
        }
        assert_eq!(carry, 0);
    }
    let negs: Vec<Pt> = points.iter().map(neg_pt).collect();
    let nb = half as usize;
    let mut acc = Acc::ID;
    let mut buckets = vec![Acc::ID; nb + 1];
    for wi in (0..top_used).rev() {
        for _ in 0..window {
            acc.dbl();
        }
        for b in buckets.iter_mut() {
            b.st = 0;
        }
        let mut touched = false;
        for i in 0..points.len() {
            let d = digits[i * nwin + wi];
            if d > 0 {
                buckets[d as usize].add(&points[i]);
                touched = true;
            } else if d < 0 {
                buckets[(-d) as usize].add(&negs[i]);
                touched = true;
            }
        }
        if !touched {
            continue;
        }
        let mut running = Acc::ID;
        let mut total = Acc::ID;
        for j in (1..=nb).rev() {
            running.add_acc(&buckets[j]);
            total.add_acc(&running);
        }
        acc.add_acc(&total);
    }
    acc
}

/// Canonical affine result of a raw accumulator (bigint2 outputs may be non-canonical).
pub fn acc_to_affine(a: &Acc) -> Affine {
    match a.get() {
        None => Affine::IDENTITY,
        Some(p) => {
            use risc0_crypto::curves::secp256k1::Fq;
            use risc0_crypto::BigInt;
            let x = Fq::reduce_from_bigint(BigInt(p[..8].try_into().unwrap()));
            let y = Fq::reduce_from_bigint(BigInt(p[8..].try_into().unwrap()));
            Affine::new(x, y).expect("raw msm left the curve")
        }
    }
}
