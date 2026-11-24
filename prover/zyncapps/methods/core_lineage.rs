// ⚠ GUEST-COMPILED: #[path]-included by the hints and utreexo zyncAPP guests, so editing it moves both images' ids.
// The includer provides `handover_pin` (the core's own prover/methods/guest/src/handover_pin.rs) and the sha2 crate.
//
// A core chain with a handover (docs/libzync/HANDOVER.md §4, §5). The core journal's `lineage` is a SHA-256 chain over
// the handovers its image pins, oldest first (`handover_pin::lineage_step`, from 32 zero bytes). The digest alone does
// not name the images, so the prover passes the list and this checks it:
//   1. the list digests to the journal's `lineage`. SHA-256 binds it: no other list can stand for that digest, so the
//      list is the one compiled into `core_id` (the core guest commits `lineage_of(LINEAGE)` at the handover and
//      carries it unchanged in every later step);
//   2. every block 1..T was proved by an image whose block leaves carry the undo hash: `handover_pin::undo_from` is 1
//      (neither `core_id` nor any image of the list, for the blocks below its `first_height`, is in PRE_UNDO_IMAGES).
//      It is the core's own function, the one the verifiers select the leaf format with.
// An empty list digests to 32 zero bytes, so a chain of one image since genesis is checked exactly as before (a zero
// lineage, and a `core_id` that is not a pre-undo image).

/// One handover entry as the prover passes it: the image, `first_height`, and the verifier digest (zero = SAME_ZKVM).
pub type Entry = ([u32; 8], u32, [u8; 32]);

/// `Ok` when `list` is the lineage the core journal commits and every block 1..T carries the undo commitment.
pub fn check(core_id: &[u32; 8], lineage: &[u8; 32], list: &[Entry]) -> Result<(), &'static str> {
    use sha2::Digest;
    let hs: alloc::vec::Vec<super::handover_pin::Handover> = list
        .iter()
        .map(|&(image, first_height, verifier)| super::handover_pin::Handover { image, first_height, verifier })
        .collect();
    let mut l = [0u8; 32];
    for h in &hs {
        let (p, n) = super::handover_pin::lineage_step(&l, h);
        l = sha2::Sha256::digest(&p[..n]).into();
    }
    if l != *lineage {
        return Err("the core lineage given is not the one the core journal commits");
    }
    if super::handover_pin::undo_from(core_id, &hs) != 1 {
        return Err("a block of the core chain was proved by a pre-undo image: it carries no undo hash");
    }
    Ok(())
}
