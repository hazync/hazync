//! Hazync's C++ consensus path on OpenVM: for every non-coinbase transaction of a block, Hazync's verify_inputs_batch
//! (Core's VerifyScript + interpreter + sighash over every input). Input 1: core blob (mkcore.py). Input 2 (batch):
//! hints, or empty = COLLECT mode (prints BATCHREC lines for the host; libsecp verifies everything itself).
extern crate alloc;
use alloc::vec::Vec;
openvm::init!();
extern "C" {
    /// Hazync's product entry point (chunk guest): one parse + Core's VerifyScript over the listed inputs.
    fn verify_inputs_batch_once(tx: *const u8, txlen: u32, prev: *const u8, plen: u32, flags: u32, n: u32, idx: *const u32,
        ch: *const u32, cb: *const u32, mtp: *const u32, out: *mut i32, leaves: *mut u8,
        seqs: *mut u32, version: *mut i32, wtxid: *mut u8, has_wit: *mut u32) -> i32;
    fn zkvm_sha256_impl(state: *const u8, input: *const u8, output: *mut u8);
    fn __libc_init_array();
    fn hz_cache_reset();
}
/// RISC0's sys_sha_compress contract, which Hazync's Core patches call: state words byte-swapped from native, the
/// 64-byte block given as two 32-byte halves. Implemented on OpenVM's SHA-256 compression (native LE words).
#[no_mangle]
pub unsafe extern "C" fn sys_sha_compress(out: *mut u32, inp: *const u32, b1: *const u32, b2: *const u32) {
    let mut st = [0u32; 8]; for i in 0..8 { st[i] = (*inp.add(i)).swap_bytes(); }
    let mut blk = [0u8; 64];
    core::ptr::copy_nonoverlapping(b1 as *const u8, blk.as_mut_ptr(), 32);
    core::ptr::copy_nonoverlapping(b2 as *const u8, blk.as_mut_ptr().add(32), 32);
    let mut o = [0u32; 8];
    zkvm_sha256_impl(st.as_ptr() as *const u8, blk.as_ptr(), o.as_mut_ptr() as *mut u8);
    for i in 0..8 { *out.add(i) = o[i].swap_bytes(); }
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
    pub static mut COLLECT: bool = false;
    static mut NEXT: usize = 0;
    pub static mut FALLBACK: u32 = 0;
    static mut DEFERRED: Vec<Deferred> = Vec::new();

    pub fn install(h: Vec<u8>) { unsafe { COLLECT = h.is_empty(); DEFERRED = Vec::with_capacity(h.len() / HINT_BYTES); HINTS = h; } }
    fn le(l: &[u32; 8]) -> [u8; 32] { let mut b = [0u8; 32]; for k in 0..8 { b[4*k..4*k+4].copy_from_slice(&l[k].to_le_bytes()); } b }
    fn be_to_le(b: &[u8]) -> [u8; 32] { let mut o = [0u8; 32]; for k in 0..32 { o[k] = b[31 - k]; } o }

    /// The scheme's acceptance test on R (Hazync batch_ecdsa.rs): ECDSA x(R) mod n == r; BIP340 x(R) == r_x, y even;
    /// BIP341 tweak (kind 3/4) x(R) == output key x, y parity == kind - 3.
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
        if kind == 2 {
            // (e, s) -> u1 = e/s, u2 = r/s on the modular accelerator (libsecp already rejected r, s == 0)
            let s = b.clone();
            a = a.div_unsafe(&s);
            b = rr_s.clone().div_unsafe(&s);
            kind = 0;
        }
        if COLLECT {
            let hx = |b: &[u8]| -> alloc::string::String { b.iter().rev().map(|x| alloc::format!("{x:02x}")).collect() };
            openvm::io::println(alloc::format!("BATCHREC {} {} {} {} {} {}", kind, hx(a.as_le_bytes()), hx(b.as_le_bytes()), hx(&le(&qxl)), hx(&le(&qyl)), hx(&le(&rl))));
            return 0;
        }
        let i = NEXT; NEXT += 1;
        let h = &*core::ptr::addr_of!(HINTS);
        let (Some(qxf0), Some(qyf0)) = (Fq::from_le_bytes(&le(&qxl)), Fq::from_le_bytes(&le(&qyl))) else { FALLBACK += 1; return 0; };
        if (i + 1) * HINT_BYTES > h.len() || u32::from_le_bytes(h[i*HINT_BYTES..i*HINT_BYTES+4].try_into().unwrap()) != 1 {
            // No valid hint (e.g. CHECKMULTISIG's expected mismatches): check THIS one directly, R = u1*G + u2*Q.
            FALLBACK += 1;
            let q = Pt::from_xy_unchecked(qxf0, qyf0);
            for v in [&a, &b] { v.assert_reduced(); }
            let rp = <k256::Secp256k1 as IntrinsicCurve>::msm(&[a.clone(), b.clone()], &[<Pt as CyclicGroup>::GENERATOR, q]);
            if rp == <Pt as Group>::IDENTITY { return 2; }
            let (x, y) = (rp.x().clone(), rp.y().clone());
            return if accept_r(kind, x.as_le_bytes(), y.as_le_bytes(), &rr_s, &le(&rl)) { 1 } else { 2 };
        }
        let rec = &h[i*HINT_BYTES+4..(i+1)*HINT_BYTES];
        let (Some(x), Some(y)) = (Fq::from_le_bytes(&be_to_le(&rec[..32])), Fq::from_le_bytes(&be_to_le(&rec[32..]))) else { FALLBACK += 1; return 0; };
        let Some(rp) = Pt::from_xy(x.clone(), y.clone()) else { FALLBACK += 1; return 0; };   // on the curve
        let xb = x.as_le_bytes(); let yb = y.as_le_bytes();
        if !accept_r(kind, xb, yb, &rr_s, &le(&rl)) { FALLBACK += 1; return 0; }
        let q = Pt::from_xy_unchecked(qxf0, qyf0);   // libsecp parsed and validated Q
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

fn rd(b: &[u8], o: &mut usize) -> usize { let v = u32::from_le_bytes(b[*o..*o+4].try_into().unwrap()) as usize; *o += 4; v }
fn main() {
    // C++ static constructors (Core's global tagged-hash midstates, e.g. HASHER_TAPSIGHASH), as Hazync's guest does.
    unsafe { __libc_init_array() };
    let input: Vec<u8> = openvm::io::read_vec();
    #[cfg(feature = "batch")]
    batch::install(openvm::io::read_vec());
    let mut o = 0usize;
    let flags = rd(&input, &mut o) as u32;
    let ntx = rd(&input, &mut o);
    let (mut inputs, mut bad) = (0u32, 0u32);
    let mut res: Vec<i32> = Vec::new(); let mut idx: Vec<u32> = Vec::new(); let mut z: Vec<u32> = Vec::new();
    for t in 0..ntx {
        let tl = rd(&input, &mut o); let tx = &input[o..o+tl]; o += tl;
        let pl = rd(&input, &mut o); let pv = &input[o..o+pl]; o += pl;
        let n = rd(&input, &mut o);
        res.clear(); res.resize(n, 0); idx.clear(); idx.extend(0..n as u32); z.clear(); z.resize(n, 0);
        let (mut seqs, mut ver, mut wt, mut hw) = (alloc::vec![0u32; n], 0i32, [0u8; 32], 0u32);
        let rc = unsafe { verify_inputs_batch_once(tx.as_ptr(), tl as u32, pv.as_ptr(), pl as u32, flags, n as u32, idx.as_ptr(),
            z.as_ptr(), z.as_ptr(), z.as_ptr(), res.as_mut_ptr(), core::ptr::null_mut(), seqs.as_mut_ptr(), &mut ver, wt.as_mut_ptr(), &mut hw) };
        assert_eq!(rc, 1, "tx {t}");
        unsafe { hz_cache_reset() };   // Hazync resets per chunk; per transaction here (each tx is parsed once either way)
        for (k, r) in res.iter().enumerate() { if *r != 1 { bad += 1; openvm::io::println(alloc::format!("REJECT tx {t} input {k} code {r}")); } }
        inputs += n as u32;
    }
    assert_eq!(bad, 0, "inputs rejected");
    let mut out = [0u8; 32];
    #[cfg(feature = "batch")]
    {
        let (ok, n, distinct) = batch::finish();
        assert!(ok, "batch equation failed");
        out[8..12].copy_from_slice(&(n as u32).to_le_bytes());
        out[12..16].copy_from_slice(&(distinct as u32).to_le_bytes());
        out[16..20].copy_from_slice(&unsafe { batch::FALLBACK }.to_le_bytes());
    }
    out[..4].copy_from_slice(&inputs.to_le_bytes());
    out[4..8].copy_from_slice(&(ntx as u32).to_le_bytes());
    openvm::io::reveal_bytes32(out);
}
