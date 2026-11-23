//! Core's libsecp256k1 v0.5.1 on OpenVM. Input 1: [mode | block blob from extract]. Input 2 (feature batch): hints.
//!   mode 1 = every ECDSA, mode 2 = every BIP340, mode 5 = both (one batch). Panics unless every check passes.
//! Without `batch`: stock libsecp verifies each signature in software (rv32im, 10x26 field).
//! With `batch`: libsecp parses + range-checks, the hook defers (Hazync's design, batch_ecdsa.rs), and one
//! weighted MSM on OpenVM's secp256k1 accelerator checks every group equation at once.
use sha2::Digest;
openvm::init!();

extern "C" {
    fn hz_ecdsa(msg32: *const u8, pub33: *const u8, sig64: *const u8) -> i32;
    fn hz_schnorr(msg32: *const u8, x32: *const u8, sig64: *const u8) -> i32;
}
/// Square root mod p on OpenVM's modular extension (hinted + checked). 1 = square (out set), 0 = not a square.
#[no_mangle]
pub unsafe extern "C" fn hz_fe_sqrt(a_be: *const u8, out_be: *mut u8) -> i32 {
    use openvm_algebra_guest::{IntMod, Sqrt};
    let a = core::slice::from_raw_parts(a_be, 32);
    let Some(x) = k256::Secp256k1Coord::from_be_bytes(a) else { return 0 };
    match x.sqrt() { Some(y) => { core::slice::from_raw_parts_mut(out_be, 32).copy_from_slice(&y.to_be_bytes()); 1 } None => 0 }
}
#[no_mangle]
pub extern "C" fn hz_fatal(code: i32) { panic!("libsecp fatal callback {code}"); }

#[cfg(feature = "batch")]
mod batch {
    use alloc::collections::BTreeMap;
    use alloc::vec::Vec;
    use k256::{Secp256k1Coord as Fq, Secp256k1Point as Pt, Secp256k1Scalar as Fr};
    use openvm_algebra_guest::{DivUnsafe, IntMod, Reduce};
    use openvm_ecc_guest::{weierstrass::{IntrinsicCurve, WeierstrassPoint}, CyclicGroup, Group};
    use sha2::{Digest, Sha256};
    extern crate alloc;

    pub const HINT_BYTES: usize = 68;
    struct Deferred { u1: Fr, u2: Fr, q: Pt, r: Pt, raw: [u8; 160], kind: u32 }
    static mut HINTS: Vec<u8> = Vec::new();
    static mut NEXT: usize = 0;
    pub static mut FALLBACK: u32 = 0;
    static mut DEFERRED: Vec<Deferred> = Vec::new();

    pub fn install(h: Vec<u8>) { unsafe { DEFERRED = Vec::with_capacity(h.len() / HINT_BYTES); HINTS = h; } }
    fn le(l: &[u32; 8]) -> [u8; 32] { let mut b = [0u8; 32]; for k in 0..8 { b[4*k..4*k+4].copy_from_slice(&l[k].to_le_bytes()); } b }
    fn be_to_le(b: &[u8]) -> [u8; 32] { let mut o = [0u8; 32]; for k in 0..32 { o[k] = b[31 - k]; } o }

    #[no_mangle]
    pub unsafe extern "C" fn hazync_batch_defer(kind: i32, u1: *const u32, u2: *const u32, qx: *const u32, qy: *const u32, r: *const u32) -> i32 {
        let rd = |p: *const u32| -> [u32; 8] { core::slice::from_raw_parts(p, 8).try_into().unwrap() };
        let (u1l, u2l, qxl, qyl, rl) = (rd(u1), rd(u2), rd(qx), rd(qy), rd(r));
        let (mut a, mut b) = (Fr::from_le_bytes_unchecked(&le(&u1l)), Fr::from_le_bytes_unchecked(&le(&u2l)));
        let rr_s = Fr::from_le_bytes_unchecked(&le(&rl));
        let mut kind = kind;
        if kind == 2 {
            // (e, s) -> u1 = e/s, u2 = r/s on the modular accelerator (libsecp already rejected r, s == 0)
            let s = b.clone();
            a = a.div_unsafe(&s);
            b = rr_s.clone().div_unsafe(&s);
            kind = 0;
        }
        let i = NEXT; NEXT += 1;
        let h = &*core::ptr::addr_of!(HINTS);
        if (i + 1) * HINT_BYTES > h.len() || u32::from_le_bytes(h[i*HINT_BYTES..i*HINT_BYTES+4].try_into().unwrap()) != 1 {
            FALLBACK += 1; return 0;
        }
        let rec = &h[i*HINT_BYTES+4..(i+1)*HINT_BYTES];
        let (Some(x), Some(y)) = (Fq::from_le_bytes(&be_to_le(&rec[..32])), Fq::from_le_bytes(&be_to_le(&rec[32..]))) else { FALLBACK += 1; return 0; };
        let Some(rp) = Pt::from_xy(x.clone(), y.clone()) else { FALLBACK += 1; return 0; };   // on the curve
        let xb = x.as_le_bytes(); let yb = y.as_le_bytes();
        let accept = match kind {
            0 => Fr::reduce_le_bytes(xb) == rr_s,                  // ECDSA: x(R) mod n == r
            1 => xb == &le(&rl)[..] && yb[0] & 1 == 0,             // BIP340: x(R) == r_x, y(R) even
            _ => false,
        };
        if !accept { FALLBACK += 1; return 0; }
        let (Some(qxf), Some(qyf)) = (Fq::from_le_bytes(&le(&qxl)), Fq::from_le_bytes(&le(&qyl))) else { FALLBACK += 1; return 0; };
        let q = Pt::from_xy_unchecked(qxf, qyf);   // libsecp parsed and validated Q
        let mut raw = [0u8; 160];
        raw[..32].copy_from_slice(a.as_le_bytes()); raw[32..64].copy_from_slice(b.as_le_bytes());
        raw[64..96].copy_from_slice(&le(&qxl)); raw[96..128].copy_from_slice(&le(&qyl)); raw[128..].copy_from_slice(xb);
        (*core::ptr::addr_of_mut!(DEFERRED)).push(Deferred { u1: a, u2: b, q, r: rp, raw, kind: kind as u32 });
        1
    }

    /// Σ z_i·u2_i·Q_i − Σ z_i·R_i + (Σ z_i·u1_i)·G == O, z_i 128-bit from a transcript over every deferred check.
    pub fn finish() -> (bool, usize, usize) {
        let d = unsafe { core::mem::take(&mut *core::ptr::addr_of_mut!(DEFERRED)) };
        let n = d.len();
        let mut h = Sha256::new();
        h.update(b"libzync/batch-sig/v2"); h.update(&(n as u32).to_le_bytes());
        for x in &d { h.update(&x.raw); } for x in &d { h.update(&x.kind.to_le_bytes()); }
        let seed: [u8; 32] = h.finalize().into();
        let mut by_key: BTreeMap<[u8; 64], (Pt, Fr)> = BTreeMap::new();
        let mut pts: Vec<Pt> = Vec::with_capacity(n + n / 2 + 1);
        let mut scs: Vec<Fr> = Vec::with_capacity(n + n / 2 + 1);
        let mut g = Fr::ZERO;
        let mut zh = [0u8; 32];
        for (i, x) in d.into_iter().enumerate() {
            if i % 2 == 0 { zh = Sha256::digest([&seed[..], &((i / 2) as u32).to_le_bytes()].concat()).into(); }
            let mut zb = [0u8; 32];
            zb[..16].copy_from_slice(if i % 2 == 0 { &zh[..16] } else { &zh[16..] });
            let z = Fr::from_le_bytes_unchecked(&zb);
            g = g + &z * &x.u1;
            let zu2 = &z * &x.u2;
            let mut key = [0u8; 64]; key.copy_from_slice(&x.raw[64..128]);
            by_key.entry(key).and_modify(|e| e.1 = &e.1 + &zu2).or_insert((x.q, zu2));
            pts.push(-x.r); scs.push(z);
        }
        let distinct = by_key.len();
        for (_, (q, s)) in by_key { pts.push(q); scs.push(s); }
        pts.push(Pt::GENERATOR); scs.push(g);
        for s in &scs { s.assert_reduced(); }
        let acc = <k256::Secp256k1 as IntrinsicCurve>::msm(&scs, &pts);
        (n == 0 || acc == <Pt as Group>::IDENTITY, n, distinct)
    }
}

fn rd_u32(b: &[u8], o: &mut usize) -> usize { let v = u32::from_le_bytes(b[*o..*o+4].try_into().unwrap()) as usize; *o += 4; v }

fn main() {
    let input: Vec<u8> = openvm::io::read_vec();
    #[cfg(feature = "batch")]
    batch::install(openvm::io::read_vec());
    let mode = input[0];
    let b = &input[1..];
    let mut o = 0usize;
    let n_ec = rd_u32(b, &mut o); let ec_at = o; o += n_ec * 129;
    let n_sc = rd_u32(b, &mut o); let sc_at = o;
    let mut checked = 0u32;
    if mode == 1 || mode == 5 {
        for i in 0..n_ec {
            let r = &b[ec_at + i*129..ec_at + (i+1)*129];
            let rc = unsafe { hz_ecdsa(r.as_ptr(), r[32..].as_ptr(), r[65..].as_ptr()) };
            assert_eq!(rc, 1, "ecdsa {i}"); checked += 1;
        }
    }
    if mode == 2 || mode == 5 {
        for i in 0..n_sc {
            let r = &b[sc_at + i*128..sc_at + (i+1)*128];
            let rc = unsafe { hz_schnorr(r.as_ptr(), r[32..].as_ptr(), r[64..].as_ptr()) };
            assert_eq!(rc, 1, "schnorr {i}"); checked += 1;
        }
    }
    let mut out = [0u8; 32];
    #[cfg(feature = "batch")]
    {
        let (ok, n, distinct) = batch::finish();
        assert!(ok, "batch equation failed");
        out[8..12].copy_from_slice(&(n as u32).to_le_bytes());
        out[12..16].copy_from_slice(&(distinct as u32).to_le_bytes());
        out[16..20].copy_from_slice(&unsafe { batch::FALLBACK }.to_le_bytes());
    }
    out[..4].copy_from_slice(&checked.to_le_bytes());
    out[4] = mode;
    let _ = sha2::Sha256::digest(b"");
    openvm::io::reveal_bytes32(out);
}
