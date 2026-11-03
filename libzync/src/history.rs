//! The host's chain history (`history.bin` in a proving run's directory): enough to rebuild the coin log at ANY proved
//! height and the block forest at the tip, so a path service can answer every zyncAPP fact (facts.rs). Not
//! guest-compiled.
//!
//! One record per height, contiguous from 0, appended after that height's receipt is written:
//!
//! ```text
//! height u32 ‖ block hash 32 (internal order) ‖ live value u64 ‖ n_flips u32 ‖ n_appends u32
//!   ‖ (position u64 ‖ tx u32 ‖ vin u32) × n_flips ‖ leaf 32 × n_appends       all integers little-endian
//! ```
//!
//! A flip is a spend of the coin at `position` by input `vin` of the transaction at block position `tx`: the new leaf
//! is `spent(original leaf, spender(height, tx, vin))` (COINLOG.md §2).
//! ```text
//! ```
//!
//! Height 0 is the genesis block with no flips and no appends (its outputs are unspendable). Replaying records 0..=h
//! gives the coin log after h; the block leaf of h is `block_leaf(hash, h, utxo_state(n, value, roots))` (HANDOVER.md §3).
//! Mainnet's history is ~100 GB (every leaf ever, plus each flip): C1's disk-backed store will page it.

use crate::coinlog::{Store, H};
use crate::facts::{block_leaf, utxo_state};
use alloc::vec::Vec;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rec {
    pub height: u32,
    pub hash: H,
    pub value: u64,
    pub flips: Vec<(u64, u32, u32)>,
    pub appends: Vec<H>,
}

impl Rec {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(52 + 16 * self.flips.len() + 32 * self.appends.len());
        b.extend_from_slice(&self.height.to_le_bytes());
        b.extend_from_slice(&self.hash);
        b.extend_from_slice(&self.value.to_le_bytes());
        b.extend_from_slice(&(self.flips.len() as u32).to_le_bytes());
        b.extend_from_slice(&(self.appends.len() as u32).to_le_bytes());
        for (p, t, v) in &self.flips {
            b.extend_from_slice(&p.to_le_bytes());
            b.extend_from_slice(&t.to_le_bytes());
            b.extend_from_slice(&v.to_le_bytes());
        }
        for l in &self.appends {
            b.extend_from_slice(l);
        }
        b
    }
}

fn take<'a>(b: &mut &'a [u8], n: usize) -> Result<&'a [u8], &'static str> {
    if b.len() < n {
        return Err("history: truncated record");
    }
    let (h, t) = b.split_at(n);
    *b = t;
    Ok(h)
}

/// The longest prefix of whole, contiguous records, and its length in bytes: a torn last append (a crash mid-write) is
/// cut off here and the file truncated to it, instead of disabling history for good (review 11-03).
pub fn decode_prefix(b: &[u8]) -> (Vec<Rec>, usize) {
    let mut out = Vec::new();
    let mut good = 0;
    while good < b.len() {
        match decode_all_one(&b[good..], out.len() as u32) {
            Some((r, n)) => { out.push(r); good += n; }
            None => break,
        }
    }
    (out, good)
}

fn decode_all_one(b: &[u8], want: u32) -> Option<(Rec, usize)> {
    let mut c = b;
    let height = u32::from_le_bytes(take(&mut c, 4).ok()?.try_into().unwrap());
    if height != want {
        return None;
    }
    let hash: H = take(&mut c, 32).ok()?.try_into().unwrap();
    let value = u64::from_le_bytes(take(&mut c, 8).ok()?.try_into().unwrap());
    let nf = u32::from_le_bytes(take(&mut c, 4).ok()?.try_into().unwrap()) as usize;
    let na = u32::from_le_bytes(take(&mut c, 4).ok()?.try_into().unwrap()) as usize;
    let mut flips = Vec::with_capacity(nf.min(1 << 20));
    for _ in 0..nf {
        let p = u64::from_le_bytes(take(&mut c, 8).ok()?.try_into().unwrap());
        let t = u32::from_le_bytes(take(&mut c, 4).ok()?.try_into().unwrap());
        let v = u32::from_le_bytes(take(&mut c, 4).ok()?.try_into().unwrap());
        flips.push((p, t, v));
    }
    let mut appends = Vec::with_capacity(na.min(1 << 20));
    for _ in 0..na {
        appends.push(take(&mut c, 32).ok()?.try_into().unwrap());
    }
    Some((Rec { height, hash, value, flips, appends }, b.len() - c.len()))
}

/// Every record, checked contiguous from height 0.
pub fn decode_all(mut b: &[u8]) -> Result<Vec<Rec>, &'static str> {
    let mut out: Vec<Rec> = Vec::new();
    while !b.is_empty() {
        let height = u32::from_le_bytes(take(&mut b, 4)?.try_into().unwrap());
        let hash: H = take(&mut b, 32)?.try_into().unwrap();
        let value = u64::from_le_bytes(take(&mut b, 8)?.try_into().unwrap());
        let nf = u32::from_le_bytes(take(&mut b, 4)?.try_into().unwrap()) as usize;
        let na = u32::from_le_bytes(take(&mut b, 4)?.try_into().unwrap()) as usize;
        let mut flips = Vec::with_capacity(nf.min(1 << 20));
        for _ in 0..nf {
            let p = u64::from_le_bytes(take(&mut b, 8)?.try_into().unwrap());
            let t = u32::from_le_bytes(take(&mut b, 4)?.try_into().unwrap());
            let v = u32::from_le_bytes(take(&mut b, 4)?.try_into().unwrap());
            flips.push((p, t, v));
        }
        let mut appends = Vec::with_capacity(na.min(1 << 20));
        for _ in 0..na {
            appends.push(take(&mut b, 32)?.try_into().unwrap());
        }
        if height as usize != out.len() {
            return Err("history: heights are not contiguous from 0");
        }
        out.push(Rec { height, hash, value, flips, appends });
    }
    Ok(out)
}

/// The chain rebuilt from records 0..=tip: the coin log after `at` (≤ tip), its live value, every coin's original
/// (unspent) leaf, who had spent each spent coin by `at`, and every block leaf 0..=tip (the block forest's leaves,
/// position = height).
pub struct Replay {
    pub log: Store,
    pub value: u64,
    pub coins: Vec<H>,
    pub spenders: alloc::collections::BTreeMap<u64, (u32, u32, u32)>,
    pub block_leaves: Vec<H>,
    /// The coin-state digest after each height 0..=tip (what each block leaf commits).
    pub states: Vec<H>,
}

pub fn replay(recs: &[Rec], at: u32, tip: u32) -> Result<Replay, &'static str> {
    if at > tip || tip as usize >= recs.len() {
        return Err("history: height not proved yet");
    }
    let mut s = Store::default();
    let mut coins: Vec<H> = Vec::new();
    let mut by: alloc::collections::BTreeMap<u64, (u32, u32, u32)> = alloc::collections::BTreeMap::new();
    let mut keep: Option<(Store, u64, alloc::collections::BTreeMap<u64, (u32, u32, u32)>)> = None;
    let mut leaves = Vec::with_capacity(tip as usize + 1);
    let mut states = Vec::with_capacity(tip as usize + 1);
    for r in &recs[..=tip as usize] {
        for (p, t, v) in &r.flips {
            if *p >= s.len() || by.contains_key(p) {
                return Err("history: a flip past the end of the log, or of a spent coin");
            }
            s.set(*p, crate::coinlog::spent(&coins[*p as usize], &crate::coinlog::spender(r.height, *t, *v)));
            by.insert(*p, (r.height, *t, *v));
        }
        for l in &r.appends {
            s.push(*l);
            coins.push(*l);
        }
        let roots = s.roots();
        let st = utxo_state(s.len(), r.value, &roots);
        leaves.push(block_leaf(&r.hash, r.height, &st));
        states.push(st);
        if r.height == at {
            keep = Some((s.clone(), r.value, by.clone()));
        }
    }
    let (log, value, spenders) = keep.expect("at <= tip");
    coins.truncate(log.len() as usize);
    Ok(Replay { log, value, coins, spenders, block_leaves: leaves, states })
}

/// A Merkle path in the block forest of `leaves` (bottom-up), with the forest's roots.
pub fn block_path(leaves: &[H], pos: u64) -> (Vec<H>, Vec<Option<H>>) {
    let n = leaves.len() as u64;
    let mut levels: Vec<Vec<H>> = alloc::vec![leaves.to_vec()];
    while levels.last().unwrap().len() > 1 {
        let next = levels.last().unwrap().chunks_exact(2).map(|c| crate::facts::block_parent(0, &c[0], &c[1])).collect();
        levels.push(next);
    }
    let mut roots: Vec<Option<H>> = (0..64).map(|_| None).collect();
    let (mut start, mut lvl) = (0u64, None);
    for l in (0..64u32).rev() {
        if (n >> l) & 1 == 1 {
            roots[l as usize] = Some(levels[l as usize][(start >> l) as usize]);
            if lvl.is_none() && pos < start + (1u64 << l) {
                lvl = Some(l);
            }
            start += 1u64 << l;
        }
    }
    while roots.last() == Some(&None) {
        roots.pop();
    }
    let path = (0..lvl.unwrap_or(0) as usize).map(|j| levels[j][((pos >> j) ^ 1) as usize]).collect();
    (path, roots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::{block_in_chain, coin_spent_by, coin_unspent};
    use sha2::{Digest, Sha256};

    fn h(i: u64) -> H {
        Sha256::digest(i.to_le_bytes()).into()
    }

    #[test]
    fn replay_answers_facts() {
        // 0: genesis; 1: appends 3; 2: spends pos 1 (tx 1, vin 0), appends 2; 3: spends pos 0, appends 1
        let mut recs = alloc::vec![Rec { height: 0, hash: h(1000), value: 0, flips: alloc::vec![], appends: alloc::vec![] }];
        recs.push(Rec { height: 1, hash: h(1001), value: 30, flips: alloc::vec![], appends: alloc::vec![h(0), h(1), h(2)] });
        recs.push(Rec { height: 2, hash: h(1002), value: 40, flips: alloc::vec![(1, 1, 0)], appends: alloc::vec![h(3), h(4)] });
        recs.push(Rec { height: 3, hash: h(1003), value: 35, flips: alloc::vec![(0, 1, 2)], appends: alloc::vec![h(5)] });
        let bytes: Vec<u8> = recs.iter().flat_map(|r| r.encode()).collect();
        let back = decode_all(&bytes).unwrap();
        assert_eq!(back, recs);
        assert!(decode_all(&bytes[..bytes.len() - 1]).is_err());
        assert!(decode_all(&bytes[recs[0].encode().len()..]).is_err(), "must start at 0");
        let (p, n) = decode_prefix(&bytes[..bytes.len() - 3]);
        assert_eq!((p.len(), n), (3, bytes.len() - recs[3].encode().len()), "a torn last record is cut off");

        let tip = replay(&back, 3, 3).unwrap();
        let (roots, n) = (tip.log.roots(), tip.log.len());
        assert!(coin_spent_by(&roots, n, 1, &h(1), 2, 1, 0, &tip.log.path(1)));
        assert!(coin_spent_by(&roots, n, 0, &h(0), 3, 1, 2, &tip.log.path(0)));
        assert!(coin_unspent(&roots, n, 5, &h(5), &tip.log.path(5)));
        assert_eq!(tip.spenders.get(&1), Some(&(2, 1, 0)));
        assert_eq!(tip.coins[0], h(0));
        let mut dbl = back.clone();
        dbl[3].flips.push((1, 2, 0));
        assert!(replay(&dbl, 3, 3).is_err(), "a second spend of the same coin");

        // coin 0 was unspent at height 2: block 2's leaf in the tip's forest, then coin 0 against block 2's roots
        let at2 = replay(&back, 2, 3).unwrap();
        let (r2, n2) = (at2.log.roots(), at2.log.len());
        assert!(coin_unspent(&r2, n2, 0, &h(0), &at2.log.path(0)));
        let st2 = utxo_state(n2, at2.value, &r2);
        let (bp, broots) = block_path(&tip.block_leaves, 2);
        assert!(block_in_chain(&broots, 4, &h(1002), 2, &st2, &bp));
        assert!(!block_in_chain(&broots, 4, &h(1002), 2, &utxo_state(n, tip.value, &roots), &bp));
    }
}
