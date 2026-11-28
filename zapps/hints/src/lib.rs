//! The SwiftSync hints zyncAPP (docs/libzync/HINTS_ZAPP.md): what a node checks before it trusts a downloaded hints file.
//! A zAPP module (docs/libzync/ZYNCAPPS.md §9): it uses the core verifier `swiftsync_verify` as a library and is not
//! part of it.
//!
//!
//! The receipt proves: the file whose SHA-256 is `hints_sha256` is the hints file (hintsfile.md, in `format`) for block
//! `height` of the chain a core receipt `core_id` proved valid from genesis (scripts checked), and `undo_sha256` is the
//! SHA-256 of that chain's per-block U0 undo hashes 1..height. The core receipt is an assumption of this one, so this
//! receipt's verification covers it; what the zyncAPP could not know (which core images to trust, the genesis anchor,
//! the network's work and checkpoint, the clock, and that block `height` is the node's own) is checked here, as for a
//! core proof.
use swiftsync_verify::{hex, hex32, identity_matches, work_at_least, EngineKey, Image, Journal, Verdict, MAX_FUTURE_BLOCK_TIME, PROFILE_SIGNET};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

// No built-in hints image id: the engine image is published at the Stage 4 freeze (engine/CUTOVER.md C3); until then a
// reader gives the id and key of the image they rebuilt, and the core image it rests on. (11c72c44 was the RISC Zero
// image; v0.27.1 checks it.)

/// The guest's statement, field for field and in order (the guests' word encoding, hazync-journal).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HintsStatement {
    pub kind: u32,
    pub format: u32,
    pub height: u32,
    pub block_hash: [u8; 32],
    pub hints_sha256: [u8; 32],
    pub hints_len: u64,
    pub n_counted: u64,
    pub n_unspent: u64,
    pub unspent_value: u64,
    pub undo_from: u32,
    pub undo_sha256: [u8; 32],
    pub core_id: [u32; 8],
    pub lineage: [u8; 32],
    pub anchor_id: [u8; 32],
    pub core_height: u32,
    pub tip_hash: [u8; 32],
    pub cum_work: [u8; 32],
    pub max_time: u32,
    pub profile: u32,
}

pub struct HintsReport {
    pub checks: Vec<(bool, String)>,
    pub statement: Option<HintsStatement>,
    pub verdict: Verdict,
    /// `expect` was given and matched: block `height` is the node's
    pub identity_checked: bool,
}

impl HintsReport {
    fn check(&mut self, ok: bool, what: impl Into<String>) -> bool {
        self.checks.push((ok, what.into()));
        ok
    }
}

/// What the reader's NODE expects (review R3-4 L1, L2): the network it runs on, and the hints format it reads. Both are
/// the caller's, never the proof's: a genuine hints file for another network, or in the other bit order, is a FAIL.
#[derive(Clone, Copy, Debug)]
pub struct HintsAsk<'a> {
    /// "mainnet" or "signet": compared with the network the proof's core chain is on.
    pub network: &'a str,
    /// The format the node reads ([`libzync::hints::FORMAT_BIP`] or [`libzync::hints::FORMAT_CRATE`]): compared with the
    /// proven file's. None: not compared, and the verdict is at most DEMO (an OK file in the other bit order would be
    /// misread, and the SwiftSync sync it was meant to save would fail at its end).
    pub format: Option<u32>,
}

impl Default for HintsAsk<'_> {
    fn default() -> Self {
        HintsAsk { network: "mainnet", format: None }
    }
}

/// A format's name as the CLI flag spells it.
pub fn format_name(f: u32) -> &'static str {
    match f { libzync::hints::FORMAT_BIP => "bip", libzync::hints::FORMAT_CRATE => "crate", _ => "unknown" }
}

/// The CLI / wasm spelling of a format ("bip" or "crate", or the number 1 / 2).
pub fn parse_format(s: &str) -> Option<u32> {
    match s { "bip" | "1" => Some(libzync::hints::FORMAT_BIP), "crate" | "2" => Some(libzync::hints::FORMAT_CRATE), _ => None }
}

fn disp(b: &[u8; 32]) -> String {
    b.iter().rev().map(|x| format!("{x:02x}")).collect()
}

/// Verifies a hints proof and the file (and optionally the undo-hash file) it is for. `receipt`: an ENGINE (OpenVM)
/// proof file of the hints image `image_hex`, verified under its pinned `key` (engine/VERIFIER.md: the key, not the image
/// id, authenticates it); `core_image_hex`: the engine core image it rests on (node format 2: an engine zAPP proof can
/// only rest on an engine core proof, its child verified in the same VM); `now`: the verifier's clock; `expect`: (H,
/// hash at H as `getblockhash` prints it) from the node's own header chain; `expect_tip`: the same for the core tip;
/// compared whenever given (also when the core tip is H); `ask`: the node's network and the format it reads. A RISC
/// Zero receipt is checked with v0.27.1's verifier.
#[allow(clippy::too_many_arguments)]
pub fn verify_hints(receipt: &[u8], file: &[u8], undo: Option<&[u8]>, image_hex: &str, key: &EngineKey, core_image_hex: &str,
                    ask: &HintsAsk, now: u64, expect: Option<(u32, &str)>, expect_tip: Option<(u32, &str)>) -> HintsReport {
    let mut rep = HintsReport { checks: Vec::new(), statement: None, verdict: Verdict::Fail, identity_checked: false };
    let Some(id) = hex32(image_hex) else {
        rep.check(false, format!("the hints image id is 32 bytes of hex ({image_hex})"));
        return rep;
    };
    let journal = match engine_run(receipt, key, &id) {
        Ok(j) => {
            rep.check(true, format!("the engine proof verifies against the hints image {image_hex} under the pinned key {}", hex(&key.pin)));
            // no hints image is built in until the Stage 4 freeze: every one is the caller's (review R3-4 M1)
            rep.checks.push((true, swiftsync_verify::caller_image_note("hints image", image_hex)));
            j
        }
        Err(e) => { rep.check(false, format!("the engine proof verifies against the hints image {image_hex}: {e}")); return rep; }
    };
    let z: HintsStatement = match journal.decode() {
        Ok(z) => z,
        Err(e) => { rep.check(false, format!("the journal is a hints statement ({e})")); return rep; }
    };
    rep.check(z.kind == libzync::hints::KIND_HINTS, "a hints statement (kind 'ZH' 1)");
    rep.check(z.format == libzync::hints::FORMAT_BIP || z.format == libzync::hints::FORMAT_CRATE, format!("a known hints format ({})", z.format));
    let format_ok = match ask.format {
        Some(f) => rep.check(z.format == f, format!("the file is in the format your node reads ({}, {}; the file is {}, {})",
                                                      format_name(f), f, format_name(z.format), z.format)),
        None => { rep.checks.push((true, format!("the file's format ({}, {}) was not compared with the one your node reads: at most DEMO",
                                                 format_name(z.format), z.format))); false }
    };
    let fs: [u8; 32] = Sha256::digest(file).into();
    rep.check(fs == z.hints_sha256 && file.len() as u64 == z.hints_len,
              format!("the file is the proven one (sha256 {}, {} bytes; proven {}, {} bytes)", hex(&fs), file.len(), hex(&z.hints_sha256), z.hints_len));
    // decoded only once it is the proven file (review H1): a hostile header can then not reach the decoder at all
    if fs == z.hints_sha256 && file.len() as u64 == z.hints_len {
        let dec = libzync::hints::decode_file(file, z.format);
        rep.check(dec.as_ref().is_some_and(|(h, l)| *h == z.height && l.iter().map(|x| x.len() as u64).sum::<u64>() == z.n_unspent),
                  format!("the file decodes (strictly) to blocks 1..{} and {} unspent indices", z.height, z.n_unspent));
    }
    if let Some(u) = undo {
        let us: [u8; 32] = Sha256::digest(u).into();
        rep.check(u.len() == 32 * z.height as usize && us == z.undo_sha256, format!("the undo-hash file is the proven one (sha256 {})", hex(&z.undo_sha256)));
    } else {
        // review R3-4 L3: say so, so an OK is not read as covering the undo hashes too
        rep.checks.push((true, format!("no undo-hash file given: the undo hashes (proven sha256 {}) were not checked", hex(&z.undo_sha256))));
    }
    rep.check(z.undo_from == 1 && z.lineage == [0u8; 32], "every block 1..H carries the undo commitment (one core image since genesis)");
    // the core statement, as for a core proof, on the network the CALLER's node runs (review R3-4 L1)
    let net = if z.profile & PROFILE_SIGNET != 0 { "signet" } else { "mainnet" };
    if !rep.check(net == ask.network, format!("the proof's core chain is {net}; your node is on {}", ask.network)) {
        return rep;
    }
    let Some(img) = Image::by_network(net).and_then(|i| i.with_method_id(&core_image_hex.to_lowercase())) else {
        rep.check(false, format!("the core image id is 64 hex digits ({core_image_hex})"));
        return rep;
    };
    rep.check(swiftsync_verify::words(&img.method_id()) == z.core_id, format!("rests on the trusted {net} core image {}", img.method_id_hex));
    if !img.is_published() {
        rep.checks.push((true, swiftsync_verify::caller_image_note("core image", img.method_id_hex)));
    }
    // the whole profile, as the core verifier compares it (review H5): network, scripts checked, coin log, trie, live hash
    rep.check(z.profile == img.profile, format!("core profile 0x{:x} == the image's 0x{:x} ({})", z.profile, img.profile, swiftsync_verify::describe_profile(z.profile)));
    rep.check(hex32(img.genesis_anchor_hex) == Some(z.anchor_id), format!("anchored at the {net} genesis"));
    rep.check(z.max_time as u64 <= now.saturating_add(MAX_FUTURE_BLOCK_TIME), "time-too-new: no core block more than 2 h ahead of this clock");
    if let Some((h, x)) = expect {
        rep.identity_checked = rep.check(identity_matches(h, &x.to_lowercase(), z.height, &z.block_hash),
                                         format!("block {h} is {x} (your header chain); the file is for block {} {}", z.height, disp(&z.block_hash)));
    }
    // a core tip given is ALWAYS compared, also when the core tip is H itself (review 11-16: it was ignored then, so a
    // wrong --expect-tip passed); when T = H the expect for H covers the tip if no tip is given
    let tip_ok = match expect_tip {
        Some((h, x)) => rep.check(identity_matches(h, &x.to_lowercase(), z.core_height, &z.tip_hash),
                                  format!("the core tip is block {h} {x}")),
        None => z.core_height == z.height && rep.identity_checked,
    };
    let failed = rep.checks.iter().any(|(ok, _)| !ok);
    let enough = work_at_least(&z.cum_work, img.min_work_hex) && z.core_height >= img.checkpoint_height;
    rep.checks.push((true, (if enough { "the core chain reaches Core's minimum work and the checkpoint" } else { "the core chain is below Core's minimum work or the checkpoint" }).to_string()));
    rep.verdict = if failed { Verdict::Fail } else if enough && rep.identity_checked && tip_ok && format_ok { Verdict::Ok } else { Verdict::Demo };
    rep.statement = Some(z);
    rep
}

fn engine_run(file: &[u8], key: &EngineKey, id: &[u8; 32]) -> Result<Journal, String> {
    if !swiftsync_verify::is_engine_proof(file) {
        return Err("not an engine proof file".into());
    }
    swiftsync_verify::engine_proof_journal(file, key, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The genesis anchors the hints guest pins are the core's table (review H2, H4).
    #[test]
    fn the_guests_genesis_anchors_are_the_cores() {
        let g = include_str!("../../../prover/zyncapps/methods/hints/src/main.rs");
        for net in ["mainnet", "signet"] {
            let a = Image::by_network(net).unwrap().genesis_anchor_hex;
            let c = if net == "mainnet" { "MAINNET_ANCHOR" } else { "SIGNET_ANCHOR" };
            assert!(g.contains(&format!("const {c}: [u8; 32] = hex32(\"{a}\");")), "the hints guest's {c} is not the {net} genesis anchor");
        }
    }
}
