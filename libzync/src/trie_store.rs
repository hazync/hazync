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
/// Tests that open disk arenas take this: each maps 16 TiB of address space (DiskArena::VIRT), and more than a few at
/// once in parallel tests run out of it (ENOMEM from mmap, seen when the Z1 tests were added).
#[cfg(all(test, feature = "std"))]
static DISK_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(all(test, feature = "std"))]
static TEST_CRASH_BEFORE_META: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

#[cfg(all(test, feature = "std"))]
static CRASH_AT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
#[cfg(feature = "std")]
macro_rules! cp { ($n:expr) => { #[cfg(test)] if CRASH_AT.load(core::sync::atomic::Ordering::SeqCst) == $n { return Err(alloc::format!("test crash point {}", $n)); } } }
#[cfg(feature = "std")]
macro_rules! cpio { ($n:expr) => { #[cfg(test)] if CRASH_AT.load(core::sync::atomic::Ordering::SeqCst) == $n { return Err(std::io::Error::new(std::io::ErrorKind::Other, alloc::format!("test crash point {}", $n))); } } }
#[cfg(all(test, feature = "std"))]
static TEST_CRASH_AFTER_META: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// One `trie/roots.log` entry (Z1): height u32 | block hash 32 | root reference u64 | root hash 32 | gen u32.
#[cfg(feature = "std")]
pub const ROOTS_REC: usize = 80;

#[cfg(any(feature = "std", feature = "reader"))]
impl Node {
    pub fn enc(&self) -> [u8; RECORD] {
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
    #[cfg_attr(not(feature = "std"), allow(dead_code))] fn dec(b: &[u8]) -> Node {
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
    /// Ask for these nodes' pages to be read ahead (asynchronously); in memory: nothing to do.
    fn prefetch(&self, _rs: &[R]) {}
    /// Ask for the pages of the next `n` slots `put` will reuse to be read ahead: a write into a slot whose page is not
    /// cached first reads that page, one fault at a time, and an update writes a new node for every node on its path.
    fn prefetch_free(&self, _n: usize) {}
    /// (hits, misses) of the arena's record cache since the last call; no cache: (0, 0).
    fn cache_stats(&self) -> (u64, u64) { (0, 0) }
    /// docs/libzync/ZAPP_FOLLOWER.md §4 (Z1), host only: the height of the block whose update frees the next slots (a
    /// slot freed at height k was last in the root of k - 1). A disk arena keeps such slots back while a reader pins a
    /// height below k (`trie/PIN`).
    #[cfg(feature = "std")]
    fn set_freeing_height(&mut self, _h: u64) {}
    /// Z1: record the root of `height` (block `hash`) in `trie/roots.log`, for a reader of that height.
    #[cfg(feature = "std")]
    fn log_root(&mut self, _height: u64, _hash: &[u8; 32], _root: R, _root_hash: &[u8; 32]) {}
    /// Z1: a new generation of the arena's history (a rewind): roots logged before it may name slots that are reused.
    #[cfg(feature = "std")]
    fn bump_gen(&mut self) {}
    /// Z1/REORG P3: cut `roots.log` entries above `height` (a rewind's orphaned blocks); returns the bytes cut.
    #[cfg(feature = "std")]
    fn cut_roots_above(&mut self, _height: u64) -> Vec<u8> { Vec::new() }
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
    /// the file's length in bytes (the mapping is larger: see VIRT)
    cap: u64,
    len: u64,
    free: Vec<R>,
    pending: Vec<R>,
    cache: core::cell::RefCell<RecCache>,
    /// Z1: (freeing height, index into `pending` where its slots start); heights only grow within an interval
    pending_marks: Vec<(u64, usize)>,
    freeing: u64,
    /// Z1: slots held back in `retain/` (not free, not live)
    retained: u64,
    /// Z1: the highest freeing height ever released to the free list (review 16.11, N2): the root of h is intact iff
    /// h >= this. Persisted in `trie/INTACT`.
    released_hw: u64,
    /// Z1: this opening's generation (`GEN`, bumped at every open and rewind) and the open `roots.log`
    gen: u32,
    roots: Option<std::fs::File>,
    /// Z1: a cut of `roots.log` (a rewind's), applied at the next flush's commit with the entries logged after it
    /// (re-review 16.11: a cut before the commit, then a crash, lost the old branch's roots)
    cut: Option<(usize, Vec<[u8; ROOTS_REC]>)>,
    /// Z1: flushes in this opening; with GEN it makes every re-tag name unique (two re-tagging flushes at one height in
    /// one session used to collide, re-review 16.11 M-2)
    flushes: u32,
    /// the writer's lock (`ARENA.LOCK`); None: a reader
    lock: Option<std::fs::File>,
}

/// A direct-mapped cache, in RAM, of the address trie's inner records (HAZYNC_TRIE_CACHE_MB, default 0: off).
/// Every update walks the address trie from the root, so its inner nodes are the hot part of the store; in the page
/// cache each sits in a 4 KiB page with 63 unrelated records, so RAM held ~64x fewer of them than it could (11-09: at
/// 420,000 the walk read ~28 random pages per update and prep ran at 0.25 blocks/s, waiting on the disk). A slot holds
/// one record and its reference; a record's bytes change only through `put`, which writes through, so a hit is
/// always the record on disk.
#[cfg(feature = "std")]
struct RecCache { tags: Vec<R>, recs: Vec<[u8; RECORD]>, bits: u32, hits: u64, misses: u64 }

#[cfg(feature = "std")]
impl RecCache {
    fn new(mb: u64) -> Self {
        let n = (mb << 20) / (RECORD as u64 + 8);
        if n < 2 { return RecCache { tags: Vec::new(), recs: Vec::new(), bits: 0, hits: 0, misses: 0 }; }
        let bits = 63 - n.leading_zeros(); // the largest power of two that fits
        let n = 1usize << bits;
        RecCache { tags: alloc::vec![EMPTY; n], recs: alloc::vec![[0u8; RECORD]; n], bits, hits: 0, misses: 0 }
    }
    #[cfg(test)]
    fn new_slots(n: usize) -> Self {
        let bits = 63 - (n as u64).leading_zeros();
        let n = 1usize << bits;
        RecCache { tags: alloc::vec![EMPTY; n], recs: alloc::vec![[0u8; RECORD]; n], bits, hits: 0, misses: 0 }
    }
    fn slot(&self, r: R) -> Option<usize> {
        (self.bits > 0).then(|| (r.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - self.bits)) as usize)
    }
    /// only the address trie's inner records (tag 3)
    fn keeps(b: &[u8]) -> bool { b[0] == 3 }
}

#[cfg(feature = "std")]
impl DiskArena {
    const STEP: u64 = 1 << 20; // records per growth step at least (64 MiB); larger files grow by 1/8
    /// The mapping's length: address space, mapped ONCE (11-07). Growing used to msync the whole file and map it again,
    /// so every page faulted back in after each 64 MiB step: on the slice run ~40% of prep's CPU went to page faults
    /// and TLB shootdowns. Now a grow only lengthens the file under the mapping (pages past the file's end are never
    /// touched). 16 TiB: the mainnet trie is ~1 TB (slice run, 11-07).
    const VIRT: u64 = 1 << 44;
    const MAGIC: &'static [u8; 8] = b"HZTRIED1";

    fn map(file: &std::fs::File) -> std::io::Result<memmap2::MmapMut> {
        let n = file.metadata()?.len().max(Self::VIRT);
        let map = unsafe { memmap2::MmapOptions::new().len(n as usize).map_mut(file) }?;
        // Random access: no read-around on a fault. The walk touches one 64-byte record per page it faults, so the
        // kernel's default read-around (read_ahead_kb, 128 KiB on hazync-proof) read ~30 pages for each one used and
        // evicted the cached trie with them; once the arena outgrew RAM (~415,000) prep fell from 0.6 to 0.27 blocks/s,
        // waiting on the disk at 9% CPU (11-09). prefetch's WILLNEED still reads exactly the pages asked for.
        #[cfg(unix)]
        map.advise(memmap2::Advice::Random)?;
        Ok(map)
    }

    /// Open (or create) the arena in `dir`; returns it with the flushed (root, counts, height, hash), if any.
    pub fn open(dir: &std::path::Path) -> std::io::Result<(Self, Option<(R, bool, u64, [u8; 32])>)> {
        let mb: u64 = std::env::var("HAZYNC_TRIE_CACHE_MB").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        Self::open_cached(dir, mb)
    }

    /// `open` with a record cache of `cache_mb` MiB (0: none).
    pub fn open_cached(dir: &std::path::Path, cache_mb: u64) -> std::io::Result<(Self, Option<(R, bool, u64, [u8; 32])>)> {
        std::fs::create_dir_all(dir)?;
        let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(dir.join("nodes.bin"))?;
        if file.metadata()?.len() < Self::STEP * RECORD as u64 { file.set_len(Self::STEP * RECORD as u64)?; }
        let map = Self::map(&file)?;
        let cap = file.metadata()?.len();
        let cache = core::cell::RefCell::new(RecCache::new(cache_mb));
        let mut a = DiskArena { dir: dir.to_path_buf(), file, map, cap, len: 0, free: Vec::new(), pending: Vec::new(), cache,
                                pending_marks: Vec::new(), freeing: 0, retained: 0, released_hw: 0, gen: 0, roots: None, cut: None, flushes: 0, lock: None };
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
        // one WRITER per arena (re-review 16.11, L-1): the first opener takes `ARENA.LOCK` and runs the Z1 recovery
        // (GEN bump, journals, cuts). A second opener, in this process or another, is a READER: no recovery, no
        // writes (put and flush refuse), so it can never undo what a live writer is doing.
        let lk = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("ARENA.LOCK"))?;
        if lk.try_lock().is_ok() {
            a.lock = Some(lk);
            a.open_z1(meta.map_or(0, |m| m.2))?;
        } else {
            a.gen = std::fs::read(dir.join("GEN")).ok().filter(|b| b.len() == 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()));
        }
        Ok((a, meta))
    }

    /// Z1 at open, before any write (ZAPP_FOLLOWER.md §4): bump `GEN` (synced), cut `roots.log` back to the flushed
    /// height `f` (the replay reuses the slots of the roots above it), drop `retain/` lists of blocks above `f` (a lost
    /// interval's: their slots are live in the flushed trie or past its length) and count the rest.
    fn open_z1(&mut self, f: u64) -> std::io::Result<()> {
        // what was released: INTACT's intact_from (written before each meta), else (a store from before it) everything
        // up to the flushed height
        // A store without INTACT: before Z1 every flush released everything up to its height, so f. One that has Z1's
        // files but lost INTACT cannot know: the highest height it ever logged or held a list for, which no release
        // can have passed (re-review 16.11, L-d: never claim more than is known).
        self.released_hw = match std::fs::read_to_string(self.dir.join("INTACT")).ok().and_then(|x| x.split_whitespace().nth(1)?.parse().ok()) {
            Some(h) => h,
            None => {
                let logged = std::fs::read(self.dir.join("roots.log")).map(|b| b.chunks_exact(ROOTS_REC)
                    .map(|c| u32::from_le_bytes(c[..4].try_into().unwrap()) as u64).max().unwrap_or(0)).unwrap_or(0);
                let held = self.retain_files().iter().map(|(k, _)| *k).max().unwrap_or(0);
                f.max(logged).max(held)
            }
        };
        let g = std::fs::read(self.dir.join("GEN")).ok().filter(|b| b.len() == 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()));
        self.gen = g.wrapping_add(1);
        self.write_gen()?;
        // a rewind's cut of roots.log: applied if its flush committed (the meta at its height), else dropped
        if let Ok(j) = std::fs::read(self.dir.join("ROOTS_CUT")) {
            let now = { use sha2::Digest as _; sha2::Sha256::digest(std::fs::read(self.dir.join("meta")).unwrap_or_default()) };
            if j.len() >= 48 && j[16..48] != now[..] { self.apply_roots_cut()?; }
            else { std::fs::remove_file(self.dir.join("ROOTS_CUT"))?; }
        }
        let rl = self.dir.join("roots.log");
        if let Ok(b) = std::fs::read(&rl) {
            let mut n = b.len() / ROOTS_REC;                          // a torn last entry goes too
            while n > 0 && u32::from_le_bytes(b[(n - 1) * ROOTS_REC..(n - 1) * ROOTS_REC + 4].try_into().unwrap()) as u64 > f { n -= 1; }
            if n * ROOTS_REC != b.len() {
                std::fs::OpenOptions::new().write(true).open(&rl)?.set_len((n * ROOTS_REC) as u64)?;
            }
        }
        // a release a crash interrupted (durable): committed if the meta on disk is the one it names
        let rel = self.dir.join("retain").join("RELEASE");
        if let Ok(r) = std::fs::read(&rel) {
            use sha2::Digest as _;
            let meta = std::fs::read(self.dir.join("meta")).unwrap_or_default();
            if r.len() >= 32 && sha2::Sha256::digest(&meta)[..] == r[..32] {
                for n in alloc::string::String::from_utf8_lossy(&r[32..]).lines().filter(|n| n.starts_with("r_") && n.ends_with(".bin") && !n.contains('/')) {
                    let _ = std::fs::remove_file(self.dir.join("retain").join(n));
                    cpio!(32);
                }
            }
            std::fs::remove_file(&rel)?;
            std::fs::File::open(self.dir.join("retain")).and_then(|d| d.sync_all())?;
        }
        // re-tags a crash left uncommitted (the meta never reached their height): undone, so the lists keep their own
        // freeing heights and the slots of the lost interval are not held under a lower one (re-review 16.11, L-a)
        let rt = self.dir.join("retain").join("RETAG");
        if let Ok(tb) = std::fs::read(&rt) {
            // read lossily: a torn journal (not UTF-8, no meta hash) is rolled back and removed, never left (L-2)
            let t = alloc::string::String::from_utf8_lossy(&tb).into_owned();
            let mut lines = t.lines();
            let was: Option<alloc::string::String> = lines.next().and_then(|l| Some(l.split_whitespace().nth(2)?.into()));
            let now: alloc::string::String = { use sha2::Digest as _; sha2::Sha256::digest(std::fs::read(self.dir.join("meta")).unwrap_or_default()).iter().map(|x| alloc::format!("{x:02x}")).collect() };
            let _ = f;
            if was.as_deref() == Some(now.as_str()) || was.is_none() {
                let rdir = self.dir.join("retain");
                let ok = |n: &str| n.starts_with("r_") && n.ends_with(".bin") && !n.contains('/');
                for l in lines {
                    let w: Vec<&str> = l.split_whitespace().collect();
                    match w.as_slice() {
                        ["rename", old, new] if ok(old) && ok(new) => { if rdir.join(new).exists() { std::fs::rename(rdir.join(new), rdir.join(old))?; cpio!(30); } }
                        ["created", new] if ok(new) => { let _ = std::fs::remove_file(rdir.join(new)); }
                        _ => {}
                    }
                }
            }
            cpio!(31);
            std::fs::remove_file(&rt)?;
            std::fs::File::open(self.dir.join("retain")).and_then(|d| d.sync_all())?;
        }
        for (k, p) in self.retain_files() {
            if k > f { std::fs::remove_file(&p)?; } else { self.retained += std::fs::metadata(&p)?.len() / 8; }
        }
        Ok(())
    }

    /// Apply `trie/ROOTS_CUT` (a committed rewind's cut of roots.log) and remove it.
    fn apply_roots_cut(&self) -> std::io::Result<()> {
        use std::io::Write as _;
        let j = std::fs::read(self.dir.join("ROOTS_CUT"))?;
        if j.len() >= 48 && (j.len() - 48) % ROOTS_REC == 0 {
            let n = u64::from_le_bytes(j[8..16].try_into().unwrap());
            let rl = self.dir.join("roots.log");
            let mut f = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(&rl)?;
            f.set_len(n)?;
            cpio!(20);
            std::io::Seek::seek(&mut f, std::io::SeekFrom::End(0))?;
            f.write_all(&j[48..])?;
            f.sync_data()?;
        }
        cpio!(21);
        std::fs::remove_file(self.dir.join("ROOTS_CUT"))?;
        std::fs::File::open(&self.dir).and_then(|d| d.sync_all())
    }

    fn write_gen(&self) -> std::io::Result<()> {
        use std::io::Write as _;
        let tmp = self.dir.join("GEN.tmp");
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&self.gen.to_le_bytes())?;
        f.sync_data()?;
        std::fs::rename(&tmp, self.dir.join("GEN"))?;
        std::fs::File::open(&self.dir).and_then(|d| d.sync_all())
    }

    /// `retain/r_<k>.bin`: (k, path), any order.
    /// `retain/r_<k>.bin`, and `r_<k>_<j>.bin` (block j's list re-tagged to k by a rewind's flush, N1): (k, path).
    fn retain_files(&self) -> Vec<(u64, std::path::PathBuf)> {
        let Ok(it) = std::fs::read_dir(self.dir.join("retain")) else { return Vec::new() };
        it.flatten().filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            let k = n.strip_prefix("r_")?.strip_suffix(".bin")?.split('_').next()?.parse().ok()?;
            Some((k, e.path()))
        }).collect()
    }

    /// The pinned height (`trie/PIN`: the height, then anything), if a reader pins one. A PIN file that is there but
    /// does not read as a height is a pin at 0 (review 16.11, M4): hold everything up to HAZYNC_TRIE_RETAIN_MAX, and
    /// say so, rather than release what a reader may be walking.
    fn pin(&self) -> Option<u64> {
        match std::fs::read_to_string(self.dir.join("PIN")) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            r => Some(r.ok().and_then(|x| x.split_whitespace().next()?.parse().ok()).unwrap_or_else(|| {
                std::eprintln!("TRIE_PIN_UNREADABLE {}/PIN does not read as a height: held as a pin at 0", self.dir.display());
                0
            })),
        }
    }

    fn grow(&mut self) {
        // no msync and no remap: the mapping already covers the new length (durable() syncs the file and its size)
        let n = (self.cap + (Self::STEP * RECORD as u64).max(self.cap / 8)).next_multiple_of(RECORD as u64);
        assert!(n <= self.map.len() as u64, "trie arena: past the {} TiB mapping", Self::VIRT >> 40);
        self.file.set_len(n).expect("trie arena: grow the file");
        self.cap = n;
    }
}

#[cfg(feature = "std")]
impl Arena for DiskArena {
    fn get(&self, r: R) -> Node {
        let i = (r - 1) as usize * RECORD;
        // inside the FILE (the mapping runs past it: a reference beyond the file must panic, not SIGBUS)
        assert!(r >= 1 && (i + RECORD) as u64 <= self.cap, "trie arena: node {r} past the file");
        let mut c = self.cache.borrow_mut();
        let Some(k) = c.slot(r) else { return Node::dec(&self.map[i..i + RECORD]) };
        if c.tags[k] == r { c.hits += 1; return Node::dec(&c.recs[k]); }
        c.misses += 1;
        let b = &self.map[i..i + RECORD];
        if RecCache::keeps(b) { c.tags[k] = r; c.recs[k].copy_from_slice(b); }
        Node::dec(b)
    }
    fn put(&mut self, n: Node) -> R {
        assert!(self.lock.is_some(), "trie arena {}: a reader cannot write (another opener holds ARENA.LOCK)", self.dir.display());
        let r = match self.free.pop() {
            Some(r) => r,
            None => {
                if (self.len + 1) * RECORD as u64 > self.cap { self.grow(); }
                self.len += 1;
                self.len
            }
        };
        let i = (r - 1) as usize * RECORD;
        let e = n.enc();
        self.map[i..i + RECORD].copy_from_slice(&e);
        // write through: the slot's cached record (if any) becomes this one, or goes
        let c = self.cache.get_mut();
        if let Some(k) = c.slot(r) {
            if RecCache::keeps(&e) { c.tags[k] = r; c.recs[k] = e; } else if c.tags[k] == r { c.tags[k] = EMPTY; }
        }
        r
    }
    fn free(&mut self, r: R) {
        if self.pending_marks.last().map(|m| m.0) != Some(self.freeing) { self.pending_marks.push((self.freeing, self.pending.len())); }
        self.pending.push(r);
    }
    fn live(&self) -> u64 { self.len - (self.free.len() + self.pending.len()) as u64 - self.retained }
    fn set_freeing_height(&mut self, h: u64) { self.freeing = h; }
    fn log_root(&mut self, height: u64, hash: &[u8; 32], root: R, root_hash: &[u8; 32]) {
        use std::io::Write as _;
        let mut b = [0u8; ROOTS_REC];
        b[..4].copy_from_slice(&(height as u32).to_le_bytes());
        b[4..36].copy_from_slice(hash);
        b[36..44].copy_from_slice(&root.to_le_bytes());
        b[44..76].copy_from_slice(root_hash);
        b[76..80].copy_from_slice(&self.gen.to_le_bytes());
        if let Some((_, after)) = &mut self.cut { after.push(b); return; }
        if self.roots.is_none() {
            self.roots = Some(std::fs::OpenOptions::new().create(true).append(true).open(self.dir.join("roots.log"))
                .unwrap_or_else(|e| panic!("trie arena {}/roots.log: {e}", self.dir.display())));
        }
        // no fsync: an entry above the flushed height is cut at the next open anyway
        self.roots.as_mut().unwrap().write_all(&b).unwrap_or_else(|e| panic!("trie arena {}/roots.log: {e}", self.dir.display()));
    }
    fn cut_roots_above(&mut self, height: u64) -> Vec<u8> {
        // nothing changes on disk until the next flush commits (then the file is cut and the entries logged since are
        // appended); a crash before keeps the old branch's roots, as the old meta keeps its trie
        self.roots = None;
        let b = std::fs::read(self.dir.join("roots.log")).unwrap_or_default();
        let hi = |e: &[u8]| u32::from_le_bytes(e[..4].try_into().unwrap()) as u64;
        // a second cut before the flush works on the log as the first left it: the file's kept prefix and the entries
        // logged since, of which those at or below `height` stay (re-review 16.11, L-3)
        let (mut n, after) = match self.cut.take() {
            Some((n, mut after)) => { after.retain(|e| hi(e) <= height); (n / ROOTS_REC, after) }
            None => (b.len() / ROOTS_REC, Vec::new()),
        };
        let full = n;
        while n > 0 && hi(&b[(n - 1) * ROOTS_REC..n * ROOTS_REC]) > height { n -= 1; }
        self.cut = Some((n * ROOTS_REC, after));
        b[n * ROOTS_REC..full * ROOTS_REC].to_vec()
    }
    fn bump_gen(&mut self) {
        self.gen = self.gen.wrapping_add(1);
        self.write_gen().unwrap_or_else(|e| panic!("trie arena {}/GEN: {e}", self.dir.display()));
    }
    #[cfg(not(unix))]
    fn prefetch(&self, _rs: &[R]) {} // madvise is Unix-only: elsewhere (Windows) reads fault in on demand, slower, same result
    #[cfg(unix)]
    fn prefetch(&self, rs: &[R]) {
        // one madvise(WILLNEED) per distinct page: the kernel reads them all ahead at once, so a level of the walk costs
        // about one read latency, not one per node (the volume serves ~75 k random reads/s in parallel, ~5 k serially)
        let c = self.cache.borrow();
        let cached = |r: R| c.slot(r).is_some_and(|k| c.tags[k] == r);
        let mut pages: Vec<u64> = rs.iter().filter(|&&r| r != EMPTY && (r as u64) * RECORD as u64 <= self.cap && !cached(r))
            .map(|&r| ((r - 1) * RECORD as u64) & !4095).collect();
        pages.sort_unstable();
        pages.dedup();
        for p in pages {
            let _ = self.map.advise_range(memmap2::Advice::WillNeed, p as usize, 4096);
        }
    }
    fn cache_stats(&self) -> (u64, u64) {
        let mut c = self.cache.borrow_mut();
        (core::mem::take(&mut c.hits), core::mem::take(&mut c.misses))
    }
    fn prefetch_free(&self, n: usize) {
        // put pops from the end of the free list
        let k = self.free.len().saturating_sub(n);
        self.prefetch(&self.free[k..]);
    }
    fn durable(&mut self, root: R, counts: bool, height: u64, hash: &[u8; 32]) -> Result<(), alloc::string::String> {
        use std::io::Write as _;
        if self.lock.is_none() { return Err(alloc::format!("trie arena {}: a reader cannot flush (another opener holds ARENA.LOCK)", self.dir.display())); }
        let e = |x: std::io::Error| alloc::format!("trie arena {}: {x}", self.dir.display());
        self.map.flush_range(0, self.cap as usize).map_err(e)?;
        self.file.sync_all().map_err(e)?; // the file's size after a grow, not only its pages
        // Z1 retention (ZAPP_FOLLOWER.md §4): with a reader's PIN, slots freed by blocks above `release` are held back in
        // retain/ (synced before the meta), not handed out; lists at or below it come back. No PIN: everything is
        // released, exactly as before Z1.
        let retain_max: u64 = std::env::var("HAZYNC_TRIE_RETAIN_MAX").ok().and_then(|v| v.parse().ok()).unwrap_or(288);
        let pin = self.pin();
        let release = pin.map_or(u64::MAX, |p| p.min(height).max(height.saturating_sub(retain_max)));
        let mut all: Vec<R> = self.free.clone();
        let mut keep: Vec<(u64, &[R])> = Vec::new();
        let mut hw = self.released_hw;
        for (i, &(k, start)) in self.pending_marks.iter().enumerate() {
            let end = self.pending_marks.get(i + 1).map_or(self.pending.len(), |m| m.1);
            if k <= release { all.extend_from_slice(&self.pending[start..end]); if end > start { hw = hw.max(k); } } else { keep.push((k, &self.pending[start..end])); }
        }
        if self.pending_marks.first().is_some_and(|m| m.1 > 0) { all.extend_from_slice(&self.pending[..self.pending_marks[0].1]); }
        // lists at or below the release come back; so do lists ABOVE the height flushed now, which only a rewind leaves
        // (blocks undone: what they freed is garbage, in no root since the rewind; an open would drop those lists and
        // lose the slots for good)
        // Lists ABOVE the height flushed now exist only after a rewind (blocks undone). They hold what the orphaned blocks'
        // FORWARD applies freed: nodes of older roots, a pinned one among them (review 16.11, N1: releasing them broke a
        // root pinned below the fork). So they stay held, re-tagged to this height (`r_<height>_<k>.bin`, a rename: no
        // slot is ever in two lists), and come back once the pin passes it; the next open keeps them (tag <= flushed).
        // The new names carry this opening's GEN, so a second rewind to the same height never renames onto the first's
        // (re-review 16.11, M1). Pending runs freed by blocks above the height (an apply and an undo in one session,
        // L-b) are written under such a name too. All of it is journaled in retain/RETAG first: an open whose meta did
        // not reach this height (the flush never committed) renames them back and drops the new ones (L-a).
        let rd = self.dir.join("retain");
        let mut lists = self.retain_files();
        let retag_lists: Vec<usize> = if release != u64::MAX { (0..lists.len()).filter(|&i| lists[i].0 > height && lists[i].0 > release).collect() } else { Vec::new() };
        let retag_pending = keep.iter().any(|(k, _)| *k > height);
        let old_meta: alloc::string::String = { use sha2::Digest as _; sha2::Sha256::digest(std::fs::read(self.dir.join("meta")).unwrap_or_default()).iter().map(|x| alloc::format!("{x:02x}")).collect() };
        self.flushes += 1;
        let tag = alloc::format!("{}x{}", self.gen, self.flushes);
        let mut retag = alloc::format!("{height} {} {old_meta}\n", self.gen);
        if !retag_lists.is_empty() || retag_pending {
            std::fs::create_dir_all(&rd).map_err(e)?;
            for &i in &retag_lists {
                let old = lists[i].1.file_name().unwrap().to_string_lossy().into_owned();
                let new = alloc::format!("r_{height}_{}_{tag}.bin", old.strip_prefix("r_").unwrap().strip_suffix(".bin").unwrap());
                retag.push_str(&alloc::format!("rename {old} {new}\n"));
            }
            for (k, _) in keep.iter().filter(|(k, _)| *k > height) { retag.push_str(&alloc::format!("created r_{height}_{k}_c{tag}.bin\n")); }
            let mut f = std::fs::File::create(rd.join("RETAG")).map_err(e)?;
            f.write_all(retag.as_bytes()).map_err(e)?;
            f.sync_data().map_err(e)?;
            std::fs::File::open(&rd).and_then(|d| d.sync_all()).map_err(e)?;
            cp!(1);
            for &i in &retag_lists {
                let old = lists[i].1.file_name().unwrap().to_string_lossy().into_owned();
                let to = rd.join(alloc::format!("r_{height}_{}_{tag}.bin", old.strip_prefix("r_").unwrap().strip_suffix(".bin").unwrap()));
                // never onto an existing list: a rename would drop its slots for good
                if to.exists() { return Err(alloc::format!("trie arena {}: re-tag target {} exists", self.dir.display(), to.display())); }
                std::fs::rename(&lists[i].1, &to).map_err(e)?;
                cp!(2);
                lists[i] = (height, to);
            }
            std::fs::File::open(&rd).and_then(|d| d.sync_all()).map_err(e)?;
            cp!(3);
        }
        let back: Vec<(u64, std::path::PathBuf)> = lists.into_iter().filter(|(k, _)| *k <= release).collect();
        let mut back_n = 0u64;
        for (k, _) in &back { hw = hw.max(*k); }
        for (_, p) in &back {
            let b = std::fs::read(p).map_err(e)?;
            all.extend(b.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())));
            back_n += b.len() as u64 / 8;
        }
        let mut kept_n = 0u64;
        if !keep.is_empty() {
            let rd = self.dir.join("retain");
            std::fs::create_dir_all(&rd).map_err(e)?;
            for (k, slots) in &keep {
                // a re-tagged run gets a new file of its own (never appended to another); an ordinary one is appended to
                // its block's list (all of which belong to this interval: an open drops them if the flush fails)
                let (name, fresh) = if *k > height { (alloc::format!("r_{height}_{k}_c{tag}.bin"), true) } else { (alloc::format!("r_{k}.bin"), false) };
                let mut f = if fresh { std::fs::OpenOptions::new().write(true).create_new(true).open(rd.join(&name)) }
                            else { std::fs::OpenOptions::new().create(true).append(true).open(rd.join(&name)) }.map_err(e)?;
                let mut b = Vec::with_capacity(8 * slots.len());
                for r in slots.iter() { b.extend_from_slice(&r.to_le_bytes()); }
                f.write_all(&b).map_err(e)?;
                f.sync_data().map_err(e)?;
                kept_n += slots.len() as u64;
            }
            std::fs::File::open(&rd).and_then(|d| d.sync_all()).map_err(e)?;
        }
        cp!(4);
        // reuse slots in ascending address order (put() pops from the end; the meta keeps the order for a restart):
        // the next interval's writes then walk the file forwards, several to a page, instead of one 64-byte record per
        // 4 KiB page at random (11-08: past RAM, prep spent half its time throttled on dirty pages). Layout only.
        all.sort_unstable_by(|a, b| b.cmp(a));
        let mut b = Vec::with_capacity(73 + 8 * all.len());
        b.extend_from_slice(Self::MAGIC);
        b.extend_from_slice(&root.to_le_bytes());
        b.push(counts as u8);
        b.extend_from_slice(&height.to_le_bytes());
        b.extend_from_slice(hash);
        b.extend_from_slice(&self.len.to_le_bytes());
        b.extend_from_slice(&(all.len() as u64).to_le_bytes());
        for r in &all { b.extend_from_slice(&r.to_le_bytes()); }
        // a release is committed by the meta (review 16.11, H2): retain/RELEASE names the lists it frees and the hash of
        // the meta that holds their slots, synced BEFORE that meta's rename. An open that finds it deletes the lists if
        // the meta on disk is that one (the rename happened) and keeps them otherwise, so a crash anywhere in between
        // frees nothing twice and loses nothing.
        let rel = self.dir.join("retain").join("RELEASE");
        if !back.is_empty() {
            use sha2::Digest as _;
            let mut r = sha2::Sha256::digest(&b).to_vec();
            for (_, p) in &back { r.extend_from_slice(p.file_name().unwrap().to_string_lossy().as_bytes()); r.push(b'\n'); }
            let mut f = std::fs::File::create(&rel).map_err(e)?;
            f.write_all(&r).map_err(e)?;
            f.sync_data().map_err(e)?;
            std::fs::File::open(self.dir.join("retain")).and_then(|d| d.sync_all()).map_err(e)?;
        }
        // INTACT (review 16.11, N2): the lowest height whose logged root is intact, i.e. the highest freeing height ever
        // released, with this flush's releases counted. Written and synced BEFORE the meta, so after any crash it
        // over-states what was released (safe for a reader), never under-states it. With roots.log made durable too.
        cp!(5);
        if let Some(f) = &self.roots { f.sync_data().map_err(e)?; }
        {
            let msg = alloc::format!("intact_from {hw} gen {} flushed {height}\n", self.gen);
            let t = self.dir.join("INTACT.tmp");
            let mut f = std::fs::File::create(&t).map_err(e)?;
            f.write_all(msg.as_bytes()).map_err(e)?;
            f.sync_data().map_err(e)?;
            std::fs::rename(&t, self.dir.join("INTACT")).map_err(e)?;
            std::fs::File::open(&self.dir).and_then(|d| d.sync_all()).map_err(e)?;
        }
        cp!(6);
        // a pending roots.log cut, journaled in trie/ROOTS_CUT (height, kept length, the entries to append) before the
        // meta: an open applies it iff the meta reached that height (the cut is part of this flush's commit)
        if let Some((n, after)) = &self.cut {
            let mut j = Vec::with_capacity(48 + ROOTS_REC * after.len());
            j.extend_from_slice(&height.to_le_bytes());
            j.extend_from_slice(&(*n as u64).to_le_bytes());
            { use sha2::Digest as _; j.extend_from_slice(&sha2::Sha256::digest(std::fs::read(self.dir.join("meta")).unwrap_or_default())); }
            for b in after { j.extend_from_slice(b); }
            let t = self.dir.join("ROOTS_CUT.tmp");
            let mut f = std::fs::File::create(&t).map_err(e)?;
            f.write_all(&j).map_err(e)?;
            f.sync_data().map_err(e)?;
            std::fs::rename(&t, self.dir.join("ROOTS_CUT")).map_err(e)?;
            std::fs::File::open(&self.dir).and_then(|d| d.sync_all()).map_err(e)?;
        }
        // test hook: a crash after the re-tags and the lists, before the meta (re-review 16.11, L-a)
        #[cfg(test)]
        if TEST_CRASH_BEFORE_META.load(core::sync::atomic::Ordering::SeqCst) && self.dir.to_string_lossy().ends_with("-abandon") {
            return Err("test: crashed before the meta".into());
        }
        cp!(7);
        let tmp = self.dir.join("meta.tmp");
        let mut f = std::fs::File::create(&tmp).map_err(e)?;
        f.write_all(&b).map_err(e)?;
        f.sync_data().map_err(e)?;
        std::fs::rename(&tmp, self.dir.join("meta")).map_err(e)?;
        std::fs::File::open(&self.dir).and_then(|d| d.sync_all()).map_err(e)?;
        // test hook: a crash right after the meta's rename, before the released lists are unlinked (review 16.11, H2)
        #[cfg(test)]
        if TEST_CRASH_AFTER_META.load(core::sync::atomic::Ordering::SeqCst) && self.dir.to_string_lossy().ends_with("-z1-h2") {
            return Err("test: crashed after the meta rename".into());
        }
        cp!(8);
        // the released lists' slots are in the meta's free list now
        for (_, p) in &back { std::fs::remove_file(p).map_err(e)?; }
        if !back.is_empty() {
            std::fs::remove_file(&rel).map_err(e)?;
            std::fs::File::open(self.dir.join("retain")).and_then(|d| d.sync_all()).map_err(e)?;
        }
        cp!(9);
        // a pending cut of roots.log, committed with the meta: applied, then its journal goes
        if self.cut.take().is_some() {
            self.apply_roots_cut().map_err(e)?;
            self.roots = None;
        }
        cp!(10);
        // the re-tags are committed by the meta just renamed
        if rd.join("RETAG").exists() {
            std::fs::remove_file(rd.join("RETAG")).map_err(e)?;
            std::fs::File::open(&rd).and_then(|d| d.sync_all()).map_err(e)?;
        }
        self.retained = self.retained + kept_n - back_n;
        self.released_hw = hw;
        if let Some(p) = pin.filter(|&p| p < release) {
            // the cap released a pinned height: the reader must know (ZAPP_FOLLOWER.md §4)
            let msg = alloc::format!("released_through {release} pin {p} height {height}\n");
            std::fs::write(self.dir.join("PIN.released"), msg.as_bytes()).map_err(e)?;
            std::eprintln!("TRIE_PIN_RELEASED the pinned height {p} is below {release} (HAZYNC_TRIE_RETAIN_MAX={retain_max}): its slots may be reused");
        }
        self.free = all;
        self.pending.clear();
        self.pending_marks.clear();
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
    /// the update being applied re-created a coin over its spent record
    replaced: bool,
    /// one entry per update applied in this session (true: it re-created a coin over its spent record), so undo can
    /// refuse any update whose replaced record it cannot restore, however many levels back (review round 7)
    applied: Vec<bool>,
}

/// One block's update list, as the aggregate's collect run logs it and the trie receipt commits.
pub type Marks = [(Key, W)];
pub type Inserts = [(Key, Option<W>, u64)];

impl TrieStore {
    fn with_arena(counts: bool, a: Box<dyn Arena + Send>) -> Self {
        TrieStore { counts, height: 0, hash: [0; 32], root: EMPTY, a, fresh: Vec::new(), garbage: Vec::new(), replaced: false, applied: Vec::new() }
    }
    pub fn new(counts: bool) -> Self { Self::with_arena(counts, Box::new(MemArena::default())) }

    /// The store kept on disk in `dir` (trie_store::DiskArena): the trie last flushed there, or a new empty one.
    #[cfg(feature = "std")]
    pub fn open_dir(dir: &std::path::Path, counts: bool) -> std::io::Result<Self> {
        let mb: u64 = std::env::var("HAZYNC_TRIE_CACHE_MB").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        Self::open_dir_cached(dir, counts, mb)
    }

    /// `open_dir` with a record cache of `cache_mb` MiB in the disk arena (0: none; see `RecCache`).
    #[cfg(feature = "std")]
    pub fn open_dir_cached(dir: &std::path::Path, counts: bool, cache_mb: u64) -> std::io::Result<Self> {
        let (a, meta) = DiskArena::open_cached(dir, cache_mb)?;
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
        if d > 256 { return Err(TErr::Witness); } // a leaf may sit at depth 256, as in the guest (trie_model::ins_top)
        let (dc, dv) = if sp.is_none() { (1u64, v) } else { (0, 0) };
        if n == EMPTY {
            let sub = self.put(Node::Coin { o: *o, sp: *sp, value: v });
            return Ok(self.put(Node::Addr { s: *s, count: dc, value: dv, sub }));
        }
        match self.get(n) {
            Node::TopInner { .. } if d >= 256 => Err(TErr::Witness),
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
                if d >= 256 { return Err(TErr::Witness); }
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
    /// Read ahead every node an update of these keys will read (mainnet prep, 11-07): the keys' paths AND each inner
    /// node's other child (its hash is read when the path is rebuilt), level by level, each level's pages requested
    /// together. Read-only: changes nothing (the update itself is `apply`).
    /// Returns the number of nodes on the keys' paths (about the nodes the update will write).
    pub fn prefetch(&self, marks: &Marks, inserts: &Inserts) -> usize {
        let mut ks: Vec<(W, W)> = marks.iter().map(|x| split(&x.0)).chain(inserts.iter().map(|x| split(&x.0))).collect();
        ks.sort_unstable();
        if ks.is_empty() || self.root == EMPTY { return ks.len(); }
        self.a.prefetch(&[self.root]);
        let mut on_path = 0usize;
        // nodes on a key's path (already requested): (node, in a coin trie, depth, its keys)
        let mut cur: Vec<(R, bool, u32, &[(W, W)])> = alloc::vec![(self.root, false, 0, &ks[..])];
        while !cur.is_empty() {
            on_path += cur.len();
            let mut kids: Vec<R> = Vec::new();
            let mut next: Vec<(R, bool, u32, &[(W, W)])> = Vec::new();
            for &(n, sub, d, k) in &cur {
                match self.get(n) {
                    Node::TopInner { l, r, .. } | Node::SubInner { l, r, .. } => {
                        let m = k.partition_point(|x| !bit(if sub { &x.1 } else { &x.0 }, d));
                        kids.push(l);
                        kids.push(r); // both: a keyless child is read for its hash, not walked
                        if m > 0 && l != EMPTY { next.push((l, sub, d + 1, &k[..m])); }
                        if m < k.len() && r != EMPTY { next.push((r, sub, d + 1, &k[m..])); }
                    }
                    Node::Addr { s, sub: c, .. } => {
                        let a = k.partition_point(|x| x.0 < s);
                        let b = k.partition_point(|x| x.0 <= s);
                        if b > a && c != EMPTY { kids.push(c); next.push((c, true, 0, &k[a..b])); }
                    }
                    Node::Coin { .. } => {}
                }
            }
            self.a.prefetch(&kids);
            cur = next;
        }
        on_path + ks.len()
    }

    /// Read ahead the pages of the next `n` slots the store will write into (see `Arena::prefetch_free`).
    pub fn prefetch_free(&self, n: usize) { self.a.prefetch_free(n); }
    /// (hits, misses) of the disk arena's record cache since the last call.
    pub fn cache_stats(&self) -> (u64, u64) { self.a.cache_stats() }

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
    fn finish(&mut self, r: Result<R, TErr>, freeing: u64) -> Result<W, TErr> {
        // Z1: the slots an APPLY frees were last in the root of `height` (the block being applied is height + 1). An
        // undo frees slots of the orphaned roots only, tagged 0: released at the next flush whatever the pin (review
        // 16.11: tagged above the flush they were dropped from retain/ at the next open and never freed again). A
        // reader is told by the rewind's GEN bump before any of them can be reused.
        #[cfg(feature = "std")]
        self.a.set_freeing_height(freeing);
        #[cfg(not(feature = "std"))]
        let _ = freeing;
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
        let out = self.finish(r, self.height + 1);
        if out.is_ok() { self.applied.push(self.replaced); }
        out
    }

    /// Undo one block's update list (the one last applied in this session): remove its inserts, unmark its marks.
    /// All or nothing. REFUSED (`Missing`) for a block that created a coin over its own spent record (BIP30, `s_ins`:
    /// the record it would restore is gone), and for any block not applied in this session (a store loaded from a
    /// snapshot does not know): restore that block's predecessor from its snapshot instead.
    pub fn undo(&mut self, marks: &Marks, inserts: &Inserts) -> Result<W, TErr> {
        if self.applied.last() != Some(&false) { return Err(TErr::Missing); }
        let r = (|| {
            let mut t = self.root;
            for (k, _, _) in inserts.iter().rev() { let (s, o) = split(k); t = self.t_rm(t, &s, &o, 0)?; }
            for (k, _) in marks.iter().rev() { let (s, o) = split(k); t = self.t_unmark(t, &s, &o, 0)?; }
            Ok(t)
        })();
        let out = self.finish(r, 0);
        if out.is_ok() { self.applied.pop(); }
        out
    }

    /// Whether the last update applied in this session re-created a coin over its own spent record (BIP30, `s_ins`),
    /// which `undo` cannot restore; None when none is applied. Prep's reorg undo records keep it
    /// (prover/host/src/reorg.rs). Host only (`std`): the WASM verifier compiles this file without it.
    #[cfg(feature = "std")]
    pub fn last_apply_replaced(&self) -> Option<bool> { self.applied.last().copied() }

    /// Z1 (ZAPP_FOLLOWER.md §4): record the current root (`height`, `hash`) in the disk arena's `roots.log`, for a
    /// reader that serves facts at that height while prep runs on. In memory: nothing. Host only.
    #[cfg(feature = "std")]
    pub fn log_root(&mut self) {
        let h = crate::coinlog::h_of(&self.root());
        self.a.log_root(self.height, &self.hash, self.root, &h);
    }

    /// Z1: a new generation (REORG's rewind, before it reuses any slot): a reader re-resolves its root. Host only.
    #[cfg(feature = "std")]
    pub fn bump_gen(&mut self) { self.a.bump_gen(); }

    /// Point a store at a logged root (`roots.log`) to READ it, e.g. to check that a pinned root survived (prep's
    /// `libzync-store-digest --root-at`). Never apply or flush through it. Host only.
    #[cfg(feature = "std")]
    pub fn set_root_for_reading(&mut self, root: u64, height: u64) { (self.root, self.height) = (root, height); }

    /// Z1/REORG P3: drop the logged roots above `height` (the orphaned blocks of a rewind); returns the entries cut.
    #[cfg(feature = "std")]
    pub fn cut_roots_above(&mut self, height: u64) -> Vec<u8> { self.a.cut_roots_above(height) }

    /// Undo one block from a RECORD of it (docs/libzync/REORG.md §6, prep's `reorg/u_<h>.bin`): the keys it marked and
    /// inserted, and whether it re-created a coin over its spent record. Unlike `undo` this works for a block applied
    /// in an EARLIER session (a store reopened from its last flush), because the record carries the `replaced` flag the
    /// in-memory `applied` stack would: refused (`Missing`, the store unchanged) when it is set. Exact otherwise, as
    /// `undo`: the trie's shape is a function of its key set. All or nothing. Host only (`std`).
    #[cfg(feature = "std")]
    pub fn undo_recorded(&mut self, marks: &[Key], inserts: &[Key], replaced: bool) -> Result<W, TErr> {
        if replaced { return Err(TErr::Missing); }
        // an update applied in this session is the one being undone (undo order is newest first): keep the stack true
        if self.applied.last() == Some(&true) { return Err(TErr::Missing); }
        let r = (|| {
            let mut t = self.root;
            for k in inserts.iter().rev() { let (s, o) = split(k); t = self.t_rm(t, &s, &o, 0)?; }
            for k in marks.iter().rev() { let (s, o) = split(k); t = self.t_unmark(t, &s, &o, 0)?; }
            Ok(t)
        })();
        let out = self.finish(r, 0);
        if out.is_ok() { self.applied.pop(); }
        out
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
    fn disk_arena_matches_memory_and_survives_a_crash() { disk_arena_run(0, 0, "nocache"); }

    /// The record cache: a tiny one (a few dozen slots: every insert collides) with a flush every 7 blocks, so freed
    /// slots are reused under cached references; and a large one. Each must match the memory store record for record.
    #[cfg(feature = "std")]
    #[test]
    fn disk_arena_record_cache_is_exact() {
        disk_arena_run(1, 7, "tinycache");
        disk_arena_run(64, 0, "bigcache");
    }

    /// REORG.md §6 (review of P1, M5): a block applied in an EARLIER session is undone from its record alone (keys +
    /// replaced flag). A disk store is flushed at every block (so the slots each block replaced are freed and reused)
    /// and reopened; each reopen undoes the newest block by `undo_recorded` and must equal, snapshot byte for byte, a
    /// memory store built from the blocks below it. Undo then a flush and a reopen: still equal. A re-creating block
    /// is refused, the store unchanged.
    #[cfg(feature = "std")]
    #[test]
    fn a_block_from_an_earlier_session_undoes_from_its_record() {
        let _serial = super::DISK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(alloc::format!("hztrie-test-{}-undo-recorded", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut x = 0x2545_f491_4f6c_dd1du64;
        let mut rnd = move || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x };
        let mut disk = TrieStore::open_dir(&dir, true).unwrap();
        let mut unspent: Vec<Key> = Vec::new();
        let mut blocks: Vec<(Vec<(Key, W)>, Vec<(Key, Option<W>, u64)>)> = Vec::new();
        for blk in 0..120u32 {
            let mut marks = Vec::new();
            for _ in 0..(rnd() % 5) {
                if unspent.is_empty() { break; }
                let k = unspent.swap_remove((rnd() % unspent.len() as u64) as usize);
                marks.push((k, [blk, 3, 1, 0, 0, 0, 0, 0]));
            }
            let mut ins = Vec::new();
            for _ in 0..(1 + rnd() % 8) {
                let mut k = [0u8; 64];
                for b in k.iter_mut() { *b = rnd() as u8; }
                k[0] &= 0x07;
                if rnd() % 3 == 0 { k[1..4].fill(0); }
                let spent = rnd() % 5 == 0;
                if !spent { unspent.push(k); }
                ins.push((k, spent.then_some([blk, 4, 0, 0, 0, 0, 0, 0]), rnd() >> 30));
            }
            disk.apply(&marks, &ins).unwrap();
            assert_eq!(disk.last_apply_replaced(), Some(false));
            disk.height = blk as u64 + 1;
            disk.flush().unwrap();
            blocks.push((marks, ins));
        }
        let built = |n: usize| -> TrieStore {
            let mut m = TrieStore::new(true);
            for (mk, i) in &blocks[..n] { m.apply(mk, i).unwrap(); }
            m.height = n as u64;
            m
        };
        for n in (100..120usize).rev() {
            drop(disk);
            disk = TrieStore::open_dir(&dir, true).unwrap();
            assert_eq!(disk.height, n as u64 + 1, "reopened at the last flush");
            let (mk, i) = &blocks[n];
            let keys_m: Vec<Key> = mk.iter().map(|m| m.0).collect();
            let keys_i: Vec<Key> = i.iter().map(|x| x.0).collect();
            // the in-session undo refuses: this block was applied in another session
            assert_eq!(disk.undo(mk, i), Err(TErr::Missing), "undo knows nothing of an earlier session");
            assert_eq!(disk.undo_recorded(&keys_m, &keys_i, true), Err(TErr::Missing), "a re-creating block is refused");
            assert_eq!(disk.height, n as u64 + 1);
            disk.undo_recorded(&keys_m, &keys_i, false).unwrap_or_else(|e| panic!("block {n}: {e:?}"));
            disk.height = n as u64;
            assert_eq!(disk.to_bytes(), built(n).to_bytes(), "block {n} undone from its record = the store below it");
            disk.flush().unwrap();
        }
        drop(disk);
        let back = TrieStore::open_dir(&dir, true).unwrap();
        assert_eq!(back.to_bytes(), built(100).to_bytes(), "the undo is durable: reopened after the flush");
        // and a re-created coin's record: refused, nothing changed
        let mut st = TrieStore::new(true);
        let c = key(4, 0, 1, 0);
        st.apply(&[], &[(c, None, 50)]).unwrap();
        st.apply(&[(c, [5, 0, 0, 0, 0, 0, 0, 0])], &[]).unwrap();
        st.apply(&[], &[(c, None, 50)]).unwrap();
        let r = st.root();
        assert_eq!(st.undo_recorded(&[], &[c], false), Err(TErr::Missing), "in session, the stack's flag still refuses");
        assert_eq!(st.root(), r);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "reader")]
    fn z1_blocks_any(n: u32, seed: u64) -> Vec<(Vec<(Key, W)>, Vec<(Key, Option<W>, u64)>)> { z1_blocks(n, seed) }

    /// Random blocks on a disk store: (marks, inserts) per block, keys as the other disk tests make them.
    #[cfg(any(feature = "std", feature = "reader"))]
    fn z1_blocks(n: u32, seed: u64) -> Vec<(Vec<(Key, W)>, Vec<(Key, Option<W>, u64)>)> {
        let mut x = seed;
        let mut rnd = move || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x };
        let mut unspent: Vec<Key> = Vec::new();
        (0..n).map(|blk| {
            let mut marks = Vec::new();
            for _ in 0..(rnd() % 6) {
                if unspent.is_empty() { break; }
                let k = unspent.swap_remove((rnd() % unspent.len() as u64) as usize);
                marks.push((k, [blk, 1, 0, 0, 0, 0, 0, 0]));
            }
            let mut ins = Vec::new();
            for _ in 0..(1 + rnd() % 8) {
                let mut k = [0u8; 64];
                for b in k.iter_mut() { *b = rnd() as u8; }
                k[0] &= 0x07;
                unspent.push(k);
                ins.push((k, None, rnd() >> 30));
            }
            (marks, ins)
        }).collect()
    }

    /// ZAPP_FOLLOWER.md §4 (Z1): with `trie/PIN` at H, the root of H (from `roots.log`) is still the trie of H after many
    /// later blocks, flushes, slot reuse and a reopen; WITHOUT the pin the same run reuses its slots and the root of H
    /// no longer reads as that trie (the positive control). Removing the pin returns the held slots; the cap
    /// (HAZYNC_TRIE_RETAIN_MAX) releases a pin and says so in `PIN.released`. Every block's root is in `roots.log`.
    #[cfg(feature = "std")]
    #[test]
    fn z1_a_pinned_root_survives_later_blocks_and_slot_reuse() {
        let _serial = super::DISK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let blocks = z1_blocks(90, 0x51ed_270b_7f1a_c3e9);
        let snap_at = |h: usize| { let mut m = TrieStore::new(true); for (mk, i) in &blocks[..h] { m.apply(mk, i).unwrap(); } m.height = h as u64; m.to_bytes() };
        let root_of = |d: &std::path::Path, h: u32| -> (R, [u8; 32], u32) {
            let b = std::fs::read(d.join("roots.log")).unwrap();
            let e = b.chunks_exact(ROOTS_REC).rev().find(|e| u32::from_le_bytes(e[..4].try_into().unwrap()) == h).expect("height in roots.log");
            (u64::from_le_bytes(e[36..44].try_into().unwrap()), e[44..76].try_into().unwrap(), u32::from_le_bytes(e[76..80].try_into().unwrap()))
        };
        // HAZYNC_TRIE_RETAIN_MAX is read only at a flush WITH a pin, and no other test pins: setting it here is safe
        std::env::set_var("HAZYNC_TRIE_RETAIN_MAX", "1000");
        let mut results = Vec::new();
        for pinned in [true, false] {
            let dir = std::env::temp_dir().join(alloc::format!("hztrie-test-{}-z1-{pinned}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let mut st = TrieStore::open_dir(&dir, true).unwrap();
            for (n, (mk, i)) in blocks[..80].iter().enumerate() {
                st.apply(mk, i).unwrap();
                st.height = n as u64 + 1;
                st.log_root();
                if n + 1 == 20 && pinned { std::fs::write(dir.join("PIN"), b"20 1700000000\n").unwrap(); }
                if (n + 1) % 5 == 0 { st.flush().unwrap(); }
            }
            drop(st);
            let st = TrieStore::open_dir(&dir, true).unwrap();
            let (r20, h20, _) = root_of(&dir, 20);
            let view = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
                let mut v = TrieStore::open_dir(&dir, true).unwrap();
                v.root = r20;
                v.height = 20;
                v.to_bytes()
            }));
            let intact = view.as_ref().is_ok_and(|b| *b == snap_at(20));
            results.push(intact);
            // every block's root was logged, with its height and the trie's root hash at that height
            let rl = std::fs::read(dir.join("roots.log")).unwrap();
            assert_eq!(rl.len(), 80 * ROOTS_REC, "one entry per block");
            assert_eq!(h20, crate::coinlog::h_of(&{ let mut m = TrieStore::new(true); for (mk, i) in &blocks[..20] { m.apply(mk, i).unwrap(); } m }.root()));
            if pinned {
                let retained: u64 = std::fs::read_dir(dir.join("retain")).unwrap().flatten().map(|e| e.metadata().unwrap().len() / 8).sum();
                assert!(retained > 0, "slots freed above the pin are held in retain/");
                // the pin goes: the next flush returns them
                std::fs::remove_file(dir.join("PIN")).unwrap();
                let mut st = st;
                for (n, (mk, i)) in blocks[80..85].iter().enumerate() { st.apply(mk, i).unwrap(); st.height = 81 + n as u64; st.log_root(); }
                st.flush().unwrap();
                assert!(std::fs::read_dir(dir.join("retain")).unwrap().next().is_none(), "no pin: every held slot back in the free list");
                // the cap: a pin at 82 with RETAIN_MAX = 2 is released at the flush of 90
                std::fs::write(dir.join("PIN"), b"82\n").unwrap();
                std::env::set_var("HAZYNC_TRIE_RETAIN_MAX", "2");
                for (n, (mk, i)) in blocks[85..90].iter().enumerate() { st.apply(mk, i).unwrap(); st.height = 86 + n as u64; st.log_root(); }
                st.flush().unwrap();
                let rel = std::fs::read_to_string(dir.join("PIN.released")).unwrap();
                assert!(rel.starts_with("released_through 88 pin 82"), "the cap released the pin: {rel}");
                std::env::set_var("HAZYNC_TRIE_RETAIN_MAX", "1000");
                // and the live trie is still exact
                assert_eq!(st.to_bytes(), { let mut m = TrieStore::new(true); for (mk, i) in &blocks[..90] { m.apply(mk, i).unwrap(); } m.height = 90; m }.to_bytes());
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
        assert_eq!(results, alloc::vec![true, false], "pinned: the root of 20 intact; unpinned (the control): its slots were reused");
    }

    /// Z2 (ZAPP_FOLLOWER.md §4, §10): a read-only Reader at a pinned root (the root of 20 from roots.log) reads the trie
    /// of 20 byte for byte while the writer applies 60 more blocks with flushes every 5 and is reopened (a new GEN: the
    /// root is looked up again and still served); a walk during which GEN moves, one past the cap, and one reading past
    /// the file are refused, never wrong. WITHOUT the pin (the positive control) the reader refuses to open at 20: its
    /// slots were released (INTACT), and the Z1 test shows they were reused.
    #[cfg(all(feature = "std", feature = "reader"))]
    #[test]
    fn z2_a_reader_serves_a_pinned_root_while_prep_runs_on() {
        let _serial = super::DISK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let blocks = z1_blocks(80, 0x2a11_9e0c_44d1_b07f);
        let hash = |h: u64| -> [u8; 32] { core::array::from_fn(|i| (h as u8).wrapping_mul(7).wrapping_add(i as u8)) };
        let snap_at = |h: usize| { let mut m = TrieStore::new(true); for (mk, i) in &blocks[..h] { m.apply(mk, i).unwrap(); } (m.height, m.hash) = (h as u64, hash(h as u64)); m.to_bytes() };
        let root20 = { let mut m = TrieStore::new(true); for (mk, i) in &blocks[..20] { m.apply(mk, i).unwrap(); } crate::coinlog::h_of(&m.root()) };
        std::env::set_var("HAZYNC_TRIE_RETAIN_MAX", "1000");
        for pinned in [true, false] {
            let dir = std::env::temp_dir().join(alloc::format!("hztrie-test-{}-z2-{pinned}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let mut st = TrieStore::open_dir(&dir, true).unwrap();
            let mut reader: Option<Reader> = None;
            for (n, (mk, i)) in blocks.iter().enumerate() {
                st.apply(mk, i).unwrap();
                (st.height, st.hash) = (n as u64 + 1, hash(n as u64 + 1));
                st.log_root();
                if n + 1 == 20 {
                    if pinned { std::fs::write(dir.join("PIN"), b"20\n").unwrap(); }
                    st.flush().unwrap();
                    let r = Reader::open(&dir, 20, &hash(20), Some(&root20), 1 << 20).unwrap();
                    assert_eq!(r.walk(|s| s.to_bytes()).unwrap(), snap_at(20), "the reader at 20, just after the flush");
                    reader = Some(r);
                } else if (n + 1) % 5 == 0 { st.flush().unwrap(); }
                if pinned && n + 1 == 50 {
                    // the writer restarts (GEN bumped at its open, roots.log cut back to the flushed 50): the root of 20 is
                    // looked up again, still there, still served
                    drop(core::mem::replace(&mut st, TrieStore::new(true)));
                    st = TrieStore::open_dir(&dir, true).unwrap();
                }
                if pinned && n + 1 > 20 && (n + 1) % 10 == 0 {
                    assert_eq!(reader.as_ref().unwrap().walk(|s| s.to_bytes()).unwrap(), snap_at(20), "the reader at 20 after block {}", n + 1);
                }
            }
            st.flush().unwrap();
            let again = Reader::open(&dir, 20, &hash(20), Some(&root20), 1 << 20);
            if pinned {
                let r = again.unwrap();
                assert_eq!(r.walk(|s| s.to_bytes()).unwrap(), snap_at(20), "a reader opened after 80 blocks");
                assert!(matches!(r.walk(|s| s.to_bytes()).map(|_| ()), Ok(())));
                // the cap: a history-sized walk is refused, not served
                let r10 = Reader::open(&dir, 20, &hash(20), None, 40).unwrap();
                assert_eq!(r10.walk(|s| s.to_bytes()), Err(WalkErr::TooLarge(40)));
                // GEN moving during a walk: refused
                let g = std::fs::read(dir.join("GEN")).unwrap();
                let moved = r.walk(|s| { std::fs::write(dir.join("GEN"), (u32::from_le_bytes(g[..].try_into().unwrap()) + 9).to_le_bytes()).unwrap(); s.to_bytes() });
                assert!(matches!(moved, Err(WalkErr::Moved(_))), "{moved:?}");
                std::fs::write(dir.join("GEN"), &g).unwrap();
                // a record past the file: refused, no SIGBUS, no abort
                let past = r.walk(|s| s.get(1 << 40));
                assert!(matches!(past, Err(WalkErr::Damaged(_))), "{past:?}");
                // the wrong trie_root, or a block hash roots.log does not hold at 20: refused
                assert!(Reader::open(&dir, 20, &hash(20), Some(&[9; 32]), 1 << 20).is_err());
                assert!(Reader::open(&dir, 20, &hash(21), None, 1 << 20).unwrap_err().contains("no root"));
                // the pin goes and prep runs on: the next flush releases 20; the open reader stops serving it
                std::fs::remove_file(dir.join("PIN")).unwrap();
                st.apply(&[], &[([3u8; 64], None, 1)]).unwrap();
                (st.height, st.hash) = (81, hash(81));
                st.log_root();
                st.flush().unwrap();
                assert!(matches!(r.walk(|s| s.to_bytes()), Err(WalkErr::Moved(_))), "released under an open reader: refused");
            } else {
                let e = again.err().unwrap_or_default();
                assert!(e.contains("no longer intact"), "unpinned (the control): the root of 20 was released: {e}");
            }
            drop(st);
            let _ = std::fs::remove_dir_all(&dir);
        }
        std::env::remove_var("HAZYNC_TRIE_RETAIN_MAX");
    }

    /// Z2: the fixture writer (zync-serve's tests) writes a disk trie a Reader opens at its root, byte for byte the store.
    #[cfg(feature = "reader")]
    #[test]
    fn z2_the_disk_fixture_reads_back() {
        let mut m = TrieStore::new(true);
        for (mk, i) in &z1_blocks_any(30, 7) { m.apply(mk, i).unwrap(); }
        (m.height, m.hash) = (30, [5; 32]);
        let dir = std::env::temp_dir().join(alloc::format!("hztrie-test-{}-z2-fixture", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        m.write_disk_fixture(&dir, 3).unwrap();
        let r = Reader::open(&dir, 30, &[5; 32], Some(&crate::coinlog::h_of(&m.root())), 1 << 20).unwrap();
        assert_eq!(r.walk(|s| s.to_bytes()).unwrap(), m.to_bytes());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Review 16.11, H2: a crash between the meta's rename (the released lists' slots now in its free list) and the
    /// lists' unlinking must not hand those slots out twice. Pinned at 10, 30 blocks with flushes; then the pin goes and
    /// the flush that releases everything "crashes" right after the meta rename; reopen, flush again, then 30 more
    /// blocks: the free list never holds a slot twice and the trie stays exact.
    #[cfg(feature = "std")]
    #[test]
    fn z1_a_crash_after_releasing_retained_slots_frees_nothing_twice() {
        let _serial = super::DISK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(alloc::format!("hztrie-test-{}-z1-h2", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let blocks = z1_blocks(60, 0x3c6e_f372_fe94_f82b);
        let free_list = |d: &std::path::Path| -> Vec<u64> {
            let b = std::fs::read(d.join("meta")).unwrap();
            let n = u64::from_le_bytes(b[65..73].try_into().unwrap()) as usize;
            (0..n).map(|i| u64::from_le_bytes(b[73 + 8 * i..81 + 8 * i].try_into().unwrap())).collect()
        };
        let dups = |v: &[u64]| { let mut w = v.to_vec(); w.sort_unstable(); w.windows(2).filter(|x| x[0] == x[1]).count() };
        let mut st = TrieStore::open_dir(&dir, true).unwrap();
        std::fs::write(dir.join("PIN"), b"10\n").unwrap();
        for (n, (mk, i)) in blocks[..30].iter().enumerate() {
            st.apply(mk, i).unwrap(); st.height = n as u64 + 1; st.log_root();
            if (n + 1) % 5 == 0 { st.flush().unwrap(); }
        }
        assert!(std::fs::read_dir(dir.join("retain")).unwrap().count() > 0, "lists held under the pin");
        std::fs::remove_file(dir.join("PIN")).unwrap();
        TEST_CRASH_AFTER_META.store(true, core::sync::atomic::Ordering::SeqCst);
        assert!(st.flush().is_err(), "the simulated crash");
        TEST_CRASH_AFTER_META.store(false, core::sync::atomic::Ordering::SeqCst);
        drop(st);
        let mut st = TrieStore::open_dir(&dir, true).unwrap();
        st.flush().unwrap();
        let fl = free_list(&dir);
        assert_eq!(dups(&fl), 0, "a slot twice in the free list ({} of {})", dups(&fl), fl.len());
        for (n, (mk, i)) in blocks[30..].iter().enumerate() {
            st.apply(mk, i).unwrap_or_else(|e| panic!("block {}: {e:?} (a slot handed out twice)", 31 + n));
            st.height = 31 + n as u64;
            if (n + 1) % 5 == 0 { st.flush().unwrap(); }
        }
        let mut m = TrieStore::new(true);
        for (mk, i) in &blocks { m.apply(mk, i).unwrap(); }
        m.height = 60;
        assert_eq!(st.to_bytes(), m.to_bytes());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Review 16.11, M4 and the rewind leak: a PIN that does not read is a pin at 0 (held, not released); a pin writes
    /// INTACT (the lowest height whose root is intact) at each flush; an UNDO's freed slots are released at the next flush
    /// whatever the pin (they were tagged above the flush, and the next open dropped their list: never freed again).
    #[cfg(feature = "std")]
    #[test]
    fn z1_an_unreadable_pin_holds_and_an_undo_leaks_nothing() {
        let _serial = super::DISK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(alloc::format!("hztrie-test-{}-z1-m4", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let blocks = z1_blocks(20, 0x2b99_2ddf_a232_49d6);
        let mut st = TrieStore::open_dir(&dir, true).unwrap();
        std::fs::write(dir.join("PIN"), b"not a height").unwrap();
        for (n, (mk, i)) in blocks.iter().enumerate() {
            st.apply(mk, i).unwrap(); st.height = n as u64 + 1; st.log_root();
            if (n + 1) % 5 == 0 { st.flush().unwrap(); }
        }
        let lists = |d: &std::path::Path| -> Vec<u64> { let mut v: Vec<u64> = std::fs::read_dir(d.join("retain")).map(|it| it.flatten()
            .filter_map(|e| e.file_name().to_string_lossy().strip_prefix("r_")?.strip_suffix(".bin")?.parse().ok()).collect()).unwrap_or_default(); v.sort(); v };
        assert_eq!(lists(&dir).len(), 20, "an unreadable PIN holds every block's freed slots (within the cap)");
        assert!(std::fs::read_to_string(dir.join("INTACT")).unwrap().starts_with("intact_from 0 "));
        // undo the last 5 blocks from their keys (a rewind to 15) and flush there
        for (mk, i) in blocks[15..].iter().rev() {
            let km: Vec<Key> = mk.iter().map(|m| m.0).collect();
            let ki: Vec<Key> = i.iter().map(|x| x.0).collect();
            st.undo_recorded(&km, &ki, false).unwrap();
        }
        st.height = 15;
        st.flush().unwrap();
        assert!(lists(&dir).iter().all(|&k| k <= 15), "no list above the flushed height (it would be dropped, its slots lost): {:?}", lists(&dir));
        drop(st);
        let st = TrieStore::open_dir(&dir, true).unwrap();
        let mut m = TrieStore::new(true);
        for (mk, i) in &blocks[..15] { m.apply(mk, i).unwrap(); }
        m.height = 15;
        assert_eq!(st.to_bytes(), m.to_bytes());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Review 16.11 (N1, the reviewer's test): a root pinned BELOW a rewind's fork must survive the rewind and the blocks
    /// after it: the lists the orphaned blocks' forward applies held are nodes of older roots, the pinned one among them.
    #[cfg(feature = "std")]
    #[test]
    fn z1_a_root_pinned_below_a_rewinds_fork_survives_it() {
        let _serial = super::DISK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(alloc::format!("hztrie-test-{}-zz-review", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let blocks = z1_blocks(20, 0x51ed_270b_7f1a_c3e9);
        let newb = z1_blocks(60, 0x1234_5678_9abc_def1);
        let snap10 = { let mut m = TrieStore::new(true); for (mk, i) in &blocks[..10] { m.apply(mk, i).unwrap(); } m.height = 10; m.to_bytes() };
        std::env::set_var("HAZYNC_TRIE_RETAIN_MAX", "1000");
        let mut st = TrieStore::open_dir(&dir, true).unwrap();
        for (n, (mk, i)) in blocks.iter().enumerate() {
            st.apply(mk, i).unwrap(); st.height = n as u64 + 1; st.log_root();
            if n + 1 == 10 { std::fs::write(dir.join("PIN"), b"10\n").unwrap(); }
            if (n + 1) % 5 == 0 { st.flush().unwrap(); }
        }
        drop(st);
        let b = std::fs::read(dir.join("roots.log")).unwrap();
        let e = b.chunks_exact(ROOTS_REC).find(|e| u32::from_le_bytes(e[..4].try_into().unwrap()) == 10).unwrap();
        let r10 = u64::from_le_bytes(e[36..44].try_into().unwrap());
        let view = |d: &std::path::Path| std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
            let mut v = TrieStore::open_dir(d, true).unwrap(); v.root = r10; v.height = 10; v.to_bytes() }));
        assert!(view(&dir).is_ok_and(|b| b == snap10), "before the rewind the root of 10 is intact");
        // the rewind, as rewind.rs: open, undo 20..16, cut, log F, flush at 15
        let mut st = TrieStore::open_dir(&dir, true).unwrap();
        for (mk, i) in blocks[15..].iter().rev() {
            let km: Vec<Key> = mk.iter().map(|m| m.0).collect();
            let ki: Vec<Key> = i.iter().map(|x| x.0).collect();
            st.undo_recorded(&km, &ki, false).unwrap();
        }
        st.cut_roots_above(14);
        st.height = 15; st.log_root();
        st.flush().unwrap();
        std::eprintln!("INTACT after rewind: {}", std::fs::read_to_string(dir.join("INTACT")).unwrap());
        assert!(view(&dir).is_ok_and(|b| b == snap10), "right after the rewind flush");
        drop(st); // (one writer per arena: ARENA.LOCK)
        let mut st = TrieStore::open_dir(&dir, true).unwrap();
        for (n, (mk, i)) in newb.iter().enumerate() {
            st.apply(mk, i).unwrap(); st.height = 16 + n as u64; st.log_root();
            if (n + 1) % 5 == 0 { st.flush().unwrap(); }
        }
        drop(st);
        std::eprintln!("INTACT at the end: {}", std::fs::read_to_string(dir.join("INTACT")).unwrap());
        let ok = view(&dir).is_ok_and(|b| b == snap10);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(ok, "PIN 10, fork 15: the root of 10 was overwritten after the rewind");
    }

    /// Review 16.11 (N2, the reviewer's test): INTACT reports what was RELEASED, not the current pin: a pin written at 42
    /// for 20 comes after the flushes that released 20's slots.
    #[cfg(feature = "std")]
    #[test]
    fn z1_intact_tells_the_truth_after_a_late_pin() {
        let _serial = super::DISK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(alloc::format!("hztrie-test-{}-zz-review2", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let blocks = z1_blocks(60, 0x51ed_270b_7f1a_c3e9);
        let snap20 = { let mut m = TrieStore::new(true); for (mk, i) in &blocks[..20] { m.apply(mk, i).unwrap(); } m.height = 20; m.to_bytes() };
        std::env::set_var("HAZYNC_TRIE_RETAIN_MAX", "1000");
        let mut st = TrieStore::open_dir(&dir, true).unwrap();
        for (n, (mk, i)) in blocks.iter().enumerate() {
            st.apply(mk, i).unwrap(); st.height = n as u64 + 1; st.log_root();
            if n + 1 == 42 { std::fs::write(dir.join("PIN"), b"20\n").unwrap(); }
            if (n + 1) % 5 == 0 { st.flush().unwrap(); }
        }
        drop(st);
        let b = std::fs::read(dir.join("roots.log")).unwrap();
        let e = b.chunks_exact(ROOTS_REC).find(|e| u32::from_le_bytes(e[..4].try_into().unwrap()) == 20).unwrap();
        let r20 = u64::from_le_bytes(e[36..44].try_into().unwrap());
        let intact = std::fs::read_to_string(dir.join("INTACT")).unwrap();
        let view = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
            let mut v = TrieStore::open_dir(&dir, true).unwrap(); v.root = r20; v.height = 20; v.to_bytes() }));
        let ok = view.is_ok_and(|b| b == snap20);
        let rel = std::fs::read_to_string(dir.join("PIN.released")).ok();
        let _ = std::fs::remove_dir_all(&dir);
        // INTACT may claim the root of 20 only if it reads as the trie of 20 (a pin written at 42 comes too late for it)
        let from: u64 = intact.split_whitespace().nth(1).and_then(|x| x.parse().ok()).expect("INTACT: intact_from <h>");
        assert!(from > 20 || ok, "INTACT says {intact:?} (PIN.released {rel:?}) but the root of 20 reads as the trie of 20: {ok}");
    }

    /// Re-review 16.11 lows: (L-1) a second opener is a READER (no recovery, it cannot write or flush); (L-3) a second
    /// cut before the flush keeps the entries logged since the first; (L-2) a RETAG that is not UTF-8 is rolled back and
    /// removed at open, never left.
    #[cfg(feature = "std")]
    #[test]
    fn z1_one_writer_cumulative_cuts_and_a_torn_retag() {
        let _serial = super::DISK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(alloc::format!("hztrie-test-{}-z1-lows", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let blocks = z1_blocks(12, 0x0bad_cafe_1234_5678);
        let mut st = TrieStore::open_dir(&dir, true).unwrap();
        for (n, (mk, i)) in blocks[..10].iter().enumerate() { st.apply(mk, i).unwrap(); st.height = n as u64 + 1; st.log_root(); }
        st.flush().unwrap();
        let gen = std::fs::read(dir.join("GEN")).unwrap();
        {
            let mut rd = TrieStore::open_dir(&dir, true).unwrap();
            assert_eq!(std::fs::read(dir.join("GEN")).unwrap(), gen, "a reader runs no recovery (no GEN bump)");
            assert!(rd.flush().is_err(), "a reader cannot flush");
            assert!(std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| rd.apply(&blocks[10].0, &blocks[10].1))).is_err(), "nor write");
        }
        // two cuts before one flush: log 10' after the first, then cut at 10 again: 10' stays
        st.cut_roots_above(8);
        st.height = 9; st.log_root();
        st.height = 10; st.log_root();
        st.cut_roots_above(9);
        st.flush().unwrap();
        let rl = std::fs::read(dir.join("roots.log")).unwrap();
        let hs: Vec<u32> = rl.chunks_exact(ROOTS_REC).map(|e| u32::from_le_bytes(e[..4].try_into().unwrap())).collect();
        assert_eq!(hs, (1..=9).collect::<Vec<u32>>(), "1..8 kept, 9 logged after the first cut kept, 10 cut by the second");
        drop(st);
        std::fs::create_dir_all(dir.join("retain")).unwrap();
        std::fs::write(dir.join("retain/RETAG"), [0xffu8, 0xfe, 0x00, 0x80]).unwrap();
        let _st = TrieStore::open_dir(&dir, true).unwrap();
        assert!(!dir.join("retain/RETAG").exists(), "a torn RETAG is removed at open");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Z1: `GEN` is bumped (and synced) at every open and every `bump_gen` (a rewind); each `roots.log` entry carries the
    /// generation it was written in. A crash after the flush at 10: the entries of 11..15 are cut at the next open (the
    /// replay reuses their slots), and a `retain/` list of a lost block is dropped.
    #[cfg(feature = "std")]
    #[test]
    fn z1_gen_counts_opens_and_rewinds_and_a_crash_cuts_the_log() {
        let _serial = super::DISK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(alloc::format!("hztrie-test-{}-z1-gen", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let gen = |d: &std::path::Path| u32::from_le_bytes(std::fs::read(d.join("GEN")).unwrap().try_into().unwrap());
        let blocks = z1_blocks(15, 0x7a3c_91d2_0e44_b5f7);
        let mut st = TrieStore::open_dir(&dir, true).unwrap();
        assert_eq!(gen(&dir), 1, "the first open");
        for (n, (mk, i)) in blocks.iter().enumerate() {
            st.apply(mk, i).unwrap();
            st.height = n as u64 + 1;
            st.log_root();
            if n + 1 == 10 { st.flush().unwrap(); }
        }
        st.bump_gen();
        assert_eq!(gen(&dir), 2, "a rewind's bump");
        let rl = std::fs::read(dir.join("roots.log")).unwrap();
        assert_eq!(rl.len(), 15 * ROOTS_REC);
        assert!(rl.chunks_exact(ROOTS_REC).all(|e| u32::from_le_bytes(e[76..80].try_into().unwrap()) == 1), "all written in gen 1");
        // "crash": no flush after 10; a stray list of a lost block
        drop(st);
        std::fs::create_dir_all(dir.join("retain")).unwrap();
        std::fs::write(dir.join("retain/r_13.bin"), 7u64.to_le_bytes()).unwrap();
        std::fs::write(dir.join("retain/r_9.bin"), 5u64.to_le_bytes()).unwrap();
        let mut st = TrieStore::open_dir(&dir, true).unwrap();
        assert_eq!((st.height, gen(&dir)), (10, 3), "back at the flush; the open bumped the generation");
        let rl = std::fs::read(dir.join("roots.log")).unwrap();
        assert_eq!(rl.len(), 10 * ROOTS_REC, "the roots above the flushed height are cut");
        assert!(!dir.join("retain/r_13.bin").exists() && dir.join("retain/r_9.bin").exists(), "a lost block's list dropped, an older one kept");
        std::fs::remove_file(dir.join("retain/r_9.bin")).unwrap(); // (its slot is made up: never let a flush release it)
        // the replay logs again in the new generation
        for (n, (mk, i)) in blocks[10..].iter().enumerate() { st.apply(mk, i).unwrap(); st.height = 11 + n as u64; st.log_root(); }
        let rl = std::fs::read(dir.join("roots.log")).unwrap();
        assert!(rl.chunks_exact(ROOTS_REC).skip(10).all(|e| u32::from_le_bytes(e[76..80].try_into().unwrap()) == 3));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "std")]
    fn disk_arena_run(cache_mb: u64, flush_every: u32, tag: &str) {
        let _serial = super::DISK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        // cache_mb 1 means a tiny cache of 64 slots (below what a MiB gives), so nearly every insert collides
        let mb = if cache_mb == 1 { 0 } else { cache_mb };
        let dir = std::env::temp_dir().join(alloc::format!("hztrie-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let open = |d: &std::path::Path| -> TrieStore {
            if cache_mb == 1 {
                let (a, meta) = DiskArena::open_cached(d, 0).unwrap();
                let mut a = a;
                a.cache = core::cell::RefCell::new(RecCache::new_slots(64));
                let mut st = TrieStore::with_arena(true, Box::new(a));
                if let Some((root, _, height, hash)) = meta { (st.root, st.height, st.hash) = (root, height, hash); }
                st
            } else { TrieStore::open_dir_cached(d, true, mb).unwrap() }
        };
        let mut x = 0x9e3779b97f4a7c15u64;
        let mut rnd = move || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x };
        let (mut mem, mut disk) = (TrieStore::new(true), open(&dir));
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
            if blk == 199 || (flush_every > 0 && blk < 199 && blk % flush_every == 0) { disk.flush().unwrap(); }
        }
        assert_eq!(mem.to_bytes(), disk.to_bytes());
        let coins = all.len() as u64;
        std::println!("trie arena: {} coins, {} nodes ({:.2} per coin, {} bytes per coin at {RECORD}-byte records)",
                      coins, disk.nodes(), disk.nodes() as f64 / coins as f64, disk.nodes() * RECORD as u64 / coins);
        if cache_mb > 0 {
            let (h, m) = disk.cache_stats();
            std::println!("record cache [{tag}]: {h} hits, {m} misses");
            assert!(h > 0, "the record cache was never hit: the test would not show it exact");
        }
        // "crash": drop the disk store without flushing blocks 200..299; reopen = the trie flushed at 199
        drop(disk);
        let mut back = open(&dir);
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

    /// Undo is refused for a re-creating update however many levels back it is, and for a store loaded from a snapshot
    /// (review round 7: the guard used to cover only the last update applied).
    #[test]
    fn undo_refuses_a_re_creation_further_back_and_a_loaded_store() {
        for counts in [true, false] {
            let mut st = TrieStore::new(counts);
            let (c, other, later) = (key(4, 0, 1, 0), key(4, 0, 9, 1), key(7, 0, 3, 0));
            st.apply(&[], &[(c, None, 50), (other, None, 7)]).unwrap();
            st.apply(&[(c, [5, 0, 0, 0, 0, 0, 0, 0])], &[]).unwrap();
            let before_a = st.root();
            assert_eq!(st.last_apply_replaced(), Some(false));
            st.apply(&[], &[(c, None, 50)]).unwrap();              // A: re-creates c over its spent record
            assert_eq!(st.last_apply_replaced(), Some(true), "A is reported as a re-creation");
            let after_a = st.root();
            st.apply(&[], &[(later, None, 3)]).unwrap();           // B: an ordinary block
            assert_eq!(st.last_apply_replaced(), Some(false));
            assert_eq!(st.undo(&[], &[(later, None, 3)]), Ok(after_a), "B undoes");
            assert_eq!(st.undo(&[], &[(c, None, 50)]), Err(TErr::Missing), "A, now the last, is refused");
            assert_eq!(st.root(), after_a);
            assert_ne!(st.root(), before_a);
            // a store loaded from a snapshot knows nothing of how its updates were applied: undo refused
            let mut back = TrieStore::from_bytes(&st.to_bytes()).unwrap();
            assert_eq!(back.last_apply_replaced(), None, "nothing applied in a loaded store's session");
            assert_eq!(back.undo(&[], &[(c, None, 50)]), Err(TErr::Missing), "loaded store");
        }
    }

    /// Two script hashes sharing their first 255 bits put both address leaves at depth 256: the guest accepts it, and
    /// so must the model and the store, with the same root (review round 7: they refused any insert at depth 256).
    /// A third address there would need a split at depth 256: refused by all three.
    #[test]
    fn address_leaves_at_depth_256_agree_with_the_guest() {
        for counts in [true, false] {
            let mut k1 = key(9, 0, 1, 0);
            k1[31] = 0x02;
            let mut k2 = k1;
            k2[31] = 0x03;                 // only bit 255 of the script hash differs
            k2[32] = 2;                    // another coin
            let mut st = TrieStore::new(counts);
            st.apply(&[], &[(k1, None, 10)]).unwrap();
            let old = st.root();
            let ins: Vec<(Key, Option<W>, u64)> = alloc::vec![(k2, None, 20)];
            let wit = st.witness(&[], &ins);
            let new = st.apply(&[], &ins).unwrap();
            assert_eq!(apply_stream_perm(&wit, &[], &ins, Some(&perm(&[], &ins))), Ok(new), "guest");
            let (_, _, mut t) = decode(&wit).unwrap();
            let mut r = old;
            model_apply(&mut r, &mut t, counts, &[], &ins).unwrap();
            assert_eq!(r, new, "model");
            // a further coin at each depth-256 address: fine everywhere
            let mut k3 = k2;
            k3[32] = 3;
            let ins3: Vec<(Key, Option<W>, u64)> = alloc::vec![(k3, None, 5)];
            let wit = st.witness(&[], &ins3);
            let new3 = st.apply(&[], &ins3).unwrap();
            assert_eq!(apply_stream_perm(&wit, &[], &ins3, Some(&perm(&[], &ins3))), Ok(new3), "guest, a second coin");
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

/// The 16.11 re-review's adversarial tests (rewinds, pins, crashes), kept as regression tests.

#[cfg(all(test, feature = "std"))]
mod adv {
    use super::*;
    use alloc::vec::Vec;
    type Blk = (Vec<(Key, W)>, Vec<(Key, Option<W>, u64)>);
    fn blocks(n: u32, seed: u64) -> Vec<Blk> {
        let mut x = seed;
        let mut rnd = move || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x };
        let mut unspent: Vec<Key> = Vec::new();
        (0..n).map(|blk| {
            let mut marks = Vec::new();
            for _ in 0..(rnd() % 6) {
                if unspent.is_empty() { break; }
                let k = unspent.swap_remove((rnd() % unspent.len() as u64) as usize);
                marks.push((k, [blk, 1, 0, 0, 0, 0, 0, 0]));
            }
            let mut ins = Vec::new();
            for _ in 0..(1 + rnd() % 8) {
                let mut k = [0u8; 64];
                for b in k.iter_mut() { *b = rnd() as u8; }
                k[0] &= 0x07;
                unspent.push(k);
                ins.push((k, None, rnd() >> 30));
            }
            (marks, ins)
        }).collect()
    }
    struct Sim { dir: std::path::PathBuf, chain: Vec<Blk>, flushed: usize, log: Vec<alloc::string::String> }
    impl Sim {
        fn new(tag: &str) -> Sim {
            let dir = std::env::temp_dir().join(alloc::format!("hztrie-adv-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::env::set_var("HAZYNC_TRIE_RETAIN_MAX", "1000");
            Sim { dir, chain: Vec::new(), flushed: 0, log: Vec::new() }
        }
        fn pin(&self, p: Option<u64>) { match p { Some(p) => std::fs::write(self.dir.join("PIN"), alloc::format!("{p}\n")).unwrap(), None => { let _ = std::fs::remove_file(self.dir.join("PIN")); } } }
        fn snap(&self, h: usize) -> Vec<u8> { let mut m = TrieStore::new(true); for (mk, i) in &self.chain[..h] { m.apply(mk, i).unwrap(); } m.height = h as u64; m.to_bytes() }
        fn model_nodes(&self, h: usize) -> u64 { let mut m = TrieStore::new(true); for (mk, i) in &self.chain[..h] { m.apply(mk, i).unwrap(); } m.nodes() }
        /// one session: open, apply, flush every `every` (and at the end)
        fn fwd(&mut self, bl: &[Blk], every: usize) {
            let mut st = TrieStore::open_dir(&self.dir, true).unwrap();
            assert_eq!(st.height as usize, self.chain.len());
            for (n, (mk, i)) in bl.iter().enumerate() {
                st.apply(mk, i).unwrap(); self.chain.push((mk.clone(), i.clone())); st.height = self.chain.len() as u64; st.log_root();
                if (n + 1) % every == 0 || n + 1 == bl.len() { st.flush().unwrap(); self.flushed = self.chain.len(); }
            }
        }
        /// as rewind.rs
        fn rewind(&mut self, f: usize) -> Result<(), alloc::string::String> {
            let mut st = TrieStore::open_dir(&self.dir, true).unwrap();
            for (mk, i) in self.chain[f..].iter().rev() {
                let km: Vec<Key> = mk.iter().map(|m| m.0).collect();
                let ki: Vec<Key> = i.iter().map(|x| x.0).collect();
                st.undo_recorded(&km, &ki, false).unwrap();
            }
            st.cut_roots_above(f as u64 - 1);
            st.height = f as u64; st.log_root();
            st.flush()?;
            self.chain.truncate(f); self.flushed = f;
            Ok(())
        }
        fn lists(&self) -> Vec<alloc::string::String> { let mut v: Vec<_> = std::fs::read_dir(self.dir.join("retain")).map(|it| it.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default(); v.sort(); v }
        fn intact(&self) -> u64 { std::fs::read_to_string(self.dir.join("INTACT")).unwrap().split_whitespace().nth(1).unwrap().parse().unwrap() }
        fn root_ok(&self, h: usize) -> bool {
            let b = std::fs::read(self.dir.join("roots.log")).unwrap();
            let Some(e) = b.chunks_exact(ROOTS_REC).filter(|e| u32::from_le_bytes(e[..4].try_into().unwrap()) == h as u32).last() else { return false };
            let r = u64::from_le_bytes(e[36..44].try_into().unwrap());
            let want = self.snap(h);
            let d = self.dir.clone();
            std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| { let mut v = TrieStore::open_dir(&d, true).unwrap(); v.root = r; v.height = h as u64; v.to_bytes() })).is_ok_and(|b| b == want)
        }
        /// every height INTACT claims (>= intact_from, <= flushed) reads as the model's trie; returns the broken ones
        fn check(&mut self, what: &str) -> Vec<usize> {
            let from = self.intact() as usize;
            let bad: Vec<usize> = (from.max(1)..=self.flushed).filter(|&h| !self.root_ok(h)).collect();
            self.log.push(alloc::format!("{what}: INTACT {from} flushed {} lists {:?} broken {:?}", self.flushed, self.lists(), bad));
            std::eprintln!("{}", self.log.last().unwrap());
            bad
        }
        /// no pin, one more flush: nothing held, and the disk store has exactly the model's nodes (no leak, no double free)
        fn leak(&mut self, extra: &[Blk]) -> (u64, u64, Vec<alloc::string::String>) {
            self.pin(None);
            self.fwd(extra, 1);
            let st = TrieStore::open_dir(&self.dir, true).unwrap();
            assert_eq!(st.to_bytes(), self.snap(self.chain.len()), "final content");
            (st.nodes(), self.model_nodes(self.chain.len()), self.lists())
        }
    }
    fn serial() -> std::sync::MutexGuard<'static, ()> { super::DISK_TESTS.lock().unwrap_or_else(|e| e.into_inner()) }

    /// two rewinds before the pin passes either: 20 -> 15, on to 25, -> 12, on; pin 10 throughout; then the pin moves on
    #[test]
    fn adv_two_rewinds_second_below_first() {
        let _s = serial();
        let mut s = Sim::new("two");
        let a = blocks(20, 1); let b = blocks(10, 2); let c = blocks(30, 3); let d = blocks(3, 4);
        s.fwd(&a[..10], 5); s.pin(Some(10)); s.fwd(&a[10..], 5);
        assert!(s.check("at 20").is_empty());
        s.rewind(15).unwrap(); assert!(s.check("rewound 15").is_empty()); assert!(s.root_ok(10));
        s.fwd(&b, 5); assert!(s.check("at 25").is_empty()); assert!(s.root_ok(10));
        s.rewind(12).unwrap(); assert!(s.check("rewound 12").is_empty()); assert!(s.root_ok(10));
        s.fwd(&c[..10], 3); assert!(s.check("at 22").is_empty()); assert!(s.root_ok(10) && s.root_ok(11) && s.root_ok(12));
        s.pin(Some(11)); s.fwd(&c[10..12], 1); assert!(s.check("pin 11").is_empty()); assert!(s.root_ok(11));
        s.pin(Some(13)); s.fwd(&c[12..14], 1); assert!(s.check("pin 13").is_empty()); assert!(s.root_ok(13));
        s.pin(Some(20)); s.fwd(&c[14..], 4); assert!(s.check("pin 20").is_empty()); assert!(s.root_ok(20));
        let (disk, model, lists) = s.leak(&d);
        assert_eq!((disk, lists.len()), (model, 0), "leak: disk {disk} model {model} lists {lists:?}");
    }

    /// the SAME fork height twice (a tip that flaps): the second rewind's r_<F>_<k> names collide with the first's
    #[test]
    fn adv_same_fork_twice() {
        let _s = serial();
        let mut s = Sim::new("same");
        let a = blocks(20, 1); let b = blocks(5, 2); let c = blocks(20, 3); let d = blocks(3, 4);
        s.fwd(&a[..10], 5); s.pin(Some(10)); s.fwd(&a[10..], 5);
        s.rewind(15).unwrap(); let l1 = s.lists();
        let sz1: u64 = l1.iter().map(|n| std::fs::metadata(s.dir.join("retain").join(n)).unwrap().len()).sum();
        s.fwd(&b, 5);
        let before: u64 = s.lists().iter().map(|n| std::fs::metadata(s.dir.join("retain").join(n)).unwrap().len()).sum();
        s.rewind(15).unwrap();
        let after: u64 = s.lists().iter().map(|n| std::fs::metadata(s.dir.join("retain").join(n)).unwrap().len()).sum();
        std::eprintln!("same fork twice: lists after 1st {l1:?} ({sz1} B); before 2nd rewind {before} B, after {after} B; {:?}", s.lists());
        assert!(s.check("rewound 15 twice").is_empty()); assert!(s.root_ok(10));
        s.fwd(&c, 5); assert!(s.check("on").is_empty()); assert!(s.root_ok(10));
        let (disk, model, lists) = s.leak(&d);
        assert_eq!(before, after, "held slots lost by the rename ({} slots)", (before - after) / 8);
        assert_eq!((disk, lists.len()), (model, 0), "leak: disk {disk} model {model} lists {lists:?}");
    }

    /// pin AT the fork, and pin between the fork and the old tip
    #[test]
    fn adv_pin_at_or_above_fork() {
        let _s = serial();
        for pin in [15u64, 18] {
            let mut s = Sim::new("pinF");
            let a = blocks(20, 1); let b = blocks(20, 2); let d = blocks(3, 4);
            s.fwd(&a[..10], 5); s.pin(Some(pin)); s.fwd(&a[10..], 5);
            s.rewind(15).unwrap(); assert!(s.check("rewound 15").is_empty(), "pin {pin}"); assert!(s.root_ok(15));
            s.fwd(&b, 5); assert!(s.check("on").is_empty(), "pin {pin}");
            if pin == 15 { assert!(s.root_ok(15), "pin 15: the new root of 15"); }
            let (disk, model, lists) = s.leak(&d);
            assert_eq!((disk, lists.len()), (model, 0), "pin {pin} leak: disk {disk} model {model} lists {lists:?}");
        }
    }

    /// a crash mid-rename at the rewind's flush (some r_<k> already r_<F>_<k>, the meta still at A), then the rewind reruns
    #[test]
    fn adv_crash_mid_rename_then_rerun() {
        let _s = serial();
        let mut s = Sim::new("midren");
        let a = blocks(20, 1); let b = blocks(20, 2); let d = blocks(3, 4);
        s.fwd(&a[..10], 5); s.pin(Some(10)); s.fwd(&a[10..], 5);
        for k in [16, 18, 20] { let _ = std::fs::rename(s.dir.join(alloc::format!("retain/r_{k}.bin")), s.dir.join(alloc::format!("retain/r_15_{k}.bin"))); }
        s.rewind(15).unwrap(); assert!(s.check("rerun").is_empty()); assert!(s.root_ok(10));
        s.fwd(&b, 5); assert!(s.check("on").is_empty()); assert!(s.root_ok(10));
        let (disk, model, lists) = s.leak(&d);
        assert_eq!((disk, lists.len()), (model, 0), "leak: disk {disk} model {model} lists {lists:?}");
    }

    /// the same crash, but the rewind is NOT rerun (abandoned: the old branch won after all) and the pin moves to 17
    #[test]
    fn adv_crash_mid_rename_then_abandoned() {
        let _s = serial();
        let mut s = Sim::new("abandon");
        let a = blocks(20, 1); let b = blocks(20, 2);
        s.fwd(&a[..10], 5); s.pin(Some(10)); s.fwd(&a[10..], 5);
        // the crash, as it happens now: the rewind's flush re-tags its lists (journaled in retain/RETAG) and dies before the
        // meta. (The reviewer's version renamed r_<k> by hand, as the pre-RETAG code left it.)
        TEST_CRASH_BEFORE_META.store(true, core::sync::atomic::Ordering::SeqCst);
        let saved = s.chain.clone();
        let r = s.rewind(15);
        TEST_CRASH_BEFORE_META.store(false, core::sync::atomic::Ordering::SeqCst);
        assert!(r.is_err() && s.dir.join("retain/RETAG").exists(), "the simulated crash, with its journal");
        s.chain = saved; s.flushed = 20;                         // the rewind is abandoned: the old branch stays
        s.pin(Some(17));
        s.fwd(&b, 5);
        let bad = s.check("abandoned, pin 17");
        assert!(bad.is_empty() && s.root_ok(17), "INTACT {} but broken {bad:?}; root 17 ok {}", s.intact(), s.root_ok(17));
    }

    /// a crash right after the meta's rename at a rewind's flush that releases the re-tagged lists (pin at the fork)
    #[test]
    fn adv_crash_after_meta_at_rewind_release() {
        let _s = serial();
        let mut s = Sim::new("x-z1-h2");
        let a = blocks(20, 1); let b = blocks(20, 2); let d = blocks(3, 4);
        s.fwd(&a[..10], 5); s.pin(Some(15)); s.fwd(&a[10..], 5);
        TEST_CRASH_AFTER_META.store(true, core::sync::atomic::Ordering::SeqCst);
        let r = s.rewind(15);
        TEST_CRASH_AFTER_META.store(false, core::sync::atomic::Ordering::SeqCst);
        assert!(r.is_err());
        std::eprintln!("after the crash: {:?}", s.lists());
        s.chain.truncate(15); s.flushed = 15;
        s.fwd(&b, 5); assert!(s.check("on").is_empty());
        let (disk, model, lists) = s.leak(&d);
        assert_eq!((disk, lists.len()), (model, 0), "leak: disk {disk} model {model} lists {lists:?}");
    }

    /// apply and undo in ONE session (no flush between), flush at the fork, reopen
    #[test]
    fn adv_apply_and_undo_in_one_session() {
        let _s = serial();
        let mut s = Sim::new("insess");
        let a = blocks(20, 1); let b = blocks(20, 2); let d = blocks(3, 4);
        s.fwd(&a[..10], 5); s.pin(Some(10)); s.fwd(&a[10..15], 5);
        {
            let mut st = TrieStore::open_dir(&s.dir, true).unwrap();
            for (n, (mk, i)) in a[15..].iter().enumerate() { st.apply(mk, i).unwrap(); st.height = 16 + n as u64; st.log_root(); }
            for (mk, i) in a[15..].iter().rev() { st.undo(mk, i).unwrap(); }
            st.cut_roots_above(14); st.height = 15; st.log_root(); st.flush().unwrap();
        }
        std::eprintln!("in-session: lists {:?}", s.lists());
        assert!(s.check("in-session undo").is_empty()); assert!(s.root_ok(10));
        s.fwd(&b, 5); assert!(s.check("on").is_empty()); assert!(s.root_ok(10));
        let (disk, model, lists) = s.leak(&d);
        assert_eq!((disk, lists.len()), (model, 0), "leak: disk {disk} model {model} lists {lists:?}");
    }

    /// the pin removed just before a rewind: INTACT afterwards
    #[test]
    fn adv_no_pin_at_rewind_intact() {
        let _s = serial();
        let mut s = Sim::new("nopin");
        let a = blocks(20, 1); let b = blocks(20, 2);
        s.fwd(&a[..10], 5); s.pin(Some(10)); s.fwd(&a[10..], 5);
        s.pin(None);
        s.rewind(15).unwrap(); s.check("no pin, rewound 15");
        let i = s.intact();
        s.pin(Some(16)); s.fwd(&b[..3], 1); let bad = s.check("pin 16 at 18");
        std::eprintln!("INTACT after a pin-less rewind to 15: {i}; later {} ; root 16 ok {} broken {bad:?}", s.intact(), s.root_ok(16));
        assert!(bad.is_empty());
    }
}

#[cfg(all(test, feature = "std"))]
mod adv2 {
    use super::*;
    use alloc::vec::Vec;
    use alloc::string::String;
    use core::sync::atomic::Ordering::SeqCst;
    type Blk = (Vec<(Key, W)>, Vec<(Key, Option<W>, u64)>);
    fn blocks(n: u32, seed: u64) -> Vec<Blk> {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut rnd = move || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x };
        let mut unspent: Vec<Key> = Vec::new();
        (0..n).map(|blk| {
            let mut marks = Vec::new();
            for _ in 0..(rnd() % 6) {
                if unspent.is_empty() { break; }
                let k = unspent.swap_remove((rnd() % unspent.len() as u64) as usize);
                marks.push((k, [blk, 1, 0, 0, 0, 0, 0, 0]));
            }
            let mut ins = Vec::new();
            for _ in 0..(1 + rnd() % 8) {
                let mut k = [0u8; 64];
                for b in k.iter_mut() { *b = rnd() as u8; }
                k[0] &= 0x07;
                unspent.push(k);
                ins.push((k, None, rnd() >> 30));
            }
            (marks, ins)
        }).collect()
    }
    pub struct Sim { pub dir: std::path::PathBuf, pub chain: Vec<Blk>, pub flushed: usize, pub pin: Option<u64>, pub trace: Vec<String> }
    impl Sim {
        pub fn new(tag: &str) -> Sim {
            let dir = std::env::temp_dir().join(alloc::format!("hztrie-adv2-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::env::set_var("HAZYNC_TRIE_RETAIN_MAX", "1000");
            Sim { dir, chain: Vec::new(), flushed: 0, pin: None, trace: Vec::new() }
        }
        fn t(&mut self, s: String) { self.trace.push(s); }
        fn set_pin(&mut self, p: Option<u64>) { self.pin = p; match p { Some(p) => std::fs::write(self.dir.join("PIN"), alloc::format!("{p}\n")).unwrap(), None => { let _ = std::fs::remove_file(self.dir.join("PIN")); } } }
        fn model(&self, h: usize) -> TrieStore { let mut m = TrieStore::new(true); for (mk, i) in &self.chain[..h] { m.apply(mk, i).unwrap(); } m.height = h as u64; m }
        fn meta_h(&self) -> usize { std::fs::read(self.dir.join("meta")).map(|b| u64::from_le_bytes(b[17..25].try_into().unwrap()) as usize).unwrap_or(0) }
        fn fwd(&mut self, bl: &[Blk], every: usize) {
            let mut st = TrieStore::open_dir(&self.dir, true).unwrap();
            assert_eq!(st.height as usize, self.chain.len(), "fwd: store height");
            for (n, (mk, i)) in bl.iter().enumerate() {
                st.apply(mk, i).unwrap(); self.chain.push((mk.clone(), i.clone())); st.height = self.chain.len() as u64; st.log_root();
                if (n + 1) % every == 0 || n + 1 == bl.len() { st.flush().unwrap(); self.flushed = self.chain.len(); }
            }
        }
        /// as rewind.rs, with a crash point in the flush; true if the rewind committed (the meta reached f)
        fn rewind(&mut self, f: usize, cp: u32) -> bool {
            {
                let mut st = TrieStore::open_dir(&self.dir, true).unwrap();
                assert_eq!(st.height as usize, self.chain.len());
                for (mk, i) in self.chain[f..].iter().rev() {
                    let km: Vec<Key> = mk.iter().map(|m| m.0).collect();
                    let ki: Vec<Key> = i.iter().map(|x| x.0).collect();
                    st.undo_recorded(&km, &ki, false).unwrap();
                }
                st.cut_roots_above(f as u64 - 1);
                st.height = f as u64; st.log_root();
                CRASH_AT.store(cp, SeqCst);
                let r = st.flush();
                CRASH_AT.store(0, SeqCst);
                if cp == 0 { r.unwrap(); }
            }
            let done = self.meta_h() == f;
            if done { self.chain.truncate(f); self.flushed = f; }
            done
        }
        fn open_crash(&self, cp: u32) -> bool { CRASH_AT.store(cp, SeqCst); let r = TrieStore::open_dir(&self.dir, true).is_err(); CRASH_AT.store(0, SeqCst); r }
        fn lists(&self) -> Vec<String> { let mut v: Vec<_> = std::fs::read_dir(self.dir.join("retain")).map(|it| it.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default(); v.sort(); v }
        fn intact(&self) -> u64 { std::fs::read_to_string(self.dir.join("INTACT")).ok().and_then(|x| x.split_whitespace().nth(1)?.parse().ok()).unwrap_or(0) }
        fn root_ok(&self, h: usize) -> bool {
            let b = std::fs::read(self.dir.join("roots.log")).unwrap_or_default();
            let Some(e) = b.chunks_exact(ROOTS_REC).filter(|e| u32::from_le_bytes(e[..4].try_into().unwrap()) == h as u32).last() else { return false };
            let r = u64::from_le_bytes(e[36..44].try_into().unwrap());
            let want = self.model(h).to_bytes();
            let d = self.dir.clone();
            std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| { let mut v = TrieStore::open_dir(&d, true).unwrap(); v.root = r; v.height = h as u64; v.to_bytes() })).is_ok_and(|b| b == want)
        }
        /// every height INTACT claims reads as the model; the pinned root too; the flushed trie is the model's
        fn check(&mut self, what: &str) -> Result<(), String> {
            { let st = TrieStore::open_dir(&self.dir, true).map_err(|e| alloc::format!("{what}: open {e}"))?;
              if st.height as usize != self.flushed { return Err(alloc::format!("{what}: store at {} expected {}", st.height, self.flushed)); }
              let ok = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| st.to_bytes())).is_ok_and(|b| b == self.model(self.flushed).to_bytes());
              if !ok { return Err(alloc::format!("{what}: the FLUSHED trie at {} is not the model's (live nodes corrupted)", self.flushed)); } }
            for j in ["RETAG", "RELEASE"] { if self.dir.join("retain").join(j).exists() { return Err(alloc::format!("{what}: retain/{j} left after an open")); } }
            if self.dir.join("ROOTS_CUT").exists() { return Err(alloc::format!("{what}: ROOTS_CUT left after an open")); }
            let from = (self.intact() as usize).max(1);
            let bad: Vec<usize> = (from..=self.flushed).filter(|&h| !self.root_ok(h)).collect();
            if !bad.is_empty() { return Err(alloc::format!("{what}: INTACT {from} flushed {} but roots {bad:?} broken; lists {:?}", self.flushed, self.lists())); }
            if let Some(p) = self.pin { if p as usize >= 1 && p as usize <= self.flushed && !self.root_ok(p as usize) { return Err(alloc::format!("{what}: PINNED root {p} broken (INTACT {from}, flushed {})", self.flushed)); } }
            Ok(())
        }
        fn leak(&mut self) -> Result<(), String> {
            self.set_pin(None);
            let extra = blocks(3, 0xdead_0000 + self.chain.len() as u64);
            self.fwd(&extra, 1);
            let st = TrieStore::open_dir(&self.dir, true).unwrap();
            let ok = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| st.to_bytes())).is_ok_and(|b| b == self.model(self.chain.len()).to_bytes());
            if !ok { return Err("final content differs from the model (corruption)".into()); }
            let (disk, model, lists) = (st.nodes(), self.model(self.chain.len()).nodes(), self.lists());
            if disk != model || !lists.is_empty() { return Err(alloc::format!("LEAK/DOUBLE-FREE: disk {disk} model {model} lists {lists:?}")); }
            Ok(())
        }
    }
    fn serial() -> std::sync::MutexGuard<'static, ()> { super::DISK_TESTS.lock().unwrap_or_else(|e| e.into_inner()) }

    /// every crash point of a rewind's flush x {rerun, abandon} x pin {below fork, at fork, above fork, none},
    /// optionally with a crash inside the open-time recovery, then run on and check for leaks
    /// ~2 min in release, more in debug (576 crash cases): run on purpose, TMPDIR on a tmpfs for speed:
    /// `TMPDIR=/dev/shm cargo test --release -p libzync -- --ignored x_`
    #[test]
    #[ignore]
    fn x_every_crash_point_of_a_rewind() {
        let _s = serial();
        let mut fails: Vec<String> = Vec::new();
        let mut n = 0;
        for pin in [Some(10u64), Some(15), Some(18), None] {
            for cp in [1u32, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 21] {
                for ocp in [0u32, 30, 31, 32, 20, 21] {
                    for rerun in [true, false] {
                        n += 1;
                        let what = alloc::format!("pin {pin:?} cp {cp} open-cp {ocp} rerun {rerun}");
                        let mut s = Sim::new("grid");
                        let a = blocks(20, 1); let b = blocks(12, 2);
                        s.fwd(&a[..10], 5); s.set_pin(pin); s.fwd(&a[10..], 5);
                        let r: Result<(), String> = (|| {
                            let mut done = s.rewind(15, cp);
                            if ocp != 0 { let _ = s.open_crash(ocp); }
                            // recovery twice
                            drop(TrieStore::open_dir(&s.dir, true).map_err(|e| alloc::format!("open: {e}"))?);
                            s.check("after crash")?;
                            if !done && rerun { done = s.rewind(15, 0); if !done { return Err("rerun did not commit".into()); } s.check("after rerun")?; }
                            let _ = done;
                            s.fwd(&b[..6], 3); s.check("on 6")?;
                            if let Some(p) = s.pin { let np = (p + 4).min(s.flushed as u64); s.set_pin(Some(np)); }
                            s.fwd(&b[6..], 2); s.check("on 12")?;
                            s.leak()
                        })();
                        if let Err(e) = r { fails.push(alloc::format!("{what}: {e}")); }
                        let _ = std::fs::remove_dir_all(&s.dir);
                    }
                }
            }
        }
        for f in &fails { std::eprintln!("GRID FAIL {f}"); }
        std::eprintln!("grid: {n} cases, {} failed", fails.len());
        assert!(fails.is_empty());
    }

    /// random histories: forward runs, pin moves, rewinds (same fork again, nested), crashes at random points, reruns
    /// and abandons
    /// (with the crash grid above: `-- --ignored x_`)
    #[test]
    #[ignore]
    fn x_random_histories() {
        let _s = serial();
        let seeds: u64 = std::env::var("ADV_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
        let mut fails: Vec<String> = Vec::new();
        for seed in 1..=seeds {
            let mut x = seed.wrapping_mul(0xD6E8_FEB8_6659_FD93) | 1;
            let mut rnd = move || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x };
            let mut s = Sim::new("rand");
            let mut bseed = seed * 1000;
            let mut last_fork = 0usize;
            let r: Result<(), String> = (|| {
                let a = blocks(12, bseed); bseed += 1;
                s.fwd(&a, 4); s.t("fwd 12".into());
                for step in 0..14 {
                    match rnd() % 10 {
                        0..=2 => { let n = 1 + (rnd() % 7) as u32; let ev = 1 + (rnd() % 4) as usize; let b = blocks(n, bseed); bseed += 1; s.fwd(&b, ev); s.t(alloc::format!("fwd {n} every {ev} -> {}", s.flushed)); }
                        3..=4 => {
                            let lo = (s.intact() as usize).max(s.pin.unwrap_or(0) as usize).max(1);
                            if rnd() % 6 == 0 { s.set_pin(None); s.t("pin None".into()); }
                            else if lo <= s.flushed { let p = lo + (rnd() as usize) % (s.flushed - lo + 1); s.set_pin(Some(p as u64)); s.t(alloc::format!("pin {p}")); }
                        }
                        _ => {
                            if s.flushed < 4 { continue; }
                            let depth = 1 + (rnd() as usize) % 8.min(s.flushed - 2);
                            let mut f = s.flushed - depth;
                            if last_fork != 0 && last_fork < s.flushed && rnd() % 3 == 0 { f = last_fork; }
                            let cps = [0u32, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 21];
                            let cp = cps[(rnd() % cps.len() as u64) as usize];
                            let from = s.flushed;
                            let mut done = s.rewind(f, cp);
                            let mut note = alloc::format!("rewind {from}->{f} cp {cp} done {done}");
                            if cp != 0 && rnd() % 2 == 0 { let ocps = [30u32, 31, 32, 20, 21]; let o = ocps[(rnd() % 5) as usize]; let c = s.open_crash(o); note.push_str(&alloc::format!(" open-cp {o} hit {c}")); }
                            if !done && rnd() % 2 == 0 { done = s.rewind(f, 0); note.push_str(" rerun"); if !done { return Err("rerun did not commit".into()); } }
                            if done { last_fork = f; }
                            s.t(note);
                        }
                    }
                    s.check(&alloc::format!("step {step}"))?;
                }
                s.leak()
            })();
            if let Err(e) = r { fails.push(alloc::format!("seed {seed}: {e}\n    trace: {:?}", s.trace)); }
            let _ = std::fs::remove_dir_all(&s.dir);
        }
        for f in &fails { std::eprintln!("RAND FAIL {f}"); }
        std::eprintln!("random: {seeds} seeds, {} failed", fails.len());
        assert!(fails.is_empty());
    }

    /// LIBRARY-LEVEL (not rewind.rs): the meta is ALREADY at F; apply and undo in one session, the flush at F crashes
    /// before the meta. RETAG's and ROOTS_CUT's commit test is "the meta is at F": true here though nothing committed.
    #[test]
    fn x_in_session_undo_crash_before_meta_same_height() {
        let _s = serial();
        let mut s = Sim::new("insess-crash");
        let a = blocks(20, 1); let b = blocks(12, 2);
        s.fwd(&a[..10], 5); s.set_pin(Some(10)); s.fwd(&a[10..15], 5);
        {
            let mut st = TrieStore::open_dir(&s.dir, true).unwrap();
            for (n, (mk, i)) in a[15..].iter().enumerate() { st.apply(mk, i).unwrap(); st.height = 16 + n as u64; st.log_root(); }
            for (mk, i) in a[15..].iter().rev() { st.undo(mk, i).unwrap(); }
            st.cut_roots_above(14); st.height = 15; st.log_root();
            CRASH_AT.store(7, SeqCst); let r = st.flush(); CRASH_AT.store(0, SeqCst);
            assert!(r.is_err());
        }
        std::eprintln!("in-session crash: lists {:?} RETAG {} ROOTS_CUT {}", s.lists(), s.dir.join("retain/RETAG").exists(), s.dir.join("ROOTS_CUT").exists());
        let mut out: Vec<String> = Vec::new();
        if let Err(e) = s.check("after the crash") { out.push(e); }
        std::eprintln!("after open: lists {:?}", s.lists());
        let r: Result<(), String> = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| -> Result<(), String> {
            s.fwd(&b[..6], 3); s.check("on 6")?;
            s.set_pin(Some(18)); s.fwd(&b[6..], 2); s.check("on 12, pin 18")?;
            s.leak()
        })).unwrap_or_else(|_| Err("PANIC while running on".into()));
        if let Err(e) = r { out.push(e); }
        for o in &out { std::eprintln!("INSESS FAIL {o}"); }
        assert!(out.is_empty());
    }

    /// LIBRARY-LEVEL: two flushes at the same height in ONE session (one GEN), each re-tagging a list of the same block
    #[test]
    fn x_same_session_two_retags_one_gen() {
        let _s = serial();
        let mut s = Sim::new("onegen");
        let a = blocks(20, 1); let b = blocks(5, 2); let c = blocks(12, 3);
        s.fwd(&a[..10], 5); s.set_pin(Some(10)); s.fwd(&a[10..], 1);
        let sz = |s: &Sim| -> u64 { s.lists().iter().map(|n| std::fs::metadata(s.dir.join("retain").join(n)).unwrap().len()).sum() };
        {
            let mut st = TrieStore::open_dir(&s.dir, true).unwrap();
            for (mk, i) in s.chain[15..].iter().rev() {
                let km: Vec<Key> = mk.iter().map(|m| m.0).collect(); let ki: Vec<Key> = i.iter().map(|x| x.0).collect();
                st.undo_recorded(&km, &ki, false).unwrap();
            }
            st.cut_roots_above(14); st.height = 15; st.log_root(); st.flush().unwrap();
            s.chain.truncate(15);
            for (mk, i) in b.iter() { st.apply(mk, i).unwrap(); s.chain.push((mk.clone(), i.clone())); st.height = s.chain.len() as u64; st.log_root(); st.flush().unwrap(); }
            let before = sz(&s);
            std::eprintln!("one gen: before the 2nd undo {:?}", s.lists());
            for (mk, i) in b.iter().rev() { st.undo(mk, i).unwrap(); }
            s.chain.truncate(15);
            st.cut_roots_above(14); st.height = 15; st.log_root(); st.flush().unwrap();
            s.flushed = 15;
            let after = sz(&s);
            std::eprintln!("one gen: after {:?}; held bytes {before} -> {after}", s.lists());
            assert_eq!(before, after, "held slots lost by a rename onto an existing list ({} slots)", (before - after) / 8);
        }
        s.check("after").unwrap();
        s.fwd(&c, 3); s.check("on").unwrap();
        s.leak().unwrap();
    }
}

// ⚠ Kept at the END of the file (after the tests): the core verifier compiles this file into its .wasm, whose bytes
// carry the line numbers of its panic locations; code inserted above them would move the pinned wasm.
// ── Z2: a read-only reader at a logged root (docs/libzync/ZAPP_FOLLOWER.md §4, §10) ─────────────────────────────────
//
// zync-serve's tip mode reads the counting trie a prep run is still writing: `nodes.bin` opened READ-ONLY (no lock, no
// GEN bump, no recovery, no write: positioned reads, so a reference past the file's end is an error, not a SIGBUS), at
// the root prep logged for (height, block hash) in `roots.log`. It is sound while nothing reachable from that root is
// overwritten: prep frees a slot only at a flush, and with `trie/PIN` at or below the height it keeps such slots back
// (Z1). Every walk is bracketed: before it, the store's generation (`GEN`, bumped at every open and rewind) and what
// was released (`INTACT`: the root of every height >= intact_from is intact); after it, the same two again. A walk
// that saw either move, read a record past the file, a record that does not decode, or more than `cap` records (the
// history-fact cap, §12 decision 4) is refused. A walk that read garbage can only produce a fact the client refuses
// anyway (it checks every fact against the proof it holds).

/// One `roots.log` entry's size (the same as ROOTS_REC, which is `std`-only).
#[cfg(feature = "reader")]
const ROOTS_REC_R: usize = 80;

#[cfg(feature = "reader")]
impl Node {
    /// One 64-byte arena record, or None for a tag that is not a node (a torn or reused slot).
    pub fn decode(b: &[u8]) -> Option<Node> {
        if b.len() < RECORD || !matches!(b[0], 1..=4) { return None; }
        let u64at = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        let wat = |i: usize| -> W { core::array::from_fn(|k| u32::from_le_bytes(b[i + 4 * k..i + 4 * k + 4].try_into().unwrap())) };
        Some(match b[0] {
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
            _ => Node::Addr { s: wat(8), count: u64at(40), value: u64at(48), sub: u64at(56) },
        })
    }
}

/// Why a reader's walk was refused.
#[cfg(feature = "reader")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkErr {
    /// The store moved under the walk (a new generation, or the root's slots released): ask again, or repin.
    Moved(alloc::string::String),
    /// The walk read more than the cap's records (a history fact of a script with very many coins).
    TooLarge(u64),
    /// A record past the file, a record that does not decode, or a panic inside the walk.
    Damaged(alloc::string::String),
}

/// The walk state a reader arena shares with its Reader: the file's length at the walk's start, records read, and
/// the first thing that went wrong.
#[cfg(feature = "reader")]
#[derive(Default)]
struct WalkState { len: core::sync::atomic::AtomicU64, reads: core::sync::atomic::AtomicU64, cap: u64, bad: std::sync::Mutex<Option<WalkErr>> }

#[cfg(feature = "reader")]
struct ReadArena { file: std::fs::File, w: std::sync::Arc<WalkState> }

#[cfg(feature = "reader")]
impl ReadArena {
    fn fail(&self, e: WalkErr) -> ! {
        let mut b = self.w.bad.lock().unwrap_or_else(|x| x.into_inner());
        if b.is_none() { *b = Some(e.clone()); }
        drop(b);
        // unwinds out of the walk (zync-serve is built with panic = "unwind" and catches it in Reader::walk)
        panic!("trie reader: {e:?}")
    }
    #[cfg(unix)]
    fn read_at(&self, buf: &mut [u8], at: u64) -> std::io::Result<()> { std::os::unix::fs::FileExt::read_exact_at(&self.file, buf, at) }
    #[cfg(windows)]
    fn read_at(&self, buf: &mut [u8], at: u64) -> std::io::Result<()> {
        let n = std::os::windows::fs::FileExt::seek_read(&self.file, buf, at)?;
        if n == buf.len() { Ok(()) } else { Err(std::io::ErrorKind::UnexpectedEof.into()) }
    }
}

#[cfg(feature = "reader")]
impl Arena for ReadArena {
    fn get(&self, r: R) -> Node {
        use core::sync::atomic::Ordering::Relaxed;
        let n = self.w.reads.fetch_add(1, Relaxed) + 1;
        if n > self.w.cap { self.fail(WalkErr::TooLarge(self.w.cap)); }
        let len = self.w.len.load(Relaxed);
        if r == EMPTY || r.checked_mul(RECORD as u64).map_or(true, |e| e > len) {
            self.fail(WalkErr::Damaged(alloc::format!("node {r} past the file ({len} bytes)")));
        }
        let mut b = [0u8; RECORD];
        if let Err(e) = self.read_at(&mut b, (r - 1) * RECORD as u64) { self.fail(WalkErr::Damaged(alloc::format!("node {r}: {e}"))); }
        Node::decode(&b).unwrap_or_else(|| self.fail(WalkErr::Damaged(alloc::format!("node {r}: record tag {}", b[0]))))
    }
    fn put(&mut self, _n: Node) -> R { panic!("trie reader: a reader cannot write") }
    fn free(&mut self, _r: R) { panic!("trie reader: a reader cannot write") }
    fn live(&self) -> u64 { 0 }
}

/// A read-only view of a disk trie at one logged root (Z2).
#[cfg(feature = "reader")]
pub struct Reader {
    dir: std::path::PathBuf,
    /// the store at the root (height, block hash); read it only through `walk`
    store: TrieStore,
    pub root_ref: u64,
    pub root_hash: [u8; 32],
    gen: core::sync::atomic::AtomicU32,
    w: std::sync::Arc<WalkState>,
}

#[cfg(feature = "reader")]
impl core::fmt::Debug for Reader {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Reader({} at {}, root {})", self.dir.display(), self.store.height, self.root_ref)
    }
}

#[cfg(feature = "reader")]
impl Reader {
    /// Open the trie in `dir` read-only at the root `roots.log` holds for block `hash` at `height` (the newest entry: a
    /// rewind logs a height again). Refused when there is none, when the root is no longer intact (`INTACT`), when the
    /// store's root hash there is not the logged one, or, given `want_root` (the trie_root of the proof's state), not
    /// that. `cap`: the records one walk may read.
    pub fn open(dir: &std::path::Path, height: u64, hash: &[u8; 32], want_root: Option<&[u8; 32]>, cap: u64) -> Result<Reader, alloc::string::String> {
        use alloc::format;
        let meta = std::fs::read(dir.join("meta")).map_err(|e| format!("{}/meta: {e}", dir.display()))?;
        if meta.len() < 73 || &meta[..8] != b"HZTRIED1" || meta[16] > 1 { return Err(format!("{}/meta: not a disk trie", dir.display())); }
        let counts = meta[16] == 1;
        let gen = read_gen(dir);
        let (root_ref, root_hash) = find_root(dir, height, hash)?
            .ok_or_else(|| format!("{}/roots.log has no root for block {} at {height}", dir.display(),
                                    hash.iter().map(|x| format!("{x:02x}")).collect::<alloc::string::String>()))?;
        let intact = read_intact(dir);
        if height < intact { return Err(format!("the root of {height} is no longer intact (INTACT: intact_from {intact}): it was released")); }
        if let Some(w) = want_root { if w != &root_hash { return Err(format!("roots.log's root hash at {height} is not the proof's trie_root")); } }
        let file = std::fs::File::open(dir.join("nodes.bin")).map_err(|e| format!("{}/nodes.bin: {e}", dir.display()))?;
        let w = std::sync::Arc::new(WalkState { cap, ..Default::default() });
        let a = ReadArena { file, w: w.clone() };
        let mut store = TrieStore::with_arena(counts, Box::new(a));
        (store.root, store.height, store.hash) = (root_ref, height, *hash);
        let r = Reader { dir: dir.to_path_buf(), store, root_ref, root_hash, gen: core::sync::atomic::AtomicU32::new(gen), w };
        // the root itself: its hash read from the arena must be the logged one
        let got = r.walk(|s| crate::coinlog::h_of(&s.root())).map_err(|e| format!("reading the root of {height}: {e:?}"))?;
        if got != root_hash { return Err(format!("the arena's root at {height} does not hash to roots.log's root hash")); }
        Ok(r)
    }

    pub fn height(&self) -> u64 { self.store.height }
    pub fn hash(&self) -> [u8; 32] { self.store.hash }

    /// Run `f` on the store at this root, bracketed as above. With a new generation since the last walk, the root is
    /// looked up again first (an entry at or below the flushed height survives a restart; one above it is cut).
    pub fn walk<T>(&self, f: impl FnOnce(&TrieStore) -> T) -> Result<T, WalkErr> {
        use core::sync::atomic::Ordering::Relaxed;
        let (g0, i0) = (read_gen(&self.dir), read_intact(&self.dir));
        let h = self.store.height;
        if h < i0 { return Err(WalkErr::Moved(alloc::format!("the root of {h} was released (intact_from {i0})"))); }
        if g0 != self.gen.load(Relaxed) {
            match find_root(&self.dir, h, &self.store.hash) {
                Ok(Some((r, rh))) if r == self.root_ref && rh == self.root_hash => self.gen.store(g0, Relaxed),
                _ => return Err(WalkErr::Moved(alloc::format!("the trie's generation moved to {g0} and the root of {h} is no longer logged"))),
            }
        }
        let len = std::fs::metadata(self.dir.join("nodes.bin")).map(|m| m.len()).unwrap_or(0);
        self.w.len.store(len, Relaxed);
        self.w.reads.store(0, Relaxed);
        *self.w.bad.lock().unwrap_or_else(|x| x.into_inner()) = None;
        let out = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| f(&self.store)));
        let bad = self.w.bad.lock().unwrap_or_else(|x| x.into_inner()).take();
        let (g1, i1) = (read_gen(&self.dir), read_intact(&self.dir));
        if g1 != g0 { return Err(WalkErr::Moved(alloc::format!("the trie's generation moved during the walk ({g0} -> {g1})"))); }
        if h < i1 { return Err(WalkErr::Moved(alloc::format!("the root of {h} was released during the walk (intact_from {i1})"))); }
        match (out, bad) {
            (_, Some(e)) => Err(e),
            (Err(_), None) => Err(WalkErr::Damaged("the walk panicked".into())),
            (Ok(v), None) => Ok(v),
        }
    }
}

#[cfg(feature = "reader")]
impl TrieStore {
    /// A FIXTURE for tests of readers (zync-serve's tip mode): this store written as a disk trie in `dir` (`nodes.bin`,
    /// `meta`, one `roots.log` entry for its height and hash, `GEN`), as prep's disk arena would hold it after a flush.
    #[doc(hidden)]
    pub fn write_disk_fixture(&self, dir: &std::path::Path, gen: u32) -> std::io::Result<()> {
        fn copy(st: &TrieStore, r: R, out: &mut Vec<u8>) -> R {
            if r == EMPTY { return EMPTY; }
            let n = match st.get(r) {
                Node::SubInner { l, r, h } => { let (l, r) = (copy(st, l, out), copy(st, r, out)); Node::SubInner { l, r, h } }
                Node::TopInner { l, r, h } => { let (l, r) = (copy(st, l, out), copy(st, r, out)); Node::TopInner { l, r, h } }
                Node::Addr { s, count, value, sub } => Node::Addr { s, count, value, sub: copy(st, sub, out) },
                c => c,
            };
            out.extend_from_slice(&n.enc());
            (out.len() / RECORD) as R
        }
        std::fs::create_dir_all(dir)?;
        let mut nodes = Vec::new();
        let root = copy(self, self.root, &mut nodes);
        std::fs::write(dir.join("nodes.bin"), &nodes)?;
        let mut m = b"HZTRIED1".to_vec();
        m.extend_from_slice(&root.to_le_bytes());
        m.push(self.counts as u8);
        m.extend_from_slice(&self.height.to_le_bytes());
        m.extend_from_slice(&self.hash);
        m.extend_from_slice(&((nodes.len() / RECORD) as u64).to_le_bytes());
        m.extend_from_slice(&0u64.to_le_bytes());
        std::fs::write(dir.join("meta"), &m)?;
        let mut e = (self.height as u32).to_le_bytes().to_vec();
        e.extend_from_slice(&self.hash);
        e.extend_from_slice(&root.to_le_bytes());
        e.extend_from_slice(&crate::coinlog::h_of(&self.root()));
        e.extend_from_slice(&gen.to_le_bytes());
        use std::io::Write as _;
        std::fs::OpenOptions::new().create(true).append(true).open(dir.join("roots.log"))?.write_all(&e)?;
        std::fs::write(dir.join("GEN"), gen.to_le_bytes())
    }
}

/// `GEN` (u32 LE), 0 when missing.
#[cfg(feature = "reader")]
fn read_gen(dir: &std::path::Path) -> u32 {
    std::fs::read(dir.join("GEN")).ok().filter(|b| b.len() == 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
}

/// `INTACT`'s intact_from (the root of every height at or above it is intact); 0 when missing: a store opened by a Z1
/// host has released nothing it logged until its first flush, which writes the file.
#[cfg(feature = "reader")]
fn read_intact(dir: &std::path::Path) -> u64 {
    std::fs::read_to_string(dir.join("INTACT")).ok().and_then(|x| x.split_whitespace().nth(1)?.parse().ok()).unwrap_or(0)
}

/// The newest `roots.log` entry for (height, block hash): (root reference, root hash).
#[cfg(feature = "reader")]
fn find_root(dir: &std::path::Path, height: u64, hash: &[u8; 32]) -> Result<Option<(u64, [u8; 32])>, alloc::string::String> {
    let b = match std::fs::read(dir.join("roots.log")) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(alloc::format!("{}/roots.log: {e}", dir.display())),
    };
    Ok(b.chunks_exact(ROOTS_REC_R).rev()
        .find(|e| u32::from_le_bytes(e[..4].try_into().unwrap()) as u64 == height && &e[4..36] == hash)
        .map(|e| (u64::from_le_bytes(e[36..44].try_into().unwrap()), e[44..76].try_into().unwrap())))
}
