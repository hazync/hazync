//! The verifier's clock rule (time-too-new) used by libzync-verify, the test-hook gate, and the strict bincode decoder
//! for files from outside this process. The RISC Zero receipt decoders and `receipt-digest` went with the RISC Zero
//! backend (CUTOVER C8 step 4); the CORE design's range, SNARK and chain verifiers before them (history/README.md).

/// bincode as `bincode::deserialize` reads it (fixed-width integers, little-endian, no size limit), but with trailing
/// bytes REFUSED: every file format here is exactly `bincode::serialize` of one value (a leaf bundle: prep.rs).
pub(crate) fn decode_exact<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    use bincode::Options as _;
    bincode::DefaultOptions::new().with_fixint_encoding().reject_trailing_bytes().deserialize(bytes).map_err(|e| e.to_string())
}

/// MAX_FUTURE_BLOCK_TIME (Core validation.h): 2 hours.
pub(crate) const MAX_FUTURE_BLOCK_TIME: u64 = 2 * 60 * 60;

/// The clock a verifier applies time-too-new against, and the raw HAZYNC_VERIFY_NOW value if that test-only
/// override is set. A verifier that sees the override never reports success (review 5): a stale export must
/// not quietly pass a future-dated chain, and an unparsable value falls back to the real clock but still counts.
pub(crate) fn verifier_clock() -> (u64, Option<String>) {
    let sys = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    match std::env::var("HAZYNC_VERIFY_NOW") {
        Ok(v) if !v.is_empty() => (v.parse().unwrap_or(sys), Some(v)),
        _ => (sys, None),
    }
}

/// The gate of the test-only hooks (HAZYNC_TEST_PREP_CRASH_AT, HAZYNC_TEST_SKIP_STALE_CHECK, HAZYNC_REFOREST_TEST_UNDO):
/// the engine's dev mode (HAZYNC_ENGINE_DEV=1), or RISC0_DEV_MODE as risc0 read it (set and not empty / "0" / "false"),
/// the switch the test scripts of the RISC Zero era set. Since CUTOVER C8 it is only a switch: no RISC Zero code reads it.
pub(crate) fn dev_mode_env() -> bool {
    std::env::var("HAZYNC_ENGINE_DEV").as_deref() == Ok("1")
        || std::env::var("RISC0_DEV_MODE").map(|v| !v.is_empty() && v != "0" && v != "false").unwrap_or(false)
}

/// Core's time-too-new (ContextualCheckBlockHeader) applied to a chain's largest timestamp on this clock.
pub(crate) fn time_ok(max_time: u32, now: u64) -> bool {
    max_time as u64 <= now + MAX_FUTURE_BLOCK_TIME
}
