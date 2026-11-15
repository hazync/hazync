// ⚠ GUEST-COMPILED (in an image only with an `xver-*` feature on) — editing this file changes METHOD_ID of such an image.
//
// Cross-version handover (docs/HANDOVER.md §3). A handover verifies the predecessor's spine receipt. When the
// predecessor ran the SAME RISC Zero as this image, `env::verify` does it. When it ran an OLDER RISC Zero, the recursion
// circuit itself may differ (3.0.5 -> 3.1.0 changed its constraints, taps and control root), so `env::verify` cannot,
// even with the old control root. This module carries the OLD version's recursion circuit (verbatim generated sources,
// one directory per version) and checks the old succinct seal in software, the way that version's own
// `SuccinctReceipt::verify_integrity` does. It then binds the seal's claim to exactly
// `ReceiptClaim::ok(predecessor image, journal)`: the predecessor image, exit Halted(0), no assumptions, this journal.
//
// The lineage entry selects the version: `Handover.verifier` is that version's SuccinctReceiptVerifierParameters digest.
// Zero means "same RISC Zero as this image" (env::verify, unchanged). An image pinning a non-zero digest must be built
// with the matching `xver-*` feature; main.rs asserts that at compile time.
//
// ⛔ Only across NON-soundness upgrades (docs/HANDOVER.md §4). A version listed here because a later one fixed a
// soundness bug in its circuit or verifier must never be pinned by a product image: what it proved cannot be inherited.
// v3_0_5 is such a version (3.1.0 fixed soundness bugs); it is here as a test vector only.
//
// The host compiles every version (verify-any of a predecessor's receipt, and the encoder); a guest only the ones
// its features name.

#![allow(dead_code)]

use risc0_zkp::{
    adapter::{CircuitCoreDef, CircuitInfo},
    core::{digest::Digest, hash::HashSuite},
    field::{baby_bear::{BabyBear, BabyBearElem}, Elem as _},
    verify::VerificationError,
};
use std::collections::VecDeque;
use std::vec::Vec;

#[cfg(any(not(target_os = "zkvm"), feature = "xver-v3_0_5"))]
pub mod v3_0_5;
#[cfg(any(not(target_os = "zkvm"), feature = "xver-v3_1_0"))]
pub mod v3_1_0;

/// One vendored RISC Zero version: its recursion circuit and the digest of its succinct verifier parameters.
pub struct Version {
    pub name: &'static str,
    pub params: [u8; 32],
    pub control_root: Digest,
    pub circuit_info: risc0_zkp::adapter::ProtocolInfo,
    verify_seal: fn(&HashSuite<BabyBear>, &[u32], &dyn Fn(u32, &Digest) -> Result<(), VerificationError>) -> Result<(), VerificationError>,
    output_size: usize,
}

/// The versions this build carries, oldest first.
pub const KNOWN: &[Version] = &[
    #[cfg(any(not(target_os = "zkvm"), feature = "xver-v3_0_5"))]
    v3_0_5::VERSION,
    #[cfg(any(not(target_os = "zkvm"), feature = "xver-v3_1_0"))]
    v3_1_0::VERSION,
];

/// `params` is carried by this build (a const fn, for main.rs's compile-time check of the lineage).
pub const fn carried(params: &[u8; 32]) -> bool {
    let mut i = 0;
    while i < KNOWN.len() {
        let mut j = 0;
        let mut eq = true;
        while j < 32 {
            if KNOWN[i].params[j] != params[j] { eq = false; }
            j += 1;
        }
        if eq { return true; }
        i += 1;
    }
    false
}

/// A 64-character hex string as 32 bytes, at compile time.
pub const fn hex32(s: &str) -> [u8; 32] {
    const fn nib(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => panic!("hex32: lowercase hex only"),
        }
    }
    let b = s.as_bytes();
    assert!(b.len() == 64, "hex32: 64 characters");
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = nib(b[2 * i]) << 4 | nib(b[2 * i + 1]);
        i += 1;
    }
    out
}

pub fn version(params: &[u8; 32]) -> Option<&'static Version> {
    KNOWN.iter().find(|v| &v.params == params)
}

/// Generic seal check for one vendored circuit (what `Version::verify_seal` points at).
pub fn verify_seal_with<C: CircuitCoreDef<BabyBear>>(
    circuit: &C,
    suite: &HashSuite<BabyBear>,
    seal: &[u32],
    check_code: &dyn Fn(u32, &Digest) -> Result<(), VerificationError>,
) -> Result<(), VerificationError> {
    risc0_zkp::verify::verify(circuit, suite, seal, check_code)
}

pub fn output_size<C: CircuitInfo>() -> usize { C::OUTPUT_SIZE }

/// The words a handover reads for a cross-version predecessor receipt (the host's `encode`):
/// `control_index, n_siblings, control_id (8), siblings (8 each), seal...`. The claim is NOT read: the guest builds the
/// only claim it accepts from the pinned image and the journal it already holds.
#[derive(Debug)]
pub struct Parsed<'a> {
    pub control_index: u32,
    pub control_id: Digest,
    pub siblings: Vec<Digest>,
    pub seal: &'a [u32],
}

fn digest_at(w: &[u32]) -> Digest {
    let mut a = [0u32; 8];
    a.copy_from_slice(&w[..8]);
    Digest::from(a)
}

pub fn parse(words: &[u32]) -> Result<Parsed<'_>, &'static str> {
    if words.len() < 10 { return Err("xver: receipt words too short"); }
    let control_index = words[0];
    let n = words[1] as usize;
    if n > 64 { return Err("xver: too many control siblings"); }
    let need = 10 + 8 * n;
    if words.len() <= need { return Err("xver: receipt words truncated"); }
    let control_id = digest_at(&words[2..10]);
    let siblings = (0..n).map(|i| digest_at(&words[10 + 8 * i..])).collect();
    Ok(Parsed { control_index, control_id, siblings, seal: &words[need..] })
}

fn merkle_root(leaf: &Digest, index: u32, sibs: &[Digest], suite: &HashSuite<BabyBear>) -> Digest {
    let mut cur = *leaf;
    let mut i = index;
    for s in sibs {
        cur = if i & 1 == 0 { *suite.hashfn.hash_pair(&cur, s) } else { *suite.hashfn.hash_pair(s, &cur) };
        i >>= 1;
    }
    cur
}

/// `risc0_binfmt::read_sha_halfs`: 16 words, each carrying 16 bits, low byte first.
fn read_sha_halfs(flat: &mut VecDeque<u32>) -> Result<Digest, &'static str> {
    if flat.len() < 16 { return Err("xver: seal output too short"); }
    let mut b = [0u8; 32];
    for (i, half) in flat.drain(0..16).enumerate() {
        if half > 0xffff { return Err("xver: seal output half out of range"); }
        b[2 * i] = (half & 0xff) as u8;
        b[2 * i + 1] = (half >> 8) as u8;
    }
    Digest::try_from(&b[..]).map_err(|_| "xver: claim digest")
}

/// The only claim a handover accepts: the predecessor image ran to Halted(0) with no assumptions and wrote `journal`.
/// The digest layout of ReceiptClaim is the same in every version vendored here (checked by the host test against
/// the real receipts' own claims).
pub fn expected_claim(image: [u32; 8], journal: &[u8]) -> Digest {
    use risc0_zkvm::sha::Digestible;
    risc0_zkvm::ReceiptClaim::ok(Digest::from(image), journal.to_vec()).digest()
}

/// Verify a predecessor's succinct receipt made under the version `params` names, against the pinned `image` and
/// `journal`. Exactly what that version's `SuccinctReceipt::verify_integrity` checks (the seal, the control id's
/// inclusion under the version's control root, the root the seal itself outputs), then the claim binding.
pub fn verify(params: &[u8; 32], image: [u32; 8], journal: &[u8], words: &[u32], suite: &HashSuite<BabyBear>) -> Result<(), &'static str> {
    let v = version(params).ok_or("xver: this build does not carry the predecessor's RISC Zero version")?;
    let p = parse(words)?;
    let check_code = |_po2: u32, control_id: &Digest| -> Result<(), VerificationError> {
        if *control_id != p.control_id {
            return Err(VerificationError::ControlVerificationError { control_id: *control_id });
        }
        if merkle_root(control_id, p.control_index, &p.siblings, suite) != v.control_root {
            return Err(VerificationError::ControlVerificationError { control_id: *control_id });
        }
        Ok(())
    };
    (v.verify_seal)(suite, p.seal, &check_code).map_err(|_| "xver: predecessor seal does not verify")?;
    if p.seal.len() < v.output_size { return Err("xver: seal too short"); }
    let mut out: VecDeque<u32> = VecDeque::with_capacity(v.output_size);
    for w in &p.seal[..v.output_size] {
        let e = BabyBearElem::new_raw(*w);
        if !e.is_reduced() { return Err("xver: seal output not a field element"); }
        out.push_back(e.as_u32());
    }
    let root_words: Vec<u32> = out.drain(0..16).enumerate().filter_map(|(i, w)| (i & 1 == 0).then_some(w)).collect();
    let mut rw = [0u32; 8];
    rw.copy_from_slice(&root_words);
    if Digest::from(rw) != v.control_root { return Err("xver: seal's control root is not the version's"); }
    let claim = read_sha_halfs(&mut out)?;
    if claim != expected_claim(image, journal) {
        return Err("xver: predecessor receipt is not Halted(0) of the pinned image with this journal");
    }
    Ok(())
}

/// The software Poseidon2 suite (host; also a guest fallback).
pub fn soft_suite() -> HashSuite<BabyBear> {
    risc0_zkp::core::hash::poseidon2::Poseidon2HashSuite::new_suite()
}

/// The host side: a receipt's words for the guest. `Receipt` is decoded with THIS host's risc0-zkvm types; the
/// SuccinctReceipt layout is the same in every version vendored here (a new one must check that).
#[cfg(not(target_os = "zkvm"))]
pub fn encode(s: &risc0_zkvm::SuccinctReceipt<risc0_zkvm::ReceiptClaim>) -> Vec<u32> {
    let mut w = vec![s.control_inclusion_proof.index, s.control_inclusion_proof.digests.len() as u32];
    w.extend_from_slice(s.control_id.as_words());
    for d in &s.control_inclusion_proof.digests { w.extend_from_slice(d.as_words()); }
    w.extend_from_slice(&s.seal);
    w
}

/// Poseidon2 on the rv32im POSEIDON2 call. Same permutation and overwrite sponge with zero padding as risc0-zkp's
/// software `unpadded_hash`; the call works on canonical cells while BabyBearElem and Digest words are Montgomery,
/// hence the conversions. A wrong hash cannot pass: the seal only verifies if every Merkle hash is bit-exact. The
/// Fiat-Shamir RNG stays in software. Measured 11-13 on a real 3.0.5 receipt: 21.9M cycles verify vs 255.5M in software.
#[cfg(target_os = "zkvm")]
pub mod accel {
    use risc0_zkp::core::{
        digest::Digest,
        hash::{poseidon2::Poseidon2Rng, HashFn, HashSuite, Rng, RngFactory},
    };
    use risc0_zkp::field::{baby_bear::{BabyBear, BabyBearElem, BabyBearExtElem}, Elem, ExtElem};
    use std::{boxed::Box, rc::Rc, vec, vec::Vec};

    const PFLAG_IS_ELEM: u32 = 0x8000_0000;
    const P: u32 = 15 * (1 << 27) + 1;

    fn hash_canon(words: &[u32]) -> Box<Digest> {
        let n = core::cmp::max(1, words.len().div_ceil(16));
        let mut buf = vec![0u32; n * 16];
        buf[..words.len()].copy_from_slice(words);
        let mut out = [0u32; 8];
        unsafe {
            risc0_zkvm_platform::syscall::sys_poseidon2(
                core::ptr::null_mut(),
                buf.as_ptr() as *const u8,
                &mut out,
                n as u32 | PFLAG_IS_ELEM,
            );
        }
        Box::new(Digest::from(out.map(|w| BabyBearElem::new(w).as_u32_montgomery())))
    }

    pub struct P2Accel;
    impl HashFn<BabyBear> for P2Accel {
        fn hash_pair(&self, a: &Digest, b: &Digest) -> Box<Digest> {
            let w: Vec<u32> = a.as_words().iter().chain(b.as_words())
                .map(|w| { let e = BabyBearElem::new_raw(*w); assert!(e.is_reduced()); e.as_u32() }).collect();
            hash_canon(&w)
        }
        fn hash_elem_slice(&self, s: &[BabyBearElem]) -> Box<Digest> {
            let w: Vec<u32> = s.iter().map(|e| e.as_u32()).collect();
            hash_canon(&w)
        }
        fn hash_ext_elem_slice(&self, s: &[BabyBearExtElem]) -> Box<Digest> {
            let w: Vec<u32> = s.iter().flat_map(|e| e.subelems().iter()).map(|e| e.as_u32()).collect();
            hash_canon(&w)
        }
        fn is_digest_valid(&self, d: &Digest) -> bool {
            d.as_words().iter().all(|w| *w < P)
        }
    }
    struct SwRng;
    impl RngFactory<BabyBear> for SwRng {
        fn new_rng(&self) -> Box<dyn Rng<BabyBear>> { Box::new(Poseidon2Rng::new()) }
    }
    pub fn suite() -> HashSuite<BabyBear> {
        HashSuite { name: "poseidon2".into(), hashfn: Rc::new(P2Accel), rng: Rc::new(SwRng) }
    }
}
