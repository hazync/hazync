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
use swiftsync_verify::{hex, hex32, identity_matches, lineage_of, receipt_kind, work_at_least, Image, Verdict, MAX_FUTURE_BLOCK_TIME, PROFILE_SIGNET};
use risc0_zkvm::sha::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// The hints image id this verifier trusts by default: the `zyncapp-hints` line of reproduce/LIBZYNC_IDS (the
/// reproducible build; reproduce/pin-libzync-ids.py sets it, and `hints_image_matches_the_published_id` checks it).
pub const HINTS_IMAGE_ID: &str = "11c72c44dd47a28f26a7d9b284fa10897ac85b08ecf20a34e87f5d120995ecfb";

/// The guest's statement, field for field and in order (risc0 serde).
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

fn disp(b: &[u8; 32]) -> String {
    b.iter().rev().map(|x| format!("{x:02x}")).collect()
}

/// Verifies a hints receipt and the file (and optionally the undo-hash file) it is for. `image_hex`: the hints image id
/// (`HINTS_IMAGE_ID` by default); `core_image_hex`: a core image to trust instead of the network's built-in one;
/// `now`: the verifier's clock; `expect`: (H, hash at H as `getblockhash` prints it) from the node's own header chain;
/// `expect_tip`: the same for the core tip; compared whenever given (also when the core tip is H).
#[allow(clippy::too_many_arguments)]
pub fn verify_hints(receipt: &[u8], file: &[u8], undo: Option<&[u8]>, image_hex: &str, core_image_hex: Option<&str>, now: u64,
                    expect: Option<(u32, &str)>, expect_tip: Option<(u32, &str)>) -> HintsReport {
    let mut rep = HintsReport { checks: Vec::new(), statement: None, verdict: Verdict::Fail, identity_checked: false };
    // as the core verify(): RISC0_DEV_MODE never makes this verifier accept anything (review H3)
    if !rep.check(!swiftsync_verify::dev_mode_env(), "RISC0_DEV_MODE is not set (unset it to verify)") {
        return rep;
    }
    let Some(id) = hex32(image_hex) else {
        rep.check(false, format!("the hints image id is 32 bytes of hex ({image_hex})"));
        return rep;
    };
    if let Err(e) = receipt_kind(receipt) {
        rep.check(false, format!("the receipt is a single succinct or Groth16 proof ({e})"));
        return rep;
    }
    let r: risc0_zkvm::Receipt = match bincode::deserialize(receipt) {
        Ok(r) => r,
        Err(e) => { rep.check(false, format!("the receipt decodes ({e})")); return rep; }
    };
    rep.check(!matches!(r.inner, risc0_zkvm::InnerReceipt::Fake(_)), "the receipt is a real proof");
    let v = r.verify(Digest::from_bytes(id));
    rep.check(v.is_ok(), format!("the receipt verifies against the hints image {image_hex}{}", v.err().map(|e| format!(": {e}")).unwrap_or_default()));
    let z: HintsStatement = match r.journal.decode() {
        Ok(z) => z,
        Err(e) => { rep.check(false, format!("the journal is a hints statement ({e})")); return rep; }
    };
    rep.check(z.kind == libzync::hints::KIND_HINTS, "a hints statement (kind 'ZH' 1)");
    rep.check(z.format == libzync::hints::FORMAT_BIP || z.format == libzync::hints::FORMAT_CRATE, format!("a known hints format ({})", z.format));
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
    }
    rep.check(z.undo_from == 1 && z.lineage == [0u8; 32], "every block 1..H carries the undo commitment (one core image since genesis)");
    // the core statement, as for a core proof
    let net = if z.profile & PROFILE_SIGNET != 0 { "signet" } else { "mainnet" };
    let Some(mut img) = Image::by_network(net) else {
        rep.check(false, "a built-in image for this network");
        return rep;
    };
    if let Some(h) = core_image_hex {
        match img.with_method_id(&h.to_lowercase()) { Some(i) => img = i, None => { rep.check(false, "--core-image is 64 hex digits"); return rep; } }
    }
    rep.check(swiftsync_verify::words(&img.method_id()) == z.core_id, format!("rests on the trusted {net} core image {}", img.method_id_hex));
    // the whole profile, as the core verifier compares it (review H5): network, scripts checked, coin log, trie, live hash
    rep.check(z.profile == img.profile, format!("core profile 0x{:x} == the image's 0x{:x} ({})", z.profile, img.profile, swiftsync_verify::describe_profile(z.profile)));
    rep.check(z.lineage == [0u8; 32] || z.lineage == lineage_of(img.lineage), "the core lineage is the image's");
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
    rep.verdict = if failed { Verdict::Fail } else if enough && rep.identity_checked && tip_ok { Verdict::Ok } else { Verdict::Demo };
    rep.statement = Some(z);
    rep
}

#[cfg(test)]
mod tests {
    use super::*;
    const H3: &str = "0000000082b5015589a3fdf2d4baff403e6f0be035a5d9742c1cae6295464449";
    fn rd(f: &str) -> Vec<u8> { std::fs::read(format!("{}/vectors/{f}", env!("CARGO_MANIFEST_DIR"))).unwrap() }
    fn now() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() }

    /// The real receipts (proved on CPU, 2026-11-14, re-proved by the published hints image 1894f12e, over core 46666b6f blocks 1..3).
    #[test]
    fn real_receipts() {
        let (r, f, u) = (rd("mainnet-3.receipt"), rd("mainnet-3.hints"), rd("mainnet-3.undo"));
        let ok = verify_hints(&r, &f, Some(&u), HINTS_IMAGE_ID, None, now(), Some((3, H3)), None);
        assert_eq!(ok.verdict, Verdict::Demo, "{:?}", ok.checks); // valid, but 3 blocks are below minimum work
        assert!(ok.identity_checked);
        let mut flip = f.clone();
        *flip.last_mut().unwrap() ^= 1;
        assert_eq!(verify_hints(&r, &flip, None, HINTS_IMAGE_ID, None, now(), Some((3, H3)), None).verdict, Verdict::Fail);
        assert_eq!(verify_hints(&r, &rd("mainnet-2.hints"), None, HINTS_IMAGE_ID, None, now(), Some((3, H3)), None).verdict, Verdict::Fail);
        assert_eq!(verify_hints(&r, &f, None, HINTS_IMAGE_ID, None, now(), Some((2, H3)), None).verdict, Verdict::Fail);
        assert_eq!(verify_hints(&r, &f, None, &"ab".repeat(32), None, now(), None, None).verdict, Verdict::Fail);
        // review H1: a 9-byte file claiming H = 2^32 - 1 is FAIL, not an abort
        let mut tiny = vec![0x55, 0x54, 0x58, 0x4f, 0x00];
        tiny.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(verify_hints(&r, &tiny, None, HINTS_IMAGE_ID, None, now(), Some((3, H3)), None).verdict, Verdict::Fail);
        // H = 2 under a core tip of 3: the tip is checked separately
        let r2 = rd("mainnet-2.receipt");
        let h2 = "000000006a625f06636b8bb6ac7b960a8d03705d1ace08b1a19da3fdcc99ddbd";
        let two = verify_hints(&r2, &rd("mainnet-2.hints"), None, HINTS_IMAGE_ID, None, now(), Some((2, h2)), Some((3, H3)));
        assert_eq!(two.verdict, Verdict::Demo, "{:?}", two.checks);
        assert_eq!(verify_hints(&r2, &rd("mainnet-2.hints"), None, HINTS_IMAGE_ID, None, now(), Some((2, h2)), Some((3, h2))).verdict, Verdict::Fail);
        // H = T = 3: a core tip given is compared too (review 11-16: it was ignored when T = H)
        assert_eq!(verify_hints(&r, &f, None, HINTS_IMAGE_ID, None, now(), Some((3, H3)), Some((3, h2))).verdict, Verdict::Fail);
        assert_eq!(verify_hints(&r, &f, None, HINTS_IMAGE_ID, None, now(), Some((3, H3)), Some((3, H3))).verdict, Verdict::Demo);
    }

    /// The hints zyncAPP's built-in id is the published one (reproduce/LIBZYNC_IDS, `zyncapp-hints` line), and the
    /// genesis anchors its guest pins are the core's table (review H2, H4).
    #[test]
    fn hints_image_matches_the_published_id() {
        let f = include_str!("../../../reproduce/LIBZYNC_IDS");
        let line = f.lines().find(|l| l.starts_with("zyncapp-hints mainnet ")).expect("reproduce/LIBZYNC_IDS has no zyncapp-hints line");
        let id = line.split_whitespace().skip_while(|w| *w != "METHOD_ID").nth(1).unwrap();
        assert_eq!(id, HINTS_IMAGE_ID, "HINTS_IMAGE_ID and reproduce/LIBZYNC_IDS disagree");
        let g = include_str!("../../../prover/zyncapps/methods/hints/src/main.rs");
        for net in ["mainnet", "signet"] {
            let a = Image::by_network(net).unwrap().genesis_anchor_hex;
            let c = if net == "mainnet" { "MAINNET_ANCHOR" } else { "SIGNET_ANCHOR" };
            assert!(g.contains(&format!("const {c}: [u8; 32] = hex32(\"{a}\");")), "the hints guest's {c} is not the {net} genesis anchor");
        }
    }
}
