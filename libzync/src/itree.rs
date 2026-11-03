//! ⚠ GUEST-COMPILED (only with the `otree` / `stree` features) — editing this file changes those images' METHOD_ID.
//!
//! An INDEXED Merkle tree: a sorted linked list of 64-byte keys kept in an append-only forest of perfect trees (the
//! coin log's shape and node hash). Each live leaf is (key, next key); a removed key's leaf becomes "dead". It proves
//! what the coin log cannot (COINLOG.md §6):
//!
//! - **absence**: no live key in (a, b) — a live leaf (a, b);
//! - **completeness of a range**: the live keys from lo to hi are exactly the leaves walked from lo's predecessor via
//!   `next` until a key past hi.
//!
//! Two trees use it (experiment E1, docs/libzync/ROADMAP.md): the OUTPOINT tree (key = SHA-256(txid ‖ vout) ‖ 0^32: "is
//! coin X live") and the SCRIPT tree (key = SHA-256(scriptPubKey) ‖ SHA-256(txid ‖ vout): "these are all the coins of
//! script S"). Both hold the live coins the coin log appends (so a key is never inserted twice) and start from one
//! sentinel leaf (0^64, 1^64) at position 0.
//!
//! Hashes (one SHA-256 compression each, domain-separated by the coin log's per-level IVs at levels no tree reaches):
//!   live(key, next) = parent(63, parent(61, key), parent(60, next))
//!   dead(key)       = parent(62, parent(61, key), 0^32)
//!   nodes           = coinlog::parent(l, L, R), as the coin log

use crate::coinlog::{parent_w, LogError, W};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

pub type Key = [u8; 64];
pub const MIN_KEY: Key = [0u8; 64];
pub const MAX_KEY: Key = [0xffu8; 64];

fn halves(k: &Key) -> (W, W) {
    let mut a = [0u32; 8];
    let mut b = [0u32; 8];
    for i in 0..8 {
        a[i] = u32::from_be_bytes(k[4 * i..4 * i + 4].try_into().unwrap());
        b[i] = u32::from_be_bytes(k[32 + 4 * i..36 + 4 * i].try_into().unwrap());
    }
    (a, b)
}

fn key_w(k: &Key) -> W {
    let (a, b) = halves(k);
    let mut o = [0u32; 8];
    parent_w(61, &a, &b, &mut o);
    o
}

pub fn live(key: &Key, next: &Key) -> W {
    let (n1, n2) = halves(next);
    let mut nw = [0u32; 8];
    parent_w(60, &n1, &n2, &mut nw);
    let mut o = [0u32; 8];
    parent_w(63, &key_w(key), &nw, &mut o);
    o
}

pub fn dead(key: &Key) -> W {
    let mut o = [0u32; 8];
    parent_w(62, &key_w(key), &[0u32; 8], &mut o);
    o
}

/// The leaf of the empty tree's sentinel.
pub fn sentinel() -> W {
    live(&MIN_KEY, &MAX_KEY)
}

#[derive(Debug, PartialEq, Eq)]
pub enum TreeError {
    /// the forest's shape, positions, witness or old roots (the coin log's refusals)
    Log(LogError),
    /// a removed key is not live in the touched set
    Missing,
    /// the touched set lacks a needed predecessor, or an inserted key is already live
    Predecessor,
}

/// One touched leaf: its position and its live (key, next) before the block.
pub type Touched = (u64, Key, Key);

/// The old and new hash of the perfect tree of 2^l leaves starting at `start`, with the changed leaves `e`
/// (position, old leaf, new leaf) inside it. False: the witness ran out. (The coin log's `update`, any new leaf.)
fn update(l: u32, start: u64, e: &[(u64, W, W)], sib: &mut core::slice::Iter<W>, old: &mut W, new: &mut W) -> bool {
    if e.is_empty() {
        let Some(h) = sib.next() else { return false };
        *old = *h;
        *new = *h;
        return true;
    }
    if l == 0 {
        *old = e[0].1;
        *new = e[0].2;
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

/// One block: remove `removes` (keys that must be live), then insert `inserts` (keys that must not be), against
/// `roots` / `n` (words), with the `touched` leaves (positions strictly increasing, live before the block) and the
/// `siblings` of their paths (the coin log's witness order). Inserted keys are appended in the order given. On success
/// `roots` / `n` are the tree after the block.
pub fn apply(roots: &mut Vec<Option<W>>, n: &mut u64, touched: &[Touched], siblings: &[W], removes: &[Key], inserts: &[Key])
             -> Result<(), TreeError> {
    use TreeError::*;
    if (0..64).any(|l| ((*n >> l) & 1 == 1) != roots.get(l).copied().flatten().is_some()) || roots.len() > 64 {
        return Err(Log(LogError::Shape));
    }
    if touched.windows(2).any(|p| p[0].0 >= p[1].0) {
        return Err(Log(LogError::Order));
    }
    if touched.last().is_some_and(|x| x.0 >= *n) {
        return Err(Log(LogError::PastEnd));
    }
    // Working set: touched leaves (slots 0..t) then appended ones; live keys indexed for predecessor search.
    let t = touched.len();
    let mut ent: Vec<(Key, Key, bool)> = touched.iter().map(|x| (x.1, x.2, true)).collect();
    let mut by_key: BTreeMap<Key, usize> = BTreeMap::new();
    for (i, e) in ent.iter().enumerate() {
        if by_key.insert(e.0, i).is_some() {
            return Err(Predecessor); // the same live key twice cannot be a real tree's
        }
    }
    for k in removes {
        let i = *by_key.get(k).ok_or(Missing)?;
        let next = ent[i].1;
        // the live leaf whose next is k: in a sorted list exactly one, the largest key below k
        let (_, &p) = by_key.range(..*k).next_back().ok_or(Predecessor)?;
        if ent[p].1 != *k {
            return Err(Predecessor);
        }
        ent[p].1 = next;
        ent[i].2 = false;
        by_key.remove(k);
    }
    for k in inserts {
        let (_, &p) = by_key.range(..*k).next_back().ok_or(Predecessor)?;
        if !(ent[p].0 < *k && *k < ent[p].1) {
            return Err(Predecessor);
        }
        let next = ent[p].1;
        ent[p].1 = *k;
        by_key.insert(*k, ent.len());
        ent.push((*k, next, true));
    }
    // New hashes of the touched leaves, checked against the old roots.
    let changes: Vec<(u64, W, W)> = touched.iter().zip(&ent[..t])
        .map(|(o, e)| (o.0, live(&o.1, &o.2), if e.2 { live(&e.0, &e.1) } else { dead(&e.0) })).collect();
    let mut r: Vec<Option<W>> = roots.clone();
    let mut sib = siblings.iter();
    let (mut start, mut k) = (0u64, 0usize);
    let (mut o, mut nw) = ([0u32; 8], [0u32; 8]);
    for l in (0..r.len()).rev() {
        let Some(root) = r[l] else { continue };
        let end = start + (1u64 << l);
        let m = k + changes[k..].partition_point(|x| x.0 < end);
        if m > k {
            if !update(l as u32, start, &changes[k..m], &mut sib, &mut o, &mut nw) {
                return Err(Log(LogError::Witness));
            }
            if o != root {
                return Err(Log(LogError::OldRoot));
            }
            r[l] = Some(nw);
        }
        k = m;
        start = end;
    }
    if sib.next().is_some() {
        return Err(Log(LogError::Witness));
    }
    let mut c = *n;
    for e in &ent[t..] {
        let (mut node, mut l) = (live(&e.0, &e.1), 0usize);
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
    *roots = r;
    *n = c;
    Ok(())
}

/// Host side: the whole tree, every leaf's (key, next, live) and every node, so a block's touched set and witness can
/// be read off, and absence / range proofs given.
#[derive(Default)]
pub struct Host {
    pub nodes: crate::coinlog::Store,
    pub leaves: Vec<(Key, Key, bool)>,
    pub live: BTreeMap<Key, u64>,
}

impl Host {
    pub fn new() -> Host {
        let mut h = Host::default();
        h.push(MIN_KEY, MAX_KEY);
        h
    }
    fn push(&mut self, k: Key, next: Key) {
        self.live.insert(k, self.leaves.len() as u64);
        self.leaves.push((k, next, true));
        self.nodes.push(crate::coinlog::h_of(&live(&k, &next)));
    }
    fn set(&mut self, p: u64, k: Key, next: Key, is_live: bool) {
        self.leaves[p as usize] = (k, next, is_live);
        let w = if is_live { live(&k, &next) } else { dead(&k) };
        self.nodes.set(p, crate::coinlog::h_of(&w));
    }
    pub fn roots(&self) -> Vec<Option<W>> {
        self.nodes.roots().iter().map(|x| x.as_ref().map(crate::coinlog::w_of)).collect()
    }
    /// The touched set and witness for a block, then the block applied here.
    pub fn block(&mut self, removes: &[Key], inserts: &[Key]) -> (Vec<Touched>, Vec<W>) {
        let mut touch: BTreeMap<u64, ()> = BTreeMap::new();
        // simulate on a scratch copy of the live index to find every predecessor the guest will need
        let mut sim: BTreeMap<Key, u64> = self.live.clone();
        let mut appended: Vec<Key> = Vec::new();
        for k in removes {
            let i = sim[k];
            touch.insert(i, ());
            let (_, &p) = sim.range(..*k).next_back().expect("sentinel");
            touch.insert(p, ());
            sim.remove(k);
        }
        for k in inserts {
            let (_, &p) = sim.range(..*k).next_back().expect("sentinel");
            if p < self.leaves.len() as u64 {
                touch.insert(p, ());
            }
            sim.insert(*k, self.leaves.len() as u64 + appended.len() as u64);
            appended.push(*k);
        }
        let touched: Vec<Touched> = touch.keys().map(|&p| { let l = self.leaves[p as usize]; (p, l.0, l.1) }).collect();
        let sp: Vec<(u64, crate::coinlog::H)> = touched.iter().map(|x| (x.0, crate::coinlog::h_of(&live(&x.1, &x.2)))).collect();
        let siblings: Vec<W> = self.nodes.witness(&sp).iter().map(crate::coinlog::w_of).collect();
        // apply here
        for k in removes {
            let i = self.live[k];
            let next = self.leaves[i as usize].1;
            let (_, &p) = self.live.range(..*k).next_back().unwrap();
            let pl = self.leaves[p as usize];
            self.set(p, pl.0, next, true);
            self.set(i, *k, next, false);
            self.live.remove(k);
        }
        for k in inserts {
            let (_, &p) = self.live.range(..*k).next_back().unwrap();
            let pl = self.leaves[p as usize];
            self.set(p, pl.0, *k, true);
            self.push(*k, pl.1);
        }
        (touched, siblings)
    }
    /// Absence of `k`: the live leaf (a, b) with a < k < b, its position and path. None if `k` is live.
    pub fn absence(&self, k: &Key) -> Option<(u64, Key, Key, Vec<crate::coinlog::H>)> {
        if self.live.contains_key(k) {
            return None;
        }
        let (_, &p) = self.live.range(..*k).next_back()?;
        let l = self.leaves[p as usize];
        Some((p, l.0, l.1, self.nodes.path(p)))
    }
}

/// MEASUREMENT ONLY (the cost table, not a chain): a touched set, witness and old roots for one block's `removes` /
/// `inserts` inside a modelled tree of `n` leaves, consistent by construction: between consecutive involved keys a
/// filler live key, every needed leaf at a random position, every untouched subtree a random hash. `apply` then does
/// exactly the work it would do on a real tree of that size.
pub fn synth(n: u64, removes: &[Key], inserts: &[Key], seed: u64) -> (Vec<Option<W>>, Vec<Touched>, Vec<W>) {
    let mut rng = seed | 1;
    let mut rnd = move || { rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17; rng };
    let bump = |k: &Key, up: bool| -> Key {
        let mut o = *k;
        for i in (0..64).rev() {
            if up { let (v, c) = o[i].overflowing_add(1); o[i] = v; if !c { break; } } else { let (v, c) = o[i].overflowing_sub(1); o[i] = v; if !c { break; } }
        }
        o
    };
    let mut inv: Vec<(Key, bool)> = removes.iter().map(|k| (*k, true)).chain(inserts.iter().map(|k| (*k, false))).collect();
    inv.sort_unstable();
    // filler f_j just above each involved key, and f_-1 just below the first
    let mut leaves: Vec<(Key, Key)> = Vec::new();
    for (j, (x, is_live)) in inv.iter().enumerate() {
        let pred = if j == 0 { bump(x, false) } else { bump(&inv[j - 1].0, true) };
        let after = bump(x, true);
        if *is_live {
            leaves.push((pred, *x));
            leaves.push((*x, after));
        } else {
            leaves.push((pred, after));
        }
    }
    // positions: distinct, random below n
    let mut pos: Vec<u64> = Vec::with_capacity(leaves.len());
    let mut seen = alloc::collections::BTreeSet::new();
    while pos.len() < leaves.len() {
        let p = rnd() % n;
        if seen.insert(p) { pos.push(p); }
    }
    let mut touched: Vec<Touched> = pos.into_iter().zip(leaves).map(|(p, (k, nx))| (p, k, nx)).collect();
    touched.sort_unstable_by_key(|x| x.0);
    let e: Vec<(u64, W)> = touched.iter().map(|x| (x.0, live(&x.1, &x.2))).collect();
    fn build(l: u32, start: u64, e: &[(u64, W)], rnd: &mut dyn FnMut() -> u64, sib: &mut Vec<W>) -> W {
        if e.is_empty() {
            let mut h = [0u32; 8];
            for x in h.iter_mut() { *x = rnd() as u32; }
            sib.push(h);
            return h;
        }
        if l == 0 {
            return e[0].1;
        }
        let mid = start + (1u64 << (l - 1));
        let m = e.partition_point(|x| x.0 < mid);
        let a = build(l - 1, start, &e[..m], rnd, sib);
        let b = build(l - 1, mid, &e[m..], rnd, sib);
        let mut o = [0u32; 8];
        parent_w(l, &a, &b, &mut o);
        o
    }
    let mut roots: Vec<Option<W>> = (0..64).map(|_| None).collect();
    let mut sib = Vec::new();
    let (mut start, mut k) = (0u64, 0usize);
    for l in (0..64u32).rev() {
        if (n >> l) & 1 == 0 { continue; }
        let end = start + (1u64 << l);
        let m = k + e[k..].partition_point(|x| x.0 < end);
        roots[l as usize] = Some(if m > k { build(l, start, &e[k..m], &mut rnd, &mut sib) } else {
            let mut h = [0u32; 8];
            for x in h.iter_mut() { *x = rnd() as u32; }
            h
        });
        k = m;
        start = end;
    }
    while roots.last() == Some(&None) { roots.pop(); }
    (roots, touched, sib)
}

/// The tree witness block the guest reads (measurement builds): [n lo, n hi, roots count, (present flag, 8 words) per
/// level, touched count, (pos lo, pos hi, key 16 words, next 16 words) per touched, siblings 8 words each].
pub fn encode_bench(roots: &[Option<W>], n: u64, touched: &[Touched], sib: &[W]) -> Vec<u32> {
    let mut v = alloc::vec![n as u32, (n >> 32) as u32, roots.len() as u32];
    for r in roots {
        v.push(r.is_some() as u32);
        v.extend_from_slice(&r.unwrap_or([0; 8]));
    }
    v.push(touched.len() as u32);
    for (p, k, nx) in touched {
        v.push(*p as u32);
        v.push((*p >> 32) as u32);
        for c in k.chunks_exact(4).chain(nx.chunks_exact(4)) { v.push(u32::from_be_bytes(c.try_into().unwrap())); }
    }
    for s in sib { v.extend_from_slice(s); }
    v
}

pub fn decode_bench(w: &[u32]) -> Option<(Vec<Option<W>>, u64, Vec<Touched>, Vec<W>)> {
    let mut i = 0usize;
    let mut next = |k: usize| -> Option<&[u32]> { let s = w.get(i..i.checked_add(k)?)?; i += k; Some(s) };
    let h = next(3)?;
    let (n, nr) = (h[0] as u64 | (h[1] as u64) << 32, h[2] as usize);
    if nr > 64 { return None; }
    let mut roots = Vec::with_capacity(nr);
    for _ in 0..nr {
        let r = next(9)?;
        roots.push((r[0] == 1).then(|| r[1..9].try_into().unwrap()));
    }
    let t = next(1)?[0] as usize;
    let mut touched = Vec::with_capacity(t.min(1 << 20));
    for _ in 0..t {
        let r = next(34)?;
        let mut k = [0u8; 64];
        let mut nx = [0u8; 64];
        for j in 0..16 {
            k[4 * j..4 * j + 4].copy_from_slice(&r[2 + j].to_be_bytes());
            nx[4 * j..4 * j + 4].copy_from_slice(&r[18 + j].to_be_bytes());
        }
        touched.push((r[0] as u64 | (r[1] as u64) << 32, k, nx));
    }
    let rest = w.get(i..)?;
    if rest.len() % 8 != 0 { return None; }
    Some((roots, n, touched, rest.chunks_exact(8).map(|c| c.try_into().unwrap()).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn key(i: u64) -> Key {
        let mut k = [0u8; 64];
        k[..32].copy_from_slice(&Sha256::digest(i.to_le_bytes()));
        k
    }

    #[test]
    fn blocks_apply_and_match_the_host() {
        let mut h = Host::new();
        let (mut roots, mut n) = (h.roots(), 1u64);
        let mut live_keys: Vec<Key> = Vec::new();
        for b in 0..20u64 {
            let removes: Vec<Key> = live_keys.iter().copied().filter(|k| k[0] % 3 == (b % 3) as u8).take(5).collect();
            live_keys.retain(|k| !removes.contains(k));
            let inserts: Vec<Key> = (0..7).map(|i| key(b * 100 + i)).collect();
            live_keys.extend(&inserts);
            let (touched, sib) = h.block(&removes, &inserts);
            apply(&mut roots, &mut n, &touched, &sib, &removes, &inserts).unwrap_or_else(|e| panic!("block {b}: {e:?}"));
            assert_eq!(roots, h.roots(), "block {b}");
            assert_eq!(n, h.leaves.len() as u64);
        }
        // absence: a key never inserted has a live gap leaf in the tree
        let k = key(999_999);
        let (p, a, b, path) = h.absence(&k).unwrap();
        assert!(a < k && k < b);
        let hr: Vec<Option<crate::coinlog::H>> = roots.iter().map(|x| x.as_ref().map(crate::coinlog::h_of)).collect();
        assert!(crate::facts::coinlog_member(&hr, n, p, &crate::coinlog::h_of(&live(&a, &b)), &path));
        assert!(h.absence(&live_keys[0]).is_none());
    }

    #[test]
    fn synth_applies_at_mainnet_size() {
        let removes: Vec<Key> = (0..300).map(key).collect();
        let inserts: Vec<Key> = (1000..1500).map(key).collect();
        let n = 180_000_000u64;
        let (mut roots, touched, sib) = synth(n, &removes, &inserts, 7);
        let w = encode_bench(&roots, n, &touched, &sib);
        let (r2, n2, t2, s2) = decode_bench(&w).unwrap();
        assert_eq!((&r2, n2, &t2, &s2), (&roots, n, &touched, &sib));
        let mut nn = n;
        apply(&mut roots, &mut nn, &touched, &sib, &removes, &inserts).unwrap();
        assert_eq!(nn, n + 500);
    }

    #[test]
    fn refusals() {
        let mut h = Host::new();
        let (mut roots, mut n) = (h.roots(), 1u64);
        let ins = [key(1), key(2), key(3)];
        let (t, s) = h.block(&[], &ins);
        apply(&mut roots, &mut n, &t, &s, &[], &ins).unwrap();
        // inserting a live key again
        let (r0, n0) = (roots.clone(), n);
        let mut h2 = Host { nodes: h.nodes.clone(), leaves: h.leaves.clone(), live: h.live.clone() };
        let (t, s) = h2.block(&[], &[key(9)]);
        assert_eq!(apply(&mut roots.clone(), &mut n.clone(), &t, &s, &[], &[key(1)]), Err(TreeError::Predecessor));
        // removing a key that is not live
        assert!(apply(&mut roots.clone(), &mut n.clone(), &t, &s, &[key(7)], &[]).is_err());
        // a forged touched leaf (wrong next) fails the old root
        let mut bad = t.clone();
        bad[0].2 = key(4);
        assert!(apply(&mut roots, &mut n, &bad, &s, &[], &[key(9)]).is_err());
        assert_eq!((roots, n), (r0, n0), "unchanged on refusal");
    }
}
