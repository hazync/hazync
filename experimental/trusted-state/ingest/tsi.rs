// EXPERIMENTAL Trusted State chunked ingest (docs/libzync/TRUSTED_STATE.md §12): what the chunk guest, the combine guest
// and the host planner (ts-anchor chunk-plan) must agree on, in ONE file, #[path]-included by all three (no_std + alloc).
//   - Sha256Stream: SHA-256 whose midstate can be carried between receipts (Core's hash_serialized_3 is one SHA-256d
//     over the whole coin stream, so a chunk boundary must cut it without finishing it);
//   - ChunkState: everything one chunk hands the next, with one canonical byte encoding and its digest;
//   - smt_bulk: the BIP30 tree's root from all its entries at once, sorted, with the empty-subtree hashes computed once
//     (the core's roots.rs per-insert `apply` recomputes all 256 of them twice per entry).
#[allow(unused_imports)]
use alloc::vec::Vec;
use sha2::{Digest, Sha256};

pub fn sha(b: &[u8]) -> [u8; 32] { Sha256::digest(b).into() }

/// SHA-256 with an exportable midstate: state words, the unhashed tail (< 64 bytes), the total length.
#[derive(Clone, PartialEq, Debug)]
pub struct Sha256Stream { pub st: [u32; 8], pub buf: Vec<u8>, pub len: u64 }

pub const SHA256_IV: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];

impl Sha256Stream {
    pub fn new() -> Self { Sha256Stream { st: SHA256_IV, buf: Vec::new(), len: 0 } }
    fn block(&mut self, b: &[u8]) {
        let mut a = sha2::digest::generic_array::GenericArray::<u8, sha2::digest::consts::U64>::default();
        a.copy_from_slice(b);
        sha2::compress256(&mut self.st, core::slice::from_ref(&a));
    }
    pub fn update(&mut self, mut d: &[u8]) {
        self.len += d.len() as u64;
        if !self.buf.is_empty() {
            let take = (64 - self.buf.len()).min(d.len());
            self.buf.extend_from_slice(&d[..take]);
            d = &d[take..];
            if self.buf.len() == 64 { let b = core::mem::take(&mut self.buf); self.block(&b); }
        }
        while d.len() >= 64 { let (h, t) = d.split_at(64); self.block(h); d = t; }
        self.buf.extend_from_slice(d);
    }
    pub fn finalize(mut self) -> [u8; 32] {
        let bits = self.len.wrapping_mul(8);
        let mut pad = alloc::vec![0x80u8];
        while (self.buf.len() + pad.len()) % 64 != 56 { pad.push(0); }
        pad.extend_from_slice(&bits.to_be_bytes());
        let mut b = core::mem::take(&mut self.buf);
        b.extend_from_slice(&pad);
        for c in b.chunks(64) { self.block(c); }
        let mut o = [0u8; 32];
        for (i, w) in self.st.iter().enumerate() { o[4 * i..4 * i + 4].copy_from_slice(&w.to_be_bytes()); }
        o
    }
}

/// What a chunk receives and hands on. `started` 0 only before the file header has been read (chunk 0's input).
#[derive(Clone, PartialEq, Debug)]
pub struct ChunkState {
    pub started: u32,
    pub magic: [u8; 4],
    pub base: [u8; 32],
    pub coins_total: u64,
    pub idx: u64,                 // coins consumed
    pub value: u64,
    // the txid group being read (Core's file: txid, CompactSize k, then k coins)
    pub g_txid: [u8; 32],
    pub g_left: u64,              // coins of the group still to come
    pub g_n: u32,                 // coins of the group read so far
    pub g_flags: u32,             // bit 0 a coinbase, bit 1 a tracked coin
    pub last_vout: Vec<u8>,       // the group's last encoded vout (Core's key order inside a group)
    pub h3: Sha256Stream,         // hash_serialized_3's inner SHA-256 over the TxOutSer stream
    pub live_set: u32,
    pub live_hash: [u32; 16],     // the ECMH partial sum (affine; the identity is live_set 0)
    pub log_n: u64,
    pub log_roots: Vec<Option<[u32; 8]>>, // the coin log's append forest, level-indexed
    pub trie_root: [u32; 8],
    pub smt_chain: [u8; 32],      // SHA-256 chain over the BIP30 entries emitted so far: c = SHA-256(c ‖ txid ‖ count LE)
    pub smt_n: u64,
}

impl ChunkState {
    /// The state before chunk 0: nothing read, empty trie (root words zero: ttree's empty trie), empty forest.
    pub fn initial() -> Self {
        ChunkState { started: 0, magic: [0; 4], base: [0; 32], coins_total: 0, idx: 0, value: 0, g_txid: [0; 32], g_left: 0,
                     g_n: 0, g_flags: 0, last_vout: Vec::new(), h3: Sha256Stream::new(), live_set: 0, live_hash: [0; 16],
                     log_n: 0, log_roots: Vec::new(), trie_root: [0; 8], smt_chain: [0; 32], smt_n: 0 }
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Vec::new();
        let w = |o: &mut Vec<u8>, x: &[u32]| for v in x { o.extend_from_slice(&v.to_le_bytes()); };
        o.extend_from_slice(&self.started.to_le_bytes());
        o.extend_from_slice(&self.magic);
        o.extend_from_slice(&self.base);
        for v in [self.coins_total, self.idx, self.value] { o.extend_from_slice(&v.to_le_bytes()); }
        o.extend_from_slice(&self.g_txid);
        o.extend_from_slice(&self.g_left.to_le_bytes());
        o.extend_from_slice(&self.g_n.to_le_bytes());
        o.extend_from_slice(&self.g_flags.to_le_bytes());
        o.push(self.last_vout.len() as u8);
        o.extend_from_slice(&self.last_vout);
        w(&mut o, &self.h3.st);
        o.push(self.h3.buf.len() as u8);
        o.extend_from_slice(&self.h3.buf);
        o.extend_from_slice(&self.h3.len.to_le_bytes());
        o.extend_from_slice(&self.live_set.to_le_bytes());
        w(&mut o, &self.live_hash);
        o.extend_from_slice(&self.log_n.to_le_bytes());
        o.push(self.log_roots.len() as u8);
        for r in &self.log_roots { match r { Some(x) => { o.push(1); w(&mut o, x); } None => o.push(0) } }
        w(&mut o, &self.trie_root);
        o.extend_from_slice(&self.smt_chain);
        o.extend_from_slice(&self.smt_n.to_le_bytes());
        o
    }
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        let mut i = 0usize;
        let mut t = |n: usize| -> Option<&[u8]> { let s = b.get(i..i + n)?; i += n; Some(s) };
        let u32_ = |s: &[u8]| u32::from_le_bytes(s.try_into().unwrap());
        let u64_ = |s: &[u8]| u64::from_le_bytes(s.try_into().unwrap());
        let words = |s: &[u8]| -> [u32; 8] { core::array::from_fn(|k| u32::from_le_bytes(s[4 * k..4 * k + 4].try_into().unwrap())) };
        let started = u32_(t(4)?);
        let magic = t(4)?.try_into().ok()?;
        let base = t(32)?.try_into().ok()?;
        let (coins_total, idx, value) = (u64_(t(8)?), u64_(t(8)?), u64_(t(8)?));
        let g_txid = t(32)?.try_into().ok()?;
        let g_left = u64_(t(8)?);
        let g_n = u32_(t(4)?);
        let g_flags = u32_(t(4)?);
        let lv = t(1)?[0] as usize;
        if lv > 9 { return None; }
        let last_vout = t(lv)?.to_vec();
        let st = words(t(32)?);
        let bl = t(1)?[0] as usize;
        if bl >= 64 { return None; }
        let buf = t(bl)?.to_vec();
        let len = u64_(t(8)?);
        let live_set = u32_(t(4)?);
        let lh = t(64)?;
        let live_hash = core::array::from_fn(|k| u32::from_le_bytes(lh[4 * k..4 * k + 4].try_into().unwrap()));
        let log_n = u64_(t(8)?);
        let nr = t(1)?[0] as usize;
        let mut log_roots = Vec::with_capacity(nr);
        for _ in 0..nr { log_roots.push(if t(1)?[0] == 1 { Some(words(t(32)?)) } else { None }); }
        let trie_root = words(t(32)?);
        let smt_chain = t(32)?.try_into().ok()?;
        let smt_n = u64_(t(8)?);
        let s = ChunkState { started, magic, base, coins_total, idx, value, g_txid, g_left, g_n, g_flags, last_vout,
                             h3: Sha256Stream { st, buf, len }, live_set, live_hash, log_n, log_roots, trie_root, smt_chain, smt_n };
        // canonical only: the bytes must be exactly this state's encoding
        if i != b.len() || s.to_bytes() != b { return None; }
        Some(s)
    }
    pub fn digest(&self) -> [u8; 32] { sha(&self.to_bytes()) }
    /// The group just finished (a new txid starts, or the file ends): a coinbase or tracked group becomes a BIP30 entry.
    pub fn close_group(&mut self) {
        if self.g_n > 0 && self.g_flags != 0 {
            let mut p = [0u8; 68];
            p[..32].copy_from_slice(&self.smt_chain);
            p[32..64].copy_from_slice(&self.g_txid);
            p[64..].copy_from_slice(&self.g_n.to_le_bytes());
            self.smt_chain = sha(&p);
            self.smt_n += 1;
        }
        self.g_n = 0;
        self.g_flags = 0;
    }
}

/// The root of a depth-`depth`-rooted subtree of the BIP30 sparse Merkle tree holding `e` (keys strictly increasing,
/// all sharing the first `depth` bits), from the core's own leaf and node hashes and its empty-subtree table `empty`
/// (computed ONCE by the caller). Equal to inserting the entries one by one with roots.rs `apply`, by construction of
/// that tree (a key's leaf sits at the end of its 256-bit path; every subtree without a key is `empty[height]`).
pub fn smt_bulk(e: &[([u8; 32], u32)], depth: usize, empty: &[[u8; 32]],
                leaf: &dyn Fn(&[u8; 32], u32) -> [u8; 32], node: &dyn Fn(&[u8; 32], &[u8; 32]) -> [u8; 32]) -> [u8; 32] {
    const D: usize = 256;
    let bit = |k: &[u8; 32], i: usize| (k[i / 8] >> (7 - (i % 8))) & 1 == 1;
    if e.is_empty() { return empty[D - depth]; }
    if e.len() == 1 {
        let (k, v) = &e[0];
        let mut n = leaf(k, *v);
        for d in (depth..D).rev() {
            let s = &empty[D - 1 - d];
            n = if bit(k, d) { node(s, &n) } else { node(&n, s) };
        }
        return n;
    }
    let m = e.partition_point(|x| !bit(&x.0, depth));
    let (l, r) = (smt_bulk(&e[..m], depth + 1, empty, leaf, node), smt_bulk(&e[m..], depth + 1, empty, leaf, node));
    node(&l, &r)
}

fn compress_blocks(blocks: &[[u8; 64]]) -> [u8; 32] {
    let mut st = SHA256_IV;
    for b in blocks {
        let mut a = sha2::digest::generic_array::GenericArray::<u8, sha2::digest::consts::U64>::default();
        a.copy_from_slice(b);
        sha2::compress256(&mut st, core::slice::from_ref(&a));
    }
    let mut o = [0u8; 32];
    for (i, w) in st.iter().enumerate() { o[4 * i..4 * i + 4].copy_from_slice(&w.to_be_bytes()); }
    o
}

/// roots.rs `hash_node` (SHA-256(0x11 ‖ L ‖ R), 65 bytes) as exactly two compressions with the padding laid out by hand.
pub fn node_raw(l: &[u8; 32], r: &[u8; 32]) -> [u8; 32] {
    let mut b1 = [0u8; 64];
    b1[0] = 0x11;
    b1[1..33].copy_from_slice(l);
    b1[33..64].copy_from_slice(&r[..31]);
    let mut b2 = [0u8; 64];
    b2[0] = r[31];
    b2[1] = 0x80;
    b2[56..].copy_from_slice(&(65u64 * 8).to_be_bytes());
    compress_blocks(&[b1, b2])
}

/// roots.rs `hash_leaf` (SHA-256(0x10 ‖ key ‖ value LE), 37 bytes) as one compression.
pub fn leaf_raw(k: &[u8; 32], v: u32) -> [u8; 32] {
    let mut b = [0u8; 64];
    b[0] = 0x10;
    b[1..33].copy_from_slice(k);
    b[33..37].copy_from_slice(&v.to_le_bytes());
    b[37] = 0x80;
    b[56..].copy_from_slice(&(37u64 * 8).to_be_bytes());
    compress_blocks(&[b])
}

/// The empty-subtree table with `node_raw`.
pub fn empty_raw() -> Vec<[u8; 32]> {
    let mut e = alloc::vec![[0u8; 32]; 257];
    for d in 1..=256 { e[d] = node_raw(&e[d - 1], &e[d - 1]); }
    e
}

// In the zkVM: the same two hashes with risc0's SHA accelerator called directly (as libzync coinlog.rs does), one call per
// 64-byte block, the state in the accelerator's form (its big-endian bytes in memory order).
#[cfg(target_os = "zkvm")]
extern "C" {
    fn sys_sha_compress(out: *mut [u32; 8], in_state: *const [u32; 8], b1: *const [u32; 8], b2: *const [u32; 8]);
}
#[cfg(target_os = "zkvm")]
fn mem_words(b: &[u8]) -> [u32; 8] { core::array::from_fn(|i| u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap())) }
#[cfg(target_os = "zkvm")]
fn sys_blocks(blocks: &[[u8; 64]]) -> [u8; 32] {
    let mut st: [u32; 8] = core::array::from_fn(|i| SHA256_IV[i].swap_bytes());
    for b in blocks {
        let (h1, h2) = (mem_words(&b[..32]), mem_words(&b[32..]));
        let mut o = [0u32; 8];
        unsafe { sys_sha_compress(&mut o, &st, &h1, &h2) };
        st = o;
    }
    let mut o = [0u8; 32];
    for (i, w) in st.iter().enumerate() { o[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes()); }
    o
}
#[cfg(target_os = "zkvm")]
pub fn node_sys(l: &[u8; 32], r: &[u8; 32]) -> [u8; 32] {
    let mut b1 = [0u8; 64];
    b1[0] = 0x11;
    b1[1..33].copy_from_slice(l);
    b1[33..64].copy_from_slice(&r[..31]);
    let mut b2 = [0u8; 64];
    b2[0] = r[31];
    b2[1] = 0x80;
    b2[56..].copy_from_slice(&(65u64 * 8).to_be_bytes());
    sys_blocks(&[b1, b2])
}
#[cfg(target_os = "zkvm")]
pub fn leaf_sys(k: &[u8; 32], v: u32) -> [u8; 32] {
    let mut b = [0u8; 64];
    b[0] = 0x10;
    b[1..33].copy_from_slice(k);
    b[33..37].copy_from_slice(&v.to_le_bytes());
    b[37] = 0x80;
    b[56..].copy_from_slice(&(37u64 * 8).to_be_bytes());
    sys_blocks(&[b])
}
#[cfg(target_os = "zkvm")]
pub fn empty_sys() -> Vec<[u8; 32]> {
    let mut e = alloc::vec![[0u8; 32]; 257];
    for d in 1..=256 { e[d] = node_sys(&e[d - 1], &e[d - 1]); }
    e
}

// ---------------------------------------------------------------------------------------------------------------------
// The split BIP30 receipt (TRUSTED_STATE.md §13.1): the tree's 2^d subtrees at depth d (keys grouped by their first d
// bits, big-endian) built by independent PIECE receipts, each a contiguous prefix range [p_lo, p_hi); the combine
// checks the pieces tile 0..2^d and carry, between them, the whole entry chain the chunks emitted, then folds the 2^d
// subtree roots up d levels.

/// A piece's journal: [d, p_lo, p_hi, e_in(8), e_out(8), e_n, r_in(8), r_out(8), r_n], then 8 words per subtree root.
pub const PIECE_HDR_WORDS: usize = 37;
pub const MAX_DEPTH: u32 = 16;

/// Cut bits of the `controls` build ONLY (see bip30/Cargo.toml): each names one check, which the build then skips.
pub const CUT_BOUNDS: u32 = 1;  // piece: 1 <= d <= MAX_DEPTH, p_lo < p_hi <= 2^d
pub const CUT_RANGE: u32 = 2;   // piece: every entry's prefix in [p_lo, p_hi)
pub const CUT_ORDER: u32 = 4;   // piece: each list strictly increasing, no key in both (no REPEATABLE txid with live coins)
pub const CUT_TILE: u32 = 8;    // combine: one depth; the ranges tile 0..2^d in order
pub const CUT_ECHAIN: u32 = 16; // combine: the entry chain runs from zero, links, and ends at the chunks' smt_chain
pub const CUT_ECOUNT: u32 = 32; // combine: the pieces' entry counts add up to the chunks' smt_n
pub const CUT_RCHAIN: u32 = 64; // combine: the REPEATABLE chain runs from zero and links

/// One step of an entry chain: SHA-256(c ‖ txid ‖ value LE). The chunks' smt_chain is this over (txid, live count) in
/// file order (`ChunkState::close_group`); a piece's REPEATABLE chain is this over (txid, REPEATABLE).
pub fn chain_step(c: &[u8; 32], k: &[u8; 32], v: u32) -> [u8; 32] {
    let mut p = [0u8; 68];
    p[..32].copy_from_slice(c);
    p[32..64].copy_from_slice(k);
    p[64..].copy_from_slice(&v.to_le_bytes());
    sha(&p)
}

/// The first `d` bits of a key, big-endian (the tree's path order: bit i is key[i/8] >> (7 - i%8)).
pub fn prefix_of(k: &[u8; 32], d: u32) -> u32 {
    if d == 0 { return 0; }
    (u32::from_be_bytes(k[..4].try_into().unwrap()) >> (32 - d)) as u32
}

/// The subtree roots at depth `d` of the prefixes p_lo..p_hi, from `e` sorted by key. An entry whose prefix is outside
/// the range is in no subtree (the piece refuses it unless CUT_RANGE).
pub fn piece_roots(e: &[([u8; 32], u32)], d: u32, p_lo: u32, p_hi: u32, empty: &[[u8; 32]],
                   leaf: &dyn Fn(&[u8; 32], u32) -> [u8; 32], node: &dyn Fn(&[u8; 32], &[u8; 32]) -> [u8; 32]) -> Vec<[u8; 32]> {
    let mut out = Vec::with_capacity((p_hi - p_lo) as usize);
    for p in p_lo..p_hi {
        let a = e.partition_point(|x| prefix_of(&x.0, d) < p);
        let b = e.partition_point(|x| prefix_of(&x.0, d) <= p);
        out.push(smt_bulk(&e[a..b.max(a)], d as usize, empty, leaf, node));
    }
    out
}

/// Fold subtree roots (prefix order) up to the root: pairs (2p, 2p+1) level by level. An odd tail is carried up
/// unpaired, which only a set that does not tile 0..2^d can reach (refused unless CUT_TILE).
pub fn smt_fold(mut r: Vec<[u8; 32]>, node: &dyn Fn(&[u8; 32], &[u8; 32]) -> [u8; 32]) -> [u8; 32] {
    if r.is_empty() { return [0; 32]; }
    while r.len() > 1 {
        r = r.chunks(2).map(|c| if c.len() == 2 { node(&c[0], &c[1]) } else { c[0] }).collect();
    }
    r[0]
}
