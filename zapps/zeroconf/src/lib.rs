//! The zero-confirmation zyncAPP's check (`zapp_zeroconf`): a proof that transaction T is valid for the block after a
//! proven tip, and that every coin it spends is unspent at that tip (prover/zyncapps/methods/zeroconf, the guest;
//! engine/zapps/zeroconf, its fixture and refusal cases). A merchant learns it before T confirms.
//!
//! [`verify_zeroconf`] checks the zAPP's ENGINE (OpenVM) proof under the zeroconf image's pinned key, then the
//! statement: that it is about the caller's T, and that it rests on a core chain the caller trusts. That is the core
//! verifier's policy (trusted core image and profile, lineage, genesis anchor, time-too-new, minimum work), the same
//! checks `zyncapp verify-zeroconf` made on a RISC Zero receipt, plus the whole profile (as the hints zAPP, review
//! H5). Engine proofs only: this module is new with the engine (engine/VERIFIER.md).
//!
//! ⚠ Valid NOW, not final: a conflicting transaction can still confirm first. The statement says nothing about the
//! mempool, and nothing about a T whose inputs come from unconfirmed parents (the guest refuses such a T).

use serde::{Deserialize, Serialize};
use swiftsync_verify::{anchor_id, hex, hex32, identity_matches, lineage_of, work_at_least, words, ChainState, EngineKey, Image,
                       Verdict, MAX_FUTURE_BLOCK_TIME};

/// The statement's kind word, as the guest commits it (prover/zyncapps/methods/zeroconf: KIND_ZYNC_ZEROCONF).
/// ⚠ The coin zAPP's guest commits the same word (KIND_ZYNC_COIN): the two are told apart by the image the proof
/// verifies under, never by this word (a finding for the guest review rounds, ROADMAP 4.2).
pub const KIND_ZEROCONF: u32 = 0x5A43_0002;

/// The guest's committed statement, field for field and in order: risc0's serde encoding of exactly this struct.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ZeroconfStatement {
    pub kind: u32,
    pub core_id: [u32; 8],
    pub lineage: [u8; 32],
    pub anchor_id: [u8; 32],
    pub tip_height: u32,
    /// internal byte order
    pub tip_hash: [u8; 32],
    pub cum_work: [u8; 32],
    pub max_time: u32,
    pub profile: u32,
    /// internal byte order
    pub txid: [u8; 32],
    pub wtxid: [u8; 32],
    pub fee: i64,
    pub weight: i64,
    pub sigops: i64,
}

impl ZeroconfStatement {
    pub fn txid_hex(&self) -> String {
        disp(&self.txid)
    }
    pub fn wtxid_hex(&self) -> String {
        disp(&self.wtxid)
    }
    pub fn tip_hex(&self) -> String {
        disp(&self.tip_hash)
    }
}

/// Every check with its result, the verdict, and (unless FAIL) the statement.
#[derive(Debug)]
pub struct ZeroconfReport {
    pub checks: Vec<(bool, String)>,
    pub verdict: Verdict,
    pub statement: Option<ZeroconfStatement>,
    /// the tip was compared with the caller's own header chain (`expect_tip`)
    pub tip_checked: bool,
}

impl ZeroconfReport {
    fn check(&mut self, ok: bool, what: impl Into<String>) -> bool {
        self.checks.push((ok, what.into()));
        ok
    }
}

/// A hash in display order (as explorers and `getblockhash` print it), from internal byte order.
fn disp(b: &[u8; 32]) -> String {
    b.iter().rev().map(|x| format!("{x:02x}")).collect()
}

/// Verifies a zeroconf proof file. `image_hex`: the zeroconf image id, and `key` its verifying key (pinned: the key,
/// not the image id, authenticates an engine proof); `core`: the core image the caller trusts (its id, profile,
/// network, genesis anchor, minimum work; an engine build, node format 2); `anchor`: a trusted state the core chain
/// was proved from instead of genesis (the verdict is then at best DEMO); `now`: the caller's clock; `txid`: the
/// transaction the caller asks about, as explorers print it; `expect_tip`: (height, hash) of the tip from the caller's
/// own header chain. OK needs the tip compared, genesis as the anchor, and the core chain at Core's minimum work and the
/// image's checkpoint; otherwise DEMO. A FAIL report carries no statement.
#[allow(clippy::too_many_arguments)]
pub fn verify_zeroconf(file: &[u8], image_hex: &str, key: &EngineKey, core: &Image, anchor: Option<&ChainState>, now: u64,
                       txid: &str, expect_tip: Option<(u32, &str)>) -> ZeroconfReport {
    let mut rep = ZeroconfReport { checks: Vec::new(), verdict: Verdict::Fail, statement: None, tip_checked: false };
    let Some(id) = hex32(image_hex) else {
        rep.check(false, format!("the zeroconf image id is 32 bytes of hex ({image_hex})"));
        return rep;
    };
    let journal = match engine_run(file, key, &id) {
        Ok(j) => {
            rep.check(true, format!("the engine proof verifies against the zeroconf image {image_hex} under the pinned key {}", hex(&key.pin)));
            j
        }
        Err(e) => {
            rep.check(false, format!("the engine proof verifies against the zeroconf image {image_hex}: {e}"));
            return rep;
        }
    };
    let z: ZeroconfStatement = match journal.decode() {
        Ok(z) => z,
        Err(e) => {
            rep.check(false, format!("the journal is a zeroconf statement ({e})"));
            return rep;
        }
    };
    rep.check(journal.bytes.len() == 4 * 210, format!("the journal is exactly one statement ({} bytes, 840 expected)", journal.bytes.len()));
    rep.check(z.kind == KIND_ZEROCONF, format!("a zeroconf statement (kind {:#x})", z.kind));
    // the caller's question: THIS transaction
    let asked = txid.trim().to_lowercase();
    rep.check(z.txid_hex() == asked, format!("the proven transaction is {} (you asked about {asked})", z.txid_hex()));
    // the core chain it rests on: the core verifier's policy
    let net = core.network;
    rep.check(words(&core.method_id()) == z.core_id, format!("rests on the trusted {net} core image {}", core.method_id_hex));
    rep.check(z.profile == core.profile, format!("core profile 0x{:x} == the image's 0x{:x} ({})", z.profile, core.profile,
                                                 swiftsync_verify::describe_profile(z.profile)));
    rep.check(z.lineage == [0u8; 32] || z.lineage == lineage_of(core.lineage), "the core lineage is zero or the image's");
    let (want, what) = match anchor {
        None => (hex32(core.genesis_anchor_hex).unwrap_or([0xff; 32]), format!("the {net} genesis")),
        Some(a) => (anchor_id(a), format!("the GIVEN anchor at height {} -- NOT genesis", a.height)),
    };
    rep.check(z.anchor_id == want, format!("the core chain is anchored at {what}"));
    rep.check(z.max_time as u64 <= now.saturating_add(MAX_FUTURE_BLOCK_TIME), "time-too-new: no core block more than 2 h ahead of this clock");
    if let Some((h, x)) = expect_tip {
        rep.tip_checked = rep.check(identity_matches(h, &x.to_lowercase(), z.tip_height, &z.tip_hash),
                                    format!("the tip is block {h} {x} (your header chain); the proof's tip is block {} {}", z.tip_height, z.tip_hex()));
    }
    let failed = rep.checks.iter().any(|(ok, _)| !ok);
    let enough = work_at_least(&z.cum_work, core.min_work_hex) && z.tip_height >= core.checkpoint_height;
    rep.checks.push((true, (if enough { "the core chain reaches Core's minimum work and the checkpoint" } else { "the core chain is below Core's minimum work or the checkpoint" }).to_string()));
    rep.verdict = if failed {
        Verdict::Fail
    } else if enough && rep.tip_checked && anchor.is_none() {
        Verdict::Ok
    } else {
        Verdict::Demo
    };
    if rep.verdict != Verdict::Fail {
        rep.statement = Some(z);
    } else {
        rep.tip_checked = false;
    }
    rep
}

fn engine_run(file: &[u8], key: &EngineKey, id: &[u8; 32]) -> Result<risc0_zkvm::Journal, String> {
    if !swiftsync_verify::is_engine_proof(file) {
        return Err("not an engine proof file".into());
    }
    swiftsync_verify::engine_proof_journal(file, key, id)
}
