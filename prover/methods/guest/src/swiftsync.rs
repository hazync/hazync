//! libzync EXPERIMENT — SwiftSync-style coin accounting in the aggregate, to MEASURE its cost.
//!
//! ⛔ Not sound on its own and not a design: it replaces the Utreexo inclusion checks and updates
//! with an ECMH running total, and SwiftSync's guarantee only exists once that total is checked
//! against the UTXO set at a handover height. This module does the per-block work only.
//!
//! Per block: every created coin is ADDED and every external coin spent is SUBTRACTED, one ECMH
//! operation each. In-block create-then-spend still cancels exactly as before. No inclusion proofs
//! are read or checked; the host strips them from the witness.
//!
//! ECMH point for a 32-byte coin leaf: try-and-increment, x = SHA-256(leaf || ctr). The host hints
//! the first ctr whose x is on the curve, its EVEN y, and for every smaller ctr a witness w with
//! w^2 == -(x^3 + 7). Because p = 3 mod 4, -1 is a non-residue, so that witness proves the skipped
//! x was NOT on the curve. x is fixed by the hash and y by its parity, so the map has exactly one
//! output per leaf, and the guest checks all of it.
//!
//! COLLECT mode adds nothing to the total, so a collect-mode aggregate must never commit a chain
//! state: `main.rs::aggregate` stops before its commit (review 2026-10-30).
//!
//! COLLECT mode (no hints yet): every leaf is logged as `ECMHREC <hex>`, in operation order.

use crate::batch_ecdsa::{neg_pt, Acc, Pt};
use alloc::vec::Vec;
use risc0_crypto::curves::secp256k1::Fq;
use risc0_crypto::BigInt;
use risc0_zkvm::guest::env;
use sha2::{Digest, Sha256};

static mut COLLECT: bool = false;
static mut HINTS: Vec<u8> = Vec::new();
static mut CUR: usize = 0;
static mut ACC: Acc = Acc::ID;
static mut ADDS: u32 = 0;
static mut SUBS: u32 = 0;

pub fn install(collect: bool, hints: Vec<u8>) {
    unsafe {
        COLLECT = collect;
        HINTS = hints;
        CUR = 0;
        ACC = Acc::ID;
        ADDS = 0;
        SUBS = 0;
    }
}

fn be_to_limbs(b: &[u8]) -> [u32; 8] {
    let mut l = [0u32; 8];
    for k in 0..8 {
        l[k] = u32::from_be_bytes(b[28 - 4 * k..32 - 4 * k].try_into().unwrap());
    }
    l
}

fn x_for(leaf: &[u8; 32], ctr: u32) -> Fq {
    let mut m = [0u8; 36];
    m[..32].copy_from_slice(leaf);
    m[32..].copy_from_slice(&ctr.to_le_bytes());
    let h: [u8; 32] = Sha256::digest(m).into();
    // x >= p has probability ~2^-224; refusing it keeps the map single-valued.
    Fq::from_bigint(BigInt(be_to_limbs(&h))).expect("ECMH: hash >= p")
}

/// The leaf's ECMH point, every step checked. None in collect mode.
fn point(leaf: &[u8; 32]) -> Option<Pt> {
    unsafe {
        if COLLECT {
            let hex: alloc::string::String = leaf.iter().map(|b| alloc::format!("{b:02x}")).collect();
            env::log(&alloc::format!("ECMHREC {hex}"));
            return None;
        }
        let h = &*core::ptr::addr_of!(HINTS);
        let mut c = CUR;
        let ctr = u32::from_le_bytes(h[c..c + 4].try_into().expect("ECMH hints exhausted"));
        let y = Fq::from_bigint(BigInt(be_to_limbs(&h[c + 4..c + 36]))).expect("ECMH: y >= p");
        c += 36;
        let seven = Fq::from_u32(7);
        for k in 0..ctr {
            let x = x_for(leaf, k);
            let w = Fq::from_bigint(BigInt(be_to_limbs(&h[c..c + 32]))).expect("ECMH: w >= p");
            c += 32;
            let rhs = &(&(&x * &x) * &x) + &seven;
            assert!(&w * &w == -&rhs, "ECMH: bad non-residue witness");
        }
        CUR = c;
        let x = x_for(leaf, ctr);
        assert!(&y * &y == &(&(&x * &x) * &x) + &seven, "ECMH: hinted y is not on the curve");
        // ONE point per leaf: the root with even y. Both y and p - y satisfy the curve equation, so
        // without this a prover picks +P or -P per operation and can cancel a fake or repeated spend
        // (review 2026-10-30, found independently by two reviewers).
        assert!(y.as_bigint().0[0] & 1 == 0, "ECMH: hinted y must be the even root");
        let mut p: Pt = [0; 16];
        p[..8].copy_from_slice(&x.as_bigint().0);
        p[8..].copy_from_slice(&y.as_bigint().0);
        Some(p)
    }
}

/// A coin enters the set.
pub fn add(leaf: &[u8; 32]) {
    if let Some(p) = point(leaf) {
        unsafe { (*core::ptr::addr_of_mut!(ACC)).add(&p) };
    }
    unsafe { ADDS += 1 };
}

/// A coin leaves the set.
pub fn sub(leaf: &[u8; 32]) {
    if let Some(p) = point(leaf) {
        unsafe { (*core::ptr::addr_of_mut!(ACC)).add(&neg_pt(&p)) };
    }
    unsafe { SUBS += 1 };
}

/// Design B (HANDOVER_DESIGN.md): this block's total starts from the previous chain state's, which the
/// aggregate verified recursively. Call once, after `install`. `set == 0` means the identity.
pub fn start(set: u32, p: &Pt) {
    if set != 0 {
        unsafe { (*core::ptr::addr_of_mut!(ACC)).add(p) };
    }
}

/// The running total to commit: (0, zeros) for the identity, else (1, point).
pub fn total() -> (u32, Pt) {
    match unsafe { (*core::ptr::addr_of!(ACC)).get() } {
        Some(p) => (1, *p),
        None => (0, [0; 16]),
    }
}

/// True in collect mode (no ECMH hints): the total is NOT maintained.
pub fn collecting() -> bool {
    unsafe { COLLECT }
}

pub fn report() {
    let (adds, subs, collect) = unsafe { (ADDS, SUBS, COLLECT) };
    let acc = unsafe { &*core::ptr::addr_of!(ACC) };
    let x = acc.get().map(|p| p[7]).unwrap_or(0);
    env::log(&alloc::format!(
        "swiftsync: adds={adds} subs={subs} collect={collect} acc_x_top_word={x:08x}"));
}
