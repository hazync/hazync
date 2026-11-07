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

/// An ADVERSARIAL hint for one deferred verify (a soundness test of the batch, I1): it always claims VALID, with
/// R = (r, y) of the parity the scheme's acceptance test wants, so R passes the guest's per-signature check (on the
/// curve, x(R) == r, parity) whenever r is a curve x. Only the batch equation can then catch a bad signature. When r is
/// not a curve x no such R exists: the honest hint.
///
/// `variant`: 0 the parity the acceptance test wants; 1 the opposite parity (refused per signature where parity is
/// checked, so it must fall back; deferred for ECDSA); 2 x = r + n for ECDSA (also passes x(R) mod n == r), else 0.
pub fn sig_hint_forged(s: &SigRecord, variant: u8) -> [u8; SIG_HINT_BYTES] {
    let p = BigUint::from_bytes_be(&P);
    let mut x = BigUint::from_bytes_be(&s.r);
    if variant == 2 && s.kind == 0 {
        x += BigUint::from_bytes_be(&N);
        if x >= p { return sig_hint(s); }
    }
    if x >= p { return sig_hint(s); }
    let rhs = (&x * &x * &x + 7u32) % &p;
    let y = rhs.modpow(&((&p + 1u32) >> 2), &p);
    if &y * &y % &p != rhs { return sig_hint(s); }
    let want_odd = matches!(s.kind, 4) != (variant == 1);
    let y = if (y.bit(0)) == want_odd { y } else { &p - &y };
    let be32 = |v: &BigUint| { let b = v.to_bytes_be(); let mut o = [0u8; 32]; o[32 - b.len()..].copy_from_slice(&b); o };
    let mut out = [0u8; SIG_HINT_BYTES];
    out[..4].copy_from_slice(&1u32.to_le_bytes());
    out[4..36].copy_from_slice(&be32(&x));
    out[36..68].copy_from_slice(&be32(&y));
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
/// Only what costs a collect pass to learn is stored. The pubkey-Y hints and the block's MTP are cheap and derived
/// at run time from the block.
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

/// ROADMAP C1: the coin log's nodes on disk, one file per level (`L<l>.bin`, 32 bytes per node, node i at offset 32·i),
/// read and written in place. The OS page cache is the cache. Mainnet's log (~2.4 bn leaves) is ~150 GB of nodes.
/// ⚠ The position index (leaf -> position, host `ClHost::pos`) is still in memory: C1b.
pub struct DiskNodes {
    dir: std::path::PathBuf,
    /// one per level; None for a level that is not stored (1..=skip)
    files: std::vec::Vec<Option<std::fs::File>>,
    counts: std::vec::Vec<u64>,
    /// levels 1..=skip are NOT stored: a node there is recomputed from its 2^l leaves when read (11-07). They are 7/8
    /// of the upper nodes at skip 3 (mainnet: ~65 GB of ~75 GB), for at most 8 leaf reads and 7 hashes per node read.
    /// Fixed per store (the `SKIP` file); a store without one (written before) stores every level.
    skip: usize,
}

/// The skip level of a NEW disk store (one with no L0.bin yet).
pub const DISK_SKIP_DEFAULT: usize = 3;

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
        if n == 0 {
            return Err(std::io::ErrorKind::WriteZero.into());
        }
        buf = &buf[n..];
        off += n as u64;
    }
    Ok(())
}

impl DiskNodes {
    /// Opens (or creates) the store in `dir`. Each level's count is its file length / 32; a torn last node (a crash
    /// mid-write) is cut off, and the caller checks the roots against a receipt before trusting the store.
    pub fn open(dir: &std::path::Path) -> std::io::Result<DiskNodes> {
        Self::open_with(dir, DISK_SKIP_DEFAULT)
    }
    /// `open`, with the skip level a NEW store gets (an existing store keeps its own: its SKIP file, or 0 without one).
    pub fn open_with(dir: &std::path::Path, skip_if_new: usize) -> std::io::Result<DiskNodes> {
        std::fs::create_dir_all(dir)?;
        let sp = dir.join("SKIP");
        let skip = if !dir.join("L0.bin").exists() {
            let _ = std::fs::remove_file(&sp);
            std::fs::write(&sp, std::format!("{skip_if_new}\n"))?;
            std::fs::File::open(dir)?.sync_all()?;
            skip_if_new
        } else if sp.exists() {
            std::fs::read_to_string(&sp)?.trim().parse().map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "coin log SKIP: not a number"))?
        } else { 0 };
        let mut d = DiskNodes { dir: dir.to_path_buf(), files: std::vec::Vec::new(), counts: std::vec::Vec::new(), skip };
        loop {
            let l = d.files.len();
            if l >= 1 && l <= skip {
                // not stored: present while the leaves fill a node at this level
                let n = d.counts[0] >> l;
                if n == 0 { break; }
                if dir.join(std::format!("L{l}.bin")).exists() {
                    return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, std::format!("coin log: L{l}.bin in a store with SKIP {skip}")));
                }
                d.files.push(None);
                d.counts.push(n);
                continue;
            }
            let p = dir.join(std::format!("L{l}.bin"));
            if !p.exists() {
                break;
            }
            let f = std::fs::OpenOptions::new().read(true).write(true).open(&p)?;
            let n = f.metadata()?.len() / 32;
            f.set_len(n * 32)?;
            d.files.push(Some(f));
            d.counts.push(n);
        }
        Ok(d)
    }
    fn stored(&self, l: usize) -> bool { l == 0 || l > self.skip }
    /// A level-l node not stored: the root of its 2^l leaves, read in one go.
    fn computed(&self, l: usize, i: u64) -> [u8; 32] {
        let w = 1usize << l;
        let mut b = std::vec![0u8; 32 * w];
        read_at(self.files[0].as_ref().unwrap(), &mut b, 32 * (i << l)).unwrap_or_else(|e| panic!("coin log store L0: {e}"));
        let mut v: std::vec::Vec<[u8; 32]> = b.chunks_exact(32).map(|c| c.try_into().unwrap()).collect();
        for k in 1..=l {
            v = v.chunks_exact(2).map(|p| crate::coinlog::parent(k as u32, &p[0], &p[1])).collect();
        }
        v[0]
    }
    fn level_file(&mut self, l: usize) -> Option<&std::fs::File> {
        while self.files.len() <= l {
            let k = self.files.len();
            if !self.stored(k) {
                self.files.push(None);
            } else {
                let p = self.dir.join(std::format!("L{k}.bin"));
                let f = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&p)
                    .unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                self.files.push(Some(f));
            }
            self.counts.push(0);
        }
        self.files[l].as_ref()
    }
    /// Cuts the store back to `n` leaves: every level keeps n >> l nodes (an append never changes an existing node, so
    /// this undoes appends exactly; flipped leaves are restored by the caller with `set`).
    pub fn truncate_to(&mut self, n: u64) -> std::io::Result<()> {
        for l in 0..self.files.len() {
            let c = n >> l;
            if c < self.counts[l] {
                if let Some(f) = &self.files[l] { f.set_len(c * 32)?; }
                self.counts[l] = c;
            }
        }
        Ok(())
    }
    /// fsync every level (after a block, before its receipt is relied on).
    pub fn sync(&self) -> std::io::Result<()> {
        for f in self.files.iter().flatten() {
            f.sync_data()?;
        }
        Ok(())
    }
}

impl crate::coinlog_forest::Nodes for DiskNodes {
    fn levels(&self) -> usize {
        self.files.len()
    }
    fn count(&self, l: usize) -> u64 {
        self.counts.get(l).copied().unwrap_or(0)
    }
    fn get(&self, l: usize, i: u64) -> [u8; 32] {
        assert!(i < self.counts[l], "coin log store: node {l}/{i} past the end");
        let Some(f) = &self.files[l] else { return self.computed(l, i) };
        let mut b = [0u8; 32];
        read_at(f, &mut b, 32 * i).unwrap_or_else(|e| panic!("coin log store L{l}: {e}"));
        b
    }
    fn put(&mut self, l: usize, i: u64, h: [u8; 32]) {
        assert!(i < self.counts[l], "coin log store: node {l}/{i} past the end");
        // a level not stored follows its leaves by itself
        if let Some(f) = &self.files[l] { write_at(f, &h, 32 * i).unwrap_or_else(|e| panic!("coin log store L{l}: {e}")); }
    }
    fn append(&mut self, l: usize, h: [u8; 32]) {
        let n = self.count(l);
        if let Some(f) = self.level_file(l) { write_at(f, &h, 32 * n).unwrap_or_else(|e| panic!("coin log store L{l}: {e}")); }
        self.counts[l] = n + 1;
    }
}

/// Either backend, chosen at run time (host: `HAZYNC_COINLOG_DIR` set = on disk).
pub enum AnyNodes {
    Mem(crate::coinlog_forest::MemNodes),
    Disk(DiskNodes),
}

impl crate::coinlog_forest::Nodes for AnyNodes {
    fn levels(&self) -> usize {
        match self { AnyNodes::Mem(m) => m.levels(), AnyNodes::Disk(d) => d.levels() }
    }
    fn count(&self, l: usize) -> u64 {
        match self { AnyNodes::Mem(m) => m.count(l), AnyNodes::Disk(d) => d.count(l) }
    }
    fn get(&self, l: usize, i: u64) -> [u8; 32] {
        match self { AnyNodes::Mem(m) => crate::coinlog_forest::Nodes::get(m, l, i), AnyNodes::Disk(d) => d.get(l, i) }
    }
    fn put(&mut self, l: usize, i: u64, h: [u8; 32]) {
        match self { AnyNodes::Mem(m) => m.put(l, i, h), AnyNodes::Disk(d) => d.put(l, i, h) }
    }
    fn append(&mut self, l: usize, h: [u8; 32]) {
        match self { AnyNodes::Mem(m) => crate::coinlog_forest::Nodes::append(m, l, h), AnyNodes::Disk(d) => d.append(l, h) }
    }
}

#[cfg(test)]
mod disk_tests {
    use super::*;
    use crate::coinlog_forest::{NodeStore, Store};

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

    /// Every skip level gives the memory store's roots, paths and witnesses, before and after a reopen, with flips
    /// and a rollback; a store written without a SKIP file (every level stored) reopens as one (11-07).
    #[test]
    fn disk_store_skip_levels_equal_memory() {
        let h = |i: u64| -> [u8; 32] { use sha2::Digest; sha2::Sha256::digest(i.to_le_bytes()).into() };
        for skip in 0..=5usize {
            let dir = std::env::temp_dir().join(std::format!("libzync-skip{skip}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let mut m = Store::default();
            let mut d = NodeStore::with_nodes(DiskNodes::open_with(&dir, skip).unwrap());
            for i in 0..1_337u64 {
                m.push(h(i));
                d.push(h(i));
                if i % 5 == 2 { m.set(i * 3 / 4, h(i + 9_000)); d.set(i * 3 / 4, h(i + 9_000)); }
            }
            let sp: std::vec::Vec<(u64, [u8; 32])> = [0u64, 1, 7, 8, 9, 300, 301, 1_023, 1_024, 1_300].iter().map(|&p| (p, m.leaf(p))).collect();
            assert_eq!(m.roots(), d.roots(), "skip {skip}");
            assert_eq!(m.witness(&sp), d.witness(&sp), "skip {skip}");
            for p in [0u64, 5, 513, 1_336] { assert_eq!(m.path(p), d.path(p), "skip {skip} path {p}"); }
            for l in 1..=skip { assert!(!dir.join(std::format!("L{l}.bin")).exists(), "skip {skip}: L{l}.bin written"); }
            drop(d);
            let mut d = NodeStore::with_nodes(DiskNodes::open_with(&dir, 0).unwrap()); // an existing store keeps its skip
            assert_eq!(m.roots(), d.roots(), "skip {skip} reopened");
            assert_eq!(m.witness(&sp), d.witness(&sp), "skip {skip} reopened");
            // a rollback across a level boundary, then more appends
            let before = (d.roots(), d.len());
            let old = [(3u64, d.leaf(3)), (1_200, d.leaf(1_200))];
            d.set(3, h(77)); d.set(1_200, h(78));
            for i in 0..100u64 { d.push(h(20_000 + i)); }
            d.nodes.truncate_to(before.1).unwrap();
            for (p, l) in old { d.set(p, l); }
            assert_eq!((d.roots(), d.len()), before, "skip {skip} rollback");
            for i in 0..50u64 { m.push(h(30_000 + i)); d.push(h(30_000 + i)); }
            assert_eq!(m.roots(), d.roots(), "skip {skip} after rollback");
            let _ = std::fs::remove_dir_all(&dir);
        }
        // a store from before SKIP existed: every level on disk, no SKIP file -> opens as skip 0
        let dir = std::env::temp_dir().join(std::format!("libzync-skipold-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut d = NodeStore::with_nodes(DiskNodes::open_with(&dir, 0).unwrap());
        for i in 0..100u64 { d.push(h(i)); }
        let r = d.roots();
        drop(d);
        std::fs::remove_file(dir.join("SKIP")).unwrap();
        let d = NodeStore::with_nodes(DiskNodes::open(&dir).unwrap());
        assert_eq!(d.roots(), r);
        assert!(dir.join("L1.bin").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disk_store_rolls_back_one_block() {
        let dir = std::env::temp_dir().join(std::format!("libzync-rb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let h = |i: u64| -> [u8; 32] { use sha2::Digest; sha2::Sha256::digest(i.to_le_bytes()).into() };
        let mut d = NodeStore::with_nodes(DiskNodes::open(&dir).unwrap());
        for i in 0..900u64 { d.push(h(i)); }
        let before = d.roots();
        // a block: two flips, 123 appends
        let old = [(17u64, d.leaf(17)), (600, d.leaf(600))];
        d.set(17, h(99_017));
        d.set(600, h(99_600));
        for i in 0..123u64 { d.push(h(50_000 + i)); }
        assert_ne!(d.roots(), before);
        // roll back: cut the appends, restore the flipped leaves
        d.nodes.truncate_to(900).unwrap();
        for (p, l) in old { d.set(p, l); }
        assert_eq!(d.len(), 900);
        assert_eq!(d.roots(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// ROADMAP C1b: the host's coin-leaf -> log-position index on disk. Open addressing in one file (`index.bin`): a header
/// `[u64 capacity][u64 used]`, then `capacity` slots of `[leaf 32][position + 1, u64 LE]` (0 = empty, u64::MAX =
/// removed). Leaves are SHA-256 outputs, so their first 8 bytes are the hash. Grows (rebuild into twice the slots) at
/// half full. Live coins only: a spend removes its leaf.
pub struct DiskIndex {
    f: std::fs::File,
    path: std::path::PathBuf,
    cap: u64,
    used: u64,
}

const IX_SLOT: u64 = 40;
const IX_HDR: u64 = 16;

impl DiskIndex {
    pub fn open(path: &std::path::Path) -> std::io::Result<DiskIndex> {
        let f = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)?;
        let mut d = DiskIndex { f, path: path.to_path_buf(), cap: 0, used: 0 };
        if d.f.metadata()?.len() < IX_HDR {
            d.init(1 << 16)?;
        } else {
            let mut h = [0u8; 16];
            read_at(&d.f, &mut h, 0)?;
            d.cap = u64::from_le_bytes(h[..8].try_into().unwrap());
            d.used = u64::from_le_bytes(h[8..].try_into().unwrap());
            let want = d.cap.checked_mul(IX_SLOT).and_then(|x| x.checked_add(IX_HDR));
            if d.cap == 0 || !d.cap.is_power_of_two() || d.used > d.cap || want != Some(d.f.metadata()?.len()) {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "index.bin: corrupt header or size"));
            }
        }
        Ok(d)
    }
    fn init(&mut self, cap: u64) -> std::io::Result<()> {
        self.f.set_len(0)?;
        self.f.set_len(IX_HDR + cap * IX_SLOT)?;
        self.cap = cap;
        self.used = 0;
        self.hdr()
    }
    fn hdr(&self) -> std::io::Result<()> {
        let mut h = [0u8; 16];
        h[..8].copy_from_slice(&self.cap.to_le_bytes());
        h[8..].copy_from_slice(&self.used.to_le_bytes());
        write_at(&self.f, &h, 0)
    }
    fn slot(&self, i: u64) -> ([u8; 32], u64) {
        let mut b = [0u8; 40];
        read_at(&self.f, &mut b, IX_HDR + i * IX_SLOT).expect("index read");
        (b[..32].try_into().unwrap(), u64::from_le_bytes(b[32..].try_into().unwrap()))
    }
    fn put_slot(&self, i: u64, leaf: &[u8; 32], v: u64) {
        let mut b = [0u8; 40];
        b[..32].copy_from_slice(leaf);
        b[32..].copy_from_slice(&v.to_le_bytes());
        write_at(&self.f, &b, IX_HDR + i * IX_SLOT).expect("index write");
    }
    fn start(&self, leaf: &[u8; 32]) -> u64 {
        u64::from_le_bytes(leaf[..8].try_into().unwrap()) & (self.cap - 1)
    }
    /// The slot holding `leaf`, or None.
    fn find(&self, leaf: &[u8; 32]) -> Option<u64> {
        let mut i = self.start(leaf);
        loop {
            let (k, v) = self.slot(i);
            if v == 0 {
                return None;
            }
            if v != u64::MAX && &k == leaf {
                return Some(i);
            }
            i = (i + 1) & (self.cap - 1);
        }
    }
    pub fn get(&self, leaf: &[u8; 32]) -> Option<u64> {
        self.find(leaf).map(|i| self.slot(i).1 - 1)
    }
    pub fn sync(&self) -> std::io::Result<()> {
        self.f.sync_data()
    }
    pub fn insert(&mut self, leaf: [u8; 32], pos: u64) {
        if let Some(i) = self.find(&leaf) {
            self.put_slot(i, &leaf, pos + 1);
            return;
        }
        if (self.used + 1) * 2 > self.cap {
            self.grow();
        }
        let mut i = self.start(&leaf);
        let fresh = loop {
            let v = self.slot(i).1;
            if v == 0 || v == u64::MAX {
                break v == 0;
            }
            i = (i + 1) & (self.cap - 1);
        };
        self.put_slot(i, &leaf, pos + 1);
        // a reused tombstone is already counted in `used`
        if fresh {
            self.used += 1;
            self.hdr().expect("index header");
        }
    }
    pub fn remove(&mut self, leaf: &[u8; 32]) -> Option<u64> {
        let i = self.find(leaf)?;
        let p = self.slot(i).1 - 1;
        self.put_slot(i, leaf, u64::MAX);
        // `used` counts occupied slots (tombstones included) so the load bound covers probe lengths too.
        Some(p)
    }
    fn grow(&mut self) {
        let mut live = std::vec::Vec::new();
        for i in 0..self.cap {
            let (k, v) = self.slot(i);
            if v != 0 && v != u64::MAX {
                live.push((k, v - 1));
            }
        }
        // Sized by the LIVE entries, not the old capacity (11-07): spends leave tombstones, so a full table is mostly
        // tombstones on a long run, and doubling it every time made the file track every coin EVER indexed (mainnet:
        // ~2.4 B, 100-200 GB) instead of the live set (~180 M). Rebuilt at the next power of two at or above 2.5x live,
        // it is at most 40% full afterwards, so the next rebuild (at 50%) is at least cap/10 fresh inserts away: never a
        // rebuild loop, amortised O(1). Mainnet ~180 M live -> 2^29 slots, ~21 GB (4x: ~43 GB; slice run 11-07).
        let cap = (live.len() as u64 * 5 / 2).next_power_of_two().max(1 << 16);
        let tmp = self.path.with_extension("grow");
        // A temp file left by a crash mid-grow may have a half-written header that open() refuses (review 11-03).
        let _ = std::fs::remove_file(&tmp);
        let mut n = DiskIndex::open(&tmp).expect("index grow");
        n.init(cap).expect("index grow");
        for (k, p) in live {
            n.insert(k, p);
        }
        n.sync().expect("index grow sync");
        drop(n);
        std::fs::rename(&tmp, &self.path).expect("index grow rename");
        *self = DiskIndex::open(&self.path).expect("index reopen");
    }
}

/// The host's position index, in memory or on disk.
pub enum AnyIndex {
    Mem(std::collections::HashMap<[u8; 32], u64>),
    Disk(DiskIndex),
}

impl AnyIndex {
    pub fn insert(&mut self, leaf: [u8; 32], pos: u64) {
        match self { AnyIndex::Mem(m) => { m.insert(leaf, pos); } AnyIndex::Disk(d) => d.insert(leaf, pos) }
    }
    pub fn remove(&mut self, leaf: &[u8; 32]) -> Option<u64> {
        match self { AnyIndex::Mem(m) => m.remove(leaf), AnyIndex::Disk(d) => d.remove(leaf) }
    }
    pub fn get(&self, leaf: &[u8; 32]) -> Option<u64> {
        match self { AnyIndex::Mem(m) => m.get(leaf).copied(), AnyIndex::Disk(d) => d.get(leaf) }
    }
}

#[cfg(test)]
mod index_tests {
    use super::*;
    #[test]
    fn disk_index_matches_a_hashmap() {
        let p = std::env::temp_dir().join(std::format!("libzync-ix-{}.bin", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let h = |i: u64| -> [u8; 32] { use sha2::Digest; sha2::Sha256::digest(i.to_le_bytes()).into() };
        let mut d = DiskIndex::open(&p).unwrap();
        let mut m = std::collections::HashMap::new();
        for i in 0..70_000u64 {
            d.insert(h(i), i);
            m.insert(h(i), i);
            if i % 3 == 0 {
                assert_eq!(d.remove(&h(i / 2)), m.remove(&h(i / 2)));
            }
        }
        for i in (0..70_000u64).step_by(97) {
            assert_eq!(d.get(&h(i)), m.get(&h(i)).copied(), "{i}");
        }
        assert_eq!(d.get(&h(1 << 40)), None);
        drop(d);
        let d = DiskIndex::open(&p).unwrap();
        assert_eq!(d.get(&h(69_999)), Some(69_999));
        let _ = std::fs::remove_file(&p);
    }

    /// A long run where most coins are spent: the file follows the LIVE set, not every coin ever indexed (11-07).
    #[test]
    fn disk_index_size_follows_the_live_set() {
        let p = std::env::temp_dir().join(std::format!("libzync-ix-live-{}.bin", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let h = |i: u64| -> [u8; 32] { use sha2::Digest; sha2::Sha256::digest(i.to_le_bytes()).into() };
        let mut d = DiskIndex::open(&p).unwrap();
        let mut m = std::collections::HashMap::new();
        // 400,000 coins created, each spent 1,000 later: never more than ~1,000 live
        for i in 0..400_000u64 {
            d.insert(h(i), i);
            m.insert(h(i), i);
            if i >= 1_000 { assert_eq!(d.remove(&h(i - 1_000)), m.remove(&h(i - 1_000))); }
        }
        for i in (398_000..400_000u64).step_by(7) { assert_eq!(d.get(&h(i)), m.get(&h(i)).copied(), "{i}"); }
        assert_eq!(d.get(&h(5)), None);
        let len = std::fs::metadata(&p).unwrap().len();
        // the minimum table (65,536 slots, 2.6 MB); doubling per rebuild reached 2^20 slots (42 MB) here
        assert!(len <= IX_HDR + (1 << 16) * IX_SLOT, "index file {len} bytes for ~1,000 live coins");
        drop(d);
        let d = DiskIndex::open(&p).unwrap();
        assert_eq!(d.get(&h(399_999)), Some(399_999));
        let _ = std::fs::remove_file(&p);
    }
}
