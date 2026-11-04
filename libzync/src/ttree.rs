//! ⚠ GUEST-COMPILED (only with the `ttree` feature) — editing this file changes that image's METHOD_ID.
//!
//! E1 MEASUREMENT (HAZYNC_TTREE): one tree for every coin fact, a two-level binary trie.
//!
//! - TOP trie, keyed by the script hash S = SHA-256(scriptPubKey), bit by bit from the most significant. Its leaves are
//!   ADDRESS leaves: (S, root of the address's coin trie, unspent count, unspent value).
//! - COIN trie per address, keyed by O = SHA-256(txid ‖ vout). Its leaves are COIN leaves: (O, spender or 0, value).
//!   A coin is never removed: a spend writes its spender (history).
//!
//! A subtree holding one leaf is that leaf (no chains of one-child nodes), so a trie of n random keys is ~log2 n deep.
//! What one proof shows at a height: a coin unspent / spent by X / absent (an empty slot or a different leaf where its
//! key leads); an address's complete history (its coin trie); its unspent count and value (one address leaf, no list).
//!
//! Hashes (one SHA-256 compression each, the coin log's per-level IVs at levels no tree reaches):
//!   inner(L, R)              = parent(50, L, R), empty = 0^32
//!   coin                     = parent(51, O, spender or 0)            [counts: parent(52, that, value)]
//!   address                  = parent(53, S, coin trie root)          [counts: parent(54, that, count ‖ value)]
//! With `counts` off the leaves carry no count or value (the plain three-state tree), one compression fewer each.
//!
//! ⛔ MEASUREMENT ONLY: the witness is the old trie pruned to the touched paths, synthesised by the host around a
//! modelled population (`synth`). The image proves the cost of the update, not a chain of tries.

use crate::coinlog::{parent_w, W};
use alloc::boxed::Box;
use alloc::vec::Vec;

pub type Key = [u8; 64];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sub {
    Empty,
    Pruned(W),
    Inner(Box<Sub>, Box<Sub>),
    Coin { o: W, sp: Option<W>, value: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Top {
    Empty,
    Pruned(W),
    Inner(Box<Top>, Box<Top>),
    Addr { s: W, count: u64, value: u64, sub: Box<Sub> },
}

#[derive(Debug, PartialEq, Eq)]
pub enum TErr {
    /// the witness's old root is not the one given
    OldRoot,
    /// a path runs into a pruned subtree, or past 256 bits
    Witness,
    /// a marked coin is not present and unspent
    Missing,
    /// an inserted coin is already present
    Duplicate,
    /// an address's count or value would go negative or overflow
    Count,
}

/// (S, O) as words from a 64-byte key S ‖ O.
pub fn split(k: &Key) -> (W, W) {
    let mut a = [0u32; 8];
    let mut b = [0u32; 8];
    for i in 0..8 {
        a[i] = u32::from_be_bytes(k[4 * i..4 * i + 4].try_into().unwrap());
        b[i] = u32::from_be_bytes(k[32 + 4 * i..36 + 4 * i].try_into().unwrap());
    }
    (a, b)
}

fn bit(k: &W, d: u32) -> bool {
    (k[(d / 32) as usize] >> (31 - d % 32)) & 1 == 1
}

fn h(l: u32, a: &W, b: &W) -> W {
    let mut o = [0u32; 8];
    parent_w(l, a, b, &mut o);
    o
}

fn num(a: u64, b: u64) -> W {
    [a as u32, (a >> 32) as u32, b as u32, (b >> 32) as u32, 0, 0, 0, 0]
}

pub fn hash_sub(n: &Sub, counts: bool) -> W {
    match n {
        Sub::Empty => [0; 8],
        Sub::Pruned(x) => *x,
        Sub::Inner(l, r) => h(50, &hash_sub(l, counts), &hash_sub(r, counts)),
        Sub::Coin { o, sp, value } => {
            let c = h(51, o, &sp.unwrap_or([0; 8]));
            if counts { h(52, &c, &num(*value, 0)) } else { c }
        }
    }
}

pub fn hash_top(n: &Top, counts: bool) -> W {
    match n {
        Top::Empty => [0; 8],
        Top::Pruned(x) => *x,
        Top::Inner(l, r) => h(50, &hash_top(l, counts), &hash_top(r, counts)),
        Top::Addr { s, count, value, sub } => {
            let a = h(53, s, &hash_sub(sub, counts));
            if counts { h(54, &a, &num(*count, *value)) } else { a }
        }
    }
}

fn mark_sub(n: &mut Sub, o: &W, sp: &W, d: u32) -> Result<u64, TErr> {
    match n {
        Sub::Inner(l, r) => {
            if d >= 256 { return Err(TErr::Witness); }
            if bit(o, d) { mark_sub(r, o, sp, d + 1) } else { mark_sub(l, o, sp, d + 1) }
        }
        Sub::Coin { o: c, sp: s, value } if c == o && s.is_none() => {
            *s = Some(*sp);
            Ok(*value)
        }
        Sub::Pruned(_) => Err(TErr::Witness),
        _ => Err(TErr::Missing),
    }
}

fn mark_top(n: &mut Top, s: &W, o: &W, sp: &W, d: u32) -> Result<(), TErr> {
    match n {
        Top::Inner(l, r) => {
            if d >= 256 { return Err(TErr::Witness); }
            if bit(s, d) { mark_top(r, s, o, sp, d + 1) } else { mark_top(l, s, o, sp, d + 1) }
        }
        Top::Addr { s: a, count, value, sub } if a == s => {
            let v = mark_sub(sub, o, sp, 0)?;
            *count = count.checked_sub(1).ok_or(TErr::Count)?;
            *value = value.checked_sub(v).ok_or(TErr::Count)?;
            Ok(())
        }
        Top::Pruned(_) => Err(TErr::Witness),
        _ => Err(TErr::Missing),
    }
}

fn ins_sub(n: &mut Sub, o: &W, sp: &Option<W>, v: u64, d: u32) -> Result<(), TErr> {
    if d >= 256 { return Err(TErr::Witness); }
    let other = match n {
        Sub::Empty => {
            *n = Sub::Coin { o: *o, sp: *sp, value: v };
            return Ok(());
        }
        Sub::Pruned(_) => return Err(TErr::Witness),
        Sub::Inner(l, r) => return if bit(o, d) { ins_sub(r, o, sp, v, d + 1) } else { ins_sub(l, o, sp, v, d + 1) },
        Sub::Coin { o: c, .. } => *c,
    };
    if other == *o { return Err(TErr::Duplicate); }
    // a leaf where the new key leads: push it one level down and try again
    let old = core::mem::replace(n, Sub::Empty);
    *n = if bit(&other, d) { Sub::Inner(Box::new(Sub::Empty), Box::new(old)) } else { Sub::Inner(Box::new(old), Box::new(Sub::Empty)) };
    ins_sub(n, o, sp, v, d)
}

fn ins_top(n: &mut Top, s: &W, o: &W, sp: &Option<W>, v: u64, d: u32) -> Result<(), TErr> {
    if d >= 256 { return Err(TErr::Witness); }
    let (dc, dv) = if sp.is_none() { (1u64, v) } else { (0, 0) };
    let other = match n {
        Top::Empty => {
            *n = Top::Addr { s: *s, count: dc, value: dv, sub: Box::new(Sub::Coin { o: *o, sp: *sp, value: v }) };
            return Ok(());
        }
        Top::Pruned(_) => return Err(TErr::Witness),
        Top::Inner(l, r) => return if bit(s, d) { ins_top(r, s, o, sp, v, d + 1) } else { ins_top(l, s, o, sp, v, d + 1) },
        Top::Addr { s: a, count, value, sub } => {
            if a == s {
                ins_sub(sub, o, sp, v, 0)?;
                *count = count.checked_add(dc).ok_or(TErr::Count)?;
                *value = value.checked_add(dv).ok_or(TErr::Count)?;
                return Ok(());
            }
            *a
        }
    };
    let old = core::mem::replace(n, Top::Empty);
    *n = if bit(&other, d) { Top::Inner(Box::new(Top::Empty), Box::new(old)) } else { Top::Inner(Box::new(old), Box::new(Top::Empty)) };
    ins_top(n, s, o, sp, v, d)
}

/// One block: check the pruned old trie `t` against `root`, mark `marks` (key S ‖ O, spender), insert `inserts`
/// (key, spender if the coin is already spent in its own block, value), and set `root` to the new root.
pub fn apply(root: &mut W, t: &mut Top, counts: bool, marks: &[(Key, W)], inserts: &[(Key, Option<W>, u64)]) -> Result<(), TErr> {
    if hash_top(t, counts) != *root { return Err(TErr::OldRoot); }
    for (k, sp) in marks {
        let (s, o) = split(k);
        mark_top(t, &s, &o, sp, 0)?;
    }
    for (k, sp, v) in inserts {
        let (s, o) = split(k);
        ins_top(t, &s, &o, sp, *v, 0)?;
    }
    *root = hash_top(t, counts);
    Ok(())
}

// ---- witness encoding: [counts, root 8 words] then the trie in preorder.
// Top: 0 empty | 1 pruned, 8 | 2 inner, L, R | 3 address, S 8, count 2, value 2, coin trie
// Sub: 0 empty | 1 pruned, 8 | 2 inner, L, R | 4 coin, O 8, spent flag 1, spender 8, value 2

fn enc_sub(n: &Sub, v: &mut Vec<u32>) {
    match n {
        Sub::Empty => v.push(0),
        Sub::Pruned(x) => { v.push(1); v.extend_from_slice(x); }
        Sub::Inner(l, r) => { v.push(2); enc_sub(l, v); enc_sub(r, v); }
        Sub::Coin { o, sp, value } => {
            v.push(4);
            v.extend_from_slice(o);
            v.push(sp.is_some() as u32);
            v.extend_from_slice(&sp.unwrap_or([0; 8]));
            v.extend_from_slice(&[*value as u32, (*value >> 32) as u32]);
        }
    }
}

fn enc_top(n: &Top, v: &mut Vec<u32>) {
    match n {
        Top::Empty => v.push(0),
        Top::Pruned(x) => { v.push(1); v.extend_from_slice(x); }
        Top::Inner(l, r) => { v.push(2); enc_top(l, v); enc_top(r, v); }
        Top::Addr { s, count, value, sub } => {
            v.push(3);
            v.extend_from_slice(s);
            v.extend_from_slice(&[*count as u32, (*count >> 32) as u32, *value as u32, (*value >> 32) as u32]);
            enc_sub(sub, v);
        }
    }
}

pub fn encode(counts: bool, root: &W, t: &Top) -> Vec<u32> {
    let mut v = alloc::vec![counts as u32];
    v.extend_from_slice(root);
    enc_top(t, &mut v);
    v
}

struct Rd<'a> { w: &'a [u32], i: usize }
impl Rd<'_> {
    fn take(&mut self, k: usize) -> Option<&[u32]> {
        let s = self.w.get(self.i..self.i.checked_add(k)?)?;
        self.i += k;
        Some(s)
    }
    fn word(&mut self) -> Option<u32> { Some(self.take(1)?[0]) }
    fn hash(&mut self) -> Option<W> { Some(self.take(8)?.try_into().unwrap()) }
    fn u64(&mut self) -> Option<u64> { let x = self.take(2)?; Some(x[0] as u64 | (x[1] as u64) << 32) }
}

fn dec_sub(r: &mut Rd, d: u32) -> Option<Sub> {
    if d > 256 { return None; }
    Some(match r.word()? {
        0 => Sub::Empty,
        1 => Sub::Pruned(r.hash()?),
        2 => { let l = dec_sub(r, d + 1)?; Sub::Inner(Box::new(l), Box::new(dec_sub(r, d + 1)?)) }
        4 => { let o = r.hash()?; let f = r.word()?; let s = r.hash()?; Sub::Coin { o, sp: (f == 1).then_some(s), value: r.u64()? } }
        _ => return None,
    })
}

fn dec_top(r: &mut Rd, d: u32) -> Option<Top> {
    if d > 256 { return None; }
    Some(match r.word()? {
        0 => Top::Empty,
        1 => Top::Pruned(r.hash()?),
        2 => { let l = dec_top(r, d + 1)?; Top::Inner(Box::new(l), Box::new(dec_top(r, d + 1)?)) }
        3 => { let s = r.hash()?; let count = r.u64()?; let value = r.u64()?; Top::Addr { s, count, value, sub: Box::new(dec_sub(r, 0)?) } }
        _ => return None,
    })
}

pub fn decode(w: &[u32]) -> Option<(bool, W, Top)> {
    let mut r = Rd { w, i: 0 };
    let counts = r.word()? == 1;
    let root = r.hash()?;
    let t = dec_top(&mut r, 0)?;
    if r.i != w.len() { return None; }
    Some((counts, root, t))
}

// ---- measurement synthesis (host): the old trie pruned to the paths a block touches, inside a modelled population.

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 { let x = &mut self.0; *x ^= *x << 13; *x ^= *x >> 7; *x ^= *x << 17; *x }
    fn w(&mut self) -> W { let mut h = [0u32; 8]; for x in h.iter_mut() { *x = self.next() as u32; } h }
    /// a key equal to `k` in its first `d` bits, random after
    fn near(&mut self, k: &W, d: u32) -> W {
        let mut o = self.w();
        for b in 0..d.min(256) {
            let (i, m) = ((b / 32) as usize, 1u32 << (31 - b % 32));
            o[i] = (o[i] & !m) | (k[i] & m);
        }
        o
    }
}

/// Split a non-involved population between two children (an odd one goes to a random side).
fn halve(q: u64, rng: &mut Rng) -> (u64, u64) {
    let a = q / 2 + if q % 2 == 1 && rng.next() & 1 == 1 { 1 } else { 0 };
    (a, q - a)
}

/// A coin trie: `keys` = (O, exists) the block touches, `q` other existing coins.
fn synth_sub(d: u32, keys: &[(W, bool)], q: u64, rng: &mut Rng) -> Sub {
    let e = keys.iter().filter(|x| x.1).count() as u64;
    if keys.is_empty() {
        return if q == 0 { Sub::Empty } else { Sub::Pruned(rng.w()) };
    }
    match e + q {
        0 => Sub::Empty,
        1 => {
            // the one existing coin, expanded (an involved one, or another the new keys must push down)
            match keys.iter().find(|x| x.1) {
                Some(k) => Sub::Coin { o: k.0, sp: None, value: rng.next() >> 24 },
                None => Sub::Coin { o: rng.near(&keys[0].0, d), sp: (rng.next() & 1 == 1).then(|| rng.w()), value: rng.next() >> 24 },
            }
        }
        _ => {
            let m = keys.partition_point(|x| !bit(&x.0, d));
            let (ql, qr) = halve(q, rng);
            Sub::Inner(Box::new(synth_sub(d + 1, &keys[..m], ql, rng)), Box::new(synth_sub(d + 1, &keys[m..], qr, rng)))
        }
    }
}

/// One address the block touches: S, its marked coins (exist), its inserted coins (new).
type AddrIn = (W, Vec<W>, Vec<W>);

fn synth_top(d: u32, addrs: &[AddrIn], q: u64, sub_extra: u64, rng: &mut Rng) -> Top {
    let exists = |a: &AddrIn| !a.1.is_empty();
    let e = addrs.iter().filter(|a| exists(a)).count() as u64;
    if addrs.is_empty() {
        return if q == 0 { Top::Empty } else { Top::Pruned(rng.w()) };
    }
    match e + q {
        0 => Top::Empty,
        1 => match addrs.iter().find(|a| exists(a)) {
            Some(a) => {
                let mut keys: Vec<(W, bool)> = a.1.iter().map(|o| (*o, true)).chain(a.2.iter().map(|o| (*o, false))).collect();
                keys.sort_unstable();
                let sub = synth_sub(0, &keys, sub_extra, rng);
                // coin values are random in the model: the address's count and value cover its marked coins
                let (mut c, mut v) = (sub_extra, rng.next() >> 20);
                fn walk(n: &Sub, c: &mut u64, v: &mut u64) {
                    match n {
                        Sub::Inner(l, r) => { walk(l, c, v); walk(r, c, v); }
                        Sub::Coin { sp: None, value, .. } => { *c += 1; *v += value; }
                        _ => {}
                    }
                }
                walk(&sub, &mut c, &mut v);
                Top::Addr { s: a.0, count: c, value: v, sub: Box::new(sub) }
            }
            None => Top::Addr { s: rng.near(&addrs[0].0, d), count: 1, value: rng.next() >> 24, sub: Box::new(Sub::Pruned(rng.w())) },
        },
        _ => {
            let m = addrs.partition_point(|a| !bit(&a.0, d));
            let (ql, qr) = halve(q, rng);
            Top::Inner(Box::new(synth_top(d + 1, &addrs[..m], ql, sub_extra, rng)),
                       Box::new(synth_top(d + 1, &addrs[m..], qr, sub_extra, rng)))
        }
    }
}

/// MEASUREMENT ONLY: the pruned old trie for one block's `marks` and `inserts` (keys S ‖ O), with `addrs` addresses in
/// the top trie and `sub_extra` other coins under each address the block spends from. An address with a marked coin
/// exists; any other inserted address is new. Returns (old root, pruned trie).
pub fn synth(marks: &[Key], inserts: &[Key], addrs: u64, sub_extra: u64, counts: bool, seed: u64) -> (W, Top) {
    let mut rng = Rng(seed | 1);
    let mut by: alloc::collections::BTreeMap<W, (Vec<W>, Vec<W>)> = alloc::collections::BTreeMap::new();
    for k in marks { let (s, o) = split(k); by.entry(s).or_default().0.push(o); }
    for k in inserts { let (s, o) = split(k); by.entry(s).or_default().1.push(o); }
    let list: Vec<AddrIn> = by.into_iter().map(|(s, (m, i))| (s, m, i)).collect();
    let e = list.iter().filter(|a| !a.1.is_empty()).count() as u64;
    let t = synth_top(0, &list, addrs.saturating_sub(e), sub_extra, &mut rng);
    (hash_top(&t, counts), t)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(s: u8, o: u8) -> Key { let mut k = [0u8; 64]; k[0] = s; k[1] = 0x5a; k[32] = o; k[33] = 0xa5; k }

    #[test]
    fn synth_applies_roundtrips_and_refuses() {
        for counts in [false, true] {
            let marks = [key(1, 1), key(1, 2), key(9, 3)];
            let ins = [key(1, 7), key(200, 8), key(201, 9)];
            let (root, t) = synth(&marks, &ins, 1_000_000, 50, counts, 7);
            let w = encode(counts, &root, &t);
            let (c2, r2, t2) = decode(&w).unwrap();
            assert_eq!((c2, r2, &t2), (counts, root, &t));
            let sp = [3u32; 8];
            let m: Vec<(Key, W)> = marks.iter().map(|k| (*k, sp)).collect();
            let i: Vec<(Key, Option<W>, u64)> = alloc::vec![(ins[0], None, 5), (ins[1], Some(sp), 6), (ins[2], None, 7)];
            let mut r = root;
            let mut tt = t.clone();
            apply(&mut r, &mut tt, counts, &m, &i).unwrap();
            assert_ne!(r, root);
            // marking twice, inserting twice, a wrong old root: refused
            let (mut r, mut tt) = (root, t.clone());
            assert_eq!(apply(&mut r, &mut tt, counts, &[m[0], m[0]], &[]), Err(TErr::Missing));
            let (mut r, mut tt) = (root, t.clone());
            assert_eq!(apply(&mut r, &mut tt, counts, &[], &[i[0], i[0]]), Err(TErr::Duplicate));
            let (mut r, mut tt) = ([9u32; 8], t.clone());
            assert_eq!(apply(&mut r, &mut tt, counts, &m, &i), Err(TErr::OldRoot));
        }
    }

    #[test]
    fn counts_track_unspent() {
        // from empty: two coins at one address, one spent in its own block; then mark the other
        let mut t = Top::Empty;
        let mut r = hash_top(&t, true);
        apply(&mut r, &mut t, true, &[], &[(key(4, 1), None, 100), (key(4, 2), Some([1; 8]), 50)]).unwrap();
        let Top::Addr { count, value, .. } = &t else { panic!() };
        assert_eq!((*count, *value), (1, 100));
        apply(&mut r, &mut t, true, &[(key(4, 1), [2; 8])], &[]).unwrap();
        let Top::Addr { count, value, .. } = &t else { panic!() };
        assert_eq!((*count, *value), (0, 0));
        assert_eq!(r, hash_top(&t, true));
    }
}
