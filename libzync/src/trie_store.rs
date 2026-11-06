//! The host's counting-trie STORE (E2 step 3; host only, never compiled into a guest): the whole trie, every inner
//! node's hash cached, advanced one block at a time. Per block it serves the guest's witness (the old trie pruned to the
//! block's paths, the format `trie_model::encode` writes and `ttree::apply_stream_perm` reads), applies the block's
//! update list, and can undo it (a reorg; exact except over a BIP30 re-creation, see `undo`). It replaces `trie_model::synth` for real proofs.
//!
//! The trie's shape is a function of its key set (a subtree holding one leaf IS that leaf), so a store built by any
//! sequence of applies and undos that leaves the same coins has the same root. Undo removes an inserted coin and
//! collapses the chain above it back to that rule; an address left with no coins is removed.
//!
//! Nodes live in an ARENA of fixed 64-byte records addressed by index (`Arena`): in memory (`Vec`), or, with `std`, a
//! memory-mapped file (`TrieStore::open_dir`, mainnet: ~3.5 billion coins ever, hundreds of GB, ROADMAP E2). Inner
//! nodes keep their hash; coin and address leaves recompute theirs (one compression). Updates are COPY-ON-WRITE: a
//! block writes new nodes along its paths and the replaced ones are freed only when the store is next made durable
//! (`flush`), so an apply is all or nothing (a refused update drops its new nodes) and a crash leaves the last flushed
//! trie intact on disk.
//!
//! Snapshot (`to_bytes` / `from_bytes`): "HZTRIE01", counts u32, height u64, block hash 32 bytes, node count u64, the
//! trie in preorder (top: 0 empty | 2 inner L R | 3 address S count value, coin trie; coin trie: 0 empty | 2 inner L R |
//! 4 coin O, spent flag, spender, value; words little-endian), then the root (8 words). Load recomputes every hash and
//! refuses a snapshot whose root does not match, or whose shape breaks the one-leaf rule or the key prefixes.

use crate::coinlog::{parent_w, W};
use crate::trie_model::{Sub, Top};
use crate::ttree::{addr_into, bit, coin_into, split, Key, TErr};
use alloc::boxed::Box;
use alloc::vec::Vec;

const ZERO: W = [0; 8];

/// A node reference: 0 = empty, else the arena index + 1.
type R = u64;
const EMPTY: R = 0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Node {
    SubInner { l: R, r: R, h: W },
    /// a coin; `sp` is the spender record (words 0-4; 5-7 are zero by the apply check)
    Coin { o: W, sp: Option<W>, value: u64 },
    TopInner { l: R, r: R, h: W },
    Addr { s: W, count: u64, value: u64, sub: R },
}

pub const RECORD: usize = 64;

#[cfg(feature = "std")]
impl Node {
    fn enc(&self) -> [u8; RECORD] {
        let mut b = [0u8; RECORD];
        let w = |b: &mut [u8], x: &W| for (i, v) in x.iter().enumerate() { b[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes()); };
        match self {
            Node::SubInner { l, r, h } | Node::TopInner { l, r, h } => {
                b[0] = if matches!(self, Node::SubInner { .. }) { 1 } else { 3 };
                b[8..16].copy_from_slice(&l.to_le_bytes());
                b[16..24].copy_from_slice(&r.to_le_bytes());
                w(&mut b[24..56], h);
            }
            Node::Coin { o, sp, value } => {
                b[0] = 2;
                b[1] = sp.is_some() as u8;
                w(&mut b[4..36], o);
                if let Some(s) = sp { for (i, v) in s[..5].iter().enumerate() { b[36 + 4 * i..40 + 4 * i].copy_from_slice(&v.to_le_bytes()); } }
                b[56..64].copy_from_slice(&value.to_le_bytes());
            }
            Node::Addr { s, count, value, sub } => {
                b[0] = 4;
                w(&mut b[8..40], s);
                b[40..48].copy_from_slice(&count.to_le_bytes());
                b[48..56].copy_from_slice(&value.to_le_bytes());
                b[56..64].copy_from_slice(&sub.to_le_bytes());
            }
        }
        b
    }
    fn dec(b: &[u8]) -> Node {
        let u64at = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        let wat = |i: usize| -> W { core::array::from_fn(|k| u32::from_le_bytes(b[i + 4 * k..i + 4 * k + 4].try_into().unwrap())) };
        match b[0] {
            1 => Node::SubInner { l: u64at(8), r: u64at(16), h: wat(24) },
            3 => Node::TopInner { l: u64at(8), r: u64at(16), h: wat(24) },
            2 => {
                let sp = (b[1] == 1).then(|| {
                    let mut s = ZERO;
                    for (i, v) in s[..5].iter_mut().enumerate() { *v = u32::from_le_bytes(b[36 + 4 * i..40 + 4 * i].try_into().unwrap()); }
                    s
                });
                Node::Coin { o: wat(4), sp, value: u64at(56) }
            }
            4 => Node::Addr { s: wat(8), count: u64at(40), value: u64at(48), sub: u64at(56) },
            t => panic!("trie arena: record tag {t} (a corrupt arena)"),
        }
    }
}

/// Where the nodes live. `put` takes a free slot (only slots no flushed trie refers to) or appends.
pub trait Arena {
    fn get(&self, r: R) -> Node;
    fn put(&mut self, n: Node) -> R;
    /// The slot may be reused (from now on in memory; after the next flush on disk).
    fn free(&mut self, r: R);
    /// Slots in use (records ever appended minus free ones).
    fn live(&self) -> u64;
    /// Make the trie with this root durable (a disk arena: records synced, then the meta written atomically, and only
    /// then are the slots freed since the last flush reusable). In memory: nothing to do.
    fn durable(&mut self, _root: R, _counts: bool, _height: u64, _hash: &[u8; 32]) -> Result<(), alloc::string::String> { Ok(()) }
}

#[derive(Default)]
pub struct MemArena { nodes: Vec<Node>, free: Vec<R> }

impl Arena for MemArena {
    fn get(&self, r: R) -> Node { self.nodes[(r - 1) as usize] }
    fn put(&mut self, n: Node) -> R {
        if let Some(r) = self.free.pop() { self.nodes[(r - 1) as usize] = n; return r; }
        self.nodes.push(n);
        self.nodes.len() as R
    }
    fn free(&mut self, r: R) { self.free.push(r); }
    fn live(&self) -> u64 { (self.nodes.len() - self.free.len()) as u64 }
}

/// The disk arena: `nodes.bin` (64-byte records, memory-mapped, grown in steps) and `meta` (the flushed root, height,
/// block hash, counts flag, record count and free slots). Slots freed since the last flush are held back, so nothing the
/// flushed trie refers to is ever overwritten: a crash leaves that trie intact.
#[cfg(feature = "std")]
pub struct DiskArena {
    dir: std::path::PathBuf,
    file: std::fs::File,
    map: memmap2::MmapMut,
    len: u64,
    free: Vec<R>,
    pending: Vec<R>,
}

#[cfg(feature = "std")]
impl DiskArena {
    const STEP: u64 = 1 << 20; // records per growth step (64 MiB)
    const MAGIC: &'static [u8; 8] = b"HZTRIED1";

    fn map(file: &std::fs::File) -> std::io::Result<memmap2::MmapMut> { unsafe { memmap2::MmapMut::map_mut(file) } }

    /// Open (or create) the arena in `dir`; returns it with the flushed (root, counts, height, hash), if any.
    pub fn open(dir: &std::path::Path) -> std::io::Result<(Self, Option<(R, bool, u64, [u8; 32])>)> {
        std::fs::create_dir_all(dir)?;
        let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(dir.join("nodes.bin"))?;
        if file.metadata()?.len() < Self::STEP * RECORD as u64 { file.set_len(Self::STEP * RECORD as u64)?; }
        let map = Self::map(&file)?;
        let mut a = DiskArena { dir: dir.to_path_buf(), file, map, len: 0, free: Vec::new(), pending: Vec::new() };
        let meta = match std::fs::read(dir.join("meta")) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
            Ok(b) => {
                let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, "trie arena meta is corrupt");
                if b.len() < 8 + 8 + 1 + 8 + 32 + 8 + 8 || &b[..8] != Self::MAGIC { return Err(bad()); }
                let u = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
                let root = u(8);
                if b[16] > 1 { return Err(bad()); }
                let counts = b[16] == 1;
                let height = u(17);
                let hash: [u8; 32] = b[25..57].try_into().unwrap();
                a.len = u(57);
                let nfree = u(65);
                // every reference in range: the root (0 = empty) and each free slot (1..=len); no size overflow
                let fits = nfree.checked_mul(8).and_then(|x| x.checked_add(73)) == Some(b.len() as u64);
                let flen = a.file.metadata()?.len();
                if !fits || a.len.checked_mul(RECORD as u64).map_or(true, |x| x > flen)
                    || root > a.len || nfree > a.len { return Err(bad()); }
                a.free = (0..nfree as usize).map(|i| u(73 + 8 * i)).collect();
                if a.free.iter().any(|&r| r == 0 || r > a.len || r == root) { return Err(bad()); }
                Some((root, counts, height, hash))
            }
        };
        Ok((a, meta))
    }

    fn grow(&mut self) {
        let cap = self.file.metadata().expect("trie arena: stat").len() / RECORD as u64;
        self.map.flush().expect("trie arena: msync");
        self.file.set_len((cap + Self::STEP) * RECORD as u64).expect("trie arena: grow the file");
        self.map = Self::map(&self.file).expect("trie arena: remap");
    }
}

#[cfg(feature = "std")]
impl Arena for DiskArena {
    fn get(&self, r: R) -> Node {
        let i = (r - 1) as usize * RECORD;
        Node::dec(&self.map[i..i + RECORD])
    }
    fn put(&mut self, n: Node) -> R {
        let r = match self.free.pop() {
            Some(r) => r,
            None => {
                if (self.len + 1) * RECORD as u64 > self.map.len() as u64 { self.grow(); }
                self.len += 1;
                self.len
            }
        };
        let i = (r - 1) as usize * RECORD;
        self.map[i..i + RECORD].copy_from_slice(&n.enc());
        r
    }
    fn free(&mut self, r: R) { self.pending.push(r); }
    fn live(&self) -> u64 { self.len - (self.free.len() + self.pending.len()) as u64 }
    fn durable(&mut self, root: R, counts: bool, height: u64, hash: &[u8; 32]) -> Result<(), alloc::string::String> {
        use std::io::Write as _;
        let e = |x: std::io::Error| alloc::format!("trie arena {}: {x}", self.dir.display());
        self.map.flush().map_err(e)?;
        self.file.sync_all().map_err(e)?; // the file's size after a grow, not only its pages
        let all: Vec<R> = self.free.iter().chain(self.pending.iter()).copied().collect();
        let mut b = Vec::with_capacity(73 + 8 * all.len());
        b.extend_from_slice(Self::MAGIC);
        b.extend_from_slice(&root.to_le_bytes());
        b.push(counts as u8);
        b.extend_from_slice(&height.to_le_bytes());
        b.extend_from_slice(hash);
        b.extend_from_slice(&self.len.to_le_bytes());
        b.extend_from_slice(&(all.len() as u64).to_le_bytes());
        for r in &all { b.extend_from_slice(&r.to_le_bytes()); }
        let tmp = self.dir.join("meta.tmp");
        let mut f = std::fs::File::create(&tmp).map_err(e)?;
        f.write_all(&b).map_err(e)?;
        f.sync_data().map_err(e)?;
        std::fs::rename(&tmp, self.dir.join("meta")).map_err(e)?;
        std::fs::File::open(&self.dir).and_then(|d| d.sync_all()).map_err(e)?;
        self.free = all;
        self.pending.clear();
        Ok(())
    }
}

fn inner(a: &W, b: &W) -> W {
    let mut o = ZERO;
    parent_w(50, a, b, &mut o);
    o
}

/// The whole trie, as of `height` (block `hash`; zero for an empty store).
pub struct TrieStore {
    counts: bool,
    pub height: u64,
    pub hash: [u8; 32],
    root: R,
    a: Box<dyn Arena + Send>,
    /// this update's new nodes (dropped if it is refused) and the nodes it replaced (freed once it stands)
    fresh: Vec<R>,
    garbage: Vec<R>,
    /// the update being applied re-created a coin over its spent record; `last_replaced`: the last one applied did
    replaced: bool,
    last_replaced: bool,
}

/// One block's update list, as the aggregate's collect run logs it and the trie receipt commits.
pub type Marks = [(Key, W)];
pub type Inserts = [(Key, Option<W>, u64)];

impl TrieStore {
    fn with_arena(counts: bool, a: Box<dyn Arena + Send>) -> Self {
        TrieStore { counts, height: 0, hash: [0; 32], root: EMPTY, a, fresh: Vec::new(), garbage: Vec::new(), replaced: false, last_replaced: false }
    }
    pub fn new(counts: bool) -> Self { Self::with_arena(counts, Box::new(MemArena::default())) }

    /// The store kept on disk in `dir` (trie_store::DiskArena): the trie last flushed there, or a new empty one.
    #[cfg(feature = "std")]
    pub fn open_dir(dir: &std::path::Path, counts: bool) -> std::io::Result<Self> {
        let (a, meta) = DiskArena::open(dir)?;
        let mut st = Self::with_arena(counts, Box::new(a));
        if let Some((root, c, height, hash)) = meta {
            if c != counts { return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "trie arena: counts mode differs")); }
            (st.root, st.height, st.hash) = (root, height, hash);
        }
        Ok(st)
    }

    /// Make the current trie (root, height, hash) durable: a disk store survives a crash at this point. In memory: no-op.
    pub fn flush(&mut self) -> Result<(), alloc::string::String> {
        self.a.durable(self.root, self.counts, self.height, &self.hash)
    }
    pub fn counts(&self) -> bool { self.counts }
    pub fn root(&self) -> W { self.top_h(self.root) }
    /// Nodes in use (for sizing: bytes on disk = this x RECORD, plus free slots).
    pub fn nodes(&self) -> u64 { self.a.live() }

    fn get(&self, r: R) -> Node { self.a.get(r) }
    fn put(&mut self, n: Node) -> R { let r = self.a.put(n); self.fresh.push(r); r }
    fn drop_node(&mut self, r: R) { self.garbage.push(r); }

    fn sub_h(&self, r: R) -> W {
        if r == EMPTY { return ZERO; }
        match self.get(r) {
            Node::SubInner { h, .. } => h,
            Node::Coin { o, sp, value } => { let mut h = ZERO; coin_into(&o, sp.as_ref(), value, self.counts, &mut h); h }
            n => panic!("trie: a coin trie holds {n:?}"),
        }
    }
    fn top_h(&self, r: R) -> W {
        if r == EMPTY { return ZERO; }
        match self.get(r) {
            Node::TopInner { h, .. } => h,
            Node::Addr { s, count, value, sub } => { let mut h = ZERO; addr_into(&s, &self.sub_h(sub), count, value, self.counts, &mut h); h }
            n => panic!("trie: the address trie holds {n:?}"),
        }
    }
    fn sinner(&mut self, l: R, r: R) -> R { let h = inner(&self.sub_h(l), &self.sub_h(r)); self.put(Node::SubInner { l, r, h }) }
    fn tinner(&mut self, l: R, r: R) -> R { let h = inner(&self.top_h(l), &self.top_h(r)); self.put(Node::TopInner { l, r, h }) }
    fn is_sub_leaf(&self, r: R) -> bool { r != EMPTY && matches!(self.get(r), Node::Coin { .. }) }
    fn is_top_leaf(&self, r: R) -> bool { r != EMPTY && matches!(self.get(r), Node::Addr { .. }) }

    // ---- apply (as trie_model's mark / ins), copy-on-write: each returns the subtree's new reference

    fn s_mark(&mut self, n: R, o: &W, sp: &W, d: u32) -> Result<(R, u64), TErr> {
        if n == EMPTY { return Err(TErr::Missing); }
        match self.get(n) {
            Node::SubInner { l, r, .. } => {
                if d >= 256 { return Err(TErr::Witness); }
                let (l, r, v) = if bit(o, d) { let (x, v) = self.s_mark(r, o, sp, d + 1)?; (l, x, v) } else { let (x, v) = self.s_mark(l, o, sp, d + 1)?; (x, r, v) };
                self.drop_node(n);
                Ok((self.sinner(l, r), v))
            }
            Node::Coin { o: x, sp: None, value } if x == *o => {
                self.drop_node(n);
                Ok((self.put(Node::Coin { o: *o, sp: Some(*sp), value }), value))
            }
            _ => Err(TErr::Missing),
        }
    }

    fn s_ins(&mut self, n: R, o: &W, sp: &Option<W>, v: u64, d: u32) -> Result<R, TErr> {
        // depth as the guest and the model (trie_model::ins_sub)
        if n == EMPTY { return Ok(self.put(Node::Coin { o: *o, sp: *sp, value: v })); }
        match self.get(n) {
            Node::SubInner { .. } if d >= 256 => Err(TErr::Witness),
            Node::SubInner { l, r, .. } => {
                let (l, r) = if bit(o, d) { (l, self.s_ins(r, o, sp, v, d + 1)?) } else { (self.s_ins(l, o, sp, v, d + 1)?, r) };
                self.drop_node(n);
                Ok(self.sinner(l, r))
            }
            Node::Coin { o: x, sp: old_sp, .. } => {
                // BIP30: a coin created again over its own spent record replaces it (ttree.rs, the coin leaf case)
                if x == *o && old_sp.is_some() {
                    self.replaced = true;
                    self.drop_node(n);
                    return Ok(self.put(Node::Coin { o: *o, sp: *sp, value: v }));
                }
                if x == *o || d >= 256 { return Err(TErr::Duplicate); }
                // the existing coin moves one level down, on its own side, and the new one goes in beside or below it
                let t = if bit(&x, d) { self.sinner(EMPTY, n) } else { self.sinner(n, EMPTY) };
                self.s_ins(t, o, sp, v, d)
            }
            n2 => panic!("trie: a coin trie holds {n2:?}"),
        }
    }

    fn t_mark(&mut self, n: R, s: &W, o: &W, sp: &W, d: u32) -> Result<R, TErr> {
        if n == EMPTY { return Err(TErr::Missing); }
        match self.get(n) {
            Node::TopInner { l, r, .. } => {
                if d >= 256 { return Err(TErr::Witness); }
                let (l, r) = if bit(s, d) { (l, self.t_mark(r, s, o, sp, d + 1)?) } else { (self.t_mark(l, s, o, sp, d + 1)?, r) };
                self.drop_node(n);
                Ok(self.tinner(l, r))
            }
            Node::Addr { s: a, count, value, sub } if a == *s => {
                let (sub, v) = self.s_mark(sub, o, sp, 0)?;
                let count = count.checked_sub(1).ok_or(TErr::Count)?;
                let value = value.checked_sub(v).ok_or(TErr::Count)?;
                self.drop_node(n);
                Ok(self.put(Node::Addr { s: a, count, value, sub }))
            }
            _ => Err(TErr::Missing),
        }
    }

    fn t_ins(&mut self, n: R, s: &W, o: &W, sp: &Option<W>, v: u64, d: u32) -> Result<R, TErr> {
        if d >= 256 { return Err(TErr::Witness); }
        let (dc, dv) = if sp.is_none() { (1u64, v) } else { (0, 0) };
        if n == EMPTY {
            let sub = self.put(Node::Coin { o: *o, sp: *sp, value: v });
            return Ok(self.put(Node::Addr { s: *s, count: dc, value: dv, sub }));
        }
        match self.get(n) {
            Node::TopInner { l, r, .. } => {
                let (l, r) = if bit(s, d) { (l, self.t_ins(r, s, o, sp, v, d + 1)?) } else { (self.t_ins(l, s, o, sp, v, d + 1)?, r) };
                self.drop_node(n);
                Ok(self.tinner(l, r))
            }
            Node::Addr { s: a, count, value, sub } => {
                if a == *s {
                    // checked before the coin goes in, so a refused insert leaves the store as it was
                    let nc = count.checked_add(dc).ok_or(TErr::Count)?;
                    let nv = value.checked_add(dv).ok_or(TErr::Count)?;
                    let sub = self.s_ins(sub, o, sp, v, 0)?;
                    self.drop_node(n);
                    return Ok(self.put(Node::Addr { s: a, count: nc, value: nv, sub }));
                }
                let t = if bit(&a, d) { self.tinner(EMPTY, n) } else { self.tinner(n, EMPTY) };
                self.t_ins(t, s, o, sp, v, d)
            }
            n2 => panic!("trie: the address trie holds {n2:?}"),
        }
    }

    // ---- undo (a reorg): unmark a spend, remove an inserted coin and collapse the chain above it

    fn s_unmark(&mut self, n: R, o: &W, d: u32) -> Result<(R, u64), TErr> {
        if n == EMPTY { return Err(TErr::Missing); }
        match self.get(n) {
            Node::SubInner { l, r, .. } => {
                if d >= 256 { return Err(TErr::Witness); }
                let (l, r, v) = if bit(o, d) { let (x, v) = self.s_unmark(r, o, d + 1)?; (l, x, v) } else { let (x, v) = self.s_unmark(l, o, d + 1)?; (x, r, v) };
                self.drop_node(n);
                Ok((self.sinner(l, r), v))
            }
            Node::Coin { o: x, sp: Some(_), value } if x == *o => {
                self.drop_node(n);
                Ok((self.put(Node::Coin { o: *o, sp: None, value }), value))
            }
            _ => Err(TErr::Missing),
        }
    }

    /// After a removal under an inner node: a subtree left with one leaf becomes that leaf, with none becomes empty.
    fn s_collapse(&mut self, l: R, r: R) -> R {
        match (l, r) {
            (EMPTY, EMPTY) => EMPTY,
            (EMPTY, x) | (x, EMPTY) if self.is_sub_leaf(x) => x,
            _ => self.sinner(l, r),
        }
    }
    fn t_collapse(&mut self, l: R, r: R) -> R {
        match (l, r) {
            (EMPTY, EMPTY) => EMPTY,
            (EMPTY, x) | (x, EMPTY) if self.is_top_leaf(x) => x,
            _ => self.tinner(l, r),
        }
    }

    /// Remove coin `o`; returns (the new subtree, value, it was unspent).
    fn s_rm(&mut self, n: R, o: &W, d: u32) -> Result<(R, u64, bool), TErr> {
        if n == EMPTY { return Err(TErr::Missing); }
        match self.get(n) {
            Node::Coin { o: x, sp, value } if x == *o => { self.drop_node(n); Ok((EMPTY, value, sp.is_none())) }
            Node::SubInner { l, r, .. } => {
                if d >= 256 { return Err(TErr::Witness); }
                let (l, r, v, u) = if bit(o, d) { let (x, v, u) = self.s_rm(r, o, d + 1)?; (l, x, v, u) } else { let (x, v, u) = self.s_rm(l, o, d + 1)?; (x, r, v, u) };
                self.drop_node(n);
                Ok((self.s_collapse(l, r), v, u))
            }
            _ => Err(TErr::Missing),
        }
    }

    fn t_unmark(&mut self, n: R, s: &W, o: &W, d: u32) -> Result<R, TErr> {
        if n == EMPTY { return Err(TErr::Missing); }
        match self.get(n) {
            Node::TopInner { l, r, .. } => {
                if d >= 256 { return Err(TErr::Witness); }
                let (l, r) = if bit(s, d) { (l, self.t_unmark(r, s, o, d + 1)?) } else { (self.t_unmark(l, s, o, d + 1)?, r) };
                self.drop_node(n);
                Ok(self.tinner(l, r))
            }
            Node::Addr { s: a, count, value, sub } if a == *s => {
                let (sub, v) = self.s_unmark(sub, o, 0)?;
                let count = count.checked_add(1).ok_or(TErr::Count)?;
                let value = value.checked_add(v).ok_or(TErr::Count)?;
                self.drop_node(n);
                Ok(self.put(Node::Addr { s: a, count, value, sub }))
            }
            _ => Err(TErr::Missing),
        }
    }

    fn t_rm(&mut self, n: R, s: &W, o: &W, d: u32) -> Result<R, TErr> {
        if n == EMPTY { return Err(TErr::Missing); }
        match self.get(n) {
            Node::Addr { s: a, count, value, sub } if a == *s => {
                let (sub, v, unspent) = self.s_rm(sub, o, 0)?;
                let (count, value) = if unspent {
                    (count.checked_sub(1).ok_or(TErr::Count)?, value.checked_sub(v).ok_or(TErr::Count)?)
                } else { (count, value) };
                self.drop_node(n);
                Ok(if sub == EMPTY { EMPTY } else { self.put(Node::Addr { s: a, count, value, sub }) })
            }
            Node::TopInner { l, r, .. } => {
                if d >= 256 { return Err(TErr::Witness); }
                let (l, r) = if bit(s, d) { (l, self.t_rm(r, s, o, d + 1)?) } else { (self.t_rm(l, s, o, d + 1)?, r) };
                self.drop_node(n);
                Ok(self.t_collapse(l, r))
            }
            _ => Err(TErr::Missing),
        }
    }

    // ---- the witness: expand every node a key's path passes through, prune the rest (sorted keys)

    fn s_wit(&self, n: R, ks: &[W], d: u32) -> Sub {
        if n == EMPTY { return Sub::Empty; }
        if ks.is_empty() { return Sub::Pruned(self.sub_h(n)); }
        match self.get(n) {
            Node::SubInner { l, r, .. } => {
                let m = ks.partition_point(|k| !bit(k, d));
                Sub::Inner(Box::new(self.s_wit(l, &ks[..m], d + 1)), Box::new(self.s_wit(r, &ks[m..], d + 1)))
            }
            Node::Coin { o, sp, value } => Sub::Coin { o, sp, value },
            n2 => panic!("trie: a coin trie holds {n2:?}"),
        }
    }

    fn t_wit(&self, n: R, ks: &[(W, W)], d: u32) -> Top {
        if n == EMPTY { return Top::Empty; }
        if ks.is_empty() { return Top::Pruned(self.top_h(n)); }
        match self.get(n) {
            Node::TopInner { l, r, .. } => {
                let m = ks.partition_point(|k| !bit(&k.0, d));
                Top::Inner(Box::new(self.t_wit(l, &ks[..m], d + 1)), Box::new(self.t_wit(r, &ks[m..], d + 1)))
            }
            Node::Addr { s, count, value, sub } => {
                // the keys at this address reach into its coin trie; others only meet the leaf (pruned coin trie)
                let os: Vec<W> = ks.iter().filter(|k| k.0 == s).map(|k| k.1).collect();
                Top::Addr { s, count, value, sub: Box::new(self.s_wit(sub, &os, 0)) }
            }
            n2 => panic!("trie: the address trie holds {n2:?}"),
        }
    }

    // ---- fact proofs (trie_fact.rs): one address's leaf, with its coin trie pruned or whole

    fn s_full(&self, n: R) -> Sub {
        if n == EMPTY { return Sub::Empty; }
        match self.get(n) {
            Node::SubInner { l, r, .. } => Sub::Inner(Box::new(self.s_full(l)), Box::new(self.s_full(r))),
            Node::Coin { o, sp, value } => Sub::Coin { o, sp, value },
            n2 => panic!("trie: a coin trie holds {n2:?}"),
        }
    }

    fn t_addr(&self, n: R, s: &W, d: u32, whole: bool) -> Top {
        if n == EMPTY { return Top::Empty; }
        let side = |x: R| if x == EMPTY { Top::Empty } else { Top::Pruned(self.top_h(x)) };
        match self.get(n) {
            Node::TopInner { l, r, .. } => if bit(s, d) {
                Top::Inner(Box::new(side(l)), Box::new(self.t_addr(r, s, d + 1, whole)))
            } else {
                Top::Inner(Box::new(self.t_addr(l, s, d + 1, whole)), Box::new(side(r)))
            },
            Node::Addr { s: a, count, value, sub } => Top::Addr { s: a, count, value,
                sub: Box::new(if a == *s && whole { self.s_full(sub) } else { Sub::Pruned(self.sub_h(sub)) }) },
            n2 => panic!("trie: the address trie holds {n2:?}"),
        }
    }

    // ---- snapshot

    fn s_enc(&self, n: R, o: &mut Wr) {
        o.1 += 1;
        if n == EMPTY { return o.u32(0); }
        match self.get(n) {
            Node::SubInner { l, r, .. } => { o.u32(2); self.s_enc(l, o); self.s_enc(r, o); }
            Node::Coin { o: k, sp, value } => { o.u32(4); o.w(&k); o.u32(sp.is_some() as u32); o.w(&sp.unwrap_or(ZERO)); o.u64(value); }
            n2 => panic!("trie: a coin trie holds {n2:?}"),
        }
    }

    fn t_enc(&self, n: R, o: &mut Wr) {
        o.1 += 1;
        if n == EMPTY { return o.u32(0); }
        match self.get(n) {
            Node::TopInner { l, r, .. } => { o.u32(2); self.t_enc(l, o); self.t_enc(r, o); }
            Node::Addr { s, count, value, sub } => { o.u32(3); o.w(&s); o.u64(count); o.u64(value); self.s_enc(sub, o); }
            n2 => panic!("trie: the address trie holds {n2:?}"),
        }
    }

    /// Returns (the node, how many leaves it holds: 0, 1 or 2 for more).
    fn s_dec(&mut self, r: &mut Rd, d: u32, pre: W) -> Result<(R, u8), TErr> {
        if d > 256 { return Err(TErr::Witness); }
        match r.u32()? {
            0 => Ok((EMPTY, 0)),
            2 => {
                if d >= 256 { return Err(TErr::Witness); }
                let mut rp = pre;
                rp[(d / 32) as usize] |= 1 << (31 - d % 32);
                let (l, a) = self.s_dec(r, d + 1, pre)?;
                let (rr, b) = self.s_dec(r, d + 1, rp)?;
                if a + b < 2 { return Err(TErr::Witness); }
                Ok((self.sinner(l, rr), 2))
            }
            4 => {
                let o = r.w()?;
                let f = r.u32()?;
                let sp = r.w()?;
                let value = r.u64()?;
                // the spender record is words 0-4; the leaf hash carries the value and flag in 5-7, so those must be zero here
                if f > 1 || (f == 0 && sp != ZERO) || sp[5..] != [0; 3] || !on_path(&o, &pre, d) { return Err(TErr::Witness); }
                Ok((self.put(Node::Coin { o, sp: (f == 1).then_some(sp), value }), 1))
            }
            _ => Err(TErr::Witness),
        }
    }

    /// The unspent count and value under a coin trie (the address checks above it).
    fn s_unspent(&self, n: R) -> (u64, u64) {
        if n == EMPTY { return (0, 0); }
        match self.get(n) {
            Node::SubInner { l, r, .. } => { let (a, b) = (self.s_unspent(l), self.s_unspent(r)); (a.0 + b.0, a.1.saturating_add(b.1)) }
            Node::Coin { sp, value, .. } => if sp.is_none() { (1, value) } else { (0, 0) },
            n2 => panic!("trie: a coin trie holds {n2:?}"),
        }
    }

    fn t_dec(&mut self, r: &mut Rd, d: u32, pre: W) -> Result<(R, u8), TErr> {
        if d > 256 { return Err(TErr::Witness); }
        match r.u32()? {
            0 => Ok((EMPTY, 0)),
            2 => {
                if d >= 256 { return Err(TErr::Witness); }
                let mut rp = pre;
                rp[(d / 32) as usize] |= 1 << (31 - d % 32);
                let (l, a) = self.t_dec(r, d + 1, pre)?;
                let (rr, b) = self.t_dec(r, d + 1, rp)?;
                if a + b < 2 { return Err(TErr::Witness); }
                Ok((self.tinner(l, rr), 2))
            }
            3 => {
                let s = r.w()?;
                let count = r.u64()?;
                let value = r.u64()?;
                if !on_path(&s, &pre, d) { return Err(TErr::Witness); }
                let (sub, _) = self.s_dec(r, 0, ZERO)?;
                if sub == EMPTY || self.s_unspent(sub) != (count, value) { return Err(TErr::Count); }
                Ok((self.put(Node::Addr { s, count, value, sub }), 1))
            }
            _ => Err(TErr::Witness),
        }
    }

    // ---- the API

    /// The guest's witness for this update list (`trie_model::encode` format), against the current root.
    pub fn witness(&self, marks: &Marks, inserts: &Inserts) -> Vec<u32> {
        let mut ks: Vec<(W, W)> = marks.iter().map(|x| split(&x.0)).chain(inserts.iter().map(|x| split(&x.0))).collect();
        ks.sort_unstable();
        crate::trie_model::encode(self.counts, &self.root(), &self.t_wit(self.root, &ks, 0))
    }

    /// A proof (trie_fact::coin) for each coin key: present, spent or absent.
    pub fn prove_coins(&self, keys: &[Key]) -> Vec<u32> {
        let mut ks: Vec<(W, W)> = keys.iter().map(split).collect();
        ks.sort_unstable();
        ks.dedup();
        crate::trie_model::encode(self.counts, &self.root(), &self.t_wit(self.root, &ks, 0))
    }

    /// A balance proof (trie_fact::balance) for the script hash `s`: its address leaf, coin trie pruned.
    pub fn prove_addr(&self, s: &[u8; 32]) -> Vec<u32> {
        crate::trie_model::encode(self.counts, &self.root(), &self.t_addr(self.root, &words_be(s), 0, false))
    }

    /// A history proof (trie_fact::history): the address leaf with its WHOLE coin trie.
    pub fn prove_history(&self, s: &[u8; 32]) -> Vec<u32> {
        crate::trie_model::encode(self.counts, &self.root(), &self.t_addr(self.root, &words_be(s), 0, true))
    }

    /// The end of an update: refused, its new nodes go back; accepted, the root moves and the replaced nodes are freed
    /// (the arena reuses a slot only once no flushed trie can refer to it).
    fn finish(&mut self, r: Result<R, TErr>) -> Result<W, TErr> {
        match r {
            Err(e) => {
                for x in core::mem::take(&mut self.fresh) { self.a.free(x); }
                self.garbage.clear();
                Err(e)
            }
            Ok(t) => {
                self.root = t;
                self.fresh.clear();
                for x in core::mem::take(&mut self.garbage) { self.a.free(x); }
                Ok(self.root())
            }
        }
    }

    /// Apply one block's update list (marks, then inserts). All or nothing: on an error the store is as it was.
    pub fn apply(&mut self, marks: &Marks, inserts: &Inserts) -> Result<W, TErr> {
        // a spender record is words 0-4 (the leaf hash overwrites 5-7): refuse more, so the store stays loadable
        let wide = |sp: &W| sp[5..] != [0; 3];
        if marks.iter().any(|m| wide(&m.1)) || inserts.iter().any(|i| i.1.as_ref().is_some_and(wide)) { return Err(TErr::Witness); }
        if crate::trie_model::repeated_key(marks.iter().map(|m| &m.0).chain(inserts.iter().map(|i| &i.0))) { return Err(TErr::Duplicate); }
        self.replaced = false;
        let r = (|| {
            let mut t = self.root;
            for (k, sp) in marks { let (s, o) = split(k); t = self.t_mark(t, &s, &o, sp, 0)?; }
            for (k, sp, v) in inserts { let (s, o) = split(k); t = self.t_ins(t, &s, &o, sp, *v, 0)?; }
            Ok(t)
        })();
        let out = self.finish(r);
        if out.is_ok() { self.last_replaced = self.replaced; }
        out
    }

    /// Undo one block's update list (the one last applied): remove its inserts, unmark its marks. All or nothing.
    /// REFUSED (`Missing`: the record it would restore is gone) after a block that created a coin over its own spent
    /// record (BIP30, `s_ins`): restore that block's predecessor from its snapshot instead.
    pub fn undo(&mut self, marks: &Marks, inserts: &Inserts) -> Result<W, TErr> {
        if self.last_replaced { return Err(TErr::Missing); }
        let r = (|| {
            let mut t = self.root;
            for (k, _, _) in inserts.iter().rev() { let (s, o) = split(k); t = self.t_rm(t, &s, &o, 0)?; }
            for (k, _) in marks.iter().rev() { let (s, o) = split(k); t = self.t_unmark(t, &s, &o, 0)?; }
            Ok(t)
        })();
        self.finish(r)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Wr(Vec::new(), 0);
        o.0.extend_from_slice(b"HZTRIE01");
        o.u32(self.counts as u32);
        o.u64(self.height);
        o.0.extend_from_slice(&self.hash);
        let at = o.0.len();
        o.u64(0);
        self.t_enc(self.root, &mut o);
        o.w(&self.root());
        let n = o.1;
        o.0[at..at + 8].copy_from_slice(&n.to_le_bytes());
        o.0
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self, TErr> {
        Self::from_bytes_into(b, Box::new(MemArena::default()))
    }

    /// Load a snapshot into a given (empty) arena.
    pub fn from_bytes_into(b: &[u8], a: Box<dyn Arena + Send>) -> Result<Self, TErr> {
        let mut r = Rd(b, 0);
        if r.take(8)? != b"HZTRIE01" { return Err(TErr::Witness); }
        let counts = match r.u32()? { 0 => false, 1 => true, _ => return Err(TErr::Witness) };
        let height = r.u64()?;
        let hash: [u8; 32] = r.take(32)?.try_into().unwrap();
        let _nodes = r.u64()?;
        let mut st = TrieStore::with_arena(counts, a);
        let (t, _) = st.t_dec(&mut r, 0, ZERO)?;
        let root = r.w()?;
        if r.1 != b.len() { return Err(TErr::Witness); }
        st.root = t;
        st.fresh.clear();
        if st.root() != root { return Err(TErr::OldRoot); }
        (st.height, st.hash) = (height, hash);
        Ok(st)
    }
}

fn words_be(b: &[u8; 32]) -> W { core::array::from_fn(|i| u32::from_be_bytes(b[4 * i..4 * i + 4].try_into().unwrap())) }

struct Wr(Vec<u8>, u64);
impl Wr {
    fn u32(&mut self, x: u32) { self.0.extend_from_slice(&x.to_le_bytes()); }
    fn u64(&mut self, x: u64) { self.0.extend_from_slice(&x.to_le_bytes()); }
    fn w(&mut self, x: &W) { for v in x { self.u32(*v); } }
}

struct Rd<'a>(&'a [u8], usize);
impl Rd<'_> {
    fn take(&mut self, k: usize) -> Result<&[u8], TErr> {
        let s = self.0.get(self.1..self.1.checked_add(k).ok_or(TErr::Witness)?).ok_or(TErr::Witness)?;
        self.1 += k;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, TErr> { Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap())) }
    fn u64(&mut self) -> Result<u64, TErr> { Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap())) }
    fn w(&mut self) -> Result<W, TErr> { let mut x = ZERO; for v in x.iter_mut() { *v = self.u32()?; } Ok(x) }
}

/// `pre`: the key bits a node at depth d must carry (its path). Refuses the shapes no apply sequence makes.
fn on_path(k: &W, pre: &W, d: u32) -> bool { (0..d).all(|i| bit(k, i) == bit(pre, i)) }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trie_model::{apply as model_apply, decode, hash_top};
    use crate::ttree::apply_stream_perm;

    fn key(s: u8, s2: u8, o: u8, o2: u8) -> Key {
        let mut k = [0u8; 64];
        (k[0], k[1], k[32], k[33]) = (s, s2, o, o2);
        k
    }

    fn perm(m: &Marks, i: &Inserts) -> Vec<u32> {
        let ks: Vec<(W, W)> = m.iter().map(|x| split(&x.0)).chain(i.iter().map(|x| split(&x.0))).collect();
        let mut p: Vec<u32> = (0..ks.len() as u32).collect();
        p.sort_by(|&a, &b| ks[a as usize].cmp(&ks[b as usize]));
        p
    }

    /// A random chain: each block spends some unspent coins and makes new ones (some spent in their own block). At
    /// every block: the store's witness decodes to a trie with the store's root; the guest's one-pass update on it and
    /// the model's reference update both give the store's new root; the snapshot round-trips. Then every block is
    /// undone in reverse and each earlier root comes back, down to the empty trie.
    #[test]
    fn store_matches_the_guest_and_the_model_and_undoes() {
        for counts in [true, false] {
            let mut x = 0x2545f4914f6cdd1du64 ^ counts as u64;
            let mut rnd = move || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x };
            let mut st = TrieStore::new(counts);
            let mut unspent: Vec<Key> = Vec::new();
            let mut all = alloc::collections::BTreeSet::new();
            let mut hist: Vec<(W, Vec<(Key, W)>, Vec<(Key, Option<W>, u64)>)> = Vec::new();
            for blk in 0..120u32 {
                let mut marks = Vec::new();
                for _ in 0..(rnd() % 6) {
                    if unspent.is_empty() { break; }
                    let k = unspent.swap_remove((rnd() as usize) % unspent.len());
                    marks.push((k, [blk, rnd() as u32, 0, 0, 0, 0, 0, 0]));
                }
                let mut ins = Vec::new();
                for _ in 0..(1 + rnd() % 8) {
                    // few distinct addresses (shared prefixes, deep chains), some byte-close coin keys
                    let k = key(rnd() as u8 & 0x1f, (rnd() % 3) as u8, rnd() as u8, (rnd() % 2) as u8);
                    if !all.insert(k) { continue; }
                    let spent = rnd() % 4 == 0;
                    if !spent { unspent.push(k); }
                    ins.push((k, spent.then(|| [blk, 9, 0, 0, 0, 0, 0, 0]), rnd() >> 40));
                }
                let old = st.root();
                let wit = st.witness(&marks, &ins);
                let (c2, r2, mut t) = decode(&wit).expect("witness decodes");
                assert_eq!((c2, r2, hash_top(&t, counts)), (counts, old, old), "block {blk}: witness root");
                let new = st.apply(&marks, &ins).unwrap();
                assert_eq!(apply_stream_perm(&wit, &marks, &ins, Some(&perm(&marks, &ins))), Ok(new), "block {blk}: guest");
                let mut r = old;
                model_apply(&mut r, &mut t, counts, &marks, &ins).unwrap();
                assert_eq!(r, new, "block {blk}: model");
                st.height = blk as u64 + 1;
                let snap = st.to_bytes();
                let back = TrieStore::from_bytes(&snap).unwrap();
                assert_eq!((back.root(), back.height, back.counts()), (new, st.height, counts));
                assert_eq!(back.to_bytes(), snap);
                hist.push((old, marks, ins));
            }
            // a store rebuilt with the same coins in another order has the same root (shape is a function of the keys)
            while let Some((old, m, i)) = hist.pop() {
                assert_eq!(st.undo(&m, &i).unwrap(), old);
            }
            assert_eq!(st.root(), ZERO);
            assert_eq!(st.root, EMPTY);
            assert_eq!(st.nodes(), 0, "every node freed after undoing to empty");
        }
    }

    /// The disk arena against the memory one: the same chain gives the same roots, witnesses and snapshots; refused
    /// updates leave both as they were; applies after the last flush are not on disk (reopen = the flushed trie), and the
    /// reopened store continues identically. Prints nodes per coin (sizing: RECORD bytes each).
    #[cfg(feature = "std")]
    #[test]
    fn disk_arena_matches_memory_and_survives_a_crash() {
        let dir = std::env::temp_dir().join(alloc::format!("hztrie-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut x = 0x9e3779b97f4a7c15u64;
        let mut rnd = move || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x };
        let (mut mem, mut disk) = (TrieStore::new(true), TrieStore::open_dir(&dir, true).unwrap());
        let mut unspent: Vec<Key> = Vec::new();
        let mut all = alloc::collections::BTreeSet::new();
        let mut blocks: Vec<(Vec<(Key, W)>, Vec<(Key, Option<W>, u64)>)> = Vec::new();
        for blk in 0..300u32 {
            let mut marks = Vec::new();
            for _ in 0..(rnd() % 8) {
                if unspent.is_empty() { break; }
                let k = unspent.swap_remove((rnd() as usize) % unspent.len());
                marks.push((k, [blk, rnd() as u32 & 0xffff, 1, 0, 0, 0, 0, 0]));
            }
            let mut ins = Vec::new();
            for _ in 0..(1 + rnd() % 12) {
                let mut k = [0u8; 64];
                for b in k.iter_mut() { *b = rnd() as u8; }
                k[0] &= 0x3f; // shared address prefixes
                if rnd() % 3 == 0 { k[1] = 0; k[2] = 0; k[3] = 0; } // some addresses reused
                if !all.insert(k) { continue; }
                let spent = rnd() % 5 == 0;
                if !spent { unspent.push(k); }
                ins.push((k, spent.then(|| [blk, 7, 2, 0, 0, 0, 0, 0]), rnd() >> 30));
            }
            assert_eq!(mem.witness(&marks, &ins), disk.witness(&marks, &ins), "block {blk}: witness");
            assert_eq!(mem.apply(&marks, &ins), disk.apply(&marks, &ins), "block {blk}: root");
            // a refused update (a duplicate of an UNSPENT coin; over a spent one it is a BIP30 re-creation) changes neither
            if let Some(i) = ins.iter().find(|i| i.1.is_none()) {
                assert_eq!(mem.apply(&[], &[*i]), Err(TErr::Duplicate));
                assert_eq!(disk.apply(&[], &[*i]), Err(TErr::Duplicate));
            }
            (mem.height, disk.height) = (blk as u64 + 1, blk as u64 + 1);
            blocks.push((marks, ins));
            if blk == 199 { disk.flush().unwrap(); }
        }
        assert_eq!(mem.to_bytes(), disk.to_bytes());
        let coins = all.len() as u64;
        std::println!("trie arena: {} coins, {} nodes ({:.2} per coin, {} bytes per coin at {RECORD}-byte records)",
                      coins, disk.nodes(), disk.nodes() as f64 / coins as f64, disk.nodes() * RECORD as u64 / coins);
        // "crash": drop the disk store without flushing blocks 200..299; reopen = the trie flushed at 199
        drop(disk);
        let mut back = TrieStore::open_dir(&dir, true).unwrap();
        let mut replay = TrieStore::new(true);
        for (m, i) in &blocks[..200] { replay.apply(m, i).unwrap(); }
        assert_eq!((back.root(), back.height), (replay.root(), 200), "reopened = flushed state");
        let h = replay.height;
        replay.height = 200;
        assert_eq!(back.to_bytes(), replay.to_bytes(), "the WHOLE flushed tree is on disk, not only the root record");
        replay.height = h;
        for (m, i) in &blocks[200..] { assert_eq!(back.apply(m, i), replay.apply(m, i)); }
        assert_eq!(back.root(), mem.root(), "continues identically after the reopen");
        assert!(TrieStore::open_dir(&dir, false).is_err(), "counts mode is checked");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refused_updates_leave_the_store_unchanged() {
        let mut st = TrieStore::new(true);
        let a = (key(1, 0, 1, 0), None, 10);
        let b = (key(1, 0, 2, 0), None, 20);
        st.apply(&[], &[a, b]).unwrap();
        let r0 = st.root();
        let snap = st.to_bytes();
        // a duplicate insert after a good one, a mark of an absent coin, a double spend: refused, nothing changes
        assert_eq!(st.apply(&[], &[(key(2, 0, 1, 0), None, 1), a]), Err(TErr::Duplicate));
        assert_eq!(st.apply(&[(key(1, 0, 3, 0), [1, 0, 0, 0, 0, 0, 0, 0])], &[]), Err(TErr::Missing));
        assert_eq!(st.apply(&[(a.0, [1, 0, 0, 0, 0, 0, 0, 0]), (a.0, [2, 0, 0, 0, 0, 0, 0, 0])], &[]), Err(TErr::Duplicate));
        assert_eq!(st.apply(&[(a.0, [0, 0, 0, 0, 0, 1, 0, 0])], &[]), Err(TErr::Witness), "spender word 5");
        assert_eq!((st.root(), st.to_bytes()), (r0, snap));
        // insertion order does not matter
        let mut st2 = TrieStore::new(true);
        st2.apply(&[], &[b]).unwrap();
        st2.apply(&[], &[a]).unwrap();
        assert_eq!(st2.root(), r0);
    }

    /// BIP30 (the regtest rig's blocks 130 / 131): a coinbase repeated byte for byte once the first was spent.
    /// Core allows it, so the guest, the model and the store must all accept it, with the same root, and the new coin
    /// counts as unspent. Over an UNSPENT coin, or spent and created again in one block, all three refuse.
    #[test]
    fn a_coin_created_again_over_its_spent_record() {
        for counts in [true, false] {
            let mut st = TrieStore::new(counts);
            let (c, other) = (key(4, 0, 1, 0), key(4, 0, 9, 1));
            st.apply(&[], &[(c, None, 50), (other, None, 7)]).unwrap();
            let unspent = st.root();
            let ups_dup: Vec<(Key, Option<W>, u64)> = alloc::vec![(c, None, 50)];
            // over the UNSPENT coin: refused everywhere
            let wit = st.witness(&[], &ups_dup);
            assert_eq!(apply_stream_perm(&wit, &[], &ups_dup, Some(&perm(&[], &ups_dup))), Err(TErr::Duplicate), "guest, unspent");
            let (_, _, mut t) = decode(&wit).unwrap();
            let mut r = unspent;
            assert_eq!(model_apply(&mut r, &mut t, counts, &[], &ups_dup), Err(TErr::Duplicate), "model, unspent");
            assert_eq!(st.apply(&[], &ups_dup), Err(TErr::Duplicate), "store, unspent");
            assert_eq!(st.root(), unspent);
            // spent and created again in the same block: refused everywhere
            let sp = [5, 0, 0, 0, 0, 0, 0, 0];
            let both: Vec<(Key, W)> = alloc::vec![(c, sp)];
            let wit = st.witness(&both, &ups_dup);
            assert_eq!(apply_stream_perm(&wit, &both, &ups_dup, None), Err(TErr::Duplicate), "guest, one block");
            assert_eq!(apply_stream_perm(&wit, &both, &ups_dup, Some(&perm(&both, &ups_dup))), Err(TErr::Duplicate), "guest, one block, host order");
            let (_, _, mut t) = decode(&wit).unwrap();
            let mut r = unspent;
            assert_eq!(model_apply(&mut r, &mut t, counts, &both, &ups_dup), Err(TErr::Duplicate), "model, one block");
            assert_eq!(st.apply(&both, &ups_dup), Err(TErr::Duplicate), "store, one block");
            // spent at one block, created again at a later one: accepted, the same root from all three
            st.apply(&both, &[]).unwrap();
            let old = st.root();
            let wit = st.witness(&[], &ups_dup);
            let new = st.apply(&[], &ups_dup).unwrap();
            assert_ne!(new, old);
            assert_eq!(apply_stream_perm(&wit, &[], &ups_dup, Some(&perm(&[], &ups_dup))), Ok(new), "guest");
            let (_, _, mut t) = decode(&wit).unwrap();
            let mut r = old;
            model_apply(&mut r, &mut t, counts, &[], &ups_dup).unwrap();
            assert_eq!(r, new, "model");
            // ... and is the same trie as one where the coin was only ever created once: the record is replaced
            let mut once = TrieStore::new(counts);
            once.apply(&[], &[(c, None, 50), (other, None, 7)]).unwrap();
            assert_eq!(once.root(), new, "the new coin replaces the spent record; counts are of unspent coins");
            // undo cannot restore the replaced record: refused, the store unchanged
            assert_eq!(st.undo(&[], &ups_dup), Err(TErr::Missing), "undo after a re-creation");
            assert_eq!(st.root(), new);
            // and it can be spent again (after which undo works again)
            st.apply(&[(c, [6, 0, 0, 0, 0, 0, 0, 0])], &[]).unwrap();
            assert_eq!(st.undo(&[(c, [6, 0, 0, 0, 0, 0, 0, 0])], &[]), Ok(new));
        }
    }

    /// A random chain that re-creates spent coins (some spent again in their own block): at every block the guest's
    /// one-pass update and the model give the store's new root (review round 6: the replace path under random tries).
    #[test]
    fn random_chain_with_re_creations_matches_the_guest_and_the_model() {
        for counts in [true, false] {
            let mut x = 0x9e3779b97f4a7c15u64 ^ counts as u64;
            let mut rnd = move || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x };
            let mut st = TrieStore::new(counts);
            let (mut unspent, mut spent): (Vec<Key>, Vec<Key>) = (Vec::new(), Vec::new());
            let mut all = alloc::collections::BTreeSet::new();
            let mut re = 0;
            for blk in 0..200u32 {
                let mut marks = Vec::new();
                for _ in 0..(rnd() % 5) {
                    if unspent.is_empty() { break; }
                    let k = unspent.swap_remove((rnd() as usize) % unspent.len());
                    marks.push((k, [blk, rnd() as u32 & 0xffff, 0, 0, 0, 0, 0, 0]));
                }
                let mut ins: Vec<(Key, Option<W>, u64)> = Vec::new();
                let mut new_spent = Vec::new();
                for _ in 0..(1 + rnd() % 6) {
                    let again = !spent.is_empty() && rnd() % 3 == 0;
                    let k = if again { spent.swap_remove((rnd() as usize) % spent.len()) }
                            else { key(rnd() as u8 & 0x0f, (rnd() % 2) as u8, rnd() as u8, (rnd() % 2) as u8) };
                    if !again && !all.insert(k) { continue; }
                    if ins.iter().any(|i| i.0 == k) || marks.iter().any(|m| m.0 == k) { continue; }
                    re += again as u32;
                    let sp = rnd() % 4 == 0;
                    if sp { new_spent.push(k); } else { unspent.push(k); }
                    ins.push((k, sp.then(|| [blk, 3, 0, 0, 0, 0, 0, 0]), rnd() >> 40));
                }
                let old = st.root();
                let wit = st.witness(&marks, &ins);
                let (_, _, mut t) = decode(&wit).expect("witness decodes");
                let new = st.apply(&marks, &ins).unwrap_or_else(|e| panic!("block {blk}: store {e:?}"));
                assert_eq!(apply_stream_perm(&wit, &marks, &ins, Some(&perm(&marks, &ins))), Ok(new), "block {blk}: guest");
                let mut r = old;
                model_apply(&mut r, &mut t, counts, &marks, &ins).unwrap();
                assert_eq!(r, new, "block {blk}: model");
                spent.extend(marks.iter().map(|m| m.0));
                spent.extend(new_spent);
            }
            assert!(re > 50, "the chain re-created coins ({re})");
        }
    }

    #[test]
    fn snapshot_refuses_tampering() {
        let mut st = TrieStore::new(true);
        st.apply(&[], &[(key(1, 0, 1, 0), None, 10), (key(1, 0, 2, 0), Some([3, 3, 3, 3, 3, 0, 0, 0]), 20), (key(7, 0, 2, 0), None, 5)]).unwrap();
        let snap = st.to_bytes();
        assert!(TrieStore::from_bytes(&snap).is_ok());
        let n = snap.len();
        assert!(TrieStore::from_bytes(&snap[..n - 1]).is_err(), "truncated");
        let mut bad = snap.clone();
        bad[n - 1] ^= 1;
        assert_eq!(TrieStore::from_bytes(&bad).err(), Some(TErr::OldRoot), "root");
        // every single-byte change in the body is refused (a value, a count, a key, a tag)
        for i in 60..n - 32 {
            let mut bad = snap.clone();
            bad[i] ^= 0x01;
            assert!(TrieStore::from_bytes(&bad).is_err(), "byte {i} accepted");
        }
    }
}
