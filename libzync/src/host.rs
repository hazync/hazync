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

/// ROADMAP C1: the coin log's nodes on disk, one file per level (`L<l>.bin`, 32 bytes per node, node i at offset 32·i),
/// read and written in place. The OS page cache is the cache. Mainnet's log (~2.4 bn leaves) is ~150 GB of nodes.
/// ⚠ The position index (leaf -> position, host `ClHost::pos`) is still in memory: C1b.
pub struct DiskNodes {
    dir: std::path::PathBuf,
    files: std::vec::Vec<std::fs::File>,
    counts: std::vec::Vec<u64>,
}

#[cfg(unix)]
fn read_at(f: &std::fs::File, buf: &mut [u8], off: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(f, buf, off)
}
#[cfg(unix)]
fn write_at(f: &std::fs::File, buf: &[u8], off: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(f, buf, off)
}
#[cfg(windows)]
fn read_at(f: &std::fs::File, mut buf: &mut [u8], mut off: u64) -> std::io::Result<()> {
    while !buf.is_empty() {
        let n = std::os::windows::fs::FileExt::seek_read(f, buf, off)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        buf = &mut buf[n..];
        off += n as u64;
    }
    Ok(())
}
#[cfg(windows)]
fn write_at(f: &std::fs::File, mut buf: &[u8], mut off: u64) -> std::io::Result<()> {
    while !buf.is_empty() {
        let n = std::os::windows::fs::FileExt::seek_write(f, buf, off)?;
        buf = &buf[n..];
        off += n as u64;
    }
    Ok(())
}

impl DiskNodes {
    /// Opens (or creates) the store in `dir`. Each level's count is its file length / 32; a torn last node (a crash
    /// mid-write) is cut off, and the caller checks the roots against a receipt before trusting the store.
    pub fn open(dir: &std::path::Path) -> std::io::Result<DiskNodes> {
        std::fs::create_dir_all(dir)?;
        let mut d = DiskNodes { dir: dir.to_path_buf(), files: std::vec::Vec::new(), counts: std::vec::Vec::new() };
        loop {
            let p = dir.join(std::format!("L{}.bin", d.files.len()));
            if !p.exists() {
                break;
            }
            let f = std::fs::OpenOptions::new().read(true).write(true).open(&p)?;
            let n = f.metadata()?.len() / 32;
            f.set_len(n * 32)?;
            d.files.push(f);
            d.counts.push(n);
        }
        Ok(d)
    }
    fn level_file(&mut self, l: usize) -> &std::fs::File {
        while self.files.len() <= l {
            let p = self.dir.join(std::format!("L{}.bin", self.files.len()));
            let f = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&p)
                .unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            self.files.push(f);
            self.counts.push(0);
        }
        &self.files[l]
    }
    /// fsync every level (after a block, before its receipt is relied on).
    pub fn sync(&self) -> std::io::Result<()> {
        for f in &self.files {
            f.sync_data()?;
        }
        Ok(())
    }
}

impl crate::coinlog::Nodes for DiskNodes {
    fn levels(&self) -> usize {
        self.files.len()
    }
    fn count(&self, l: usize) -> u64 {
        self.counts.get(l).copied().unwrap_or(0)
    }
    fn get(&self, l: usize, i: u64) -> [u8; 32] {
        assert!(i < self.counts[l], "coin log store: node {l}/{i} past the end");
        let mut b = [0u8; 32];
        read_at(&self.files[l], &mut b, 32 * i).unwrap_or_else(|e| panic!("coin log store L{l}: {e}"));
        b
    }
    fn put(&mut self, l: usize, i: u64, h: [u8; 32]) {
        assert!(i < self.counts[l], "coin log store: node {l}/{i} past the end");
        write_at(&self.files[l], &h, 32 * i).unwrap_or_else(|e| panic!("coin log store L{l}: {e}"));
    }
    fn append(&mut self, l: usize, h: [u8; 32]) {
        let n = self.count(l);
        let f = self.level_file(l);
        write_at(f, &h, 32 * n).unwrap_or_else(|e| panic!("coin log store L{l}: {e}"));
        self.counts[l] = n + 1;
    }
}

/// Either backend, chosen at run time (host: `HAZYNC_COINLOG_DIR` set = on disk).
pub enum AnyNodes {
    Mem(crate::coinlog::MemNodes),
    Disk(DiskNodes),
}

impl crate::coinlog::Nodes for AnyNodes {
    fn levels(&self) -> usize {
        match self { AnyNodes::Mem(m) => m.levels(), AnyNodes::Disk(d) => d.levels() }
    }
    fn count(&self, l: usize) -> u64 {
        match self { AnyNodes::Mem(m) => m.count(l), AnyNodes::Disk(d) => d.count(l) }
    }
    fn get(&self, l: usize, i: u64) -> [u8; 32] {
        match self { AnyNodes::Mem(m) => crate::coinlog::Nodes::get(m, l, i), AnyNodes::Disk(d) => d.get(l, i) }
    }
    fn put(&mut self, l: usize, i: u64, h: [u8; 32]) {
        match self { AnyNodes::Mem(m) => m.put(l, i, h), AnyNodes::Disk(d) => d.put(l, i, h) }
    }
    fn append(&mut self, l: usize, h: [u8; 32]) {
        match self { AnyNodes::Mem(m) => crate::coinlog::Nodes::append(m, l, h), AnyNodes::Disk(d) => d.append(l, h) }
    }
}

#[cfg(test)]
mod disk_tests {
    use super::*;
    use crate::coinlog::{NodeStore, Store};

    #[test]
    fn disk_store_equals_memory_store() {
        let dir = std::env::temp_dir().join(std::format!("libzync-disk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut m = Store::default();
        let mut d = NodeStore::with_nodes(DiskNodes::open(&dir).unwrap());
        let h = |i: u64| -> [u8; 32] { use sha2::Digest; sha2::Sha256::digest(i.to_le_bytes()).into() };
        for i in 0..1000u64 {
            m.push(h(i));
            d.push(h(i));
            if i % 7 == 3 {
                m.set(i / 2, h(i + 5000));
                d.set(i / 2, h(i + 5000));
            }
        }
        assert_eq!(m.roots(), d.roots());
        assert_eq!(m.path(377), d.path(377));
        let sp = [(10u64, m.leaf(10)), (600, m.leaf(600))];
        assert_eq!(m.witness(&sp), d.witness(&sp));
        drop(d);
        // reopen: same state from the files alone
        let d2 = NodeStore::with_nodes(DiskNodes::open(&dir).unwrap());
        assert_eq!(d2.len(), 1000);
        assert_eq!(m.roots(), d2.roots());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
