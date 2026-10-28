//! libzync EXPERIMENT — batch ECDSA verification inside the chunk guest.
//!
//! ⛔ Not reviewed, not consensus-safe, moves METHOD_ID. Exists to MEASURE what batching saves on a
//! real block (see experimental/libzync-bench/README.md, which measured the primitive alone).
//!
//! libsecp still parses the signature and key, checks r,s != 0 and computes u1 = e/s, u2 = r/s.
//! The hook in ecdsa_impl.h then calls `hazync_batch_ecdsa`. Per call, the host supplies a hint:
//! a VALID bit and the point R = u1*G + u2*Q.
//!   - bit 0, or no hint: return 0 and libsecp verifies this one itself, exactly as before. This is
//!     how CHECKMULTISIG's expected key/signature mismatches still evaluate to false.
//!   - bit 1: check R is on the curve and x(R) mod n == r (the actual ECDSA acceptance test), then
//!     defer the linear part. `finish` checks every deferred one at once:
//!         Σ z_i·u2_i·Q_i − Σ z_i·R_i + (Σ z_i·u1_i)·G == O
//!     with 128-bit z_i from a hash over every deferred (u1, u2, Q, R). A wrong hint can only make
//!     the batch fail, which marks the chunk invalid; it cannot make a bad signature pass.
//!
//! COLLECT mode (no hints yet): every call is logged as `BATCHREC <hex u1 u2 qx qy r>` and falls
//! through to libsecp; the host turns the log into hints for the measured run.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use risc0_crypto::curves::secp256k1::{Affine, Fq, Fr};
use risc0_crypto::BigInt;
use risc0_zkvm::guest::env;
use sha2::{Digest, Sha256};

/// Bytes per hint: u32 valid flag (LE), then R.x and R.y big-endian.
pub const HINT_BYTES: usize = 68;

/// One deferred check: u1·G + u2·Q == R, all as little-endian 8x32 limbs (libsecp's own layout).
#[derive(Clone, Copy)]
#[repr(C)]
struct Deferred {
    u1: [u32; 8],
    u2: [u32; 8],
    q: Pt,
    r: Pt,
}

static mut COLLECT: bool = false;
static mut HINTS: Vec<u8> = Vec::new();
static mut NEXT: usize = 0;
static mut FALLBACK: u32 = 0;
static mut SCHNORR_N: u32 = 0;
static mut DEFERRED: Vec<Deferred> = Vec::new();

pub fn install(collect: bool, hints: Vec<u8>) {
    unsafe {
        // One Deferred per hint at most: reserve once instead of growing (and copying) as it fills.
        DEFERRED = Vec::with_capacity(hints.len() / HINT_BYTES);
        COLLECT = collect;
        HINTS = hints;
        NEXT = 0;
        FALLBACK = 0;
        SCHNORR_N = 0;
    }
}

fn be_to_limbs(b: &[u8]) -> [u32; 8] {
    let mut l = [0u32; 8];
    for k in 0..8 {
        l[k] = u32::from_be_bytes(b[28 - 4 * k..32 - 4 * k].try_into().unwrap());
    }
    l
}

fn limbs_to_be_hex(l: &[u32; 8]) -> alloc::string::String {
    let mut s = alloc::string::String::with_capacity(64);
    for k in (0..8).rev() {
        s.push_str(&alloc::format!("{:08x}", l[k]));
    }
    s
}

/// kind 0 = ECDSA (`r` is the signature's r, a scalar), kind 1 = BIP340 Schnorr (`r` is r_x, a field
/// element). u1, u2, qx, qy, r are 8-limb little-endian arrays straight from libsecp.
///
/// # Safety
/// All pointers valid for 8 u32. Called only from the hooks apply_batch_hook.py installs.
#[no_mangle]
pub unsafe extern "C" fn hazync_batch_defer(
    kind: i32, u1: *const u32, u2: *const u32, qx: *const u32, qy: *const u32, r: *const u32,
) -> i32 {
    let rd = |p: *const u32| -> [u32; 8] { core::slice::from_raw_parts(p, 8).try_into().unwrap() };
    let (mut u1, mut u2, qx, qy, r) = (rd(u1), rd(u2), rd(qx), rd(qy), rd(r));
    let mut kind = kind;
    if kind == 2 {
        // ECDSA handed over BEFORE libsecp's scalar arithmetic: (u1, u2) arrive as (e, s). Compute
        // w = 1/s, u1 = e*w, u2 = r*w on the coprocessor. libsecp has already rejected r or s == 0,
        // and its scalars are reduced mod n, so from_bigint cannot fail.
        let e = Fr::from_bigint(BigInt(u1)).unwrap();
        let w = Fr::from_bigint(BigInt(u2)).unwrap().inverse();
        let rr = Fr::from_bigint(BigInt(r)).unwrap();
        u1 = (&e * &w).to_bigint().0;
        u2 = (&rr * &w).to_bigint().0;
        kind = 0;
    }
    if COLLECT {
        env::log(&alloc::format!("BATCHREC{} {}{}{}{}{}", kind, limbs_to_be_hex(&u1), limbs_to_be_hex(&u2),
                                 limbs_to_be_hex(&qx), limbs_to_be_hex(&qy), limbs_to_be_hex(&r)));
        return 0;
    }
    let i = NEXT;
    NEXT += 1;
    let h = &*core::ptr::addr_of!(HINTS);
    if (i + 1) * HINT_BYTES > h.len() || h[i * HINT_BYTES] == 0 {
        FALLBACK += 1;
        return 0;
    }
    let rec = &h[i * HINT_BYTES + 4..(i + 1) * HINT_BYTES];
    let (rx, ry) = (be_to_limbs(&rec[..32]), be_to_limbs(&rec[32..]));
    // R must be a canonical curve point: y^2 == x^3 + 7.
    let (Some(x), Some(y)) = (Fq::from_bigint(BigInt(rx)), Fq::from_bigint(BigInt(ry))) else {
        FALLBACK += 1;
        return 0;
    };
    if &y * &y != &(&(&x * &x) * &x) + &Fq::from_u32(7) {
        FALLBACK += 1;
        return 0;
    }
    // The scheme's own acceptance test on R.
    let accept = if kind == 0 {
        // ECDSA: x(R) mod n == r.
        Fr::reduce_from_bigint(BigInt(rx)) == Fr::reduce_from_bigint(BigInt(r))
    } else {
        // BIP340: x(R) == r_x exactly, and y(R) even.
        rx == r && ry[0] & 1 == 0
    };
    if !accept {
        FALLBACK += 1;
        return 0;
    }
    if kind == 1 {
        SCHNORR_N += 1;
    }
    let mut q: Pt = [0; 16];
    q[..8].copy_from_slice(&qx);
    q[8..].copy_from_slice(&qy);
    let mut rp: Pt = [0; 16];
    rp[..8].copy_from_slice(&rx);
    rp[8..].copy_from_slice(&ry);
    (*core::ptr::addr_of_mut!(DEFERRED)).push(Deferred { u1, u2, q, r: rp });
    1
}

/// Check every deferred signature at once. Returns false if the batch does not vanish.
///
///   Σ z_i·u2_i·Q_i − Σ z_i·R_i + (Σ z_i·u1_i)·G == O
///
/// - z_i are 128-bit weights from a hash over EVERY deferred tuple (u1, u2, Q, R), hashed as one
///   contiguous buffer. One SHA-256 output yields two weights.
/// - Signatures sharing a public key Q are MERGED: their z_i·u2_i add into one scalar on one point.
///   That is the same sum regrouped, so it changes the cost and not the equation.
pub fn finish() -> bool {
    let t0 = env::cycle_count();
    let (collect, fallback, schnorr, deferred) = unsafe {
        (COLLECT, FALLBACK, SCHNORR_N, core::mem::take(&mut *core::ptr::addr_of_mut!(DEFERRED)))
    };
    if collect {
        env::log("batch: collect mode, every verify ran in libsecp");
        return true;
    }
    let n = deferred.len();
    let seed: [u8; 32] = {
        // Deferred is repr(C) and made only of u32 arrays, so its bytes are exactly its limbs.
        let bytes = unsafe {
            core::slice::from_raw_parts(deferred.as_ptr() as *const u8, n * core::mem::size_of::<Deferred>())
        };
        let mut h = Sha256::new();
        h.update(b"libzync/batch-sig/v1");
        h.update(bytes);
        h.finalize().into()
    };
    let t1 = env::cycle_count();
    let mut by_key: BTreeMap<Pt, Fr> = BTreeMap::new();
    let mut pts: Vec<Pt> = Vec::with_capacity(n + n / 2 + 1);
    let mut scs: Vec<Fr> = Vec::with_capacity(n + n / 2 + 1);
    let mut g = Fr::ZERO;
    let mut zh = [0u8; 32];
    for (i, d) in deferred.iter().enumerate() {
        if i % 2 == 0 {
            zh = Sha256::digest([&seed[..], &((i / 2) as u32).to_le_bytes()].concat()).into();
        }
        let half = if i % 2 == 0 { &zh[..16] } else { &zh[16..] };
        let mut zl = [0u32; 8];
        for k in 0..4 {
            zl[k] = u32::from_le_bytes(half[4 * k..4 * k + 4].try_into().unwrap());
        }
        let z = Fr::from_bigint(BigInt(zl)).unwrap();
        let u1 = Fr::reduce_from_bigint(BigInt(d.u1));
        let u2 = Fr::reduce_from_bigint(BigInt(d.u2));
        g = &g + &(&z * &u1);
        let zu2 = &z * &u2;
        by_key.entry(d.q).and_modify(|s| *s = &*s + &zu2).or_insert(zu2);
        pts.push(neg_pt(&d.r));
        scs.push(z);
    }
    let distinct = by_key.len();
    for (q, s) in by_key {
        pts.push(q);
        scs.push(s);
    }
    let mut gp: Pt = [0; 16];
    {
        let (gx, gy) = Affine::GENERATOR.xy().unwrap();
        gp[..8].copy_from_slice(&gx.as_bigint().0);
        gp[8..].copy_from_slice(&gy.as_bigint().0);
    }
    pts.push(gp);
    scs.push(g);
    let t2 = env::cycle_count();
    let m = pts.len();
    let window = match m { 0..=1200 => 8, 1201..=6000 => 10, 6001..=20000 => 11, 20001..=60000 => 12, _ => 13 };
    let ok = n == 0 || msm_raw(&pts, &scs, window).get().is_none();
    env::log(&alloc::format!(
        "batch: deferred={n} (schnorr={schnorr}) distinct_keys={distinct} fallback={fallback} window={window} ok={ok} \
         transcript={} prep={} msm={} finish_cycles={}",
        t1 - t0, t2 - t1, env::cycle_count() - t2, env::cycle_count() - t0));
    ok
}

static mut ADDS: u64 = 0;
static mut DBLS: u64 = 0;

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

