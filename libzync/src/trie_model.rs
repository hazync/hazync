//! The counting trie's MODEL (host and tests only; never compiled into a guest): the in-memory trie, its reference
//! update (`apply`), the witness encoder / decoder, and the measurement synthesiser (`synth`), which builds the old trie
//! pruned to a block's paths inside a modelled population. `ttree.rs` holds what ships: the guest's one-pass update
//! (`apply_stream_perm`), the hashes and the update-list digest. The tests here check the two agree.

use crate::coinlog::{parent_w, W};
use crate::ttree::{addr_into, bit, coin_into, split, Key, TErr};
#[cfg(test)]
use crate::ttree::{apply_stream, apply_stream_perm, list_digest};
use alloc::boxed::Box;
use alloc::vec::Vec;

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


fn h(l: u32, a: &W, b: &W) -> W {
    let mut o = [0u32; 8];
    parent_w(l, a, b, &mut o);
    o
}

/// A coin leaf: one compression, the value and spent flag packed into the spender record's zero words.
/// (The spender record uses words 0-2; words 0-4 are kept, 5-6 carry the value, 7 the spent flag.)
pub fn coin_h(o: &W, sp: &Option<W>, value: u64, counts: bool) -> W {
    let mut out = [0u32; 8];
    coin_into(o, sp.as_ref(), value, counts, &mut out);
    out
}


pub fn hash_sub(n: &Sub, counts: bool) -> W {
    match n {
        Sub::Empty => [0; 8],
        Sub::Pruned(x) => *x,
        Sub::Inner(l, r) => h(50, &hash_sub(l, counts), &hash_sub(r, counts)),
        Sub::Coin { o, sp, value } => coin_h(o, sp, *value, counts),
    }
}

pub fn hash_top(n: &Top, counts: bool) -> W {
    match n {
        Top::Empty => [0; 8],
        Top::Pruned(x) => *x,
        Top::Inner(l, r) => h(50, &hash_top(l, counts), &hash_top(r, counts)),
        Top::Addr { s, count, value, sub } => {
            let mut o = [0u32; 8];
            addr_into(s, &hash_sub(sub, counts), *count, *value, counts, &mut o);
            o
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
    // depth as the guest (ttree.rs): a leaf or an empty slot at 256 is handled, an inner node there is a bad witness,
    // two coins that would need splitting there are a duplicate
    let other = match n {
        Sub::Empty => {
            *n = Sub::Coin { o: *o, sp: *sp, value: v };
            return Ok(());
        }
        Sub::Pruned(_) => return Err(TErr::Witness),
        Sub::Inner(_, _) if d >= 256 => return Err(TErr::Witness),
        Sub::Inner(l, r) => return if bit(o, d) { ins_sub(r, o, sp, v, d + 1) } else { ins_sub(l, o, sp, v, d + 1) },
        Sub::Coin { o: c, sp: old_sp, .. } => {
            // BIP30: a coin created again over its own spent record replaces it (ttree.rs, the coin leaf case)
            if *c == *o && old_sp.is_some() { *n = Sub::Coin { o: *o, sp: *sp, value: v }; return Ok(()); }
            *c
        }
    };
    if other == *o || d >= 256 { return Err(TErr::Duplicate); }
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

/// A key twice in one update list (marks and inserts together): the guest refuses it (`Duplicate`), so do the
/// model and the store. (Spending a coin and creating it again in one block: Core refuses that under BIP30.)
pub fn repeated_key<'a>(keys: impl Iterator<Item = &'a Key>) -> bool {
    let mut v: Vec<&Key> = keys.collect();
    v.sort_unstable();
    v.windows(2).any(|p| p[0] == p[1])
}

/// One block: check the pruned old trie `t` against `root`, mark `marks` (key S ‖ O, spender), insert `inserts`
/// (key, spender if the coin is already spent in its own block, value), and set `root` to the new root.
pub fn apply(root: &mut W, t: &mut Top, counts: bool, marks: &[(Key, W)], inserts: &[(Key, Option<W>, u64)]) -> Result<(), TErr> {
    if hash_top(t, counts) != *root { return Err(TErr::OldRoot); }
    if repeated_key(marks.iter().map(|m| &m.0).chain(inserts.iter().map(|i| &i.0))) { return Err(TErr::Duplicate); }
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
// Both: 5 inner whose LEFT child is pruned: its hash 8, then R | 6 inner whose RIGHT child is pruned: its hash 8, then L
// (batch 3: the one-sided nodes on a touched path, sibling first, so the guest walks a chain of them in a loop)

fn enc_sub(n: &Sub, v: &mut Vec<u32>) {
    match n {
        Sub::Empty => v.push(0),
        Sub::Pruned(x) => { v.push(1); v.extend_from_slice(x); }
        Sub::Inner(l, r) => match (&**l, &**r) {
            (Sub::Pruned(h), x) if !matches!(x, Sub::Pruned(_)) => { v.push(5); v.extend_from_slice(h); enc_sub(x, v); }
            (x, Sub::Pruned(h)) if !matches!(x, Sub::Pruned(_)) => { v.push(6); v.extend_from_slice(h); enc_sub(x, v); }
            _ => { v.push(2); enc_sub(l, v); enc_sub(r, v); }
        },
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
        Top::Inner(l, r) => match (&**l, &**r) {
            (Top::Pruned(h), x) if !matches!(x, Top::Pruned(_)) => { v.push(5); v.extend_from_slice(h); enc_top(x, v); }
            (x, Top::Pruned(h)) if !matches!(x, Top::Pruned(_)) => { v.push(6); v.extend_from_slice(h); enc_top(x, v); }
            _ => { v.push(2); enc_top(l, v); enc_top(r, v); }
        },
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
        5 => { let h = r.hash()?; Sub::Inner(Box::new(Sub::Pruned(h)), Box::new(dec_sub(r, d + 1)?)) }
        6 => { let h = r.hash()?; Sub::Inner(Box::new(dec_sub(r, d + 1)?), Box::new(Sub::Pruned(h))) }
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
        5 => { let h = r.hash()?; Top::Inner(Box::new(Top::Pruned(h)), Box::new(dec_top(r, d + 1)?)) }
        6 => { let h = r.hash()?; Top::Inner(Box::new(dec_top(r, d + 1)?), Box::new(Top::Pruned(h))) }
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
            assert_eq!(apply_stream(&w, &m, &i), Ok(r), "one-pass update gives apply's root");
            assert_eq!(apply_stream(&w, &[m[0], m[0]], &[]), Err(TErr::Duplicate));
            let mut bad = w.clone();
            bad[1] ^= 1;
            assert_eq!(apply_stream(&bad, &m, &i), Err(TErr::OldRoot));
            // marking twice, inserting twice, a wrong old root: refused
            let (mut r, mut tt) = (root, t.clone());
            assert_eq!(apply(&mut r, &mut tt, counts, &[m[0], m[0]], &[]), Err(TErr::Duplicate));
            let (mut r, mut tt) = (root, t.clone());
            assert_eq!(apply(&mut r, &mut tt, counts, &[], &[i[0], i[0]]), Err(TErr::Duplicate));
            let (mut r, mut tt) = ([9u32; 8], t.clone());
            assert_eq!(apply(&mut r, &mut tt, counts, &m, &i), Err(TErr::OldRoot));
        }
    }

    #[test]
    fn stream_matches_apply_on_random_blocks() {
        let mut x = 0x9e3779b97f4a7c15u64;
        let mut rnd = move || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x };
        for round in 0..300u64 {
            let counts = round % 2 == 0;
            // a few addresses, several coins each: marks on some, inserts on others (new and existing addresses)
            let na = 1 + rnd() % 6;
            let addrs: Vec<u8> = (0..na).map(|_| rnd() as u8).collect();
            let mut used = alloc::collections::BTreeSet::new();
            let (mut marks, mut ins) = (Vec::new(), Vec::new());
            for _ in 0..(1 + rnd() % 12) {
                let k = key(addrs[(rnd() % na) as usize], rnd() as u8);
                if !used.insert(k) { continue; }
                if rnd() % 2 == 0 { marks.push(k) } else { ins.push(k) }
            }
            let (root, t) = synth(&marks, &ins, 1 + rnd() % 5000, rnd() % 40, counts, rnd());
            let w = encode(counts, &root, &t);
            let m: Vec<(Key, W)> = marks.iter().map(|k| (*k, [rnd() as u32; 8])).collect();
            let i: Vec<(Key, Option<W>, u64)> = ins.iter().map(|k| (*k, (rnd() % 3 == 0).then(|| [7u32; 8]), rnd() >> 30)).collect();
            let (mut r, mut tt) = (root, t.clone());
            let want = apply(&mut r, &mut tt, counts, &m, &i).map(|_| r);
            assert_eq!(apply_stream(&w, &m, &i), want, "round {round}");
            // the host-supplied order gives the same root; a wrong or repeated order is refused
            let keys: Vec<(W, W)> = m.iter().map(|x| split(&x.0)).chain(i.iter().map(|x| split(&x.0))).collect();
            let mut p: Vec<u32> = (0..keys.len() as u32).collect();
            p.sort_by(|&a, &b| keys[a as usize].cmp(&keys[b as usize]));
            assert_eq!(apply_stream_perm(&w, &m, &i, Some(&p)), want, "round {round} perm");
            if p.len() >= 2 {
                let mut bad = p.clone(); bad.swap(0, 1);
                assert!(apply_stream_perm(&w, &m, &i, Some(&bad)).is_err());
                let mut rep = p.clone(); rep[1] = rep[0];
                assert!(apply_stream_perm(&w, &m, &i, Some(&rep)).is_err());
            }
            if round == 0 { assert!(w.iter().any(|&x| x == 5 || x == 6), "the witness uses one-sided tags"); }
        }
    }

    #[test]
    fn list_digest_is_sha256_of_the_list_bytes() {
        use sha2::{Digest, Sha256};
        let marks: Vec<(Key, W)> = (0..40u8).map(|i| (key(i, i), [i as u32; 8])).collect();
        let ins: Vec<(Key, Option<W>, u64)> = (0..50u8).map(|i| (key(i, 200 - i), (i % 3 == 0).then_some([7; 8]), i as u64 * 1000)).collect();
        let mut b = Vec::new();
        b.extend_from_slice(&(marks.len() as u32).to_le_bytes());
        b.extend_from_slice(&(ins.len() as u32).to_le_bytes());
        for (k, sp) in &marks { b.extend_from_slice(k); for x in sp { b.extend_from_slice(&x.to_le_bytes()); } }
        for (k, sp, v) in &ins {
            b.extend_from_slice(k);
            b.push(sp.is_some() as u8);
            for x in sp.unwrap_or([0; 8]) { b.extend_from_slice(&x.to_le_bytes()); }
            b.extend_from_slice(&v.to_le_bytes());
        }
        let want: [u8; 32] = Sha256::digest(&b).into();
        assert_eq!(list_digest(&marks, &ins), want);
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

