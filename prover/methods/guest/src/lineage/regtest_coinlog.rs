// ⚠ GUEST-COMPILED CRATE — editing this file changes METHOD_ID (this variant's image only).
// The handovers the regtest coinlog image pins, oldest first (docs/libzync/HANDOVER.md §4). EMPTY: this image starts its own
// chain at genesis. One file per image variant, so pinning one variant's predecessor never changes another's image id.
// The assumevalid build of a network shares its scripts-checked file (it is not published).
pub const LINEAGE: &[crate::handover_pin::Handover] = &[];
