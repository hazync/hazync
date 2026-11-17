//! The receipt size bound of verify_utreexo's decode (docs/libzync/ZAPP_TESTING.md §3.5, the cargo-mutants pass of
//! 17.11.2026): a receipt of exactly MAX_RECEIPT_BYTES is decoded (and refused, as zeros, by the decoder), one byte more
//! is refused by the bound before any decoding.

use zapp_utreexo::*;

#[test]
fn the_receipt_size_bound_is_inclusive() {
    if let Err(e) = decode_receipt(&vec![0u8; MAX_RECEIPT_BYTES]) {
        assert!(!e.contains("-byte limit"), "{e}");
    }
    assert!(decode_receipt(&vec![0u8; MAX_RECEIPT_BYTES + 1]).unwrap_err().contains("-byte limit"));
    let r = verify_utreexo(&vec![0u8; MAX_RECEIPT_BYTES + 1], UTREEXO_IMAGE_ID, None, 1_800_000_000, None, None);
    assert_eq!(r.verdict, swiftsync_verify::Verdict::Fail);
}
