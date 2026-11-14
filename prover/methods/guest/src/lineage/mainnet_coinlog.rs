// ⚠ GUEST-COMPILED CRATE — editing this file changes METHOD_ID (this variant's image only).
// The handovers the mainnet coinlog image pins, oldest first (docs/libzync/HANDOVER.md §4). One file per image variant,
// so pinning one variant's predecessor never changes another's image id. The assumevalid build of a network shares its
// scripts-checked file (it is not published).
//
// v0.26.0 (2026-11-14): the live board's image 8888d59c (v0.23.0-v0.25.0, RISC Zero 3.1.0) proves blocks 1..1999; this
// image takes over at block 2000 on the same RISC Zero, so SAME_ZKVM. Words are 8888d59c4b8c699f... as u32x8, checked
// against `host method-id` of the deployed 8888d59c host.
pub const LINEAGE: &[crate::handover_pin::Handover] = &[crate::handover_pin::Handover {
    image: [2631239816, 2674494539, 4198157524, 4069463800, 1375117754, 2714639643, 453263298, 4095025963],
    first_height: 2000,
    verifier: crate::handover_pin::SAME_ZKVM,
}];
