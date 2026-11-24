//! OpenVM replacement for prover/methods/guest/src/batch_ecdsa.rs: SAME public items (HINT_BYTES, install, finish, Pt,
//! Acc, neg_pt), SAME hint layout (u32 valid, R.x BE, R.y BE), SAME transcript ("libzync/batch-sig/v2", count, every
//! tuple, every kind), SAME acceptance tests and batch equation Σ z·u2·Q − Σ z·R + (Σ z·u1)·G == O, keys merged.
//! Group arithmetic on OpenVM's secp256k1 extension instead of risc0 bigint2 circuits. A call with no valid hint is
//! checked DIRECTLY (u1·G + u2·Q) and returns 1 (valid) / 2 (invalid) — the libsecp hook (bench/ovm-secp/apply_hook.py)
//! maps 2 to libsecp's own "invalid".
use alloc::vec::Vec;
use k256::{Secp256k1Coord as Fq, Secp256k1Point as P, Secp256k1Scalar as Fr};
use openvm_algebra_guest::{DivUnsafe, IntMod, Reduce};
use openvm_ecc_guest::{weierstrass::WeierstrassPoint, CyclicGroup, Group};

#[path = "msm_ovm.rs"]
mod msm;
use risc0_zkvm::guest::env;
use sha2::{Digest, Sha256};

pub const HINT_BYTES: usize = 68;
/// x then y, 8 little-endian limbs each (libsecp's layout), as Hazync's Pt.
pub type Pt = [u32; 16];

struct Deferred { u1: Fr, u2: Fr, q: P, r: P, raw: [u8; 160], kind: u32 }
static mut COLLECT: bool = false;
static mut HINTS: Vec<u8> = Vec::new();
static mut NEXT: usize = 0;
static mut FALLBACK: u32 = 0;
static mut DEFERRED: Vec<Deferred> = Vec::new();

pub fn install(collect: bool, hints: Vec<u8>) {
    unsafe { DEFERRED = Vec::with_capacity(hints.len() / HINT_BYTES); COLLECT = collect; HINTS = hints; NEXT = 0; FALLBACK = 0; }
}
fn le(l: &[u32; 8]) -> [u8; 32] { let mut b = [0u8; 32]; for k in 0..8 { b[4*k..4*k+4].copy_from_slice(&l[k].to_le_bytes()); } b }
fn be_to_le(b: &[u8]) -> [u8; 32] { let mut o = [0u8; 32]; for k in 0..32 { o[k] = b[31 - k]; } o }
fn limbs(b: &[u8]) -> [u32; 8] { let mut l = [0u32; 8]; for k in 0..8 { l[k] = u32::from_le_bytes(b[4*k..4*k+4].try_into().unwrap()); } l }
fn accept_r(kind: i32, xb: &[u8], yb: &[u8], r_s: &Fr, r_le: &[u8; 32]) -> bool {
    match kind {
        0 => Fr::reduce_le_bytes(xb) == *r_s,
        1 => xb == &r_le[..] && yb[0] & 1 == 0,
        3 | 4 => xb == &r_le[..] && (yb[0] & 1) as i32 == kind - 3,
        _ => false,
    }
}

#[no_mangle]
pub unsafe extern "C" fn hazync_batch_defer(kind: i32, u1: *const u32, u2: *const u32, qx: *const u32, qy: *const u32, r: *const u32) -> i32 {
    let rd = |p: *const u32| -> [u32; 8] { core::slice::from_raw_parts(p, 8).try_into().unwrap() };
    let (u1l, u2l, qxl, qyl, rl) = (rd(u1), rd(u2), rd(qx), rd(qy), rd(r));
    let (mut a, mut b) = (Fr::from_le_bytes_unchecked(&le(&u1l)), Fr::from_le_bytes_unchecked(&le(&u2l)));
    let rr_s = Fr::from_le_bytes_unchecked(&le(&rl));
    let mut kind = kind;
    if kind == 2 { let s = b.clone(); a = a.div_unsafe(&s); b = rr_s.clone().div_unsafe(&s); kind = 0; }
    if COLLECT {
        let hx = |b: &[u8]| -> alloc::string::String { b.iter().rev().map(|x| alloc::format!("{x:02x}")).collect() };
        env::log(&alloc::format!("BATCHREC{} {}{}{}{}{}", kind, hx(a.as_le_bytes()), hx(b.as_le_bytes()), hx(&le(&qxl)), hx(&le(&qyl)), hx(&le(&rl))));
        return 0;
    }
    let i = NEXT; NEXT += 1;
    let h = &*core::ptr::addr_of!(HINTS);
    let (Some(qxf), Some(qyf)) = (Fq::from_le_bytes(&le(&qxl)), Fq::from_le_bytes(&le(&qyl))) else { FALLBACK += 1; return 0; };
    let q = unsafe { P::from_xy_unchecked(qxf, qyf) };   // libsecp parsed and validated Q
    if (i + 1) * HINT_BYTES > h.len() || u32::from_le_bytes(h[i*HINT_BYTES..i*HINT_BYTES+4].try_into().unwrap()) != 1 {
        FALLBACK += 1;
        for v in [&a, &b] { v.assert_reduced(); }
        let rp = msm::msm(&[a.clone(), b.clone()], &[<P as CyclicGroup>::GENERATOR, q]);
        if rp == <P as Group>::IDENTITY { return 2; }
        return if accept_r(kind, rp.x().as_le_bytes(), rp.y().as_le_bytes(), &rr_s, &le(&rl)) { 1 } else { 2 };
    }
    let rec = &h[i*HINT_BYTES+4..(i+1)*HINT_BYTES];
    let (Some(x), Some(y)) = (Fq::from_le_bytes(&be_to_le(&rec[..32])), Fq::from_le_bytes(&be_to_le(&rec[32..]))) else { FALLBACK += 1; return 0; };
    let Some(rp) = P::from_xy(x.clone(), y.clone()) else { FALLBACK += 1; return 0; };
    if !accept_r(kind, x.as_le_bytes(), y.as_le_bytes(), &rr_s, &le(&rl)) { FALLBACK += 1; return 0; }
    let mut raw = [0u8; 160];
    raw[..32].copy_from_slice(a.as_le_bytes()); raw[32..64].copy_from_slice(b.as_le_bytes());
    raw[64..96].copy_from_slice(&le(&qxl)); raw[96..128].copy_from_slice(&le(&qyl)); raw[128..].copy_from_slice(x.as_le_bytes());
    (*core::ptr::addr_of_mut!(DEFERRED)).push(Deferred { u1: a, u2: b, q, r: rp, raw, kind: kind as u32 });
    1
}

pub fn finish() -> bool {
    let (collect, d) = unsafe { (COLLECT, core::mem::take(&mut *core::ptr::addr_of_mut!(DEFERRED))) };
    if collect { env::log("batch: collect mode, every verify ran in libsecp"); return true; }
    let n = d.len();
    let mut h = Sha256::new();
    h.update(b"libzync/batch-sig/v2"); h.update((n as u32).to_le_bytes());
    for x in &d { h.update(&x.raw[..]); } for x in &d { h.update(x.kind.to_le_bytes()); }
    let seed = h.finalize();
    // z_i from the transcript, in tuple order (unchanged); keys merged by their raw x||y as before, through a sorted
    // index instead of a BTreeMap (the sum is the same in any order; the map moved 160-byte entries on every insert).
    let mut pts: Vec<P> = Vec::with_capacity(n + n / 2 + 1);
    let mut scs: Vec<Fr> = Vec::with_capacity(n + n / 2 + 1);
    let mut zu2: Vec<Fr> = Vec::with_capacity(n);
    let mut g = Fr::ZERO;
    let mut zh = [0u8; 32];
    for (i, x) in d.iter().enumerate() {
        if i % 2 == 0 { let mut m = [0u8; 36]; m[..32].copy_from_slice(&seed); m[32..].copy_from_slice(&((i / 2) as u32).to_le_bytes()); zh = Sha256::digest(m); }
        let mut zb = [0u8; 32];
        zb[..16].copy_from_slice(if i % 2 == 0 { &zh[..16] } else { &zh[16..] });
        let z = Fr::from_le_bytes_unchecked(&zb);
        g = g + &z * &x.u1;
        zu2.push(&z * &x.u2);
        pts.push(-&x.r); scs.push(z);
    }
    fn key(d: &[Deferred], i: u32) -> &[u8] { &d[i as usize].raw[64..128] }
    let mut idx: Vec<u32> = (0..n as u32).collect();
    idx.sort_unstable_by(|&a, &b| key(&d, a).cmp(key(&d, b)));
    let mut k = 0;
    while k < n {
        let (first, mut s) = (idx[k], zu2[idx[k] as usize].clone());
        k += 1;
        while k < n && key(&d, idx[k]) == key(&d, first) { s = &s + &zu2[idx[k] as usize]; k += 1; }
        pts.push(d[first as usize].q.clone()); scs.push(s);
    }
    pts.push(<P as CyclicGroup>::GENERATOR); scs.push(g);
    for s in &scs { s.assert_reduced(); }
    let fb = unsafe { FALLBACK };
    let ok = n == 0 || msm::msm(&scs, &pts) == <P as Group>::IDENTITY;
    env::log(&alloc::format!("batch: deferred={n} fallback={fb} ok={ok}"));
    ok
}

// ---- the point accumulator the live-coin hash (swiftsync.rs) uses ----
fn to_p(p: &Pt) -> P {
    let mut x = [0u32; 8]; let mut y = [0u32; 8]; x.copy_from_slice(&p[..8]); y.copy_from_slice(&p[8..]);
    unsafe { P::from_xy_unchecked(Fq::from_le_bytes(&le(&x)).expect("Pt x"), Fq::from_le_bytes(&le(&y)).expect("Pt y")) }
}
fn from_p(p: &P) -> Pt {
    let mut o = [0u32; 16]; o[..8].copy_from_slice(&limbs(p.x().as_le_bytes())); o[8..].copy_from_slice(&limbs(p.y().as_le_bytes())); o
}
pub fn neg_pt(p: &Pt) -> Pt { from_p(&-to_p(p)) }
/// Running sum of points; `get` is None at the identity.
pub struct Acc { p: Option<P>, cache: Pt }
impl Acc {
    pub const ID: Acc = Acc { p: None, cache: [0; 16] };
    pub fn get(&self) -> Option<&Pt> { self.p.as_ref().map(|_| &self.cache) }
    pub fn add(&mut self, q: &Pt) {
        let qp = to_p(q);
        let s = match self.p.take() { None => qp, Some(a) => a + qp };
        if s == <P as Group>::IDENTITY { self.p = None; self.cache = [0; 16]; } else { self.cache = from_p(&s); self.p = Some(s); }
    }
}
