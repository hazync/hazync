//! The host's counting-trie STORE (E2 step 3; host only, never compiled into a guest): the whole trie, every node's hash
//! cached, advanced one block at a time. Per block it serves the guest's witness (the old trie pruned to the block's
//! paths, the format `trie_model::encode` writes and `ttree::apply_stream_perm` reads), applies the block's update
//! list, and can undo it exactly (a reorg). It replaces `trie_model::synth` for real proofs.
//!
//! The trie's shape is a function of its key set (a subtree holding one leaf IS that leaf), so a store built by any
//! sequence of applies and undos that leaves the same coins has the same root. Undo removes an inserted coin and
//! collapses the chain above it back to that rule; an address left with no coins is removed.
//!
//! Snapshot (`to_bytes` / `from_bytes`): "HZTRIE01", counts u32, height u64, block hash 32 bytes, node count u64, the
//! trie in preorder (top: 0 empty | 2 inner L R | 3 address S count value, coin trie; coin trie: 0 empty | 2 inner L R |
//! 4 coin O, spent flag, spender, value; words little-endian), then the root (8 words). Load recomputes every hash and
//! refuses a snapshot whose root does not match, or whose shape breaks the one-leaf rule or the key prefixes.
//!
//! In memory, each node is a heap box (~100-150 bytes): fine for regtest, signet and the test pass. Mainnet's every coin
//! ever made (~3.5 billion) needs a disk-backed node arena behind the same API.

use crate::coinlog::{parent_w, W};
use crate::trie_model::{Sub, Top};
use crate::ttree::{addr_into, bit, coin_into, split, Key, TErr};
use alloc::boxed::Box;
use alloc::vec::Vec;

const ZERO: W = [0; 8];

enum SN {
    Empty,
    Inner(Box<SN>, Box<SN>, W),
    Coin { o: W, sp: Option<W>, value: u64, h: W },
}

enum TN {
    Empty,
    Inner(Box<TN>, Box<TN>, W),
    Addr { s: W, count: u64, value: u64, sub: Box<SN>, h: W },
}

fn inner(a: &W, b: &W) -> W {
    let mut o = ZERO;
    parent_w(50, a, b, &mut o);
    o
}

impl SN {
    fn h(&self) -> W {
        match self { SN::Empty => ZERO, SN::Inner(_, _, h) | SN::Coin { h, .. } => *h }
    }
    fn coin(o: &W, sp: Option<W>, value: u64, c: bool) -> SN {
        let mut h = ZERO;
        coin_into(o, sp.as_ref(), value, c, &mut h);
        SN::Coin { o: *o, sp, value, h }
    }
    fn rehash(&mut self, c: bool) {
        match self {
            SN::Empty => {}
            SN::Inner(l, r, h) => *h = inner(&l.h(), &r.h()),
            SN::Coin { o, sp, value, h } => coin_into(o, sp.as_ref(), *value, c, h),
        }
    }
    fn is_leaf(&self) -> bool { matches!(self, SN::Coin { .. }) }
}

impl TN {
    fn h(&self) -> W {
        match self { TN::Empty => ZERO, TN::Inner(_, _, h) | TN::Addr { h, .. } => *h }
    }
    fn rehash(&mut self, c: bool) {
        match self {
            TN::Empty => {}
            TN::Inner(l, r, h) => *h = inner(&l.h(), &r.h()),
            TN::Addr { s, count, value, sub, h } => addr_into(s, &sub.h(), *count, *value, c, h),
        }
    }
    fn is_leaf(&self) -> bool { matches!(self, TN::Addr { .. }) }
}

// ---- apply (as trie_model's mark / ins, keeping the hashes)

fn s_mark(n: &mut SN, o: &W, sp: &W, d: u32, c: bool) -> Result<u64, TErr> {
    let v = match n {
        SN::Inner(l, r, _) => {
            if d >= 256 { return Err(TErr::Witness); }
            if bit(o, d) { s_mark(r, o, sp, d + 1, c)? } else { s_mark(l, o, sp, d + 1, c)? }
        }
        SN::Coin { o: x, sp: s, value, .. } if x == o && s.is_none() => { *s = Some(*sp); *value }
        _ => return Err(TErr::Missing),
    };
    n.rehash(c);
    Ok(v)
}

fn s_ins(n: &mut SN, o: &W, sp: &Option<W>, v: u64, d: u32, c: bool) -> Result<(), TErr> {
    if d >= 256 { return Err(TErr::Witness); }
    let other = match n {
        SN::Empty => { *n = SN::coin(o, *sp, v, c); return Ok(()); }
        SN::Inner(l, r, _) => {
            if bit(o, d) { s_ins(r, o, sp, v, d + 1, c)? } else { s_ins(l, o, sp, v, d + 1, c)? }
            n.rehash(c);
            return Ok(());
        }
        SN::Coin { o: x, .. } => *x,
    };
    if other == *o { return Err(TErr::Duplicate); }
    let old = core::mem::replace(n, SN::Empty);
    *n = if bit(&other, d) { SN::Inner(Box::new(SN::Empty), Box::new(old), ZERO) } else { SN::Inner(Box::new(old), Box::new(SN::Empty), ZERO) };
    s_ins(n, o, sp, v, d, c)
}

fn t_mark(n: &mut TN, s: &W, o: &W, sp: &W, d: u32, c: bool) -> Result<(), TErr> {
    match n {
        TN::Inner(l, r, _) => {
            if d >= 256 { return Err(TErr::Witness); }
            if bit(s, d) { t_mark(r, s, o, sp, d + 1, c)? } else { t_mark(l, s, o, sp, d + 1, c)? }
        }
        TN::Addr { s: a, count, value, sub, .. } if a == s => {
            let v = s_mark(sub, o, sp, 0, c)?;
            *count = count.checked_sub(1).ok_or(TErr::Count)?;
            *value = value.checked_sub(v).ok_or(TErr::Count)?;
        }
        _ => return Err(TErr::Missing),
    }
    n.rehash(c);
    Ok(())
}

fn t_ins(n: &mut TN, s: &W, o: &W, sp: &Option<W>, v: u64, d: u32, c: bool) -> Result<(), TErr> {
    if d >= 256 { return Err(TErr::Witness); }
    let (dc, dv) = if sp.is_none() { (1u64, v) } else { (0, 0) };
    let other = match n {
        TN::Empty => {
            *n = TN::Addr { s: *s, count: dc, value: dv, sub: Box::new(SN::coin(o, *sp, v, c)), h: ZERO };
            n.rehash(c);
            return Ok(());
        }
        TN::Inner(l, r, _) => {
            if bit(s, d) { t_ins(r, s, o, sp, v, d + 1, c)? } else { t_ins(l, s, o, sp, v, d + 1, c)? }
            n.rehash(c);
            return Ok(());
        }
        TN::Addr { s: a, count, value, sub, .. } => {
            if a == s {
                // checked before the coin goes in, so a refused insert leaves the store as it was
                let nc = count.checked_add(dc).ok_or(TErr::Count)?;
                let nv = value.checked_add(dv).ok_or(TErr::Count)?;
                s_ins(sub, o, sp, v, 0, c)?;
                (*count, *value) = (nc, nv);
                n.rehash(c);
                return Ok(());
            }
            *a
        }
    };
    let old = core::mem::replace(n, TN::Empty);
    *n = if bit(&other, d) { TN::Inner(Box::new(TN::Empty), Box::new(old), ZERO) } else { TN::Inner(Box::new(old), Box::new(TN::Empty), ZERO) };
    t_ins(n, s, o, sp, v, d, c)
}

// ---- undo (a reorg): unmark a spend, remove an inserted coin and collapse the chain above it

fn s_unmark(n: &mut SN, o: &W, d: u32, c: bool) -> Result<u64, TErr> {
    let v = match n {
        SN::Inner(l, r, _) => {
            if d >= 256 { return Err(TErr::Witness); }
            if bit(o, d) { s_unmark(r, o, d + 1, c)? } else { s_unmark(l, o, d + 1, c)? }
        }
        SN::Coin { o: x, sp: s, value, .. } if x == o && s.is_some() => { *s = None; *value }
        _ => return Err(TErr::Missing),
    };
    n.rehash(c);
    Ok(v)
}

/// After a removal under an inner node: a subtree left with one leaf becomes that leaf, with none becomes empty.
macro_rules! collapse {
    ($n:expr, $T:ident, $c:expr) => {{
        let fold = match &*$n {
            $T::Inner(l, r, _) => match (&**l, &**r) {
                ($T::Empty, $T::Empty) => Some(None),
                ($T::Empty, x) | (x, $T::Empty) if x.is_leaf() => Some(Some(matches!(&**l, $T::Empty))),
                _ => None,
            },
            _ => None,
        };
        match fold {
            Some(None) => *$n = $T::Empty,
            Some(Some(right)) => {
                let $T::Inner(l, r, _) = core::mem::replace($n, $T::Empty) else { unreachable!() };
                *$n = if right { *r } else { *l };
            }
            None => $n.rehash($c),
        }
    }};
}

/// Remove coin `o`; returns (value, it was unspent).
fn s_rm(n: &mut SN, o: &W, d: u32, c: bool) -> Result<(u64, bool), TErr> {
    match n {
        SN::Coin { o: x, sp, value, .. } if x == o => {
            let r = (*value, sp.is_none());
            *n = SN::Empty;
            Ok(r)
        }
        SN::Inner(l, r, _) => {
            if d >= 256 { return Err(TErr::Witness); }
            let out = if bit(o, d) { s_rm(r, o, d + 1, c)? } else { s_rm(l, o, d + 1, c)? };
            collapse!(n, SN, c);
            Ok(out)
        }
        _ => Err(TErr::Missing),
    }
}

fn t_unmark(n: &mut TN, s: &W, o: &W, d: u32, c: bool) -> Result<(), TErr> {
    match n {
        TN::Inner(l, r, _) => {
            if d >= 256 { return Err(TErr::Witness); }
            if bit(s, d) { t_unmark(r, s, o, d + 1, c)? } else { t_unmark(l, s, o, d + 1, c)? }
        }
        TN::Addr { s: a, count, value, sub, .. } if a == s => {
            let v = s_unmark(sub, o, 0, c)?;
            *count = count.checked_add(1).ok_or(TErr::Count)?;
            *value = value.checked_add(v).ok_or(TErr::Count)?;
        }
        _ => return Err(TErr::Missing),
    }
    n.rehash(c);
    Ok(())
}

fn t_rm(n: &mut TN, s: &W, o: &W, d: u32, c: bool) -> Result<(), TErr> {
    match n {
        TN::Addr { s: a, count, value, sub, .. } if a == s => {
            let (v, unspent) = s_rm(sub, o, 0, c)?;
            if unspent {
                *count = count.checked_sub(1).ok_or(TErr::Count)?;
                *value = value.checked_sub(v).ok_or(TErr::Count)?;
            }
            if matches!(**sub, SN::Empty) { *n = TN::Empty; } else { n.rehash(c); }
            Ok(())
        }
        TN::Inner(l, r, _) => {
            if d >= 256 { return Err(TErr::Witness); }
            if bit(s, d) { t_rm(r, s, o, d + 1, c)? } else { t_rm(l, s, o, d + 1, c)? }
            collapse!(n, TN, c);
            Ok(())
        }
        _ => Err(TErr::Missing),
    }
}

// ---- the witness: expand every node a key's path passes through, prune the rest (sorted keys)

fn s_wit(n: &SN, ks: &[W], d: u32) -> Sub {
    match n {
        SN::Empty => Sub::Empty,
        _ if ks.is_empty() => Sub::Pruned(n.h()),
        SN::Inner(l, r, _) => {
            let m = ks.partition_point(|k| !bit(k, d));
            Sub::Inner(Box::new(s_wit(l, &ks[..m], d + 1)), Box::new(s_wit(r, &ks[m..], d + 1)))
        }
        SN::Coin { o, sp, value, .. } => Sub::Coin { o: *o, sp: *sp, value: *value },
    }
}

fn t_wit(n: &TN, ks: &[(W, W)], d: u32) -> Top {
    match n {
        TN::Empty => Top::Empty,
        _ if ks.is_empty() => Top::Pruned(n.h()),
        TN::Inner(l, r, _) => {
            let m = ks.partition_point(|k| !bit(&k.0, d));
            Top::Inner(Box::new(t_wit(l, &ks[..m], d + 1)), Box::new(t_wit(r, &ks[m..], d + 1)))
        }
        TN::Addr { s, count, value, sub, .. } => {
            // the keys at this address reach into its coin trie; others only meet the leaf (pruned coin trie)
            let os: Vec<W> = ks.iter().filter(|k| k.0 == *s).map(|k| k.1).collect();
            Top::Addr { s: *s, count: *count, value: *value, sub: Box::new(s_wit(sub, &os, 0)) }
        }
    }
}

// ---- snapshot

struct Wr(Vec<u8>, u64);
impl Wr {
    fn u32(&mut self, x: u32) { self.0.extend_from_slice(&x.to_le_bytes()); }
    fn u64(&mut self, x: u64) { self.0.extend_from_slice(&x.to_le_bytes()); }
    fn w(&mut self, x: &W) { for v in x { self.u32(*v); } }
}

fn s_enc(n: &SN, o: &mut Wr) {
    o.1 += 1;
    match n {
        SN::Empty => o.u32(0),
        SN::Inner(l, r, _) => { o.u32(2); s_enc(l, o); s_enc(r, o); }
        SN::Coin { o: k, sp, value, .. } => { o.u32(4); o.w(k); o.u32(sp.is_some() as u32); o.w(&sp.unwrap_or(ZERO)); o.u64(*value); }
    }
}

fn t_enc(n: &TN, o: &mut Wr) {
    o.1 += 1;
    match n {
        TN::Empty => o.u32(0),
        TN::Inner(l, r, _) => { o.u32(2); t_enc(l, o); t_enc(r, o); }
        TN::Addr { s, count, value, sub, .. } => { o.u32(3); o.w(s); o.u64(*count); o.u64(*value); s_enc(sub, o); }
    }
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

fn s_dec(r: &mut Rd, d: u32, pre: W, c: bool) -> Result<SN, TErr> {
    if d > 256 { return Err(TErr::Witness); }
    let mut n = match r.u32()? {
        0 => SN::Empty,
        2 => {
            if d >= 256 { return Err(TErr::Witness); }
            let mut rp = pre;
            rp[(d / 32) as usize] |= 1 << (31 - d % 32);
            let l = s_dec(r, d + 1, pre, c)?;
            let rr = s_dec(r, d + 1, rp, c)?;
            let leaves = |x: &SN| match x { SN::Empty => 0, SN::Coin { .. } => 1, _ => 2 };
            if leaves(&l) + leaves(&rr) < 2 { return Err(TErr::Witness); }
            SN::Inner(Box::new(l), Box::new(rr), ZERO)
        }
        4 => {
            let o = r.w()?;
            let f = r.u32()?;
            let sp = r.w()?;
            let value = r.u64()?;
            // the spender record is words 0-4; the leaf hash carries the value and flag in 5-7, so those must be zero here
            if f > 1 || (f == 0 && sp != ZERO) || sp[5..] != [0; 3] || !on_path(&o, &pre, d) { return Err(TErr::Witness); }
            SN::Coin { o, sp: (f == 1).then_some(sp), value, h: ZERO }
        }
        _ => return Err(TErr::Witness),
    };
    n.rehash(c);
    Ok(n)
}

/// Returns the node and (its unspent count, value) for the address checks above it.
fn s_unspent(n: &SN) -> (u64, u64) {
    match n {
        SN::Empty => (0, 0),
        SN::Inner(l, r, _) => { let (a, b) = (s_unspent(l), s_unspent(r)); (a.0 + b.0, a.1.saturating_add(b.1)) }
        SN::Coin { sp, value, .. } => if sp.is_none() { (1, *value) } else { (0, 0) },
    }
}

fn t_dec(r: &mut Rd, d: u32, pre: W, c: bool) -> Result<TN, TErr> {
    if d > 256 { return Err(TErr::Witness); }
    let mut n = match r.u32()? {
        0 => TN::Empty,
        2 => {
            if d >= 256 { return Err(TErr::Witness); }
            let mut rp = pre;
            rp[(d / 32) as usize] |= 1 << (31 - d % 32);
            let l = t_dec(r, d + 1, pre, c)?;
            let rr = t_dec(r, d + 1, rp, c)?;
            let leaves = |x: &TN| match x { TN::Empty => 0, TN::Addr { .. } => 1, _ => 2 };
            if leaves(&l) + leaves(&rr) < 2 { return Err(TErr::Witness); }
            TN::Inner(Box::new(l), Box::new(rr), ZERO)
        }
        3 => {
            let s = r.w()?;
            let count = r.u64()?;
            let value = r.u64()?;
            if !on_path(&s, &pre, d) { return Err(TErr::Witness); }
            let sub = s_dec(r, 0, ZERO, c)?;
            if matches!(sub, SN::Empty) || s_unspent(&sub) != (count, value) { return Err(TErr::Count); }
            TN::Addr { s, count, value, sub: Box::new(sub), h: ZERO }
        }
        _ => return Err(TErr::Witness),
    };
    n.rehash(c);
    Ok(n)
}

/// The whole trie, as of `height` (block `hash`; zero for an empty store).
pub struct TrieStore {
    counts: bool,
    pub height: u64,
    pub hash: [u8; 32],
    t: TN,
}

/// One block's update list, as the aggregate's collect run logs it and the trie receipt commits.
pub type Marks = [(Key, W)];
pub type Inserts = [(Key, Option<W>, u64)];

impl TrieStore {
    pub fn new(counts: bool) -> Self { TrieStore { counts, height: 0, hash: [0; 32], t: TN::Empty } }
    pub fn counts(&self) -> bool { self.counts }
    pub fn root(&self) -> W { self.t.h() }

    /// The guest's witness for this update list (`trie_model::encode` format), against the current root.
    pub fn witness(&self, marks: &Marks, inserts: &Inserts) -> Vec<u32> {
        let mut ks: Vec<(W, W)> = marks.iter().map(|x| split(&x.0)).chain(inserts.iter().map(|x| split(&x.0))).collect();
        ks.sort_unstable();
        crate::trie_model::encode(self.counts, &self.root(), &t_wit(&self.t, &ks, 0))
    }

    /// Apply one block's update list (marks, then inserts). All or nothing: on an error the store is as it was.
    pub fn apply(&mut self, marks: &Marks, inserts: &Inserts) -> Result<W, TErr> {
        let c = self.counts;
        // a spender record is words 0-4 (the leaf hash overwrites 5-7): refuse more, so the store stays loadable
        let wide = |sp: &W| sp[5..] != [0; 3];
        if marks.iter().any(|m| wide(&m.1)) || inserts.iter().any(|i| i.1.as_ref().is_some_and(wide)) { return Err(TErr::Witness); }
        let mut done = (0, 0);
        let r = (|| {
            for (k, sp) in marks {
                let (s, o) = split(k);
                t_mark(&mut self.t, &s, &o, sp, 0, c)?;
                done.0 += 1;
            }
            for (k, sp, v) in inserts {
                let (s, o) = split(k);
                t_ins(&mut self.t, &s, &o, sp, *v, 0, c)?;
                done.1 += 1;
            }
            Ok(())
        })();
        if let Err(e) = r {
            self.undo(&marks[..done.0], &inserts[..done.1]).expect("rolling back a partial apply");
            return Err(e);
        }
        Ok(self.root())
    }

    /// Undo one block's update list (the one last applied): remove its inserts, unmark its marks.
    pub fn undo(&mut self, marks: &Marks, inserts: &Inserts) -> Result<W, TErr> {
        let c = self.counts;
        for (k, _, _) in inserts.iter().rev() {
            let (s, o) = split(k);
            t_rm(&mut self.t, &s, &o, 0, c)?;
        }
        for (k, _) in marks.iter().rev() {
            let (s, o) = split(k);
            t_unmark(&mut self.t, &s, &o, 0, c)?;
        }
        Ok(self.root())
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Wr(Vec::new(), 0);
        o.0.extend_from_slice(b"HZTRIE01");
        o.u32(self.counts as u32);
        o.u64(self.height);
        o.0.extend_from_slice(&self.hash);
        let at = o.0.len();
        o.u64(0);
        t_enc(&self.t, &mut o);
        o.w(&self.root());
        let n = o.1;
        o.0[at..at + 8].copy_from_slice(&n.to_le_bytes());
        o.0
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self, TErr> {
        let mut r = Rd(b, 0);
        if r.take(8)? != b"HZTRIE01" { return Err(TErr::Witness); }
        let counts = match r.u32()? { 0 => false, 1 => true, _ => return Err(TErr::Witness) };
        let height = r.u64()?;
        let hash: [u8; 32] = r.take(32)?.try_into().unwrap();
        let _nodes = r.u64()?;
        let t = t_dec(&mut r, 0, ZERO, counts)?;
        let root = r.w()?;
        if r.1 != b.len() { return Err(TErr::Witness); }
        if t.h() != root { return Err(TErr::OldRoot); }
        Ok(TrieStore { counts, height, hash, t })
    }
}

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
            assert!(matches!(st.t, TN::Empty));
        }
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
        assert_eq!(st.apply(&[(a.0, [1, 0, 0, 0, 0, 0, 0, 0]), (a.0, [2, 0, 0, 0, 0, 0, 0, 0])], &[]), Err(TErr::Missing));
        assert_eq!(st.apply(&[(a.0, [0, 0, 0, 0, 0, 1, 0, 0])], &[]), Err(TErr::Witness), "spender word 5");
        assert_eq!((st.root(), st.to_bytes()), (r0, snap));
        // insertion order does not matter
        let mut st2 = TrieStore::new(true);
        st2.apply(&[], &[b]).unwrap();
        st2.apply(&[], &[a]).unwrap();
        assert_eq!(st2.root(), r0);
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
