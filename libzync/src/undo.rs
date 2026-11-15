//! The per-block undo-data commitment (format U0, #729; docs/libzync/UNDO.md), host and verifier side. Not
//! guest-compiled: the guest's own definition is `prover/methods/guest/src/handover_pin.rs` (`undo_record_head`,
//! `block_leaf_undo_preimage`) and `agg_lite.rs`; this file is written from the document, and the verifier's tests
//! check the two agree.
//!
//! - one spent coin's record: `code u32 LE (= creation height * 2 + coinbase) ‖ amount u64 LE ‖ len(spk) u32 LE ‖ spk`;
//! - its digest: SHA-256(record);
//! - the block's undo hash: SHA-256(d_1 ‖ … ‖ d_n) over every input of every non-coinbase transaction, in block order
//!   (SHA-256 of nothing for a block without one);
//! - the block leaf that commits it: `SHA-256(0x05 ‖ block hash ‖ height u32 LE ‖ coin state ‖ undo hash)`, for every
//!   block proved by an image that has the commitment (block 0 and blocks proved by an earlier image keep the 0x02
//!   leaf, `facts::block_leaf`).
//!
//! Core's undo data (`CBlockUndo`: one `CTxUndo` per non-coinbase transaction, one `Coin` per input, in order) holds
//! exactly these fields, so a node computes a block's undo hash from its own `rev*.dat` and checks it against a proof.

use crate::coinlog::H;
use crate::facts::{block_leaf, block_parent};
use sha2::{Digest, Sha256};

pub const TAG_BLOCK_LEAF_UNDO: u8 = 0x05;

/// One spent coin's undo-record digest. `coinbase` is the coin's coinbase flag only.
pub fn undo_digest(height: u32, coinbase: bool, value: u64, spk: &[u8]) -> H {
    let mut h = Sha256::new();
    h.update((height.wrapping_mul(2) | coinbase as u32).to_le_bytes());
    h.update(value.to_le_bytes());
    h.update((spk.len() as u32).to_le_bytes());
    h.update(spk);
    h.finalize().into()
}

/// Accumulates a block's undo hash, one spent coin at a time, in block order.
#[derive(Clone, Default)]
pub struct UndoHasher { h: Sha256, n: u64 }

impl UndoHasher {
    pub fn new() -> Self { Self::default() }
    pub fn coin(&mut self, height: u32, coinbase: bool, value: u64, spk: &[u8]) {
        self.h.update(undo_digest(height, coinbase, value, spk));
        self.n += 1;
    }
    /// The number of coins so far.
    pub fn inputs(&self) -> u64 { self.n }
    pub fn finish(self) -> H { self.h.finalize().into() }
}

/// The block leaf with the undo hash (`hash` in internal byte order, `state` = `facts::utxo_state` after the block).
pub fn block_leaf_undo(hash: &H, height: u32, state: &H, undo: &H) -> H {
    let mut h = Sha256::new();
    h.update([TAG_BLOCK_LEAF_UNDO]);
    h.update(hash);
    h.update(height.to_le_bytes());
    h.update(state);
    h.update(undo);
    h.finalize().into()
}

/// The block's leaf in either format: with `undo`, the 0x05 leaf; without, the 0x02 leaf. Which one a block has is
/// fixed by the image that proved it (the verifier's `leaf_has_undo`); a caller passes `undo` exactly for those blocks.
pub fn block_leaf_any(hash: &H, height: u32, state: &H, undo: Option<&H>) -> H {
    match undo {
        Some(u) => block_leaf_undo(hash, height, state, u),
        None => block_leaf(hash, height, state),
    }
}

/// `facts::block_in_chain` for a leaf in either format: the block with `hash` at `height`, coin state `state` after
/// it and (for a block whose leaf commits one) undo hash `undo`, is in the chain whose block forest is
/// (`block_roots`, `block_leaves`). The two formats differ in tag and length, so no opening in one can match a leaf of
/// the other.
pub fn block_in_chain_any(block_roots: &[Option<H>], block_leaves: u64, hash: &H, height: u32, state: &H, undo: Option<&H>,
                          sib: &[H]) -> bool {
    let n = block_leaves;
    let pos = height as u64;
    // as facts::member: the roots present must be exactly the set bits of n, the path exactly the tree's height
    if block_roots.len() > 64 || (0..64).any(|l| block_roots.get(l).copied().flatten().is_some() != ((n >> l) & 1 == 1)) {
        return false;
    }
    let mut start = 0u64;
    let mut tree = None;
    for l in (0..64u32).rev() {
        if (n >> l) & 1 == 1 {
            let end = start + (1u64 << l);
            if pos < end { tree = Some(l); break; }
            start = end;
        }
    }
    let Some(l) = tree else { return false };
    if sib.len() != l as usize {
        return false;
    }
    let mut node = block_leaf_any(hash, height, state, undo);
    for (j, s) in sib.iter().enumerate() {
        node = if (pos >> j) & 1 == 0 { block_parent(j as u32 + 1, &node, s) } else { block_parent(j as u32 + 1, s, &node) };
    }
    block_roots.get(l as usize).copied().flatten() == Some(node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::block_in_chain;
    use alloc::vec::Vec;

    fn h(i: u64) -> H { Sha256::digest(i.to_le_bytes()).into() }

    #[test]
    fn undo_hash_of_nothing_is_sha256_of_nothing() {
        let e: H = Sha256::digest([]).into();
        assert_eq!(UndoHasher::new().finish(), e);
    }

    #[test]
    fn undo_digest_is_the_record() {
        // record: code 7*2+1 = 15, amount 5000000000, len 2, spk 51 ac
        let rec = [15u8, 0, 0, 0, 0x00, 0xf2, 0x05, 0x2a, 1, 0, 0, 0, 2, 0, 0, 0, 0x51, 0xac];
        let want: H = Sha256::digest(rec).into();
        assert_eq!(undo_digest(7, true, 5_000_000_000, &[0x51, 0xac]), want);
        let mut u = UndoHasher::new();
        u.coin(7, true, 5_000_000_000, &[0x51, 0xac]);
        assert_eq!(u.inputs(), 1);
        assert_eq!(u.finish(), <[u8; 32]>::from(Sha256::digest(want)));
    }

    /// A forest of 11 blocks, 0..3 with the 0x02 leaf (a pre-undo image's), 4..10 with the 0x05 leaf: each opens only in
    /// its own format and only with its own undo hash.
    #[test]
    fn both_leaf_formats_across_a_handover_height() {
        let from = 4u32;
        let leaves: Vec<H> = (0..11u32).map(|i| {
            let (b, s, u) = (h(i as u64), h(100 + i as u64), h(200 + i as u64));
            if i < from { block_leaf(&b, i, &s) } else { block_leaf_undo(&b, i, &s, &u) }
        }).collect();
        let (_, roots) = crate::history::block_path(&leaves, 0);
        for p in 0..11u64 {
            let (path, _) = crate::history::block_path(&leaves, p);
            let (b, s, u) = (h(p), h(100 + p), h(200 + p));
            let i = p as u32;
            if i < from {
                assert!(block_in_chain_any(&roots, 11, &b, i, &s, None, &path), "{p}");
                assert!(block_in_chain(&roots, 11, &b, i, &s, &path), "{p}: facts::block_in_chain agrees");
                assert!(!block_in_chain_any(&roots, 11, &b, i, &s, Some(&u), &path), "{p}: an undo hash on a 0x02 leaf");
            } else {
                assert!(block_in_chain_any(&roots, 11, &b, i, &s, Some(&u), &path), "{p}");
                assert!(!block_in_chain_any(&roots, 11, &b, i, &s, None, &path), "{p}: a 0x05 leaf opened as 0x02");
                assert!(!block_in_chain_any(&roots, 11, &b, i, &s, Some(&h(999)), &path), "{p}: a wrong undo hash");
                assert!(!block_in_chain_any(&roots, 11, &b, i, &h(999), Some(&u), &path), "{p}: a wrong state");
            }
        }
    }
}
