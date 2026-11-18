//! The utreexo zyncAPP's TIP statements (docs/libzync/UTREEXO_ZAPP.md §10): what a Floresta node checks before it
//! starts from the newest proven roots, a few blocks below the tip.
//!
//! A tip receipt proves: starting from a BASE (the utreexo image's walk from genesis to block `base_height`, which
//! rested on core image `base_core_id`), `steps` steps of the TIP image applied every block up to `height` to Floresta's
//! accumulator with rustreexo's `Stump::modify`, and block `height` (hash `block_hash`) lies on the chain that this
//! step's core receipt `core_id` proved valid from genesis to `core_height`. Every earlier step and the base are
//! assumptions of the newest receipt, so its verification covers them. What the guests could not know (which images to
//! trust, the anchor, minimum work, the clock, that block `height` is the node's own) is checked here.
use crate::{decode_receipt, disp, image_source, network_bits, UtreexoReport, UtreexoStatement, IMAGE_BUILTIN_UNPINNED, IMAGE_CALLER,
            NETWORKS, UTREEXO_IMAGE_LABEL};
use risc0_zkvm::sha::Digest;
use serde::{Deserialize, Serialize};
use swiftsync_verify::{hex, hex32, identity_matches, receipt_kind, work_at_least, Image, Verdict, MAX_FUTURE_BLOCK_TIME};

/// The tip statement's kind word ('ZU', version 2; version 1 is the base).
pub const KIND_UTREEXO_TIP: u32 = 0x5A55_0002;

/// The utreexo TIP image id this checker trusts by default: the reproducible container's build, pinned in
/// reproduce/LIBZYNC_IDS (`zyncapp-utreexo-tip`, 18.11.2026, two agreeing builds: hazync-coord and a laptop).
pub const UTREEXO_TIP_IMAGE_ID: &str = "ab6f7a26fe4cb13446c3d0b3ee352e6bf3508a897e0363a0a2ef6a559d51353e";
/// Whether [`UTREEXO_TIP_IMAGE_ID`] is pinned (a reproducible build). While it is not, a proof under it is at most DEMO.
pub const UTREEXO_TIP_IMAGE_PINNED: bool = true;

/// The tip guest's statement, field for field and in order (risc0 serde).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct TipStatement {
    pub kind: u32,
    pub self_id: [u32; 8],
    pub base_id: [u32; 8],
    pub network: u32,
    pub base_height: u32,
    pub base_core_id: [u32; 8],
    pub steps: u32,
    pub from: u32,
    pub height: u32,
    pub block_hash: [u8; 32],
    pub leaves: u64,
    pub roots: Vec<[u8; 32]>,
    pub core_id: [u32; 8],
    pub lineage: [u8; 32],
    pub anchor_id: [u8; 32],
    pub core_height: u32,
    pub tip_hash: [u8; 32],
    pub cum_work: [u8; 32],
    pub max_time: u32,
    pub profile: u32,
}

impl TipStatement {
    /// The proven value in the base statement's shape (the fields Floresta's `AssumeUtreexoValue` has, and the core
    /// tip), for the printing helpers [`UtreexoStatement::roots_hex`] and [`UtreexoStatement::floresta_rust`].
    /// `runs` = steps; `n_live` is not known in tip mode (0).
    pub fn as_value(&self) -> UtreexoStatement {
        UtreexoStatement { kind: self.kind, self_id: self.self_id, network: self.network, height: self.height, to: self.height,
                           runs: self.steps, done: 1, block_hash: self.block_hash, leaves: self.leaves, roots: self.roots.clone(),
                           core_id: self.core_id, lineage: self.lineage, anchor_id: self.anchor_id, core_height: self.core_height,
                           tip_hash: self.tip_hash, cum_work: self.cum_work, max_time: self.max_time, profile: self.profile,
                           ..Default::default() }
    }
}

/// [`IMAGE_BUILTIN_UNPINNED`], `builtin-pinned` or [`IMAGE_CALLER`] for a tip image id.
pub fn tip_image_source(id: &[u8; 32]) -> &'static str {
    match (hex32(UTREEXO_TIP_IMAGE_ID).as_ref() == Some(id), UTREEXO_TIP_IMAGE_PINNED) {
        (true, false) => IMAGE_BUILTIN_UNPINNED,
        (true, true) => crate::IMAGE_BUILTIN_PINNED,
        (false, _) => IMAGE_CALLER,
    }
}

/// The kind word of a receipt's journal (unverified: only to pick which statement to check it as).
pub fn journal_kind(receipt: &[u8]) -> Option<u32> {
    let r = decode_receipt(receipt).ok()?;
    r.journal.bytes.get(..4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
}

pub struct TipReport {
    /// checks, verdict, identity, network and the TIP image's source; `statement` = [`TipStatement::as_value`]
    pub report: UtreexoReport,
    pub tip: Option<TipStatement>,
}

/// Verifies a tip receipt for the network the CALLER names. `tip_image_hex` / `base_image_hex`: the tip and base
/// utreexo image ids ([`UTREEXO_TIP_IMAGE_ID`] / [`crate::UTREEXO_IMAGE_ID`] by default); the rest as
/// [`crate::verify_utreexo_on`]. A FAIL report carries no statement.
#[allow(clippy::too_many_arguments)]
pub fn verify_utreexo_tip_on(network: &str, receipt: &[u8], tip_image_hex: &str, base_image_hex: &str, core_image_hex: Option<&str>,
                             now: u64, expect_at: Option<(u32, &str)>, expect_tip: Option<(u32, &str)>) -> TipReport {
    let mut rep = UtreexoReport { checks: Vec::new(), statement: None, verdict: Verdict::Fail, identity_checked: false,
                                  network: network.to_string(), image_source: hex32(tip_image_hex).as_ref().map(tip_image_source) };
    let fail = |rep: UtreexoReport| TipReport { report: rep, tip: None };
    if !rep_check(&mut rep, !swiftsync_verify::dev_mode_env(), "RISC0_DEV_MODE is not set (unset it to verify)") {
        return fail(rep);
    }
    let Some(mut img) = NETWORKS.contains(&network).then(|| Image::by_network(network)).flatten() else {
        rep_check(&mut rep, false, format!("the network is mainnet or signet ({network:?})"));
        return fail(rep);
    };
    if let Some(h) = core_image_hex {
        match img.with_method_id(&h.to_lowercase()) { Some(i) => img = i, None => { rep_check(&mut rep, false, "--core-image is 64 hex digits"); return fail(rep); } }
    }
    let (Some(id), Some(base)) = (hex32(tip_image_hex), hex32(base_image_hex)) else {
        rep_check(&mut rep, false, format!("the tip and base image ids are 32 bytes of hex ({tip_image_hex}, {base_image_hex})"));
        return fail(rep);
    };
    if let Err(e) = receipt_kind(receipt) {
        rep_check(&mut rep, false, format!("the receipt is a single succinct or Groth16 proof ({e})"));
        return fail(rep);
    }
    let r: risc0_zkvm::Receipt = match decode_receipt(receipt) {
        Ok(r) => r,
        Err(e) => { rep_check(&mut rep, false, format!("the receipt decodes ({e})")); return fail(rep); }
    };
    rep_check(&mut rep, !matches!(r.inner, risc0_zkvm::InnerReceipt::Fake(_)), "the receipt is a real proof");
    let v = r.verify(Digest::from_bytes(id));
    rep_check(&mut rep, v.is_ok(), format!("the receipt verifies against the utreexo tip image {tip_image_hex}{}", v.err().map(|e| format!(": {e}")).unwrap_or_default()));
    let z: TipStatement = match r.journal.decode() {
        Ok(z) => z,
        Err(e) => { rep_check(&mut rep, false, format!("the journal is a utreexo tip statement ({e})")); return fail(rep); }
    };
    let receipt_ok = rep.checks.iter().all(|c| c.0);
    let mut s = judge_tip_statement(z, &id, &base, img, now, expect_at, expect_tip);
    s.report.checks.splice(0..0, rep.checks);
    if !receipt_ok {
        s.report.verdict = Verdict::Fail;
    }
    if s.report.verdict == Verdict::Fail {
        s.report.statement = None;
        s.report.identity_checked = false;
        s.tip = None;
    }
    s
}

fn rep_check(rep: &mut UtreexoReport, ok: bool, what: impl Into<String>) -> bool {
    rep.checks.push((ok, what.into()));
    ok
}

/// The verdict on a decoded tip statement (a pure function of it; its OK means nothing unless the receipt verified
/// under `id` first, as [`verify_utreexo_tip_on`] does). FAIL if any check fails; OK needs block H matched
/// (`expect_at`), the core tip matched (`expect_tip`, or T = H), minimum work and the checkpoint, and neither the tip
/// nor the base image the unpinned built-in one; otherwise DEMO.
pub fn judge_tip_statement(z: TipStatement, id: &[u8; 32], base: &[u8; 32], img: &Image, now: u64, expect_at: Option<(u32, &str)>,
                           expect_tip: Option<(u32, &str)>) -> TipReport {
    let net = img.network;
    let mut rep = UtreexoReport { checks: Vec::new(), statement: None, verdict: Verdict::Fail, identity_checked: false,
                                  network: net.to_string(), image_source: Some(tip_image_source(id)) };
    let core_words = swiftsync_verify::words(&img.method_id());
    rep_check(&mut rep, z.kind == KIND_UTREEXO_TIP, "a utreexo tip statement (kind 'ZU' 2)");
    // every earlier step was verified inside the zkVM under the self_id this statement commits: it must be this image
    rep_check(&mut rep, z.self_id == swiftsync_verify::words(&Digest::from_bytes(*id)), "every step of the chain is this tip image (self_id)");
    rep_check(&mut rep, z.base_id == swiftsync_verify::words(&Digest::from_bytes(*base)),
              format!("the chain starts from a walk of the trusted base image {}", hex(base)));
    rep_check(&mut rep, z.steps >= 1 && z.from < z.height && z.base_height <= z.from, format!("{} steps from the base at {} (the last from {} to {})", z.steps, z.base_height, z.from, z.height));
    rep_check(&mut rep, z.roots.len() as u32 == z.leaves.count_ones(), format!("{} roots for {} leaves (one per set bit)", z.roots.len(), z.leaves));
    match network_bits(net) {
        Some(b) => rep_check(&mut rep, z.network == b, format!("the proof is for {net}, the network asked for (its network word {}, {net}'s {b})", z.network)),
        None => rep_check(&mut rep, false, format!("the network is mainnet or signet ({net:?})")),
    };
    // the base's coin-state claims were bound by ITS core receipt: that image must be trusted too
    rep_check(&mut rep, z.base_core_id == core_words, format!("the base rested on the trusted {net} core image {}", img.method_id_hex));
    rep_check(&mut rep, z.core_id == core_words, format!("this step rests on the trusted {net} core image {}", img.method_id_hex));
    // the core verifier's lineage rule: a step needs no undo commitment, so a handover lineage of the image is fine
    rep_check(&mut rep, z.lineage == [0u8; 32] || z.lineage == swiftsync_verify::lineage_of(img.lineage), "the core lineage is zero or the core image's own");
    rep_check(&mut rep, z.profile == img.profile, format!("core profile 0x{:x} == the {net} image's 0x{:x} ({})", z.profile, img.profile, swiftsync_verify::describe_profile(z.profile)));
    rep_check(&mut rep, hex32(img.genesis_anchor_hex) == Some(z.anchor_id), format!("anchored at the {net} genesis"));
    rep_check(&mut rep, z.max_time as u64 <= now.saturating_add(MAX_FUTURE_BLOCK_TIME), "time-too-new: no core block more than 2 h ahead of this clock");
    if let Some((h, x)) = expect_at {
        rep.identity_checked = rep_check(&mut rep, identity_matches(h, &x.to_lowercase(), z.height, &z.block_hash),
                                         format!("block {h} is {x} (your header chain); the roots are for block {} {}", z.height, disp(&z.block_hash)));
    }
    let tip_ok = match expect_tip {
        Some((h, x)) => rep_check(&mut rep, identity_matches(h, &x.to_lowercase(), z.core_height, &z.tip_hash),
                                  format!("the core tip is block {h} {x}; the core proof's tip is block {} {}", z.core_height, disp(&z.tip_hash))),
        None => z.core_height == z.height && rep.identity_checked,
    };
    let failed = rep.checks.iter().any(|(ok, _)| !ok);
    let enough = work_at_least(&z.cum_work, img.min_work_hex) && z.core_height >= img.checkpoint_height;
    rep.checks.push((true, (if enough { "the core chain reaches Core's minimum work and the checkpoint" } else { "the core chain is below Core's minimum work or the checkpoint" }).to_string()));
    let (src, bsrc) = (tip_image_source(id), image_source(base));
    if src == IMAGE_BUILTIN_UNPINNED {
        rep.checks.push((true, format!("the tip image {} is the built-in one ({UTREEXO_IMAGE_LABEL}): at most DEMO", hex(id))));
    } else if src == IMAGE_CALLER {
        rep.checks.push((true, format!("the tip image {} is the one you named: this checker does not know its code (trust it only if you built it)", hex(id))));
    }
    if bsrc == IMAGE_BUILTIN_UNPINNED {
        rep.checks.push((true, format!("the base image {} is the built-in one ({UTREEXO_IMAGE_LABEL}): at most DEMO", hex(base))));
    } else if bsrc == IMAGE_CALLER {
        rep.checks.push((true, format!("the base image {} is the one you named: this checker does not know its code", hex(base))));
    }
    let pinned = src != IMAGE_BUILTIN_UNPINNED && bsrc != IMAGE_BUILTIN_UNPINNED;
    rep.verdict = if failed { Verdict::Fail } else if enough && rep.identity_checked && tip_ok && pinned { Verdict::Ok } else { Verdict::Demo };
    rep.statement = Some(z.as_value());
    TipReport { report: rep, tip: Some(z) }
}
