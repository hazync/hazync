// The Merkle Mountain Range (MMR) of every block hash the header-chain zyncAPP has checked. ONE source: the guest
// compiles it as `mod mmr`, swiftsync-verify `include!`s this file (src/lib.rs, `pub mod mmr`) and the zyncapp host
// uses swiftsync-verify's copy, so the guest, the verifier and the native path builder cannot hash differently.
// (No inner `//!` doc comments here: the file is `include!`d.)
//
// Leaves are the block hashes in height order (block h is leaf h; leaf count = height of the last header + 1).
//   leaf(block)  = SHA256(0x00 || block hash, internal byte order)
//   node(l, r)   = SHA256(0x01 || l || r)
//   bag(p, acc)  = SHA256(0x02 || p || acc)
// The three one-byte prefixes keep a leaf, an inner node and a bagged root apart: none can be presented as another.
//
// The MMR of n leaves is the list of PEAKS, left to right, largest first: one perfect binary tree per set bit of n,
// of 2^k leaves for bit k, from the highest bit down. So the number of peaks is popcount(n) and each peak's size is
// fixed by n alone. Appending a leaf pushes it as a one-leaf peak, then merges the two rightmost peaks while they are
// the same size; that happens once per trailing 1 bit of the leaf count BEFORE the append.
//
// root = the peaks bagged right to left: acc = the last peak, then acc = bag(peak, acc) for each peak before it.
// The root of no peaks is 32 zero bytes. The root does not commit to n on its own: pair it with the height.
//
// An inclusion path for leaf i is the sibling hashes from the leaf up to its peak (lowest first); its length is
// that peak's k, fixed by i and n. A path that reaches a hash that is not the committed peak for i is refused.
use alloc::vec::Vec;
use sha2::{Digest, Sha256};

pub const LEAF_TAG: u8 = 0x00;
pub const NODE_TAG: u8 = 0x01;
pub const BAG_TAG: u8 = 0x02;

fn tagged(tag: u8, a: &[u8; 32], b: Option<&[u8; 32]>) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update([tag]);
    h.update(a);
    if let Some(b) = b { h.update(b); }
    h.finalize().into()
}

/// The leaf for a block hash (internal byte order).
pub fn leaf(block_hash: &[u8; 32]) -> [u8; 32] { tagged(LEAF_TAG, block_hash, None) }
/// An inner node.
pub fn node(l: &[u8; 32], r: &[u8; 32]) -> [u8; 32] { tagged(NODE_TAG, l, Some(r)) }
fn bag(p: &[u8; 32], acc: &[u8; 32]) -> [u8; 32] { tagged(BAG_TAG, p, Some(acc)) }

/// Append a block hash to the MMR whose `peaks` hold `leaves_before` leaves.
pub fn append(peaks: &mut Vec<[u8; 32]>, leaves_before: u64, block_hash: &[u8; 32]) {
    let mut acc = leaf(block_hash);
    let mut n = leaves_before;
    while n & 1 == 1 {
        let l = peaks.pop().expect("mmr: fewer peaks than the leaf count's bits");
        acc = node(&l, &acc);
        n >>= 1;
    }
    peaks.push(acc);
}

/// The peaks bagged right to left (header comment); 32 zero bytes for no peaks.
pub fn root(peaks: &[[u8; 32]]) -> [u8; 32] {
    match peaks.split_last() {
        None => [0; 32],
        Some((last, rest)) => rest.iter().rev().fold(*last, |acc, p| bag(p, &acc)),
    }
}

/// For leaf `index` of an MMR of `leaves` leaves: (its peak's position in the peak list, the peak's height k, the
/// leaf's index inside that peak). None when `index >= leaves`.
pub fn peak_of(leaves: u64, index: u64) -> Option<(usize, u32, u64)> {
    if index >= leaves { return None; }
    let (mut offset, mut pos) = (0u64, 0usize);
    for k in (0..64u32).rev() {
        if leaves >> k & 1 == 0 { continue; }
        let size = 1u64 << k;
        if index < offset + size { return Some((pos, k, index - offset)); }
        offset += size;
        pos += 1;
    }
    None
}

/// Fold a leaf up a path inside its peak: bit j of `local` (the leaf's index in the peak) says whether the node at
/// level j is a right child (sibling on the left).
pub fn climb(block_hash: &[u8; 32], local: u64, path: &[[u8; 32]]) -> [u8; 32] {
    let mut acc = leaf(block_hash);
    for (j, s) in path.iter().enumerate() {
        acc = if local >> j & 1 == 0 { node(&acc, s) } else { node(s, &acc) };
    }
    acc
}

/// Is `block_hash` leaf `index` of the MMR with these `peaks` and `leaves` leaves, by this `path`?
pub fn verify(peaks: &[[u8; 32]], leaves: u64, index: u64, block_hash: &[u8; 32], path: &[[u8; 32]]) -> bool {
    if peaks.len() != leaves.count_ones() as usize { return false; }
    let Some((pos, k, local)) = peak_of(leaves, index) else { return false };
    path.len() == k as usize && climb(block_hash, local, path) == peaks[pos]
}

/// The MMR of a run of block hashes from leaf 0 (the native builder).
pub fn peaks_of(block_hashes: &[[u8; 32]]) -> Vec<[u8; 32]> {
    let mut p = Vec::new();
    for (i, h) in block_hashes.iter().enumerate() { append(&mut p, i as u64, h); }
    p
}

/// The inclusion path of leaf `index` in the MMR of `block_hashes` (all leaves from 0). None when out of range.
pub fn path(block_hashes: &[[u8; 32]], index: u64) -> Option<Vec<[u8; 32]>> {
    let n = block_hashes.len() as u64;
    let (_pos, k, local) = peak_of(n, index)?;
    let start = (index - local) as usize;
    let mut level: Vec<[u8; 32]> = block_hashes[start..start + (1usize << k)].iter().map(leaf).collect();
    let mut i = local as usize;
    let mut out = Vec::with_capacity(k as usize);
    while level.len() > 1 {
        out.push(level[i ^ 1]);
        level = level.chunks_exact(2).map(|c| node(&c[0], &c[1])).collect();
        i >>= 1;
    }
    Some(out)
}
