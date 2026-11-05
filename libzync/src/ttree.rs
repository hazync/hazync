//! ⚠ GUEST-COMPILED (feature `trie`, and the trie guest) — editing this file changes those images' METHOD_IDs.
//!
//! The counting trie: one tree for every coin fact, a two-level binary trie (chosen 11-04, ROADMAP E1c).
//!
//! - TOP trie, keyed by the script hash S = SHA-256(scriptPubKey), bit by bit from the most significant. Its leaves are
//!   ADDRESS leaves: (S, root of the address's coin trie, unspent count, unspent value).
//! - COIN trie per address, keyed by O = the txid with the output index XORed into its FIRST 4 bytes (one transaction's
//!   coins at one address part at the top). Its leaves are COIN leaves: (O, spender or 0, value). A coin is never
//!   removed: a spend writes its spender (history).
//!
//! A subtree holding one leaf is that leaf, so a trie of n random keys is ~log2 n deep. What one proof shows at a
//! height: a coin unspent / spent by X / absent (an empty slot or a different leaf where its key leads); an address's
//! complete history (its coin trie); its unspent count and value (one address leaf, no list).
//!
//! Hashes (one SHA-256 compression each, the coin log's per-level IVs at levels no tree reaches):
//!   inner(L, R)  = parent(50, L, R), empty = 0^32
//!   coin         = parent(51, O, P): P = the spender record (height, tx, vin; words 0-4) or 0, the value in words 5-6
//!                  (counts) and 1 in word 7 if spent
//!   address      = parent(53, S, coin trie root), then (counts) parent(54, that, count ‖ value): the full 256-bit S
//!
//! This file is what ships: the guest's one-pass update (`apply_stream_perm`) over a witness of the old trie pruned to
//! the block's paths, and the update-list digest. The in-memory model, the witness encoder and the measurement
//! synthesiser are in `trie_model.rs` (host and tests only).

use crate::coinlog::{parent_w, W};
use alloc::vec::Vec;

pub type Key = [u8; 64];

/// The trie receipt's journal (guest mode 17, E2): [KIND_TRIE, old root 8 words, new root 8 words, update-list digest 8
/// words (its bytes as little-endian words)]. The aggregate verifies the receipt, requires the old root to be the
/// previous chain state's trie root and the digest to be its own update list's, and commits the new root.
pub const KIND_TRIE: u32 = 0xC4A1_0017;
pub const TRIE_JOURNAL_WORDS: usize = 25;
pub fn trie_journal(old: &W, new: &W, digest: &[u8; 32]) -> [u32; TRIE_JOURNAL_WORDS] {
    let mut j = [0u32; TRIE_JOURNAL_WORDS];
    j[0] = KIND_TRIE;
    j[1..9].copy_from_slice(old);
    j[9..17].copy_from_slice(new);
    for (x, c) in j[17..25].iter_mut().zip(digest.chunks_exact(4)) { *x = u32::from_le_bytes(c.try_into().unwrap()); }
    j
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

pub(crate) fn bit(k: &W, d: u32) -> bool {
    (k[(d / 32) as usize] >> (31 - d % 32)) & 1 == 1
}

// ---- the guest's update (trim, 11-04): ONE pass over the witness in preorder, no tree built. Each expanded node yields
// its old and new hash at once (as itree's `update`); a slot where inserts land (empty, or a leaf they must share) gets
// its new subtree built straight from the sorted inserts. Same roots as `apply` on the decoded trie (tested).
// Trim 2: hashes are written in place (out-parameters) and the witness's hashes are read by reference, so a node costs
// its compressions and no 32-byte copies (the profile's memcpy).

const ZERO: W = [0; 8];

/// One update, sorted by (S, O). `k`: index into the marks (bit 31 clear) or the inserts (bit 31 set).
#[derive(Clone, Copy)]
pub struct Up { pub s: W, pub o: W, pub k: u32 }

struct Cx<'x> { marks: &'x [(Key, W)], ins: &'x [(Key, Option<W>, u64)], counts: bool }
impl Cx<'_> {
    #[inline(always)]
    fn mark(&self, u: &Up) -> Option<&W> { if u.k >> 31 == 0 { Some(&self.marks[u.k as usize].1) } else { None } }
    #[inline(always)]
    fn ins(&self, u: &Up) -> Option<(Option<&W>, u64)> {
        if u.k >> 31 == 1 { let x = &self.ins[(u.k & 0x7fff_ffff) as usize]; Some((x.1.as_ref(), x.2)) } else { None }
    }
}

#[derive(Default)]
struct Acc { mc: u64, mv: u64, pc: u64, pv: u64 }

struct Rs<'a> { w: &'a [u32], i: usize }
impl<'a> Rs<'a> {
    #[inline(always)]
    fn word(&mut self) -> Result<u32, TErr> { let x = *self.w.get(self.i).ok_or(TErr::Witness)?; self.i += 1; Ok(x) }
    #[inline(always)]
    fn peek(&self) -> u32 { self.w.get(self.i).copied().unwrap_or(u32::MAX) }
    #[inline(always)]
    fn hash(&mut self) -> Result<&'a W, TErr> {
        let w = self.w;
        let s = w.get(self.i..self.i + 8).ok_or(TErr::Witness)?;
        self.i += 8;
        Ok(s.try_into().unwrap())
    }
    #[inline(always)]
    fn u64(&mut self) -> Result<u64, TErr> { Ok(self.word()? as u64 | (self.word()? as u64) << 32) }
}

#[inline(always)]
pub(crate) fn coin_into(o: &W, sp: Option<&W>, value: u64, counts: bool, out: &mut W) {
    let mut p = ZERO;
    if let Some(s) = sp { p[0] = s[0]; p[1] = s[1]; p[2] = s[2]; p[3] = s[3]; p[4] = s[4]; p[7] = 1; }
    if counts { p[5] = value as u32; p[6] = (value >> 32) as u32; }
    parent_w(51, o, &p, out);
}

/// An address leaf: the FULL 256-bit script hash and the coin trie root, then (counts) the unspent count and value, both
/// 64-bit. Two compressions with counts: binding all 256 bits costs one compression per address (decided 11-04: a
/// 160-bit leaf saved under a point but let two scripts one party controls collide in ~2^80).
#[inline(always)]
pub(crate) fn addr_into(s: &W, sub: &W, count: u64, value: u64, counts: bool, out: &mut W) {
    if counts {
        let mut a = ZERO;
        parent_w(53, s, sub, &mut a);
        let n = [count as u32, (count >> 32) as u32, value as u32, (value >> 32) as u32, 0, 0, 0, 0];
        parent_w(54, &a, &n, out);
    } else {
        parent_w(53, s, sub, out);
    }
}

/// Batch 3: walk a chain of one-sided inner nodes (tags 5 / 6) in a loop, keeping each pruned sibling by reference, then
/// run `node` at the chain's end and fold the siblings back up: two compressions per level, no call, no zeroed outputs,
/// no partition. Every update must lie on the expanded side (they are sorted, so the first and last decide). A chain
/// longer than CH is finished by the next call (the loop ends and `node` sees the tag).
pub(crate) const CH: usize = if cfg!(test) { 3 } else { 64 };  // tests: 3, so both the loop and the fallback run
#[inline(always)]
fn chain<'a>(r: &mut Rs<'a>, d0: u32, ups: &[Up], key: fn(&Up) -> &W, old: &mut W, new: &mut W,
             node: &mut dyn FnMut(&mut Rs<'a>, u32, &mut W, &mut W) -> Result<(), TErr>) -> Result<(), TErr> {
    let mut sib: [core::mem::MaybeUninit<(&'a W, bool)>; CH] = unsafe { core::mem::MaybeUninit::uninit().assume_init() };
    let (mut n, mut d) = (0usize, d0);
    while n < CH {
        let t = r.peek();
        if t != 5 && t != 6 { break; }
        if d >= 256 { return Err(TErr::Witness); }
        r.word()?;
        let h = r.hash()?;
        let left_pruned = t == 5;
        if let (Some(f), Some(l)) = (ups.first(), ups.last()) {
            // left pruned: every update goes right (bit 1), so the smallest has bit 1; right pruned: the largest has bit 0
            if left_pruned && !bit(key(f), d) { return Err(TErr::Witness); }
            if !left_pruned && bit(key(l), d) { return Err(TErr::Witness); }
        }
        sib[n].write((h, left_pruned));
        n += 1;
        d += 1;
    }
    if n == 0 { return node(r, d, old, new); }
    let mut buf = [[ZERO; 2]; 2];
    {
        let (a, b) = buf[0].split_at_mut(1);
        node(r, d, &mut a[0], &mut b[0])?;
    }
    let mut c = 0usize;
    for i in (0..n).rev() {
        let (h, lp) = unsafe { sib[i].assume_init() };
        let (x, y) = buf.split_at_mut(1);
        let (src, dst) = if c == 0 { (&x[0], &mut y[0]) } else { (&y[0], &mut x[0]) };
        if lp { parent_w(50, h, &src[0], &mut dst[0]); parent_w(50, h, &src[1], &mut dst[1]); }
        else { parent_w(50, &src[0], h, &mut dst[0]); parent_w(50, &src[1], h, &mut dst[1]); }
        c ^= 1;
    }
    *old = buf[c][0];
    *new = buf[c][1];
    Ok(())
}

fn sub_pass(r: &mut Rs, d: u32, ups: &[Up], cx: &Cx, acc: &mut Acc, old: &mut W, new: &mut W) -> Result<(), TErr> {
    chain(r, d, ups, |u| &u.o, old, new, &mut |r, d, o, n| sub_node(r, d, ups, cx, acc, o, n))
}

fn top_pass(r: &mut Rs, d: u32, ups: &[Up], cx: &Cx, old: &mut W, new: &mut W) -> Result<(), TErr> {
    chain(r, d, ups, |u| &u.s, old, new, &mut |r, d, o, n| top_node(r, d, ups, cx, o, n))
}

/// A one-sided node reached by `node` itself (a chain longer than CH): handled recursively.
fn one_sided(r: &mut Rs, d: u32, tag: u32, ups: &[Up], key: fn(&Up) -> &W, old: &mut W, new: &mut W,
             rest: &mut dyn FnMut(&mut Rs, u32, &mut W, &mut W) -> Result<(), TErr>) -> Result<(), TErr> {
    if d >= 256 { return Err(TErr::Witness); }
    let h = *r.hash()?;
    let lp = tag == 5;
    if let (Some(f), Some(l)) = (ups.first(), ups.last()) {
        if lp && !bit(key(f), d) { return Err(TErr::Witness); }
        if !lp && bit(key(l), d) { return Err(TErr::Witness); }
    }
    let (mut a, mut b) = (ZERO, ZERO);
    rest(r, d + 1, &mut a, &mut b)?;
    if lp { parent_w(50, &h, &a, old); parent_w(50, &h, &b, new); } else { parent_w(50, &a, &h, old); parent_w(50, &b, &h, new); }
    Ok(())
}

/// A new coin trie from inserts only (`a`, then `b`, sorted by O) plus at most one existing coin leaf (O, hash).
fn build_sub(d: u32, a: &[Up], b: &[Up], extra: Option<(&W, &W)>, cx: &Cx, acc: &mut Acc, out: &mut W) -> Result<(), TErr> {
    let n = a.len() + b.len() + extra.is_some() as usize;
    if n == 0 { *out = ZERO; return Ok(()); }
    if n == 1 {
        if let Some((_, h)) = extra { *out = *h; return Ok(()); }
        let u = if a.is_empty() { &b[0] } else { &a[0] };
        let (sp, v) = cx.ins(u).ok_or(TErr::Missing)?;
        if sp.is_none() { acc.pc += 1; acc.pv = acc.pv.checked_add(v).ok_or(TErr::Count)?; }
        coin_into(&u.o, sp, v, cx.counts, out);
        return Ok(());
    }
    if d >= 256 { return Err(TErr::Duplicate); }
    let (ma, mb) = (a.partition_point(|u| !bit(&u.o, d)), b.partition_point(|u| !bit(&u.o, d)));
    let (el, er) = match extra { Some(e) if bit(e.0, d) => (None, Some(e)), e => (e, None) };
    let (mut l, mut r) = (ZERO, ZERO);
    build_sub(d + 1, &a[..ma], &b[..mb], el, cx, acc, &mut l)?;
    build_sub(d + 1, &a[ma..], &b[mb..], er, cx, acc, &mut r)?;
    parent_w(50, &l, &r, out);
    Ok(())
}

fn sub_node(r: &mut Rs, d: u32, ups: &[Up], cx: &Cx, acc: &mut Acc, old: &mut W, new: &mut W) -> Result<(), TErr> {
    if d > 256 { return Err(TErr::Witness); }
    match r.word()? {
        0 => { *old = ZERO; build_sub(d, ups, &[], None, cx, acc, new) }
        1 => { let x = r.hash()?; if !ups.is_empty() { return Err(TErr::Witness); } *old = *x; *new = *x; Ok(()) }
        2 => {
            let m = ups.partition_point(|u| !bit(&u.o, d));
            let (mut a0, mut a1, mut b0, mut b1) = (ZERO, ZERO, ZERO, ZERO);
            let (l0, l1): (&W, &W) = if m == 0 && r.peek() == 1 { r.word()?; let x = r.hash()?; (x, x) } else {
                sub_pass(r, d + 1, &ups[..m], cx, acc, &mut a0, &mut a1)?; (&a0, &a1) };
            let (r0, r1): (&W, &W) = if m == ups.len() && r.peek() == 1 { r.word()?; let x = r.hash()?; (x, x) } else {
                sub_pass(r, d + 1, &ups[m..], cx, acc, &mut b0, &mut b1)?; (&b0, &b1) };
            parent_w(50, l0, r0, old);
            parent_w(50, l1, r1, new);
            Ok(())
        }
        t @ (5 | 6) => one_sided(r, d, t, ups, |u| &u.o, old, new, &mut |r, d, o, n| sub_pass(r, d, ups, cx, acc, o, n)),
        4 => {
            let o0 = r.hash()?;
            let f = r.word()?;
            let sp = r.hash()?;
            let value = r.u64()?;
            let spent = f == 1;
            coin_into(o0, spent.then_some(sp), value, cx.counts, old);
            let (lo, hi) = if ups.is_empty() { (0, 0) } else if ups.len() == 1 && ups[0].o == *o0 { (0, 1) }
                else { (ups.partition_point(|u| u.o < *o0), ups.partition_point(|u| u.o <= *o0)) };
            let mut leaf = ZERO;
            let lref: &W = match &ups[lo..hi] {
                [] => old,
                [u] => match cx.mark(u) {
                    Some(m) if !spent => {
                        acc.mc += 1;
                        acc.mv = acc.mv.checked_add(value).ok_or(TErr::Count)?;
                        coin_into(o0, Some(m), value, cx.counts, &mut leaf);
                        &leaf
                    }
                    Some(_) => return Err(TErr::Missing),
                    None => return Err(TErr::Duplicate),
                },
                _ => return Err(TErr::Duplicate),
            };
            if lo == 0 && hi == ups.len() { *new = *lref; return Ok(()); }
            let lv = *lref;
            build_sub(d, &ups[..lo], &ups[hi..], Some((o0, &lv)), cx, acc, new)
        }
        _ => Err(TErr::Witness),
    }
}

/// New top subtree: new addresses (`a`, then `b`; inserts only) plus at most one existing address leaf (S, hash).
fn build_top(d: u32, a: &[Up], b: &[Up], extra: Option<(&W, &W)>, cx: &Cx, out: &mut W) -> Result<(), TErr> {
    let one = |x: &[Up]| x.is_empty() || x[0].s == x[x.len() - 1].s;
    let groups = (!a.is_empty()) as usize + (!b.is_empty()) as usize + extra.is_some() as usize;
    if groups == 0 { *out = ZERO; return Ok(()); }
    if groups == 1 && one(a) && one(b) {
        if let Some((_, h)) = extra { *out = *h; return Ok(()); }
        let g = if a.is_empty() { b } else { a };
        let mut acc = Acc::default();
        let mut sub = ZERO;
        build_sub(0, g, &[], None, cx, &mut acc, &mut sub)?;
        addr_into(&g[0].s, &sub, acc.pc, acc.pv, cx.counts, out);
        return Ok(());
    }
    if d >= 256 { return Err(TErr::Witness); }
    let (ma, mb) = (a.partition_point(|u| !bit(&u.s, d)), b.partition_point(|u| !bit(&u.s, d)));
    let (el, er) = match extra { Some(e) if bit(e.0, d) => (None, Some(e)), e => (e, None) };
    let (mut l, mut r) = (ZERO, ZERO);
    build_top(d + 1, &a[..ma], &b[..mb], el, cx, &mut l)?;
    build_top(d + 1, &a[ma..], &b[mb..], er, cx, &mut r)?;
    parent_w(50, &l, &r, out);
    Ok(())
}

fn top_node(r: &mut Rs, d: u32, ups: &[Up], cx: &Cx, old: &mut W, new: &mut W) -> Result<(), TErr> {
    if d > 256 { return Err(TErr::Witness); }
    match r.word()? {
        0 => { *old = ZERO; build_top(d, ups, &[], None, cx, new) }
        1 => { let x = r.hash()?; if !ups.is_empty() { return Err(TErr::Witness); } *old = *x; *new = *x; Ok(()) }
        2 => {
            let m = ups.partition_point(|u| !bit(&u.s, d));
            let (mut a0, mut a1, mut b0, mut b1) = (ZERO, ZERO, ZERO, ZERO);
            let (l0, l1): (&W, &W) = if m == 0 && r.peek() == 1 { r.word()?; let x = r.hash()?; (x, x) } else {
                top_pass(r, d + 1, &ups[..m], cx, &mut a0, &mut a1)?; (&a0, &a1) };
            let (r0, r1): (&W, &W) = if m == ups.len() && r.peek() == 1 { r.word()?; let x = r.hash()?; (x, x) } else {
                top_pass(r, d + 1, &ups[m..], cx, &mut b0, &mut b1)?; (&b0, &b1) };
            parent_w(50, l0, r0, old);
            parent_w(50, l1, r1, new);
            Ok(())
        }
        t @ (5 | 6) => one_sided(r, d, t, ups, |u| &u.s, old, new, &mut |r, d, o, n| top_pass(r, d, ups, cx, o, n)),
        3 => {
            let s0 = r.hash()?;
            let (count, value) = (r.u64()?, r.u64()?);
            // fast path: every update is this address's (the common case on a touched leaf)
            let (lo, hi) = if ups.is_empty() { (0, 0) } else if ups[0].s == *s0 && ups[ups.len() - 1].s == *s0 { (0, ups.len()) }
                else { (ups.partition_point(|u| u.s < *s0), ups.partition_point(|u| u.s <= *s0)) };
            let mut acc = Acc::default();
            let (mut so, mut sn) = (ZERO, ZERO);
            sub_pass(r, 0, &ups[lo..hi], cx, &mut acc, &mut so, &mut sn)?;
            addr_into(s0, &so, count, value, cx.counts, old);
            let mut leaf = ZERO;
            let lref: &W = if lo == hi { old } else {
                let c = count.checked_sub(acc.mc).and_then(|c| c.checked_add(acc.pc)).ok_or(TErr::Count)?;
                let v = value.checked_sub(acc.mv).and_then(|v| v.checked_add(acc.pv)).ok_or(TErr::Count)?;
                addr_into(s0, &sn, c, v, cx.counts, &mut leaf);
                &leaf
            };
            if lo == 0 && hi == ups.len() { *new = *lref; return Ok(()); }
            let lv = *lref;
            build_top(d, &ups[..lo], &ups[hi..], Some((s0, &lv)), cx, new)
        }
        _ => Err(TErr::Witness),
    }
}

/// The guest's update: the witness `w` (as `encode`) against its old root, with `marks` and `inserts` (as `apply`).
/// Returns the new root.
pub fn apply_stream(w: &[u32], marks: &[(Key, W)], inserts: &[(Key, Option<W>, u64)]) -> Result<W, TErr> {
    apply_stream_perm(w, marks, inserts, None)
}

/// `apply_stream` with the update order supplied (batch 3): `perm[j]` = the index (marks, then inserts) of the j-th
/// smallest key. The guest checks it is a permutation (each index once) and that the keys strictly increase in that
/// order — which also refuses a repeated key — instead of sorting. None: sort here.
pub fn apply_stream_perm(w: &[u32], marks: &[(Key, W)], inserts: &[(Key, Option<W>, u64)], perm: Option<&[u32]>)
                         -> Result<W, TErr> {
    let mut r = Rs { w, i: 0 };
    let counts = r.word()? == 1;
    let root = *r.hash()?;
    let keys: Vec<(W, W)> = marks.iter().map(|x| split(&x.0)).chain(inserts.iter().map(|x| split(&x.0))).collect();
    if keys.len() >= 1 << 31 { return Err(TErr::Witness); }
    let idx: Vec<u32> = match perm {
        Some(p) => {
            if p.len() != keys.len() { return Err(TErr::Witness); }
            let mut seen = alloc::vec![0u32; keys.len().div_ceil(32)];
            for &i in p {
                let i = i as usize;
                if i >= keys.len() || seen[i / 32] >> (i % 32) & 1 == 1 { return Err(TErr::Witness); }
                seen[i / 32] |= 1 << (i % 32);
            }
            if p.windows(2).any(|q| keys[q[0] as usize] >= keys[q[1] as usize]) { return Err(TErr::Duplicate); }
            p.to_vec()
        }
        None => {
            let mut idx: Vec<u32> = (0..keys.len() as u32).collect();
            idx.sort_unstable_by(|&x, &y| keys[x as usize].cmp(&keys[y as usize]));
            idx
        }
    };
    let nm = marks.len() as u32;
    let ups: Vec<Up> = idx.iter().map(|&i| {
        let k = &keys[i as usize];
        Up { s: k.0, o: k.1, k: if i < nm { i } else { (i - nm) | 1 << 31 } }
    }).collect();
    if perm.is_none() && ups.windows(2).any(|p| p[0].s == p[1].s && p[0].o == p[1].o) { return Err(TErr::Duplicate); }
    let cx = Cx { marks, ins: inserts, counts };
    let (mut old, mut new) = (ZERO, ZERO);
    top_pass(&mut r, 0, &ups, &cx, &mut old, &mut new)?;
    if r.i != w.len() { return Err(TErr::Witness); }
    if old != root { return Err(TErr::OldRoot); }
    Ok(new)
}

/// Split mode (E1 measurement): the digest of one block's update list, which the trie receipt commits and the aggregate
/// recomputes from the chunks' keys. SHA-256 over [marks count, inserts count] (u32 LE), each mark (key, spender words
/// LE), each insert (key, spent flag byte, spender words LE, value u64 LE).
pub fn list_digest(marks: &[(Key, W)], inserts: &[(Key, Option<W>, u64)]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    // One byte buffer, one hash. (A streamed version through a 1 KB stack buffer measured SLOWER, 11-04: +1.5 M on
    // 962,000, many small copies; reverted.)
    let mut b: Vec<u8> = Vec::with_capacity(8 + marks.len() * 96 + inserts.len() * 105);
    b.extend_from_slice(&(marks.len() as u32).to_le_bytes());
    b.extend_from_slice(&(inserts.len() as u32).to_le_bytes());
    for (k, sp) in marks { b.extend_from_slice(k); for x in sp { b.extend_from_slice(&x.to_le_bytes()); } }
    for (k, sp, v) in inserts {
        b.extend_from_slice(k);
        b.push(sp.is_some() as u8);
        for x in sp.unwrap_or([0; 8]) { b.extend_from_slice(&x.to_le_bytes()); }
        b.extend_from_slice(&v.to_le_bytes());
    }
    Sha256::digest(&b).into()
}
