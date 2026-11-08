//! Coin log prototype: every coin that outlives its block is APPENDED, in birth order, to a Merkle log; spending a coin
//! flips it to "spent" in place and re-hashes its path. Nothing is ever moved or deleted. Shared by the bench guest
//! and host (`#[path]`). ⛔ EXPERIMENT ONLY, not consensus code.
//!
//! - Leaf: the coin's leaf hash while unspent; spent(c) = one SHA-256 compression of c ‖ 0^32 from IV_SPENT.
//! - Parent at level l (a tree of 2^l leaves): one SHA-256 compression of L ‖ R from IV_NODE[l].
//! - The log is a forest of perfect trees, one per set bit of N (the number of leaves), largest (oldest) first:
//!   `roots[l]` is the tree of 2^l leaves, as utreexo's stump. A leaf's position never changes.
//!
//! Block update: spends = (position, coin leaf) sorted by position; the witness is the hash of every untouched subtree
//! met on the way down, in the order `update` reads them (the positions alone fix the shape). Then appends.

use risc0_zkvm::sha::{Digest as D, Impl, Sha256 as _};
use sha2::{Digest, Sha256};

pub type H = [u8; 32];
pub const ZERO: H = [0u8; 32];
pub static mut HASHES: u64 = 0;
static mut IVS: Option<Vec<D>> = None; // [0..64) node levels, [64] spent

fn ivs() -> &'static Vec<D> {
    unsafe {
        #[allow(static_mut_refs)]
        IVS.get_or_insert_with(|| {
            let mut v: Vec<D> = (0..64u8).map(|l| D::from(<[u8; 32]>::from(Sha256::digest([&b"coinlog-node"[..], &[l]].concat())))).collect();
            v.push(D::from(<[u8; 32]>::from(Sha256::digest(b"coinlog-spent"))));
            v
        })
    }
}

fn compress(iv: &D, l: &H, r: &H) -> H {
    unsafe { HASHES += 1 };
    let out = Impl::compress(iv, &D::from(*l), &D::from(*r));
    let mut o = [0u8; 32];
    o.copy_from_slice(out.as_bytes());
    o
}

pub fn parent(l: u32, a: &H, b: &H) -> H {
    compress(&ivs()[l as usize], a, b)
}
pub fn spent(c: &H) -> H {
    compress(&ivs()[64], c, &ZERO)
}

pub struct Rd<'a> {
    pub b: &'a [u8],
    pub i: usize,
}
impl<'a> Rd<'a> {
    pub fn h(&mut self) -> H {
        self.i += 32;
        self.b[self.i - 32..self.i].try_into().unwrap()
    }
}

/// (old, new) hash of the perfect tree of 2^l leaves starting at `start`, with `e` the spends inside it.
pub fn update(l: u32, start: u64, e: &[(u64, H)], w: &mut Rd) -> (H, H) {
    if e.is_empty() {
        let h = w.h();
        return (h, h);
    }
    if l == 0 {
        assert!(e.len() == 1 && e[0].0 == start, "spend position");
        return (e[0].1, spent(&e[0].1));
    }
    let mid = start + (1u64 << (l - 1));
    let m = e.partition_point(|x| x.0 < mid);
    let (a, b) = (update(l - 1, start, &e[..m], w), update(l - 1, mid, &e[m..], w));
    (parent(l, &a.0, &b.0), parent(l, &a.1, &b.1))
}

/// Spends against the current roots (asserting each touched tree's old root), then appends. Returns the new N.
pub fn apply_block(roots: &mut Vec<Option<H>>, n: u64, spends: &[(u64, H)], appends: &[H], w: &mut Rd) -> u64 {
    for p in spends.windows(2) {
        assert!(p[0].0 < p[1].0, "spends not strictly increasing");
    }
    if let Some(last) = spends.last() {
        assert!(last.0 < n, "spend past the end of the log");
    }
    let mut start = 0u64;
    let mut k = 0usize;
    for l in (0..roots.len()).rev() {
        if let Some(r) = roots[l] {
            let end = start + (1u64 << l);
            let m = k + spends[k..].partition_point(|x| x.0 < end);
            if m > k {
                let (o, nw) = update(l as u32, start, &spends[k..m], w);
                assert!(o == r, "old root of a touched tree");
                roots[l] = Some(nw);
            }
            k = m;
            start = end;
        }
    }
    let mut n = n;
    for leaf in appends {
        let (mut node, mut l) = (*leaf, 0usize);
        while l < roots.len() && roots[l].is_some() {
            node = parent(l as u32 + 1, &roots[l].take().unwrap(), &node);
            l += 1;
        }
        if l == roots.len() {
            roots.push(None);
        }
        roots[l] = Some(node);
        n += 1;
    }
    n
}
