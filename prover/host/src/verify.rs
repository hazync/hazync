//! Receipt checks: receipt-digest, and the verifier's clock rule (time-too-new) used by libzync-verify. The CORE
//! design's range, SNARK and chain verifiers were removed with it (history/README.md).

use methods::METHOD_ID;
use crate::hex;

/// A receipt file from OUTSIDE this process (a contributor's upload, a downloaded proof, a file named on the command
/// line), decoded strictly, or why it is not one. The same rules as `swiftsync_verify::decode_receipt` (#758, fuzz round
/// 11-17), replicated rather than depended on: that crate builds risc0-zkvm with `disable-dev-mode`, which feature
/// unification would switch on for this whole prover (and with it the dev-mode test paths), and its decoder refuses a
/// FakeReceipt outright, where the callers here refuse one themselves with their own message (or accept it under the
/// two-switch test opt-in). Keep the two in step:
/// - a composite receipt is refused before decoding;
/// - bincode as `bincode::deserialize` reads it (fixed-width integers, no size limit), but with trailing bytes REFUSED:
///   a receipt file is exactly `bincode::serialize(&receipt)` (a receipt with bytes appended verified before);
/// - every word of a Poseidon2 succinct receipt's control_id, control inclusion proof and seal is a BabyBear field
///   element: risc0 `assert!`s that in `hash_pair` (an ABORT, not an error) before checking anything else, so a hostile
///   word must be refused here. A word refused here is one no valid receipt holds.
pub(crate) fn decode_receipt(bytes: &[u8]) -> Result<risc0_zkvm::Receipt, String> {
    decode_receipt_with(bytes, false)
}

/// A chunk receipt copied from another machine (HAZYNC_RECEIPTS / AGG_EARLY): `decode_receipt`'s rules, except that a
/// COMPOSITE receipt is decoded, not refused. The aggregate still converts one (`composite_to_succinct`) so that a set
/// staged by an older host, which proved chunks composite, still aggregates; these files come from the operator's own
/// fleet, not from a contributor.
pub(crate) fn decode_chunk_receipt(bytes: &[u8]) -> Result<risc0_zkvm::Receipt, String> {
    decode_receipt_with(bytes, true)
}

fn decode_receipt_with(bytes: &[u8], allow_composite: bool) -> Result<risc0_zkvm::Receipt, String> {
    // A composite receipt nests without bound (a hostile one overflows the stack while decoding: cold review 11-06),
    // and every receipt this prover writes is succinct or Groth16, so it is refused before decoding, as libzync-verify
    // and swiftsync-verify do. bincode's first u32 is the InnerReceipt variant: 0 Composite, 1 Succinct, 2 Groth16, 3 Fake.
    if !allow_composite && bytes.get(..4) == Some(&[0, 0, 0, 0]) {
        return Err("a composite receipt: refused before decoding (a chain proof is succinct or Groth16)".into());
    }
    let r: risc0_zkvm::Receipt = decode_exact(bytes).map_err(|e| format!("not a receipt: {e}"))?;
    if let risc0_zkvm::InnerReceipt::Succinct(s) = &r.inner {
        succinct_well_formed(s)?;
    }
    Ok(r)
}

/// bincode as `bincode::deserialize` reads it (fixed-width integers, little-endian, no size limit), but with trailing
/// bytes REFUSED: every wire and file format here is exactly `bincode::serialize` of one value. For any untrusted
/// bincode value (seg.rs's worker wire: Segment, SegmentReceipt, SuccinctReceipt).
pub(crate) fn decode_exact<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    use bincode::Options as _;
    bincode::DefaultOptions::new().with_fixint_encoding().reject_trailing_bytes().deserialize(bytes).map_err(|e| e.to_string())
}

/// The BabyBear prime, 15 * 2^27 + 1: every word of a Poseidon2 digest and of a Poseidon2 STARK seal is below it.
pub(crate) const BABYBEAR_P: u32 = 15 * (1 << 27) + 1;

/// The BabyBear check above for one Poseidon2 succinct receipt: risc0 `assert!`s these words are field elements in
/// `hash_pair` while checking the control inclusion proof, before anything else.
pub(crate) fn succinct_well_formed<C>(s: &risc0_zkvm::SuccinctReceipt<C>) -> Result<(), String> {
    if s.hashfn != "poseidon2" {
        return Ok(()); // SHA-256 and Blake2b accept any digest word
    }
    let bad = |what: &str, w: &[u32]| match w.iter().position(|&x| x >= BABYBEAR_P) {
        Some(i) => Err(format!("malformed receipt: {what} word {i} is 0x{:08x}, not a BabyBear field element", w[i])),
        None => Ok(()),
    };
    bad("control_id", s.control_id.as_words())?;
    for (j, d) in s.control_inclusion_proof.digests.iter().enumerate() {
        bad(&format!("control inclusion proof digest {j}"), d.as_words())?;
    }
    bad("seal", &s.seal)
}

/// An untrusted SuccinctReceipt (a worker's lift, join or resolve): `decode_exact`, then `succinct_well_formed`.
pub(crate) fn decode_succinct<C: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<risc0_zkvm::SuccinctReceipt<C>, String> {
    let s: risc0_zkvm::SuccinctReceipt<C> = decode_exact(bytes).map_err(|e| format!("not a succinct receipt: {e}"))?;
    succinct_well_formed(&s)?;
    Ok(s)
}

/// `decode_receipt` on a file, the error naming it.
pub(crate) fn read_receipt(path: &str) -> Result<risc0_zkvm::Receipt, String> {
    decode_receipt(&std::fs::read(path).map_err(|e| format!("{path}: {e}"))?).map_err(|e| format!("{path}: {e}"))
}

/// An untrusted receipt's journal decoded as `T`, as `swiftsync_verify::decode_journal`: risc0's `from_slice` PANICS on a
/// word-aligned byte length that is not a whole number of words, so the length is checked first.
pub(crate) fn decode_journal<T: serde::de::DeserializeOwned>(j: &risc0_zkvm::Journal) -> Result<T, String> {
    if j.bytes.len() % 4 != 0 {
        return Err(format!("the journal is {} bytes, not whole words", j.bytes.len()));
    }
    j.decode().map_err(|e| e.to_string())
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
    let r = read_receipt(path).unwrap_or_else(|e| { println!("VERIFY FAILED: {e}"); std::process::exit(1) });
    match r.verify(METHOD_ID) {
        Ok(()) => println!("VERIFIED against METHOD_ID"),
        Err(e) => { println!("VERIFY FAILED: {e}"); std::process::exit(1); }
    }
    println!("journal_bytes {}", r.journal.bytes.len());
    println!("journal_digest {}", hex(r.journal.digest().as_bytes()));
}
