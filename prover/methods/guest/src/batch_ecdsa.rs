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

use alloc::vec::Vec;
use risc0_crypto::curves::secp256k1::{Affine, Fq, Fr};
use risc0_zkvm::guest::env;
use sha2::{Digest, Sha256};

/// Bytes per hint: u32 valid flag (LE), then R.x and R.y big-endian.
pub const HINT_BYTES: usize = 68;

static mut COLLECT: bool = false;
static mut HINTS: Vec<u8> = Vec::new();
static mut NEXT: usize = 0;
static mut FALLBACK: u32 = 0;
static mut DEFERRED: Vec<([u8; 32], [u8; 32], Pt, Pt)> = Vec::new();

pub fn install(collect: bool, hints: Vec<u8>) {
    unsafe {
        COLLECT = collect;
        HINTS = hints;
        NEXT = 0;
        FALLBACK = 0;
        DEFERRED = Vec::new();
    }
}

fn be_to_limbs(b: &[u8]) -> [u32; 8] {
    let mut l = [0u32; 8];
    for k in 0..8 {
        l[k] = u32::from_be_bytes(b[28 - 4 * k..32 - 4 * k].try_into().unwrap());
    }
    l
}

fn hex(b: &[u8]) -> alloc::string::String {
    b.iter().map(|x| alloc::format!("{x:02x}")).collect()
}

/// # Safety
/// All pointers valid for 32 bytes. Called only from the hook in ecdsa_impl.h.
#[no_mangle]
pub unsafe extern "C" fn hazync_batch_ecdsa(
    u1: *const u8, u2: *const u8, qx: *const u8, qy: *const u8, r: *const u8,
) -> i32 {
    let s = |p: *const u8| -> [u8; 32] { core::slice::from_raw_parts(p, 32).try_into().unwrap() };
    let (u1, u2, qx, qy, r) = (s(u1), s(u2), s(qx), s(qy), s(r));
    if COLLECT {
        let mut rec = Vec::with_capacity(160);
        for v in [&u1, &u2, &qx, &qy, &r] {
            rec.extend_from_slice(v);
        }
        env::log(&alloc::format!("BATCHREC {}", hex(&rec)));
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
    let (rx, ry) = (&rec[..32], &rec[32..]);
    // R must be a real curve point with canonical coordinates...
    let (Some(x), Some(y)) = (Fq::from_bigint(risc0_crypto::BigInt(be_to_limbs(rx))),
                              Fq::from_bigint(risc0_crypto::BigInt(be_to_limbs(ry)))) else {
        FALLBACK += 1;
        return 0;
    };
    if Affine::new(x, y).is_none() {
        FALLBACK += 1;
        return 0;
    }
    // ...whose x, reduced mod n, is the signature's r. This is ECDSA's acceptance test.
    if Fr::from_be_bytes_mod_order(rx) != Fr::from_be_bytes_mod_order(&r) {
        FALLBACK += 1;
        return 0;
    }
    let mut q: Pt = [0; 16];
    q[..8].copy_from_slice(&be_to_limbs(&qx));
    q[8..].copy_from_slice(&be_to_limbs(&qy));
    let mut rp: Pt = [0; 16];
    rp[..8].copy_from_slice(&be_to_limbs(rx));
    rp[8..].copy_from_slice(&be_to_limbs(ry));
    (*core::ptr::addr_of_mut!(DEFERRED)).push((u1, u2, q, rp));
    1
}

/// Check every deferred signature at once. Returns false if the batch does not vanish.
pub fn finish() -> bool {
    let t0 = env::cycle_count();
    let (collect, fallback, deferred) = unsafe {
        (COLLECT, FALLBACK, core::mem::take(&mut *core::ptr::addr_of_mut!(DEFERRED)))
    };
    if collect {
        env::log("batch: collect mode, every verify ran in libsecp");
        return true;
    }
    let n = deferred.len();
    let mut tr = Sha256::new();
    tr.update(b"libzync/batch-ecdsa/v0");
    for (u1, u2, q, r) in &deferred {
        tr.update(u1);
        tr.update(u2);
        for w in q.iter().chain(r.iter()) {
            tr.update(w.to_le_bytes());
        }
    }
    let seed: [u8; 32] = tr.finalize().into();
    let mut pts: Vec<Pt> = Vec::with_capacity(2 * n + 1);
    let mut scs: Vec<Fr> = Vec::with_capacity(2 * n + 1);
    let mut g = Fr::ZERO;
    for (i, (u1, u2, q, r)) in deferred.iter().enumerate() {
        let zh: [u8; 32] = Sha256::digest([&seed[..], &(i as u32).to_le_bytes()].concat()).into();
        let mut zb = [0u8; 32];
        zb[16..].copy_from_slice(&zh[..16]);
        let z = Fr::from_be_bytes_mod_order(&zb);
        g = &g + &(&z * &Fr::from_be_bytes_mod_order(u1));
        pts.push(*q);
        scs.push(&z * &Fr::from_be_bytes_mod_order(u2));
        pts.push(neg_pt(r));
        scs.push(z);
    }
    let mut gp: Pt = [0; 16];
    {
        let (gx, gy) = Affine::GENERATOR.xy().unwrap();
        gp[..8].copy_from_slice(&gx.as_bigint().0);
        gp[8..].copy_from_slice(&gy.as_bigint().0);
    }
    pts.push(gp);
    scs.push(g);
    let window = match n { 0..=600 => 8, 601..=3000 => 10, 3001..=10000 => 11, 10001..=30000 => 12, _ => 13 };
    let ok = n == 0 || msm_raw(&pts, &scs, window).get().is_none();
    env::log(&alloc::format!(
        "batch: deferred={n} fallback={fallback} window={window} ok={ok} finish_cycles={}",
        env::cycle_count() - t0));
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

