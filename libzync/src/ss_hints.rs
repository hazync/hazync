//! SwiftSync design B: the hint bits and the hint hash a proof commits.
//!
//! A hints file (hintsfile v0, 2140-dev/swiftsync) lists, per height 1..=H, the indices of that
//! block's INDEXED outputs still unspent at H. Indexing: spendable outputs only (not OP_RETURN, not a
//! script over 10,000 bytes), coinbase first, then each transaction in vout order; the coinbase
//! outputs of the two BIP30-unspendable blocks (91,722 and 91,812 on mainnet) are not indexed.
//!
//! The guest turns a height's list into one bit per indexed output (`1` = unspent at H) and extends
//! a hash chain with it. At H the chain equals [`file_hash`] of the file ALONE, so a verifier needs
//! no blocks to check that a proof is about a given hints file:
//!
//! ```text
//! h_0 = 0^32
//! h_k = SHA-256("libzync/swiftsync-hints/v2" || h_{k-1} || k as u32 LE || n as u32 LE || i_1 .. i_n as u32 LE)
//! ```
//! with i_1 < .. < i_n the unspent indices at height k.
//!
//! Included by the guest via #[path]: `sha2` + `alloc` only.

use sha2::{Digest, Sha256};

pub const HINTS_DOMAIN: &[u8] = b"libzync/swiftsync-hints/v2";

/// One step of the chain from a sorted index list (what a hints file stores).
pub fn step(prev: &[u8; 32], height: u32, indices: &[u32]) -> [u8; 32] {
    let mut d = Sha256::new();
    d.update(HINTS_DOMAIN);
    d.update(prev);
    d.update(height.to_le_bytes());
    d.update((indices.len() as u32).to_le_bytes());
    for k in indices {
        d.update(k.to_le_bytes());
    }
    d.finalize().into()
}

/// The same step from hint bits (LSB-first LE words), without materialising the list. Equal to
/// [`step`] over [`indices`] of the same words.
pub fn step_bits(prev: &[u8; 32], height: u32, words: &[u32]) -> [u8; 32] {
    let mut d = Sha256::new();
    d.update(HINTS_DOMAIN);
    d.update(prev);
    d.update(height.to_le_bytes());
    let ones: u32 = words.iter().map(|x| x.count_ones()).sum();
    d.update(ones.to_le_bytes());
    for (wi, x) in words.iter().enumerate() {
        let mut v = *x;
        while v != 0 {
            d.update(((wi as u32) * 32 + v.trailing_zeros()).to_le_bytes());
            v &= v - 1;
        }
    }
    d.finalize().into()
}

/// The set bits as a sorted index list.
pub fn indices(words: &[u32]) -> alloc::vec::Vec<u32> {
    (0..words.len() * 32).filter(|k| (words[k / 32] >> (k % 32)) & 1 == 1).map(|k| k as u32).collect()
}

/// `nbits` indices packed into words: exactly ceil(nbits / 32) words and no bit at or past `nbits`.
/// A padding bit would be hashed by [`step_bits`] while naming no output, so a proof could match a
/// file that lists an index the block does not have.
pub fn bits_canonical(nbits: u32, words: &[u32]) -> bool {
    if words.len() != (nbits as usize).div_ceil(32) {
        return false;
    }
    match (nbits % 32, words.last()) {
        (0, _) | (_, None) => true,
        (r, Some(last)) => last >> r == 0,
    }
}

/// Pack a sorted index list into hint bits for a block with `nbits` indexed outputs.
/// None if an index is out of range.
pub fn bits_from_indices(nbits: u32, idx: &[u32]) -> Option<alloc::vec::Vec<u32>> {
    let mut w = alloc::vec![0u32; (nbits as usize).div_ceil(32)];
    for &k in idx {
        if k >= nbits {
            return None;
        }
        w[k as usize / 32] |= 1 << (k % 32);
    }
    Some(w)
}
