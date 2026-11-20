//! Coin liveness facts (docs/libzync/LIVENESS_ZAPP.md): ONE outpoint's state at a height H, checked against a verified
//! core `ChainState` and nothing else.
//!
//! A liveness proof names a coin by ALL the fields its coin-log leaf hashes (outpoint, amount, scriptPubKey, creation
//! height, coinbase flag, BIP30 flag, creation MTP; UTREEXO_FORMAT.md §2), and claims either
//!   - UNSPENT after block H: the coin's own leaf sits at its log position in the coin log after H, or
//!   - SPENT by input `vin` of the transaction at block position `tx` of block `h` (h ≤ H): the flipped leaf
//!     `spent(leaf, spender(h, tx, vin))` sits there (COINLOG.md §2).
//! For H below the proof's tip, the coin log after H is opened through the tip's block forest: H's block leaf commits
//! `utxo_state(leaves, value, roots)` (HANDOVER.md §3; with its undo hash from the commitment on, UNDO.md §3).
//!
//! Nothing here is new data: these are the checks of `facts.rs` (coin_unspent, coin_spent_by, block_in_chain) on one
//! strict binary encoding. Not guest-compiled; a zyncAPP guest may `#[path]`-include it beside coinlog.rs, facts.rs and
//! undo.rs.
//!
//! **One encoding per fact** (the #695 lesson: no witness word the check does not bind). Every byte is read, every
//! enumerated value is checked, the roots are exactly one per set bit of the leaf count, a path's length must be its
//! tree's height, and `decode` refuses any input whose re-encoding differs. The test `every_byte_is_bound` flips each
//! bit of a real fact.
//!
//! ⚠ What it does not show: that an outpoint is ABSENT (the log is in birth order, not keyed; COINLOG.md §6), and
//! anything about a coin created and spent in the same block (it never enters the log).

use crate::coinlog::{self, H};
use crate::facts;
use alloc::vec::Vec;

pub const MAGIC: [u8; 4] = *b"LZLV";
pub const VERSION: u8 = 1;
/// Core's MAX_SCRIPT_SIZE: a longer scriptPubKey is unspendable and never a leaf (UTREEXO_FORMAT.md §3).
pub const MAX_SCRIPT: usize = 10_000;
/// A path is at most a tree's height; a coin log or block forest has fewer than 2^63 leaves.
const MAX_PATH: usize = 63;

const F_COINBASE: u8 = 1;
const F_TRACKED: u8 = 2;
const F_OPENING: u8 = 4;
const F_UNDO: u8 = 8;

/// The coin: every field its leaf commits. `txid` in internal byte order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coin {
    pub txid: H,
    pub vout: u32,
    pub value: u64,
    pub script_pubkey: Vec<u8>,
    pub height: u32,
    pub coinbase: bool,
    pub tracked: bool,
    pub creation_mtp: u32,
}

impl Coin {
    pub fn leaf(&self) -> H {
        facts::coin_leaf(&self.txid, self.vout, self.value, &self.script_pubkey, self.height, self.coinbase, self.tracked, self.creation_mtp)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Unspent,
    /// spent by input `vin` of the transaction at block position `tx` (coinbase 0) of block `height`
    Spent { height: u32, tx: u32, vin: u32 },
}

/// The coin log after a height below the tip, and that block's leaf in the tip's block forest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opening {
    pub log_leaves: u64,
    pub log_value: u64,
    /// One root per set bit of `log_leaves`, lowest level first.
    pub log_roots: Vec<H>,
    /// The block at H, internal byte order.
    pub block_hash: H,
    /// Its undo hash, exactly when its leaf commits one (UNDO.md).
    pub undo: Option<H>,
    pub block_path: Vec<H>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proof {
    pub coin: Coin,
    pub state: State,
    /// H: the coin state is the one after this block.
    pub at: u32,
    /// The coin's position in the coin log (birth order), and its path there after H, bottom-up.
    pub pos: u64,
    pub path: Vec<H>,
    /// Present exactly when H is below the proof's tip.
    pub opening: Option<Opening>,
}

/// The fields of a VERIFIED core ChainState the check reads, with the verifier's `undo_from` for its image (UNDO.md).
pub struct Chain<'a> {
    pub height: u32,
    pub utxo_roots: &'a [Option<H>],
    pub utxo_leaves: u64,
    pub block_roots: &'a [Option<H>],
    pub block_leaves: u64,
    pub undo_from: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// not a well-formed, canonical liveness proof
    Decode(&'static str),
    /// H is above the proof's tip
    AboveTip,
    /// an opening at the tip, or none below it
    Opening,
    /// the undo hash given for a block whose leaf has none, or missing for one that has
    Undo,
    /// H's block, with that coin state, is not in the proof's block forest
    Block,
    /// the coin is not in the claimed state at that position in the coin log after H
    NotInLog,
}

impl Error {
    pub fn text(self) -> &'static str {
        match self {
            Error::Decode(m) => m,
            Error::AboveTip => "the height asked about is above the proof's tip",
            Error::Opening => "a fact at the tip carries no block opening; one below the tip needs it",
            Error::Undo => "the undo hash does not match the block leaf's format for that height (UNDO.md)",
            Error::Block => "that block, with that coin state, is not in the proof's block forest",
            Error::NotInLog => "the coin is not in the claimed state at that position of the coin log",
        }
    }
}

fn put_path(b: &mut Vec<u8>, p: &[H]) {
    b.push(p.len() as u8);
    for h in p {
        b.extend_from_slice(h);
    }
}

/// The one encoding of a proof (all integers little-endian):
/// ```text
/// "LZLV" ‖ version 1 ‖ state (0 unspent, 1 spent) ‖ flags (bit 0 coinbase, 1 tracked, 2 opening, 3 undo)
/// ‖ txid 32 ‖ vout u32 ‖ value u64 ‖ height u32 ‖ creation_mtp u32 ‖ script length u16 ‖ script
/// ‖ [spent: height u32 ‖ tx u32 ‖ vin u32] ‖ at u32 ‖ pos u64 ‖ path length u8 ‖ path 32 × length
/// ‖ [opening: log_leaves u64 ‖ log_value u64 ‖ roots 32 × popcount(log_leaves), lowest level first
///             ‖ block hash 32 ‖ [undo: undo hash 32] ‖ block path length u8 ‖ block path 32 × length]
/// ```
pub fn encode(p: &Proof) -> Vec<u8> {
    let c = &p.coin;
    let mut b = Vec::with_capacity(96 + c.script_pubkey.len() + 32 * p.path.len());
    b.extend_from_slice(&MAGIC);
    b.push(VERSION);
    b.push(match p.state { State::Unspent => 0, State::Spent { .. } => 1 });
    let mut f = 0u8;
    if c.coinbase { f |= F_COINBASE; }
    if c.tracked { f |= F_TRACKED; }
    if let Some(o) = &p.opening {
        f |= F_OPENING;
        if o.undo.is_some() { f |= F_UNDO; }
    }
    b.push(f);
    b.extend_from_slice(&c.txid);
    b.extend_from_slice(&c.vout.to_le_bytes());
    b.extend_from_slice(&c.value.to_le_bytes());
    b.extend_from_slice(&c.height.to_le_bytes());
    b.extend_from_slice(&c.creation_mtp.to_le_bytes());
    b.extend_from_slice(&(c.script_pubkey.len() as u16).to_le_bytes());
    b.extend_from_slice(&c.script_pubkey);
    if let State::Spent { height, tx, vin } = p.state {
        b.extend_from_slice(&height.to_le_bytes());
        b.extend_from_slice(&tx.to_le_bytes());
        b.extend_from_slice(&vin.to_le_bytes());
    }
    b.extend_from_slice(&p.at.to_le_bytes());
    b.extend_from_slice(&p.pos.to_le_bytes());
    put_path(&mut b, &p.path);
    if let Some(o) = &p.opening {
        b.extend_from_slice(&o.log_leaves.to_le_bytes());
        b.extend_from_slice(&o.log_value.to_le_bytes());
        for r in &o.log_roots {
            b.extend_from_slice(r);
        }
        b.extend_from_slice(&o.block_hash);
        if let Some(u) = &o.undo {
            b.extend_from_slice(u);
        }
        put_path(&mut b, &o.block_path);
    }
    b
}

struct Rd<'a>(&'a [u8]);

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        if self.0.len() < n {
            return Err(Error::Decode("truncated"));
        }
        let (h, t) = self.0.split_at(n);
        self.0 = t;
        Ok(h)
    }
    fn u8(&mut self) -> Result<u8, Error> { Ok(self.take(1)?[0]) }
    fn u16(&mut self) -> Result<u16, Error> { Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap())) }
    fn u32(&mut self) -> Result<u32, Error> { Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap())) }
    fn u64(&mut self) -> Result<u64, Error> { Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap())) }
    fn h(&mut self) -> Result<H, Error> { Ok(self.take(32)?.try_into().unwrap()) }
    fn path(&mut self) -> Result<Vec<H>, Error> {
        let n = self.u8()? as usize;
        if n > MAX_PATH {
            return Err(Error::Decode("path longer than any tree"));
        }
        (0..n).map(|_| self.h()).collect()
    }
}

/// Strict decode: every byte read, every flag and enumerated value checked, nothing after the end, and the
/// re-encoding equal to the input.
pub fn decode(b: &[u8]) -> Result<Proof, Error> {
    let mut r = Rd(b);
    if r.take(4)? != MAGIC {
        return Err(Error::Decode("not a liveness proof (magic)"));
    }
    if r.u8()? != VERSION {
        return Err(Error::Decode("unknown liveness proof version"));
    }
    let spent = match r.u8()? {
        0 => false,
        1 => true,
        _ => return Err(Error::Decode("state is neither 0 (unspent) nor 1 (spent)")),
    };
    let f = r.u8()?;
    if f & !(F_COINBASE | F_TRACKED | F_OPENING | F_UNDO) != 0 || (f & F_UNDO != 0 && f & F_OPENING == 0) {
        return Err(Error::Decode("unknown flag bits"));
    }
    let txid = r.h()?;
    let vout = r.u32()?;
    let value = r.u64()?;
    let height = r.u32()?;
    let creation_mtp = r.u32()?;
    let sl = r.u16()? as usize;
    if sl > MAX_SCRIPT {
        return Err(Error::Decode("scriptPubKey over 10,000 bytes (unspendable, never a coin)"));
    }
    let script_pubkey = r.take(sl)?.to_vec();
    let state = if spent { State::Spent { height: r.u32()?, tx: r.u32()?, vin: r.u32()? } } else { State::Unspent };
    let at = r.u32()?;
    let pos = r.u64()?;
    let path = r.path()?;
    let opening = if f & F_OPENING != 0 {
        let log_leaves = r.u64()?;
        let log_value = r.u64()?;
        let log_roots = (0..log_leaves.count_ones()).map(|_| r.h()).collect::<Result<Vec<_>, _>>()?;
        let block_hash = r.h()?;
        let undo = if f & F_UNDO != 0 { Some(r.h()?) } else { None };
        let block_path = r.path()?;
        Some(Opening { log_leaves, log_value, log_roots, block_hash, undo, block_path })
    } else {
        None
    };
    if !r.0.is_empty() {
        return Err(Error::Decode("bytes after the end of the proof"));
    }
    let p = Proof {
        coin: Coin { txid, vout, value, script_pubkey, height, coinbase: f & F_COINBASE != 0, tracked: f & F_TRACKED != 0, creation_mtp },
        state, at, pos, path, opening,
    };
    // belt and braces: the format above has one encoding per value, and this says so
    if encode(&p) != b {
        return Err(Error::Decode("not the canonical encoding"));
    }
    Ok(p)
}

/// The roots as `facts` takes them (index = level) from one per set bit of `n`, lowest level first.
pub fn roots_by_level(n: u64, roots: &[H]) -> Option<Vec<Option<H>>> {
    if roots.len() != n.count_ones() as usize {
        return None;
    }
    let top = 64 - n.leading_zeros() as usize;
    let mut out: Vec<Option<H>> = (0..top).map(|_| None).collect();
    let mut it = roots.iter();
    for (l, slot) in out.iter_mut().enumerate() {
        if (n >> l) & 1 == 1 {
            *slot = Some(*it.next()?);
        }
    }
    Some(out)
}

/// The inverse of [`roots_by_level`] (for a producer): the roots present, lowest level first.
pub fn roots_present(roots: &[Option<H>]) -> Vec<H> {
    roots.iter().flatten().copied().collect()
}

/// Checks a decoded proof against a verified chain. Ok: the coin is in `p.state` after block `p.at` of that chain.
pub fn check(p: &Proof, c: &Chain) -> Result<(), Error> {
    if p.at > c.height {
        return Err(Error::AboveTip);
    }
    let opened;
    let (roots, n): (&[Option<H>], u64) = match (&p.opening, p.at == c.height) {
        (None, true) => (c.utxo_roots, c.utxo_leaves),
        (Some(o), false) => {
            if (p.at >= c.undo_from) != o.undo.is_some() {
                return Err(Error::Undo);
            }
            opened = roots_by_level(o.log_leaves, &o.log_roots).ok_or(Error::Decode("roots do not match the leaf count"))?;
            let state = facts::utxo_state(o.log_leaves, o.log_value, &opened);
            if !crate::undo::block_in_chain_any(c.block_roots, c.block_leaves, &o.block_hash, p.at, &state, o.undo.as_ref(), &o.block_path) {
                return Err(Error::Block);
            }
            (&opened, o.log_leaves)
        }
        _ => return Err(Error::Opening),
    };
    let leaf = p.coin.leaf();
    let ok = match p.state {
        State::Unspent => facts::coin_unspent(roots, n, p.pos, &leaf, &p.path),
        State::Spent { height, tx, vin } => height <= p.at && facts::coin_spent_by(roots, n, p.pos, &leaf, height, tx, vin, &p.path),
    };
    if ok { Ok(()) } else { Err(Error::NotInLog) }
}

/// Decode and check. Ok: the proof, every field of which the chain now vouches for.
pub fn verify(b: &[u8], c: &Chain) -> Result<Proof, Error> {
    let p = decode(b)?;
    check(&p, c)?;
    Ok(p)
}

/// The spent leaf a [`State::Spent`] proof opens (for a producer's self-check).
pub fn state_leaf(coin: &Coin, s: State) -> H {
    match s {
        State::Unspent => coin.leaf(),
        State::Spent { height, tx, vin } => coinlog::spent(&coin.leaf(), &coinlog::spender(height, tx, vin)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coinlog_forest::Store;

    fn coin(i: u32) -> Coin {
        Coin { txid: [i as u8; 32], vout: i, value: 1000 + i as u64, script_pubkey: alloc::vec![0x51, i as u8], height: 1 + i,
               coinbase: i % 3 == 0, tracked: false, creation_mtp: 77 }
    }

    /// A log of 13 coins, coin 5 spent by (9, 2, 1) at height 9: a chain whose tip is height 9 with no opening.
    fn tip_chain() -> (Store, Vec<Coin>) {
        let mut s = Store::default();
        let cs: Vec<Coin> = (0..13).map(coin).collect();
        for c in &cs {
            s.push(c.leaf());
        }
        s.set(5, state_leaf(&cs[5], State::Spent { height: 9, tx: 2, vin: 1 }));
        (s, cs)
    }

    #[test]
    fn tip_facts_and_refusals() {
        let (s, cs) = tip_chain();
        let roots = s.roots();
        let c = Chain { height: 9, utxo_roots: &roots, utxo_leaves: s.len(), block_roots: &[], block_leaves: 0, undo_from: u32::MAX };
        for p in 0..13u64 {
            let st = if p == 5 { State::Spent { height: 9, tx: 2, vin: 1 } } else { State::Unspent };
            let pr = Proof { coin: cs[p as usize].clone(), state: st, at: 9, pos: p, path: s.path(p), opening: None };
            let b = encode(&pr);
            assert_eq!(verify(&b, &c), Ok(pr.clone()), "{p}");
            // the opposite claim is refused
            let flip = Proof { state: if p == 5 { State::Unspent } else { State::Spent { height: 9, tx: 2, vin: 1 } }, ..pr.clone() };
            assert_eq!(check(&flip, &c), Err(Error::NotInLog));
            // a wrong amount, script, creation height or flag is a different leaf
            let mut w = pr.clone(); w.coin.value += 1; assert_eq!(check(&w, &c), Err(Error::NotInLog));
            let mut w = pr.clone(); w.coin.script_pubkey.push(0); assert_eq!(check(&w, &c), Err(Error::NotInLog));
            let mut w = pr.clone(); w.coin.height += 1; assert_eq!(check(&w, &c), Err(Error::NotInLog));
            let mut w = pr.clone(); w.coin.tracked = true; assert_eq!(check(&w, &c), Err(Error::NotInLog));
            // above the tip; an opening at the tip
            let mut w = pr.clone(); w.at = 10; assert_eq!(check(&w, &c), Err(Error::AboveTip));
            let w = Proof { opening: Some(Opening { log_leaves: 0, log_value: 0, log_roots: alloc::vec![], block_hash: [0; 32], undo: None, block_path: alloc::vec![] }), ..pr.clone() };
            assert_eq!(check(&w, &c), Err(Error::Opening));
        }
        // a spent coin with the wrong spender, or a spender above H
        let base = Proof { coin: cs[5].clone(), state: State::Spent { height: 9, tx: 2, vin: 0 }, at: 9, pos: 5, path: s.path(5), opening: None };
        assert_eq!(check(&base, &c), Err(Error::NotInLog));
        // an outpoint that never existed
        let ghost = Proof { coin: coin(99), state: State::Unspent, at: 9, pos: 3, path: s.path(3), opening: None };
        assert_eq!(check(&ghost, &c), Err(Error::NotInLog));
    }

    #[test]
    fn decode_is_strict() {
        let (s, cs) = tip_chain();
        let pr = Proof { coin: cs[5].clone(), state: State::Spent { height: 9, tx: 2, vin: 1 }, at: 9, pos: 5, path: s.path(5),
                         opening: Some(Opening { log_leaves: 5, log_value: 3, log_roots: alloc::vec![[1; 32], [2; 32]], block_hash: [3; 32],
                                                 undo: Some([4; 32]), block_path: alloc::vec![[5; 32]] }) };
        let b = encode(&pr);
        assert_eq!(decode(&b), Ok(pr));
        for n in 0..b.len() {
            assert!(decode(&b[..n]).is_err(), "truncated at {n}");
        }
        let mut long = b.clone();
        long.push(0);
        assert!(decode(&long).is_err());
        for (i, v) in [(4usize, 2u8), (5, 2), (6, 0x10), (6, F_UNDO)] {
            let mut x = b.clone();
            x[i] = v;
            assert!(decode(&x).is_err(), "byte {i} = {v}");
        }
        assert_eq!(roots_by_level(5, &[[1; 32]]), None);
        assert_eq!(roots_by_level(5, &[[1; 32], [2; 32]]), Some(alloc::vec![Some([1; 32]), None, Some([2; 32])]));
    }
}
