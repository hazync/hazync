// The image lineage this guest continues, oldest first. EMPTY means this image starts its own chain at genesis.
//
// This file is source on purpose, not an environment variable or a build flag: it is compiled into the guest, so
// an image's id commits to every image and height listed here, and anyone rebuilding the image gets the same list.
// The host and verifier include the same text. docs/libzync/HANDOVER.md says when an entry may be added: a rule
// change, yes; a soundness fix in the listed image, NO (that chain cannot be inherited; re-prove it).
//
// A chain continues image `image` from `first_height`, the first block the NEW image proves. The last entry is the
// direct predecessor, whose receipt the handover step verifies; earlier entries are its own lineage.

pub struct Handover {
    pub image: [u32; 8],
    pub first_height: u32,
}

pub const LINEAGE: &[Handover] = &[];

/// Domain tags. 0x00 coin leaf and 0x01 forest node are `utreexo.rs`; 0x11 is the coinbase SMT.
pub const TAG_BLOCK_LEAF: u8 = 0x02;
pub const TAG_LINEAGE: u8 = 0x03;

/// The preimage of one step of the lineage digest:
/// `L' = SHA-256(0x03 ‖ L ‖ image, as 8 u32 LE ‖ first_height u32 LE)`, starting from 32 zero bytes.
pub fn lineage_preimage(prev: &[u8; 32], h: &Handover) -> [u8; 69] {
    let mut p = [0u8; 69];
    p[0] = TAG_LINEAGE;
    p[1..33].copy_from_slice(prev);
    for (i, w) in h.image.iter().enumerate() {
        p[33 + i * 4..37 + i * 4].copy_from_slice(&w.to_le_bytes());
    }
    p[65..69].copy_from_slice(&h.first_height.to_le_bytes());
    p
}

/// The preimage of a block leaf: `SHA-256(0x02 ‖ block hash, internal byte order ‖ height u32 LE)`.
pub fn block_leaf_preimage(block_hash: &[u8; 32], height: u32) -> [u8; 37] {
    let mut p = [0u8; 37];
    p[0] = TAG_BLOCK_LEAF;
    p[1..33].copy_from_slice(block_hash);
    p[33..37].copy_from_slice(&height.to_le_bytes());
    p
}
