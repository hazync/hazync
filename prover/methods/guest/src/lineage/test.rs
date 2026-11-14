// ⚠ GUEST-COMPILED CRATE — but only into the TEST VARIANT (guest feature test-lineage, HAZYNC_TEST_LINEAGE=1), never
// into a published image. The mainnet coin-log image with THIS lineage instead of lineage/mainnet_coinlog.rs: same
// source otherwise, a different image id that is in no pin table (reproduce/LIBZYNC_IDS, the lineage files). The host
// says so (`method-id`, `libzync-lineage`: TEST_VARIANT).
//
// WHY: the published mainnet image pins its predecessor below its handover height, so by design it never proves those
// blocks; a test proving mainnet from genesis (scripts/libzync/ci/chain.sh) needs an image with no predecessor. EMPTY
// as committed. CI's board rehearsal pins the empty-lineage test image here at a test height and builds again
// (scripts/libzync/ci/build-hosts.sh host_mN), then reverts the edit.
pub const LINEAGE: &[crate::handover_pin::Handover] = &[];
