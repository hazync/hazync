//! The zero-confirmation zyncAPP's check (`zapp_zeroconf`): a proof that transaction T is valid for the block after a
//! proven tip, and that every coin it spends is unspent at that tip (prover/zyncapps/methods/zeroconf, the guest;
//! engine/zapps/zeroconf, its fixture and refusal cases). A merchant learns it before T confirms.
//!
//! [`verify_zeroconf`] checks the zAPP's ENGINE (OpenVM) proof under the zeroconf image's pinned key, then the
//! statement: that it is about the caller's T, and that it rests on a core chain the caller trusts. That is the core
//! verifier's policy (trusted core image and profile, lineage, genesis anchor, time-too-new, minimum work), plus the whole profile (as the hints zAPP, review
//! H5). Engine proofs only: this module is new with the engine (engine/VERIFIER.md).
//!
//! What OK means: T is CONSENSUS-valid for the block after the proven tip (Core's transaction rules, scripts, coins
//! unspent at that tip, and within one block's weight and sigop-cost limits), at the tip the caller passed. It is NOT
//! relay policy (standardness, minimum fee, dust, RBF: a T can be consensus-valid and still not propagate), and NOT
//! final: a conflicting transaction can still confirm first. Pass your CURRENT tip (`expect_tip`): a proof against an
//! older block of your own chain is about that block, and misses every spend since. Nothing about a T whose inputs
//! come from unconfirmed parents (the guest refuses such a T).

use serde::{Deserialize, Serialize};
use swiftsync_verify::{anchor_id, hex, hex32, identity_matches, lineage_of, work_at_least, words, ChainState, EngineKey, Image,
                       Verdict, MAX_FUTURE_BLOCK_TIME};

/// The statement's kind word, as the guest commits it (prover/zyncapps/methods/zeroconf: KIND_ZYNC_ZEROCONF): 'Z' 'Z'
/// 1. Its own since 27.11.2026; before, the coin zAPP's 0x5A43_0002, so only the image id told the two apart.
pub const KIND_ZEROCONF: u32 = 0x5A5A_0001;

/// The guest's committed statement, field for field and in order: the guests' word encoding (hazync-journal) of exactly
/// this struct.
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
    verify_zeroconf_asked(file, image_hex, key, core, anchor, now, Asked { txid, wtxid: None }, expect_tip)
}

/// [`verify_zeroconf`] with the wtxid of the caller's bytes too (`asked.wtxid`).
#[allow(clippy::too_many_arguments)]
pub fn verify_zeroconf_asked(file: &[u8], image_hex: &str, key: &EngineKey, core: &Image, anchor: Option<&ChainState>,
                             now: u64, asked: Asked, expect_tip: Option<(u32, &str)>) -> ZeroconfReport {
    let mut rep = ZeroconfReport { checks: Vec::new(), verdict: Verdict::Fail, statement: None, tip_checked: false };
    let Some(id) = hex32(image_hex) else {
        rep.check(false, format!("the zeroconf image id is 32 bytes of hex ({image_hex})"));
        return rep;
    };
    // the trusted core image must name an image (review R1-6 #4): a network record with none is no image to rest on
    if !rep.check(core.has_image(), format!("the trusted {} core image record names an image", core.network)) {
        return rep;
    }
    let journal = match engine_run(file, key, &id) {
        Ok(j) => {
            rep.check(true, format!("the engine proof verifies against the zeroconf image {image_hex} under the pinned key {}", hex(&key.pin)));
            // no zeroconf image is built in until the Stage 4 freeze: every one is the caller's (review R3-4 M1)
            rep.checks.push((true, swiftsync_verify::caller_image_note("zeroconf image", image_hex)));
            j
        }
        Err(e) => {
            rep.check(false, format!("the engine proof verifies against the zeroconf image {image_hex}: {e}"));
            return rep;
        }
    };
    // through decode_journal: its whole-words check first (review R1-6 #3; unreachable after the proof, but cheap)
    let z: ZeroconfStatement = match swiftsync_verify::decode_journal(&journal) {
        Ok(z) => z,
        Err(e) => {
            rep.check(false, format!("the journal is a zeroconf statement ({e})"));
            return rep;
        }
    };
    rep.check(journal.bytes.len() == 4 * 210, format!("the journal is exactly one statement ({} bytes, 840 expected)", journal.bytes.len()));
    check_statement(rep, z, core, anchor, now, asked, expect_tip)
}

/// Core's consensus limits for one block (consensus/consensus.h): a transaction above either can never be included,
/// so it is never "valid for the next block" (review R2-6 #1: the guest commits T's weight and sigop cost, Core's
/// GetTransactionWeight and GetTransactionSigOpCost, which only the BLOCK rules bound).
pub const MAX_BLOCK_WEIGHT: i64 = 4_000_000;
pub const MAX_BLOCK_SIGOPS_COST: i64 = 80_000;
/// Core's relay policy limits (policy/policy.h), reported, not enforced: a T above them is valid but will not relay.
pub const MAX_STANDARD_TX_WEIGHT: i64 = 400_000;
pub const MAX_STANDARD_TX_SIGOPS_COST: i64 = 16_000;

/// What the caller asks about: the txid, as explorers print it, and optionally the wtxid of the bytes the caller holds
/// (review R2-6 #3: the txid alone fixes T but not its witness).
#[derive(Clone, Copy, Debug)]
pub struct Asked<'a> {
    pub txid: &'a str,
    pub wtxid: Option<&'a str>,
}

/// The statement's own checks, on a statement whose proof `rep` already holds the checks of: the kind, the caller's
/// T, T's weight and sigop cost against one block's limits, and the core chain it rests on (the core verifier's
/// policy). Public so the policy can be tested on a statement without a proof.
#[allow(clippy::too_many_arguments)]
pub fn check_statement(mut rep: ZeroconfReport, z: ZeroconfStatement, core: &Image, anchor: Option<&ChainState>, now: u64,
                       asked: Asked, expect_tip: Option<(u32, &str)>) -> ZeroconfReport {
    rep.check(z.kind == KIND_ZEROCONF, format!("a zeroconf statement (kind {:#x})", z.kind));
    // the caller's question: THIS transaction
    let want = asked.txid.trim().to_lowercase();
    rep.check(z.txid_hex() == want, format!("the proven transaction is {} (you asked about {want})", z.txid_hex()));
    if let Some(w) = asked.wtxid {
        let w = w.trim().to_lowercase();
        rep.check(z.wtxid_hex() == w, format!("the proven transaction's wtxid is {} (your bytes' wtxid {w})", z.wtxid_hex()));
    }
    rep.check((0..=MAX_BLOCK_WEIGHT).contains(&z.weight),
              format!("T's weight {} fits one block (Core's MAX_BLOCK_WEIGHT {MAX_BLOCK_WEIGHT})", z.weight));
    rep.check((0..=MAX_BLOCK_SIGOPS_COST).contains(&z.sigops),
              format!("T's sigop cost {} fits one block (Core's MAX_BLOCK_SIGOPS_COST {MAX_BLOCK_SIGOPS_COST})", z.sigops));
    let standard = z.weight <= MAX_STANDARD_TX_WEIGHT && z.sigops <= MAX_STANDARD_TX_SIGOPS_COST;
    rep.checks.push((true, if standard {
        format!("within Core's relay limits for size and sigops (weight <= {MAX_STANDARD_TX_WEIGHT}, sigop cost <= {MAX_STANDARD_TX_SIGOPS_COST}); relay policy is not otherwise checked")
    } else {
        format!("ABOVE Core's relay limits (weight <= {MAX_STANDARD_TX_WEIGHT}, sigop cost <= {MAX_STANDARD_TX_SIGOPS_COST}): consensus-valid, but nodes will not relay it")
    }));
    // the core chain it rests on: the core verifier's policy
    let net = core.network;
    rep.check(words(&core.method_id()) == z.core_id, format!("rests on the trusted {net} core image {}", core.method_id_hex));
    if !core.is_published() {
        rep.checks.push((true, swiftsync_verify::caller_image_note("core image", core.method_id_hex)));
    }
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

fn engine_run(file: &[u8], key: &EngineKey, id: &[u8; 32]) -> Result<swiftsync_verify::Journal, String> {
    if !swiftsync_verify::is_engine_proof(file) {
        return Err("not an engine proof file".into());
    }
    swiftsync_verify::engine_proof_journal(file, key, id)
}
