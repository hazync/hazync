//! Host side: the hint PRODUCERS and the per-block hint bundle file.
//!
//! The guest checks every hint (wire.rs), so nothing here is trusted: a bug in a producer makes a run
//! fall back to libsecp or fail, never accept a bad block.

use crate::wire::{BATCHREC_TAG, ECMHREC_TAG, SIG_HINT_BYTES};
use num_bigint::BigUint;
use secp256k1::{PublicKey, Scalar, SecretKey, SECP256K1};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::{format, string::String, vec::Vec};

const N: [u8; 32] = crate::net::hex_be("fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141");
const P: [u8; 32] = crate::net::hex_be("fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f");


fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 { return None; }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

/// Big-endian a - b for 32-byte values with a >= b.
fn sub_be(a: &[u8], b: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut borrow = 0i16;
    for k in (0..32).rev() {
        let d = a[k] as i16 - b[k] as i16 - borrow;
        borrow = (d < 0) as i16;
        out[k] = (d + 256 * borrow) as u8;
    }
    out
}

/// One deferred verify as the guest logs it in collect mode: `BATCHREC<kind> <hex u1 u2 qx qy r>`.
pub struct SigRecord {
    pub kind: u8,
    pub u1: [u8; 32],
    pub u2: [u8; 32],
    pub q: [u8; 64],
    pub r: [u8; 32],
}

/// Every BATCHREC record in a guest log, in call order. Panics on a malformed one: the format is
/// ours, so a bad line is a bug, not data.
pub fn sig_records(log: &str) -> Vec<SigRecord> {
    log.lines()
        .filter_map(|line| {
            let at = line.find(BATCHREC_TAG)?;
            let rest = &line[at + BATCHREC_TAG.len()..];
            let kind = rest.as_bytes()[0].wrapping_sub(b'0');
            let rec = unhex(rest[2..].trim()).filter(|r| r.len() == 160);
            let rec = match (rec, kind) { (Some(r), 0 | 1 | 3 | 4) => r, _ => panic!("bad {BATCHREC_TAG} line: {line}") };
            let f = |k: usize| -> [u8; 32] { rec[32 * k..32 * k + 32].try_into().unwrap() };
            let mut q = [0u8; 64];
            q.copy_from_slice(&rec[64..128]);
            Some(SigRecord { kind, u1: f(0), u2: f(1), q, r: f(4) })
        })
        .collect()
}

/// The hint for one deferred verify: R = u1·G + u2·Q by libsecp, and whether the scheme's acceptance
/// test holds on it. An invalid signature gets valid = 0 and the guest verifies it in libsecp.
pub fn sig_hint(s: &SigRecord) -> [u8; SIG_HINT_BYTES] {
    let mut qs = [4u8; 65];
    qs[1..].copy_from_slice(&s.q);
    // u·P with u = 0 is the point at infinity: None, and so is a sum that cancels.
    let a = SecretKey::from_slice(&s.u1).ok().map(|k| PublicKey::from_secret_key(SECP256K1, &k));
    let b = PublicKey::from_slice(&qs).ok()
        .and_then(|q| Scalar::from_be_bytes(s.u2).ok().and_then(|t| q.mul_tweak(SECP256K1, &t).ok()));
    let rp = match (a, b) {
        (Some(a), Some(b)) => a.combine(&b).ok(),
        (x, None) | (None, x) => x,
    };
    let xy = rp.map(|p| p.serialize_uncompressed());
    let ok = match &xy {
        None => false,
        Some(p) => {
            let (x, yodd) = (&p[1..33], p[64] & 1);
            match s.kind {
                // ECDSA: x(R) mod n == r. x < p and p - n < 2^129, so x mod n is x or x - n.
                0 => x == s.r.as_slice() || (x >= N.as_slice() && sub_be(x, &N) == s.r),
                1 => x == s.r.as_slice() && yodd == 0,
                k => x == s.r.as_slice() && yodd == k - 3,
            }
        }
    };
    let mut out = [0u8; SIG_HINT_BYTES];
    out[..4].copy_from_slice(&(ok as u32).to_le_bytes());
    if let Some(p) = xy { out[4..].copy_from_slice(&p[1..65]); }
    out
}

/// Every ECMHREC leaf in a guest log, in operation order.
pub fn ecmh_leaves(log: &str) -> Vec<[u8; 32]> {
    log.lines()
        .filter_map(|l| l.find(ECMHREC_TAG).map(|at| {
            unhex(l[at + ECMHREC_TAG.len()..].trim()).and_then(|v| v.try_into().ok())
                .unwrap_or_else(|| panic!("bad {ECMHREC_TAG}line: {l}"))
        }))
        .collect()
}

/// One ECMH hint: ctr u32 LE | y (32 B BE) | one w (32 B BE) per skipped ctr.
/// x = SHA-256(leaf || ctr LE), the first ctr with x^3 + 7 square; y = rhs^((p+1)/4); and for every
/// skipped ctr a w with w^2 = -(x^3 + 7), which proves that x was NOT on the curve (p = 3 mod 4).
pub fn ecmh_hint(leaf: &[u8; 32]) -> Vec<u8> {
    let p = BigUint::from_bytes_be(&P);
    let e = (&p + 1u32) >> 2;
    let sqrt = |a: &BigUint| { let r = a.modpow(&e, &p); (&r * &r % &p == *a).then_some(r) };
    let be32 = |v: &BigUint| { let b = v.to_bytes_be(); let mut o = [0u8; 32]; o[32 - b.len()..].copy_from_slice(&b); o };
    let mut skipped = Vec::new();
    for ctr in 0u32.. {
        let mut h = Sha256::new();
        h.update(leaf);
        h.update(ctr.to_le_bytes());
        let x = BigUint::from_bytes_be(&h.finalize());
        assert!(x < p, "ECMH: hash >= p");
        let rhs = (&x * &x * &x + 7u32) % &p;
        if let Some(y) = sqrt(&rhs) {
            // The guest accepts only the EVEN root: one point per leaf.
            let y = if y.bit(0) { &p - &y } else { y };
            // Negative-test hook (inert unless set): hand the guest the ODD root, which it must refuse.
            let y = if std::env::var("HAZYNC_TEST_ECMH_ODD").is_ok() { &p - &y } else { y };
            let mut out = ctr.to_le_bytes().to_vec();
            out.extend_from_slice(&be32(&y));
            for w in &skipped { out.extend_from_slice(&be32(w)); }
            return out;
        }
        skipped.push(sqrt(&((&p - &rhs) % &p)).expect("p = 3 mod 4: -rhs must be a square"));
    }
    unreachable!()
}

/// Everything expensive a block's proof needs, in ONE file (`libzync-hints` writes it).
///
/// Only what costs a collect pass to learn is stored. The pubkey-Y hints, the block's MTP and the
/// SwiftSync bits are cheap and derived at run time (from the block and from the hints file).
///
/// ```text
/// "LZB1" version=1 height block_hash[32] nchunks { lo hi sig_len sig[sig_len] }* ecmh_len ecmh[ecmh_len]
/// ```
/// (all u32 LE). Chunk bounds are stored so that a bundle made under one chunk layout cannot be
/// applied to another: `chunk_sig` returns None and the guest collects instead of desynchronising.
#[derive(Debug, Default, PartialEq)]
pub struct BlockBundle {
    pub height: u32,
    pub block_hash: [u8; 32],
    pub chunks: Vec<(u32, u32, Vec<u8>)>,
    pub ecmh: Vec<u8>,
}

const BUNDLE_MAGIC: &[u8; 4] = b"LZB1";
const BUNDLE_VERSION: u32 = 1;

impl BlockBundle {
    pub fn chunk_sig(&self, lo: u32, hi: u32) -> Option<&[u8]> {
        self.chunks.iter().find(|c| c.0 == lo && c.1 == hi).map(|c| c.2.as_slice())
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::new();
        let u = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
        v.extend_from_slice(BUNDLE_MAGIC);
        u(&mut v, BUNDLE_VERSION);
        u(&mut v, self.height);
        v.extend_from_slice(&self.block_hash);
        u(&mut v, self.chunks.len() as u32);
        for (lo, hi, s) in &self.chunks {
            u(&mut v, *lo);
            u(&mut v, *hi);
            u(&mut v, s.len() as u32);
            v.extend_from_slice(s);
        }
        u(&mut v, self.ecmh.len() as u32);
        v.extend_from_slice(&self.ecmh);
        v
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self, String> {
        let mut r = b;
        let mut take = |n: usize| -> Result<Vec<u8>, String> {
            if r.len() < n { return Err("hint bundle: truncated".into()); }
            let (a, rest) = r.split_at(n);
            r = rest;
            Ok(a.to_vec())
        };
        if take(4)? != BUNDLE_MAGIC { return Err("hint bundle: bad magic".into()); }
        let u = |t: &mut dyn FnMut(usize) -> Result<Vec<u8>, String>| -> Result<u32, String> {
            Ok(u32::from_le_bytes(t(4)?.try_into().unwrap()))
        };
        let ver = u(&mut take)?;
        if ver != BUNDLE_VERSION { return Err(format!("hint bundle: version {ver}, expected {BUNDLE_VERSION}")); }
        let height = u(&mut take)?;
        let block_hash: [u8; 32] = take(32)?.try_into().unwrap();
        let n = u(&mut take)?;
        // No capacity from the untrusted count: a huge n would abort on allocation instead of failing cleanly.
        let mut chunks = Vec::new();
        for _ in 0..n {
            let (lo, hi, len) = (u(&mut take)?, u(&mut take)?, u(&mut take)?);
            if len as usize % SIG_HINT_BYTES != 0 { return Err("hint bundle: sig hints not whole records".into()); }
            chunks.push((lo, hi, take(len as usize)?));
        }
        let el = u(&mut take)?;
        let ecmh = take(el as usize)?;
        drop(take);
        if !r.is_empty() { return Err("hint bundle: trailing bytes".into()); }
        Ok(BlockBundle { height, block_hash, chunks, ecmh })
    }

    /// Atomic write (tmp + rename): a prover polling for the file never reads half of it.
    pub fn write(&self, path: &str) -> std::io::Result<()> {
        let tmp = format!("{path}.tmp");
        std::fs::File::create(&tmp)?.write_all(&self.to_bytes())?;
        std::fs::rename(tmp, path)
    }

    pub fn read(path: &str) -> Result<Self, String> {
        let mut b = Vec::new();
        std::fs::File::open(path).and_then(|mut f| f.read_to_end(&mut b)).map_err(|e| format!("{path}: {e}"))?;
        Self::from_bytes(&b).map_err(|e| format!("{path}: {e}"))
    }
}

/// The hint hash a proof at a hints file's stop height must commit, from the file ALONE.
/// Err if a height's list is not STRICTLY ascending: the Elias-Fano decoder does not enforce order, and a
/// client walking the list as sorted would mark the wrong outputs (review 2). Sorting it here instead would
/// let a proof vouch for a file that is not what it says.
pub fn file_hash(file: &hintsfile::Hintsfile) -> Result<[u8; 32], String> {
    let mut h = [0u8; 32];
    for height in 1..=file.stop_height() {
        let idx = file.indices_at_height(height).ok_or_else(|| format!("height {height}: missing"))?;
        if !idx.windows(2).all(|p| p[0] < p[1]) {
            return Err(format!("height {height}: index list is not strictly ascending"));
        }
        h = crate::ss_hints::step(&h, height, &idx);
    }
    Ok(h)
}

/// The file re-encoded from what it decodes to. A hints file is CANONICAL when this equals its bytes: the
/// hintsfile crate's Elias-Fano decoder does not check low-bit padding, values against the header's bound or
/// trailing bytes, so two byte-different files can decode alike and another decoder may read either one
/// differently (audit 3). A proof vouches for the decoded lists; requiring canonical bytes makes that the file.
pub fn canonical_bytes(file: &hintsfile::Hintsfile) -> Result<Vec<u8>, String> {
    use hintsfile::{EliasFano, HintsfileBuilder};
    let mut out = Vec::new();
    {
        let mut b = HintsfileBuilder::new(&mut out).initialize(file.stop_height()).map_err(|e| format!("{e}"))?;
        for h in 1..=file.stop_height() {
            let idx = file.indices_at_height(h).ok_or_else(|| format!("height {h}: missing"))?;
            b.append(EliasFano::compress(&idx)).map_err(|e| format!("{e}"))?;
        }
        b.finish().map_err(|e| format!("{e}"))?;
    }
    Ok(out)
}
