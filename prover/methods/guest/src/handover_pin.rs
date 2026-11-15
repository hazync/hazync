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
/// 0x04 and 0x05 below. The coin log uses its own IVs (libzync/src/coinlog.rs).
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

/// The block leaf WITH the block's undo-data commitment (docs/libzync/UNDO.md): every block an image proves, except
/// in the images of [`PRE_UNDO_IMAGES`], whose leaves are [`block_leaf_preimage`]'s. Block 0 (the genesis anchor,
/// proved by no image) always keeps the 0x02 leaf, so the per-network genesis anchor never moves.
pub const TAG_BLOCK_LEAF_UNDO: u8 = 0x05;

/// `SHA-256(0x05 ‖ block hash, internal byte order ‖ height u32 LE ‖ utxo state ‖ undo hash)`, 101 bytes. The undo
/// hash is [`undo_record_head`]'s format over the block's spent coins (docs/libzync/UNDO.md). A different tag and length
/// from the 0x02 leaf, so no opening in one format can stand for a leaf of the other.
pub fn block_leaf_undo_preimage(block_hash: &[u8; 32], height: u32, utxo_state: &[u8; 32], undo_hash: &[u8; 32]) -> [u8; 101] {
    let mut p = [0u8; 101];
    p[..69].copy_from_slice(&block_leaf_preimage(block_hash, height, utxo_state));
    p[0] = TAG_BLOCK_LEAF_UNDO;
    p[69..101].copy_from_slice(undo_hash);
    p
}

/// The first 16 bytes of one spent coin's undo record (format U0, #729); the record is these, then the scriptPubKey:
/// `code u32 LE (= creation height * 2 + coinbase) ‖ amount u64 LE ‖ len(scriptPubKey) u32 LE ‖ scriptPubKey`.
/// The input's digest is SHA-256(record); the block's undo hash is SHA-256 of the digests of every input of every
/// non-coinbase transaction, in block order (SHA-256 of nothing for a block with none). `coinbase` is the coin's
/// coinbase flag only (bit 0), never the BIP30-tracked bit.
pub fn undo_record_head(height: u32, coinbase: bool, value: u64, spk_len: u32) -> [u8; 16] {
    let mut h = [0u8; 16];
    h[0..4].copy_from_slice(&(height.wrapping_mul(2) | coinbase as u32).to_le_bytes());
    h[4..12].copy_from_slice(&value.to_le_bytes());
    h[12..16].copy_from_slice(&spk_len.to_le_bytes());
    h
}

/// Published images whose block leaves carry NO undo hash (0x02 leaves): the v0.23.0-v0.25.0 mainnet image 8888d59c
/// and signet image d4210dae. Every later image uses the 0x05 leaf for the blocks it proves. A verifier selects a
/// block's leaf format from the image that proved it ([`leaf_has_undo`]).
pub const PRE_UNDO_IMAGES: &[[u32; 8]] = &[
    [2631239816, 2674494539, 4198157524, 4069463800, 1375117754, 2714639643, 453263298, 4095025963],
    [2920096212, 1950372677, 2131348028, 3996009003, 3234416280, 3822776362, 4201124381, 2280912094],
];

/// The image that proved block `h` of a chain proved by `image` with handovers `lineage` (oldest first): the first
/// entry with `h < first_height`, else `image` (entry k's image proved every block below its first_height that an
/// earlier entry did not).
pub fn image_at(image: &[u32; 8], lineage: &[Handover], h: u32) -> [u32; 8] {
    lineage.iter().find(|e| h < e.first_height).map(|e| e.image).unwrap_or(*image)
}

/// Does block `h`'s leaf carry an undo hash, in a chain whose tip proof is by `image` with `lineage` (the list its
/// `lineage` field commits; empty when that field is zero)? Never for block 0; else exactly when the image that proved
/// `h` is not in [`PRE_UNDO_IMAGES`].
pub fn leaf_has_undo(image: &[u32; 8], lineage: &[Handover], h: u32) -> bool {
    h != 0 && !PRE_UNDO_IMAGES.contains(&image_at(image, lineage, h))
}

/// No block of the chain carries an undo hash ([`undo_from`]).
pub const NO_UNDO: u32 = u32::MAX;

/// The first height whose leaf carries an undo hash ([`leaf_has_undo`] is false below it and true from it on, because
/// pre-undo images only ever precede undo images in a lineage), or [`NO_UNDO`].
pub fn undo_from(image: &[u32; 8], lineage: &[Handover]) -> u32 {
    if PRE_UNDO_IMAGES.contains(image) { return NO_UNDO; }
    lineage.iter().filter(|e| PRE_UNDO_IMAGES.contains(&e.image)).map(|e| e.first_height).max().unwrap_or(0).max(1)
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
