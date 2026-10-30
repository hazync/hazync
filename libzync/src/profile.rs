// libzync build profile: what `HAZYNC_LIBZYNC` means, as ONE list.
//
// Included with `include!` by prover/methods/build.rs (cargo features) and by
// prover/methods/guest/build.rs (C defines), so the two halves of a guest build cannot disagree about
// which levers are on. Plain items only: no `use`, no attributes that a build script would reject.
//
//   HAZYNC_LIBZYNC=1            the full libzync guest (scripts verified)
//   HAZYNC_LIBZYNC=assumevalid  the same, with script and signature checks skipped (SwiftSync's
//                               assumevalid mode: proves the UTXO set and the hints, not the scripts)
//   HAZYNC_NETWORK=signet       orthogonal: build for default signet instead of mainnet
//
// A lever set explicitly to 1 in the environment stays on with or without the profile; the profile
// only ever turns levers ON, so an old per-lever command line still builds what it always built.

/// Every per-lever env var `HAZYNC_LIBZYNC` implies, exactly as the build scripts read them.
/// The canonical-guest levers (FIELD_BIGINT2, LIFTX_HINT, ECMULT_WINDOW=21) are included so that a
/// libzync build is self-contained: it needs no other variable set.
pub const LIBZYNC_LEVERS: &[&str] = &[
    // GHOST: the coprocessor under libsecp and SHA-256
    "HAZYNC_BIGINT2_ECDSA",
    "HAZYNC_BIGINT2_SCHNORR",
    "HAZYNC_SCALAR_INV_ACCEL",
    "HAZYNC_SHA_FASTPATH",
    "HAZYNC_SHA_D64_ACCEL",
    "HAZYNC_AGG_READSLICE",
    // canonical CORE channel
    "HAZYNC_LIFTX_HINT",
    "HAZYNC_FIELD_BIGINT2",
    // libzync proper
    "HAZYNC_BATCH_ECDSA",
    "HAZYNC_TX_ONCE",
    "HAZYNC_SWIFTSYNC",
    "HAZYNC_AGG_LITE",
    "HAZYNC_FAST_COMMIT",
    "HAZYNC_SHA_STREAM",
    "HAZYNC_TXDATA_LAZY",
    "HAZYNC_LAZY_TXID",
    "HAZYNC_H160_MEMO",
    "HAZYNC_LEGACY_SIGHASH",
    "HAZYNC_D64_STREAM",
    "HAZYNC_RAW_TXID",
    "HAZYNC_PUBKEY_MEMO",
    "HAZYNC_LEAF_ONEBUF",
];

/// libsecp's ECMULT_WINDOW_SIZE under the profile (the measured optimum, canonical since v0.21).
pub const LIBZYNC_ECMULT_WINDOW: u32 = 21;

/// The guest cargo feature the profile adds on top of the per-lever features.
pub const LIBZYNC_FEATURE: &str = "libzync";

/// `HAZYNC_LIBZYNC` parsed: (profile on, assumevalid). Panics on an unknown value rather than
/// building something nobody asked for.
pub fn libzync_profile(v: Option<&str>) -> (bool, bool) {
    match v {
        None | Some("") | Some("0") => (false, false),
        Some("1") => (true, false),
        Some("assumevalid") => (true, true),
        Some(x) => panic!("HAZYNC_LIBZYNC={x}: expected 1 or assumevalid"),
    }
}

/// `HAZYNC_NETWORK` parsed: true for signet. Panics on an unknown value.
pub fn libzync_signet(v: Option<&str>) -> bool {
    match v {
        None | Some("") | Some("main") | Some("mainnet") => false,
        Some("signet") => true,
        Some(x) => panic!("HAZYNC_NETWORK={x}: expected mainnet or signet"),
    }
}
