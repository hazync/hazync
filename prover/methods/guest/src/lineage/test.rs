// ⚠ GUEST-COMPILED CRATE — but only into the TEST VARIANT (guest feature test-lineage, HAZYNC_TEST_LINEAGE=1), never
// into a published image. The mainnet coin-log image with THIS lineage instead of lineage/mainnet_coinlog.rs: same
// source otherwise, a different image id that is in no pin table (reproduce/LIBZYNC_IDS, the lineage files). The host
// says so (`method-id`, `libzync-lineage`: TEST_VARIANT).
//
// WHY: a published mainnet image may pin its predecessor below its handover height, and then by design never proves
// those blocks; a test proving mainnet from genesis (scripts/libzync/ci/chain.sh) needs an image with no predecessor,
// and the board rehearsal needs a second image pinning it. EMPTY as committed. CI's board rehearsal pins the
// empty-lineage test image here at a test height and builds again (scripts/libzync/ci/build-hosts.sh host_mN), then
// reverts the edit.
pub const LINEAGE: &[crate::handover_pin::Handover] = &[];

// THE TEST VARIANT'S OWN IMAGE ID. Since v0.26.0 the published mainnet lineage is empty too, so with only the lineage
// file differing the test image would compile to the published image's ELF and id, and a test proof would be
// indistinguishable from a published one (build-hosts.sh refuses that). This constructor, which `__libc_init_array`
// runs at the start of every guest run, makes the test variant's program, and so its id, its own. It writes one word
// nobody reads: no check, input or output changes. It is here, not in main.rs, so the published images' source and
// ids do not move. The host includes this file too (`lineage_test`): there it is not compiled (zkVM target only).
#[cfg(target_os = "zkvm")]
static mut TEST_VARIANT_SINK: u32 = 0;
#[cfg(target_os = "zkvm")]
extern "C" fn test_variant_mark() {
    unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!(TEST_VARIANT_SINK), 0x7e57_7a12) }
}
#[cfg(target_os = "zkvm")]
#[used]
#[link_section = ".init_array"]
static TEST_VARIANT_MARK: extern "C" fn() = test_variant_mark;
