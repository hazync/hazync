//! Receipt checks: receipt-digest, and the verifier's clock rule (time-too-new) used by libzync-verify. The CORE
//! design's range, SNARK and chain verifiers were removed with it (history/README.md).

use methods::METHOD_ID;
use crate::hex;

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

/// RISC0_DEV_MODE as risc0 reads it: set and not empty / "0" / "false". Both verifiers refuse it (review 2, #16).
pub(crate) fn dev_mode_env() -> bool {
    std::env::var("RISC0_DEV_MODE").map(|v| !v.is_empty() && v != "0" && v != "false").unwrap_or(false)
}

/// Core's time-too-new (ContextualCheckBlockHeader) applied to a chain's largest timestamp on this clock.
pub(crate) fn time_ok(max_time: u32, now: u64) -> bool {
    max_time as u64 <= now + MAX_FUTURE_BLOCK_TIME
}

// Print a saved receipt's journal digest and verify it. The gate for segment distribution is that a
// distributed prove and a monolithic prove of the SAME chunk agree on this value -- proving that
// routing every segment across a wire changed nothing about what was proved.
//
// Printing the digest from `seg-distribute` alone proves nothing: it would agree with itself.
pub(crate) fn receipt_digest_cmd(path: &str) {
    use risc0_zkvm::sha::Digestible;
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let r: risc0_zkvm::Receipt = bincode::deserialize(&bytes).expect("deserialize receipt");
    match r.verify(METHOD_ID) {
        Ok(()) => println!("VERIFIED against METHOD_ID"),
        Err(e) => { println!("VERIFY FAILED: {e}"); std::process::exit(1); }
    }
    println!("journal_bytes {}", r.journal.bytes.len());
    println!("journal_digest {}", hex(r.journal.digest().as_bytes()));
}
