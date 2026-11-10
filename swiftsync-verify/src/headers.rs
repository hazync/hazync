//! The header-chain zyncAPP (ROADMAP K1, N3): what a verifier checks on its receipts.
//!
//! The guest (`prover/zyncapps/methods/headers`) runs Core's header checks over a run of mainnet headers and commits
//! `(sha256 of its input state, its output state)`. A run from genesis must start from the one empty state, so a chain
//! of runs is valid from genesis when the first run's input digest is the empty state's and every later run's input
//! digest is the digest of the state the run before it ended in. That is how the proof follows the tip: new headers
//! are a short run chained to the last one.
//!
//! Verdict, as for the core proof: `Ok` when the chain's work reaches Core's minimum chain work for mainnet, `Demo`
//! below it, `Fail` on any failed check. Chain identity (that this is the network's chain, not only a valid one) is
//! the tip hash, compared with a source the verifier trusts (`expect`).
use crate::{receipt_kind, work_at_least, Verdict, IMAGES, MAX_FUTURE_BLOCK_TIME};
use risc0_zkvm::sha::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// The header-chain image id this verifier trusts by default. Set from the reproducible build (`reproduce/`), as the
/// core's ids are; `--image <hex>` replaces it for a reader who rebuilt the image themselves.
pub const HEADERS_IMAGE_ID: &str = "50512831aa62a82cb80762ddd65c952e7517c6d175bf21ca1afbe357409ce801";

/// The guest's state, field for field and in order (risc0 serde): the last header's height, hash (internal byte
/// order), chainwork (little-endian), nBits, time, its retarget epoch's first time, the last <= 11 times, and the
/// largest time seen.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct HeaderState {
    pub height: i64,
    pub hash: [u8; 32],
    pub work: [u8; 32],
    pub bits: u32,
    pub time: u32,
    pub epoch_first_time: u32,
    pub recent_times: Vec<u32>,
    pub max_time: u32,
}

impl HeaderState {
    /// The state before genesis: the only state a run from genesis may start from.
    pub fn empty() -> Self {
        HeaderState { height: -1, hash: [0; 32], work: [0; 32], bits: 0, time: 0, epoch_first_time: 0,
                      recent_times: Vec::new(), max_time: 0 }
    }
    /// What the guest commits for an input state: sha256 of risc0's serde words, as little-endian bytes.
    pub fn digest(&self) -> [u8; 32] {
        let w = risc0_zkvm::serde::to_vec(self).expect("a HeaderState always serialises");
        Sha256::digest(w.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>()).into()
    }
    /// The tip hash as block explorers print it (reversed, hex).
    pub fn hash_hex(&self) -> String { self.hash.iter().rev().map(|b| format!("{b:02x}")).collect() }
    pub fn work_hex(&self) -> String { self.work.iter().rev().map(|b| format!("{b:02x}")).collect() }
}

pub struct HeadersReport {
    pub checks: Vec<(bool, String)>,
    /// The state the last run ended in, when every receipt decoded.
    pub state: Option<HeaderState>,
    pub verdict: Verdict,
}

impl HeadersReport {
    fn check(&mut self, ok: bool, what: impl Into<String>) -> bool { self.checks.push((ok, what.into())); ok }
}

/// Verify a chain of header-chain receipts (the first from genesis, each next one starting where the one before ended)
/// against the headers image id `image_hex`. `now` is the verifier's clock (unix seconds) for time-too-new;
/// `expect` = (height, hash as explorers print it) from a source the verifier trusts, for chain identity.
pub fn verify_headers(receipts: &[&[u8]], image_hex: &str, now: u64, expect: Option<(i64, &str)>) -> HeadersReport {
    let mut rep = HeadersReport { checks: Vec::new(), state: None, verdict: Verdict::Fail };
    let Some(id) = hex32(image_hex) else {
        rep.check(false, format!("the headers image id is 32 bytes of hex ({image_hex})"));
        return rep;
    };
    let image = Digest::from_bytes(id);
    if !rep.check(!receipts.is_empty(), "at least one receipt") { return rep; }
    let mut want_in = HeaderState::empty().digest();
    let mut last: Option<HeaderState> = None;
    for (i, bytes) in receipts.iter().enumerate() {
        let n = i + 1;
        if let Err(e) = receipt_kind(bytes) {
            rep.check(false, format!("receipt {n} is a single succinct or Groth16 proof ({e})"));
            return rep;
        }
        let r: risc0_zkvm::Receipt = match bincode::deserialize(bytes) {
            Ok(r) => r,
            Err(e) => { rep.check(false, format!("receipt {n} decodes ({e})")); return rep; }
        };
        rep.check(!matches!(r.inner, risc0_zkvm::InnerReceipt::Fake(_)), format!("receipt {n} is a real proof"));
        let v = r.verify(image);
        rep.check(v.is_ok(), format!("receipt {n} verifies against the headers image {image_hex}{}",
                                     v.err().map(|e| format!(": {e}")).unwrap_or_default()));
        let (din, out): ([u8; 32], HeaderState) = match r.journal.decode() {
            Ok(j) => j,
            Err(e) => { rep.check(false, format!("receipt {n}'s journal is (input digest, header state) ({e})")); return rep; }
        };
        let from = last.as_ref().map_or(-1, |s| s.height);
        rep.check(din == want_in, if i == 0 {
            "the first run starts from genesis (its input is the empty state)".to_string()
        } else {
            format!("run {n} starts where run {i} ended (height {from})")
        });
        rep.check(out.height > from, format!("run {n} adds headers ({}..{})", from + 1, out.height));
        want_in = out.digest();
        last = Some(out);
    }
    let s = last.expect("at least one receipt decoded");
    rep.check(s.max_time as u64 <= now.saturating_add(MAX_FUTURE_BLOCK_TIME),
              format!("time-too-new: largest header time {} <= this clock {} + {} s", s.max_time, now, MAX_FUTURE_BLOCK_TIME));
    if let Some((h, hash)) = expect {
        rep.check(s.height == h && s.hash_hex() == hash.to_ascii_lowercase(),
                  format!("chain identity: the tip is block {h} {hash} (from a source this verifier trusts)"));
    }
    let failed = rep.checks.iter().any(|(ok, _)| !ok);
    let mainnet = IMAGES.iter().find(|i| i.network == "mainnet").expect("IMAGES holds mainnet");
    let enough = work_at_least(&s.work, mainnet.min_work_hex);
    rep.checks.push((true, format!("chainwork {} {} Core's minimum {}", s.work_hex(),
                                   if enough { ">=" } else { "<" }, mainnet.min_work_hex)));
    rep.verdict = if failed { Verdict::Fail } else if enough { Verdict::Ok } else { Verdict::Demo };
    rep.state = Some(s);
    rep
}

fn hex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 { return None; }
    let mut o = [0u8; 32];
    for i in 0..32 { o[i] = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?; }
    Some(o)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The empty state's digest must equal what the guest computes over the same serde words: pinned here so a
    /// change to HeaderState's layout (which would break every chain of runs) fails a test.
    #[test]
    fn empty_state_serialises_as_the_guest_reads_it() {
        let w = risc0_zkvm::serde::to_vec(&HeaderState::empty()).unwrap();
        // height i64 (2 words), hash (32 words, one per byte), work (32), bits, time, epoch_first_time, len 0, max_time
        assert_eq!(w.len(), 2 + 32 + 32 + 3 + 1 + 1);
        assert_eq!(&w[..2], &[0xffff_ffff, 0xffff_ffff]);
    }
}
