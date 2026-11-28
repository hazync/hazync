// The product guest's PROFILE for the host (CUTOVER C8 step 4a, 28.11.2026): what the engine guest this host writes
// streams for is built with (libzync/src/profile.rs, the same env vars as the guest's own build and
// engine/guest/assemble.sh), as constants in OUT_DIR/profile.rs. No guest is built or embedded here any more: the
// engine builds it (hzengine build) and the image id comes from its key directory (engine_backend). RISC Zero's
// risc0-build, METHOD_ELF and METHOD_ID are gone with the RISC Zero backend.
//
// (Below, the comment block of the RISC Zero era is kept where it still describes this file's rerun rules.)
//
// The `rerun-if-changed` lines below are not boilerplate — they are the difference between a stale
// METHOD_ID and a correct one.
//
// Cargo's DEFAULT for a build script is "re-run if anything in this package changed", and that default
// is silently DISABLED the moment a build script emits any `rerun-if-changed` of its own. So this list
// has to be complete: `guest` and `build.rs` are here to restore what the default covered, not as
// decoration. Deleting either re-introduces the bug in the other direction.
//
// What the default never covered is the reason this exists. Since #88 the guest `#[path]`-includes
// source from OUTSIDE this package (`coinbase-smt/src/roots.rs`, `bip30.rs`). Cargo does not watch
// those, so editing the SMT would leave the previously-built guest ELF in place and the build would
// report success while the id silently described the OLD source. A wrong id that looks like a clean
// build is worse than a build failure.
//
// The includes are PARSED from the guest rather than listed here, because a hand-maintained copy of
// that list is exactly what goes stale — and it would go stale in the direction that fails silently.
// `scripts/check-guest-inputs.sh` independently asserts that every `#[path]` include is covered here.

use std::path::Path;

// libzync: what HAZYNC_LIBZYNC means, shared with the guest's own build.rs (one list).
include!("../../libzync/src/profile.rs");

/// A lever is on if its env var is "1", off if it is "0", else on exactly when the libzync profile includes it.
fn lever(name: &str) -> bool {
    let (lz, _) = libzync_profile(std::env::var("HAZYNC_LIBZYNC").ok().as_deref());
    match std::env::var(name).as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => lz && LIBZYNC_LEVERS.contains(&name),
    }
}

/// `profile.rs` in OUT_DIR: what the embedded guest was built for, so the HOST writes the stream that
/// guest reads instead of trusting a matching set of run-time env vars.
fn write_profile(libzync: bool, assumevalid: bool, network: &str, coinlog: bool, txlog: bool, test_lineage: bool) {
    let (trie, utxohash) = (coinlog && lever("HAZYNC_TRIE"), coinlog && lever("HAZYNC_UTXOHASH"));
    let out = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("profile.rs");
    std::fs::write(&out, format!(
        "/// The embedded guest reads libzync hint blocks (libzync/src/wire.rs).\n\
         pub const LIBZYNC: bool = {libzync};\n\
         /// Script and signature checks skipped (HAZYNC_LIBZYNC=assumevalid).\n\
         pub const LIBZYNC_ASSUMEVALID: bool = {assumevalid};\n\
         /// \"mainnet\", \"signet\" or \"regtest\".\n\
         pub const NETWORK: &str = {:?};\n\
         /// The coin log instead of design B's SwiftSync forest (HAZYNC_COINLOG=1).\n\
         pub const COINLOG: bool = {coinlog};\n\
         /// Pre-proved transactions: speculative chunks + the block's record order (HAZYNC_TXLOG=1).\n\
         pub const TXLOG: bool = {txlog};\n\
         /// The counting trie (HAZYNC_TRIE; on in the libzync profile).\n\
         pub const TRIE: bool = {trie};\n\
         /// The live-coin ECMH for instant IBD (HAZYNC_UTXOHASH; on in the libzync profile).\n\
         pub const UTXOHASH: bool = {utxohash};\n\
         /// TEST VARIANT: the guest's lineage is lineage/test.rs, not the published one (HAZYNC_TEST_LINEAGE=1).\n\
         pub const TEST_LINEAGE: bool = {test_lineage};\n",
        network)).expect("write profile.rs");
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../libzync/src/profile.rs");
    for v in ["HAZYNC_LIBZYNC", "HAZYNC_NETWORK", "HAZYNC_NO_SCRIPTS", "HAZYNC_COINLOG", "HAZYNC_TRIE", "HAZYNC_UTXOHASH",
              "HAZYNC_TXLOG", "HAZYNC_TEST_LINEAGE"] {
        println!("cargo:rerun-if-env-changed={v}");
    }
    let (libzync, assumevalid) = libzync_profile(std::env::var("HAZYNC_LIBZYNC").ok().as_deref());
    // The guest skips scripts under the profile's assumevalid OR a bare HAZYNC_NO_SCRIPTS=1: the recorded profile must
    // say so in both cases, or a scripts-off image reads as a full one (review 2).
    let assumevalid = assumevalid || std::env::var("HAZYNC_NO_SCRIPTS").as_deref() == Ok("1");
    let signet = libzync_signet(std::env::var("HAZYNC_NETWORK").ok().as_deref());
    let regtest = libzync_regtest(std::env::var("HAZYNC_NETWORK").ok().as_deref());
    assert!(!regtest || libzync, "HAZYNC_NETWORK=regtest needs HAZYNC_LIBZYNC");
    assert!(!signet || libzync, "HAZYNC_NETWORK=signet needs HAZYNC_LIBZYNC (the signet check lives in the libzync aggregate)");
    let network = if signet { "signet" } else if regtest { "regtest" } else { "mainnet" };
    let coinlog = lever("HAZYNC_COINLOG");
    assert!(!coinlog || libzync, "HAZYNC_COINLOG needs HAZYNC_LIBZYNC");
    assert!(!libzync || coinlog, "a libzync guest needs the coin log (HAZYNC_COINLOG must not be 0)");
    for env_name in ["HAZYNC_TRIE", "HAZYNC_UTXOHASH"] {
        assert!(!lever(env_name) || coinlog, "{env_name} needs HAZYNC_COINLOG");
    }
    let txlog = lever("HAZYNC_TXLOG");
    assert!(!txlog || libzync, "HAZYNC_TXLOG needs HAZYNC_LIBZYNC");
    let test_lineage = std::env::var("HAZYNC_TEST_LINEAGE").as_deref() == Ok("1");
    assert!(!test_lineage || (coinlog && network == "mainnet"), "HAZYNC_TEST_LINEAGE is the mainnet coin-log image's test variant");
    write_profile(libzync, assumevalid, network, coinlog, txlog, test_lineage);
}
