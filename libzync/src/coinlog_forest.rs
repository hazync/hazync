//! ⚠ GUEST-COMPILED CRATE — editing this file changes METHOD_ID (it is #[path]-included into the main guest, beside coinlog.rs).
//! The coin log's forest: a block's update, its witness and the host's node store. The hashing is `coinlog.rs`.
//!
//! A block's update: the spends of coins from earlier blocks as (position, leaf, spender), sorted and strictly increasing, then
//! the appends. The witness is the hash of every untouched subtree met on the way down, in the order `update` reads
//! them; the positions alone fix the shape. An unspent leaf at the claimed position is the only way a touched tree's
//! old root can match: a coin that does not exist, or was spent before, cannot be spent.

use crate::coinlog::{h_of, parent, parent_w, spent_w, w_of, H, W};
use alloc::vec::Vec;

/// One spend of an earlier coin: its position in the log, its unspent leaf and its spender record, as words.
pub type Spend = (u64, W, W);

#[derive(Debug, PartialEq, Eq)]
pub enum LogError {
    /// positions not strictly increasing (the same coin twice, or out of order)
    Order,
    /// a position at or past the end of the log
    PastEnd,
    /// a touched tree's old root does not match: the coin is not unspent at that position
    OldRoot,
    /// the roots do not have the shape the leaf count forces
    Shape,
    /// the witness ran out, or was longer than the update used
    Witness,
}

/// The old and new hash of the perfect tree of 2^l leaves starting at `start`, with the spends `e` inside it, written
/// into `old` / `new` (no tuples returned by value: in the guest those were memcpy calls). False: the witness ran out.
pub fn update(l: u32, start: u64, e: &[Spend], sib: &mut core::slice::Iter<W>, old: &mut W, new: &mut W) -> bool {
    if e.is_empty() {
        let Some(h) = sib.next() else { return false };
        *old = *h;
        *new = *h;
        return true;
    }
    if l == 0 {
        *old = e[0].1;
        spent_w(&e[0].1, &e[0].2, new);
        return true;
    }
    let mid = start + (1u64 << (l - 1));
    let m = e.partition_point(|x| x.0 < mid);
    let (mut a0, mut a1, mut b0, mut b1) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
    if !update(l - 1, start, &e[..m], sib, &mut a0, &mut a1) || !update(l - 1, mid, &e[m..], sib, &mut b0, &mut b1) {
        return false;
    }
    parent_w(l, &a0, &b0, old);
    parent_w(l, &a1, &b1, new);
    true
}

/// The roots have a tree exactly at each set bit of `n`.
pub fn shape_ok(roots: &[Option<H>], n: u64) -> bool {
    (0..64).all(|l| ((n >> l) & 1 == 1) == roots.get(l).copied().flatten().is_some()) && roots.len() <= 64
}

/// One block: `spends` (position, unspent leaf, spender), positions strictly increasing, against `roots`; then `appends`.
/// `roots` / `n` are updated only on success.
pub fn apply(roots: &mut Vec<Option<H>>, n: &mut u64, spends: &[Spend], appends: &[H], siblings: &[W]) -> Result<(), LogError> {
    if !shape_ok(roots, *n) {
        return Err(LogError::Shape);
    }
    if spends.windows(2).any(|p| p[0].0 >= p[1].0) {
        return Err(LogError::Order);
    }
    if spends.last().is_some_and(|x| x.0 >= *n) {
        return Err(LogError::PastEnd);
    }
    let mut r: Vec<Option<W>> = roots.iter().map(|x| x.as_ref().map(w_of)).collect();
    let mut sib = siblings.iter();
    let (mut start, mut k) = (0u64, 0usize);
    let (mut o, mut nw) = ([0u32; 8], [0u32; 8]);
    for l in (0..r.len()).rev() {
        let Some(root) = r[l] else { continue };
        let end = start + (1u64 << l);
        let m = k + spends[k..].partition_point(|x| x.0 < end);
        if m > k {
            if !update(l as u32, start, &spends[k..m], &mut sib, &mut o, &mut nw) {
                return Err(LogError::Witness);
            }
            if o != root {
                return Err(LogError::OldRoot);
            }
            r[l] = Some(nw);
        }
        k = m;
        start = end;
    }
    if sib.next().is_some() {
        return Err(LogError::Witness);
    }
    let mut c = *n;
    for leaf in appends {
        let (mut node, mut l) = (w_of(leaf), 0usize);
        while l < r.len() && r[l].is_some() {
            let left = r[l].take().unwrap();
            let mut up = [0u32; 8];
            parent_w(l as u32 + 1, &left, &node, &mut up);
            node = up;
            l += 1;
        }
        if l == r.len() {
            r.push(None);
        }
        r[l] = Some(node);
        c += 1;
    }
    while r.last() == Some(&None) {
        r.pop();
    }
    *roots = r.iter().map(|x| x.as_ref().map(h_of)).collect();
    *n = c;
    Ok(())
}

/// The witness a block's aggregate reads after its hint block: [k, positions as (lo, hi) words × k, 8 words per sibling].
/// Position i belongs to the i-th spend of an earlier coin in block order; the guest sorts them.
pub fn encode_witness(positions: &[u64], siblings: &[H]) -> Vec<u32> {
    let mut v = Vec::with_capacity(1 + 2 * positions.len() + 8 * siblings.len());
    v.push(positions.len() as u32);
    for p in positions {
        v.push(*p as u32);
        v.push((*p >> 32) as u32);
    }
    for s in siblings {
        for c in s.chunks_exact(4) {
            v.push(u32::from_le_bytes([c[0], c[1], c[2], c[3]]));
        }
    }
    v
}

pub fn decode_witness(w: &[u32]) -> Option<(Vec<u64>, Vec<W>)> {
    let k = *w.first()? as usize;
    // Checked: on rv32 a huge k would wrap 1 + 2k (review 11-03); a refusal, not a capacity panic.
    let rest = w.get(k.checked_mul(2)?.checked_add(1)?..)?;
    if rest.len() % 8 != 0 {
        return None;
    }
    let pos = (0..k).map(|i| w[1 + 2 * i] as u64 | (w[2 + 2 * i] as u64) << 32).collect();
    let sib = rest.chunks_exact(8).map(|c| c.try_into().unwrap()).collect();
    Some((pos, sib))
}

/// Where a [`Store`] keeps its nodes: level l, index i covers leaves [i·2^l, (i+1)·2^l). In memory ([`MemNodes`]) or
/// on disk (`host::DiskNodes`, ROADMAP C1); the algorithms below are the same for both.
pub trait Nodes {
    fn levels(&self) -> usize;
    fn count(&self, l: usize) -> u64;
    fn get(&self, l: usize, i: u64) -> H;
    fn put(&mut self, l: usize, i: u64, h: H);
    /// Appends at level l (creating the level if it is the next one).
    fn append(&mut self, l: usize, h: H);
}

pub type MemNodes = Vec<Vec<H>>;

impl Nodes for MemNodes {
    fn levels(&self) -> usize {
        self.len()
    }
    fn count(&self, l: usize) -> u64 {
        <[Vec<H>]>::get(self, l).map(|v| v.len() as u64).unwrap_or(0)
    }
    fn get(&self, l: usize, i: u64) -> H {
        self[l][i as usize]
    }
    fn put(&mut self, l: usize, i: u64, h: H) {
        self[l][i as usize] = h;
    }
    fn append(&mut self, l: usize, h: H) {
        if self.len() == l {
            self.push(Vec::new());
        }
        self[l].push(h);
    }
}

/// Host side: the whole log, every node kept, so any block's witness can be read off. Nodes at level l are aligned:
/// node i covers leaves [i·2^l, (i+1)·2^l), present once that whole range exists.
#[derive(Clone, Default)]
pub struct NodeStore<N: Nodes> {
    pub nodes: N,
}

/// The in-memory log (tests, signet, short runs).
pub type Store = NodeStore<MemNodes>;

impl<N: Nodes> NodeStore<N> {
    pub fn with_nodes(nodes: N) -> Self {
        NodeStore { nodes }
    }
    pub fn len(&self) -> u64 {
        self.nodes.count(0)
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn leaf(&self, p: u64) -> H {
        self.nodes.get(0, p)
    }
    pub fn push(&mut self, leaf: H) {
        self.nodes.append(0, leaf);
        let mut l = 0;
        while self.nodes.count(l) % 2 == 0 {
            let n = self.nodes.count(l);
            let p = parent(l as u32 + 1, &self.nodes.get(l, n - 2), &self.nodes.get(l, n - 1));
            self.nodes.append(l + 1, p);
            l += 1;
        }
    }
    pub fn set(&mut self, p: u64, leaf: H) {
        self.nodes.put(0, p, leaf);
        let mut i = p;
        for l in 0..self.nodes.levels() - 1 {
            let j = i / 2;
            if j >= self.nodes.count(l + 1) {
                break;
            }
            let h = parent(l as u32 + 1, &self.nodes.get(l, 2 * j), &self.nodes.get(l, 2 * j + 1));
            self.nodes.put(l + 1, j, h);
            i = j;
        }
    }
    /// The forest's roots (index = level), as the ChainState carries them.
    pub fn roots(&self) -> Vec<Option<H>> {
        let n = self.len();
        let mut r: Vec<Option<H>> = (0..64).map(|_| None).collect();
        let mut start = 0u64;
        for l in (0..64).rev() {
            if (n >> l) & 1 == 1 {
                r[l] = Some(self.nodes.get(l, start >> l));
                start += 1u64 << l;
            }
        }
        while r.last() == Some(&None) {
            r.pop();
        }
        r
    }
    fn wit(&self, l: u32, start: u64, e: &[(u64, H)], out: &mut Vec<H>) {
        if e.is_empty() {
            out.push(self.nodes.get(l as usize, start >> l));
            return;
        }
        if l == 0 {
            return;
        }
        let mid = start + (1u64 << (l - 1));
        let m = e.partition_point(|x| x.0 < mid);
        self.wit(l - 1, start, &e[..m], out);
        self.wit(l - 1, mid, &e[m..], out);
    }
    /// One leaf's path: the sibling at each level, bottom-up, up to its tree's root (facts::coinlog_member).
    pub fn path(&self, p: u64) -> Vec<H> {
        let n = self.len();
        let (mut start, mut lvl) = (0u64, 0u32);
        for l in (0..64u32).rev() {
            if (n >> l) & 1 == 1 {
                if p < start + (1u64 << l) {
                    lvl = l;
                    break;
                }
                start += 1u64 << l;
            }
        }
        (0..lvl as usize).map(|j| self.nodes.get(j, (p >> j) ^ 1)).collect()
    }
    /// The siblings `apply` reads for these spends (sorted by position).
    pub fn witness(&self, spends: &[(u64, H)]) -> Vec<H> {
        let n = self.len();
        let mut out = Vec::new();
        let (mut start, mut k) = (0u64, 0usize);
        for l in (0..64u32).rev() {
            if (n >> l) & 1 == 0 {
                continue;
            }
            let end = start + (1u64 << l);
            let m = k + spends[k..].partition_point(|x| x.0 < end);
            if m > k {
                self.wit(l, start, &spends[k..m], &mut out);
            }
            k = m;
            start = end;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coinlog::{spender, spent, H};
    use sha2::{Digest, Sha256};

    fn ws(sp: &[(u64, H)]) -> Vec<Spend> {
        sp.iter().map(|(p, h)| (*p, w_of(h), w_of(&spender(7, *p as u32, 1)))).collect()
    }
    fn wv(v: &[H]) -> Vec<W> {
        v.iter().map(w_of).collect()
    }

    fn h(a: u64, b: u64) -> H {
        Sha256::digest([a.to_le_bytes(), b.to_le_bytes()].concat()).into()
    }

    #[test]
    fn apply_matches_the_store() {
        // Grow a log block by block; each block spends some unspent coins and appends some. The guest-side `apply`
        // over the store's witness must give the store's own roots after every block.
        for t in 0..40u64 {
            let mut st = Store::default();
            let (mut roots, mut n): (Vec<Option<H>>, u64) = (Vec::new(), 0);
            let mut live: Vec<u64> = Vec::new();
            for b in 0..30u64 {
                let mut sp: Vec<(u64, H)> = live.iter().filter(|p| h(t * 1000 + b, **p)[0] < 50).map(|p| (*p, st.leaf(*p))).collect();
                sp.sort();
                let wit = st.witness(&sp);
                let app: Vec<H> = (0..(h(t, b)[0] % 23) as u64).map(|i| h(t * 7 + b, i + 1_000_000)).collect();
                apply(&mut roots, &mut n, &ws(&sp), &app, &wv(&wit)).unwrap();
                for (p, c) in &sp {
                    st.set(*p, spent(c, &spender(7, *p as u32, 1)));
                    live.retain(|x| x != p);
                }
                for a in &app {
                    live.push(st.len());
                    st.push(*a);
                }
                assert_eq!(roots, st.roots(), "trial {t} block {b}");
                assert_eq!(n, st.len());
            }
        }
    }

    #[test]
    fn refusals() {
        let mut st = Store::default();
        for i in 0..20u64 {
            st.push(h(9, i));
        }
        let (r0, n0) = (st.roots(), st.len());
        let one = |sp: &[(u64, H)], wit: &[H]| {
            let (mut r, mut n) = (r0.clone(), n0);
            apply(&mut r, &mut n, &ws(sp), &[], &wv(wit))
        };
        let sp = [(5u64, st.leaf(5))];
        assert!(one(&sp, &st.witness(&sp)).is_ok());
        // a coin that is not there
        let fake = [(5u64, h(1, 1))];
        assert_eq!(one(&fake, &st.witness(&fake)), Err(LogError::OldRoot));
        // the right coin at the wrong position
        let wrong = [(6u64, st.leaf(5))];
        assert_eq!(one(&wrong, &st.witness(&wrong)), Err(LogError::OldRoot));
        // the same coin twice
        let twice = [(5u64, st.leaf(5)), (5u64, st.leaf(5))];
        assert_eq!(one(&twice, &[]), Err(LogError::Order));
        // past the end
        let past = [(20u64, h(2, 2))];
        assert_eq!(one(&past, &[]), Err(LogError::PastEnd));
        // a coin already spent
        let mut st2 = st.clone();
        st2.set(5, spent(&st.leaf(5), &spender(7, 5, 1)));
        let (mut r, mut n) = (st2.roots(), st2.len());
        assert_eq!(apply(&mut r, &mut n, &ws(&sp), &[], &wv(&st2.witness(&sp))), Err(LogError::OldRoot));
        // a short or long witness
        let mut w = st.witness(&sp);
        w.pop();
        assert_eq!(one(&sp, &w), Err(LogError::Witness));
        let mut w = st.witness(&sp);
        w.push([0u8; 32]);
        assert_eq!(one(&sp, &w), Err(LogError::Witness));
        // the witness codec round-trips
        let enc = encode_witness(&[3, 1 << 40], &[h(1, 2), h(3, 4)]);
        assert_eq!(decode_witness(&enc), Some((alloc::vec![3, 1 << 40], alloc::vec![w_of(&h(1, 2)), w_of(&h(3, 4))])));
        assert_eq!(decode_witness(&[u32::MAX]), None);
        assert_eq!(decode_witness(&[]), None);
        // the spender is part of the new root: the same spend by another input gives another root
        let (mut r1, mut n1) = (r0.clone(), n0);
        apply(&mut r1, &mut n1, &ws(&sp), &[], &wv(&st.witness(&sp))).unwrap();
        let (mut r2, mut n2) = (r0.clone(), n0);
        let other = [(5u64, w_of(&st.leaf(5)), w_of(&spender(7, 5, 2)))];
        apply(&mut r2, &mut n2, &other, &[], &wv(&st.witness(&sp))).unwrap();
        assert_ne!(r1, r2);
        assert_eq!(spender(0x01020304, 5, 6)[..12], [4, 3, 2, 1, 5, 0, 0, 0, 6, 0, 0, 0]);
        // against scripts/libzync/coinlog_ref.py, which shares no code with this file
        let x: H = Sha256::digest(b"c").into();
        let y: H = Sha256::digest(b"d").into();
        let hx = |h: H| h.iter().map(|b| alloc::format!("{b:02x}")).collect::<alloc::string::String>();
        assert_eq!(hx(spent(&x, &spender(974000, 5, 1))), "cdcc3dda5f2c80be4780225cde93b2442e7d270052ab938d73be4107b388993e");
        assert_eq!(hx(parent(3, &x, &y)), "fcd6a12230b8fc914334039dfe3be49a9796fe16193c15e05de61a536c29f14f");
        // words and bytes round-trip
        assert_eq!(h_of(&w_of(&h(5, 6))), h(5, 6));
    }
}
