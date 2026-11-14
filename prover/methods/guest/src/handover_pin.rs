// The handover format: the Handover entry, the lineage digest, the block-forest leaf and the coin-state digest. The
// lineage itself (the handovers an image pins) is in src/lineage/<variant>.rs, one file per image variant.
//
// The lineage is source on purpose, not an environment variable or a build flag: it is compiled into the guest, so
// an image's id commits to every image and height listed here, and anyone rebuilding the image gets the same list.
// The host and verifier include the same text. docs/libzync/HANDOVER.md says when an entry may be added: a rule
// change, yes; a soundness fix in the listed image, NO (that chain cannot be inherited; re-prove it).
//
// A chain continues image `image` from `first_height`, the first block the NEW image proves. The last entry is the
// direct predecessor, whose receipt the handover step verifies; earlier entries are its own lineage.

pub struct Handover {
    pub image: [u32; 8],
    pub first_height: u32,
    /// The RISC Zero version the predecessor's receipts were made under, as that version's succinct verifier-parameter
    /// digest. ZERO = the same RISC Zero as this image (the handover step uses `env::verify`). Non-zero = an older one,
    /// verified by its vendored recursion circuit (src/xver/); the image must be built with that version's `xver-*`
    /// feature. ⛔ Only across a NON-soundness upgrade (docs/HANDOVER.md §4).
    pub verifier: [u8; 32],
}

/// `verifier` for a predecessor on the same RISC Zero.
pub const SAME_ZKVM: [u8; 32] = [0u8; 32];

/// The preimage of a lineage step, at its length: 69 bytes, plus the 32-byte verifier digest when it is not
/// SAME_ZKVM. A same-version entry therefore digests exactly as before the field existed.
pub fn lineage_step(prev: &[u8; 32], h: &Handover) -> ([u8; 101], usize) {
    let mut p = [0u8; 101];
    p[..69].copy_from_slice(&lineage_preimage(prev, h));
    if h.verifier == SAME_ZKVM { return (p, 69); }
    p[69..101].copy_from_slice(&h.verifier);
    (p, 101)
}

/// Domain tags. 0x00 coin leaf and 0x01 forest node are `utreexo.rs`; 0x10 / 0x11 are the coinbase SMT's leaf / node;
/// 0x04 below. The coin log uses its own IVs (libzync/src/coinlog.rs).
pub const TAG_BLOCK_LEAF: u8 = 0x02;
pub const TAG_LINEAGE: u8 = 0x03;

/// The first 69 bytes of one step of the lineage digest (use `lineage_step`):
/// `L' = SHA-256(0x03 ‖ L ‖ image, as 8 u32 LE ‖ first_height u32 LE [‖ verifier, if not SAME_ZKVM])`, starting from
/// 32 zero bytes.
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

/// The preimage of a block leaf: `SHA-256(0x02 ‖ block hash, internal byte order ‖ height u32 LE ‖ utxo state)`, where
/// `utxo state` is [`utxo_state_preimage`]'s digest of the coin commitment AFTER this block (HANDOVER.md §3). With it, a
/// proof at the tip commits every past block's coin state: a zyncAPP opens a block leaf, then a coin path against
/// that block's roots, to show a coin's state at any height.
pub fn block_leaf_preimage(block_hash: &[u8; 32], height: u32, utxo_state: &[u8; 32]) -> [u8; 69] {
    let mut p = [0u8; 69];
    p[0] = TAG_BLOCK_LEAF;
    p[1..33].copy_from_slice(block_hash);
    p[33..37].copy_from_slice(&height.to_le_bytes());
    p[37..69].copy_from_slice(utxo_state);
    p
}

pub const TAG_UTXO_STATE: u8 = 0x04;

/// The preimage of a block's coin-state digest, fed in pieces to `feed` (no allocation, so the host, the guest and
/// the verifier share it): `0x04 ‖ utxo_leaves u64 LE ‖ utxo_value u64 LE ‖ each present root, the largest (highest
/// level) first`. The leaf count fixes which roots are present, so absent ones need no marker.
pub fn utxo_state_preimage(leaves: u64, value: u64, roots: &[Option<[u8; 32]>], feed: &mut impl FnMut(&[u8])) {
    feed(&[TAG_UTXO_STATE]);
    feed(&leaves.to_le_bytes());
    feed(&value.to_le_bytes());
    for r in roots.iter().rev().flatten() {
        feed(r);
    }
}
