//! The Utreexo roots zyncAPP's checks (docs/libzync/UTREEXO_ZAPP.md §6): what a Utreexo node (Floresta) checks before
//! it starts from proven roots instead of a hard-coded `AssumeUtreexoValue`.
//!
//! The final run's receipt proves: after block `height` (hash `block_hash`) of the chain that a core receipt `core_id`
//! proved valid from genesis (scripts checked), Floresta's accumulator holds `leaves` leaves with these `roots`. The
//! earlier runs and the core receipt are assumptions of the final receipt, so its verification covers them. What the
//! zyncAPP could not know (which core images to trust, the genesis anchor, minimum work, the clock, and that block
//! `height` is the node's own) is checked here, with the core verifier's own tables (swiftsync-verify as a library).
use serde::{Deserialize, Serialize};
use swiftsync_verify::{hex, hex32, identity_matches, work_at_least, EngineKey, Image, Journal, Verdict, MAX_FUTURE_BLOCK_TIME};

/// Tip mode (UTREEXO_ZAPP.md §10): the newest roots a few blocks below the tip, a chain of steps from a base.
#[cfg(feature = "tip")]
mod tip;
#[cfg(feature = "tip")]
pub use tip::*;

/// The utreexo (base walk) image id this checker trusts by default: none until the engine image is published at the
/// Stage 4 freeze (engine/CUTOVER.md C3), so a reader gives the id and key of the image they rebuilt. (00b2d1e8 was the
/// RISC Zero image; v0.27.1 checks it.)
pub const UTREEXO_IMAGE_ID: &str = "";
/// How [`UTREEXO_IMAGE_ID`] is labelled wherever it is printed, until it is pinned.
pub const UTREEXO_IMAGE_LABEL: &str = "no engine utreexo image is published yet";
/// Whether [`UTREEXO_IMAGE_ID`] is a reproducible build pinned in reproduce/LIBZYNC_IDS. While it is not, nobody can
/// check which code the id stands for, so a proof under it is at most DEMO ([`image_source`]).
pub const UTREEXO_IMAGE_PINNED: bool = false;

/// The networks a caller can name (the core verifier's `Image` table has these two).
pub const NETWORKS: [&str; 2] = ["mainnet", "signet"];

/// Where the utreexo image `id` comes from, as the report and the JSON say it.
pub const IMAGE_BUILTIN_UNPINNED: &str = "builtin-unpinned";
pub const IMAGE_BUILTIN_PINNED: &str = "builtin-pinned";
/// An id the caller named (`--image`): this checker does not know its code; trusting it is the caller's choice (e.g.
/// an image they rebuilt themselves).
pub const IMAGE_CALLER: &str = "caller";

/// [`IMAGE_BUILTIN_UNPINNED`], [`IMAGE_BUILTIN_PINNED`] or [`IMAGE_CALLER`] for the utreexo image `id`. Only the
/// first caps the verdict at DEMO.
pub fn image_source(id: &[u8; 32]) -> &'static str {
    match (hex32(UTREEXO_IMAGE_ID).as_ref() == Some(id), UTREEXO_IMAGE_PINNED) {
        (true, false) => IMAGE_BUILTIN_UNPINNED,
        (true, true) => IMAGE_BUILTIN_PINNED,
        (false, _) => IMAGE_CALLER,
    }
}

/// The statement's kind word ('ZU', version 1), as libzync::utreexo_roots::KIND_UTREEXO.
pub const KIND_UTREEXO: u32 = libzync::utreexo_roots::KIND_UTREEXO;

/// The guest's statement, field for field and in order (the guests' word encoding, hazync-journal).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct UtreexoStatement {
    pub kind: u32,
    pub self_id: [u32; 8],
    pub network: u32,
    pub height: u32,
    pub to: u32,
    pub runs: u32,
    pub state_out: [u8; 32],
    pub done: u32,
    pub block_hash: [u8; 32],
    pub leaves: u64,
    pub roots: Vec<[u8; 32]>,
    pub n_live: u64,
    pub core_id: [u32; 8],
    pub lineage: [u8; 32],
    pub anchor_id: [u8; 32],
    pub core_height: u32,
    pub tip_hash: [u8; 32],
    pub cum_work: [u8; 32],
    pub max_time: u32,
    pub profile: u32,
}

impl UtreexoStatement {
    /// The block hash as `getblockhash` prints it.
    pub fn block_hash_hex(&self) -> String {
        disp(&self.block_hash)
    }
    /// The roots as rustreexo's Display and Floresta's `acchashes!` write them (the bytes in order); an Empty root as
    /// "empty" (rustreexo's own Display). ⚠ In the journal (the raw statement) an Empty root is 32 zero bytes; a reader
    /// must load those as `BitcoinNodeHash::Empty`, never as the hash 00…00 (a different root, a different Stump).
    pub fn roots_hex(&self) -> Vec<String> {
        self.roots.iter().map(|r| if *r == [0u8; 32] { "empty".to_string() } else { hex(r) }).collect()
    }
    /// Floresta's `AssumeUtreexoValue` (floresta-chain src/pruned_utreexo/chainparams.rs) as Rust source, under a comment
    /// line naming the network it is for (the struct has no network field; a value is only ever valid for one network's
    /// chainparams). Floresta's `acchashes!` only takes hex literals (an "empty" literal does not parse), so with an
    /// Empty root the list is written out with an explicit `BitcoinNodeHash::Empty` instead.
    pub fn floresta_rust(&self, network: &str) -> String {
        let has_empty = self.roots.iter().any(|r| *r == [0u8; 32]);
        let roots = if has_empty {
            let items: Vec<String> = self.roots.iter().map(|r| if *r == [0u8; 32] { "        BitcoinNodeHash::Empty,".to_string() }
                                                    else { format!("        \"{}\".parse::<BitcoinNodeHash>().unwrap(),", hex(r)) }).collect();
            format!("vec![\n{}\n    ]", items.join("\n"))
        } else {
            let items: Vec<String> = self.roots_hex().iter().map(|r| format!("        \"{r}\",")).collect();
            format!("acchashes![\n{}\n    ]\n    .to_vec()", items.join("\n"))
        };
        format!("// {network}: the accumulator after block {}\nAssumeUtreexoValue {{\n    height: {},\n    block_hash: bhash!(\"{}\"),\n    roots: {roots},\n    leaves: {},\n}}",
                self.height, self.height, self.block_hash_hex(), self.leaves)
    }
}

pub(crate) fn disp(b: &[u8; 32]) -> String {
    b.iter().rev().map(|x| format!("{x:02x}")).collect()
}

pub struct UtreexoReport {
    pub checks: Vec<(bool, String)>,
    pub statement: Option<UtreexoStatement>,
    pub verdict: Verdict,
    /// `expect_at` was given and matched: block `height` is the node's
    pub identity_checked: bool,
    /// The network the CALLER named (the one checked), never read from the proof.
    pub network: String,
    /// [`image_source`] of the utreexo image checked against ([`IMAGE_BUILTIN_UNPINNED`] caps the verdict at DEMO);
    /// None when the image id given is not 64 hex digits.
    pub image_source: Option<&'static str>,
}

impl UtreexoReport {
    fn check(&mut self, ok: bool, what: impl Into<String>) -> bool {
        self.checks.push((ok, what.into()));
        ok
    }
}

/// Verifies the final run's proof on mainnet: [`verify_utreexo_on`]`("mainnet", ..)`, the default every zAPP takes
/// when the caller names no network.
pub fn verify_utreexo(receipt: &[u8], image_hex: &str, key: &EngineKey, core_image_hex: &str, now: u64,
                      expect_at: Option<(u32, &str)>, expect_tip: Option<(u32, &str)>) -> UtreexoReport {
    verify_utreexo_on("mainnet", receipt, image_hex, key, core_image_hex, now, expect_at, expect_tip)
}

/// Verifies the final run's proof for the network the CALLER names (`network`: "mainnet" or "signet"; a proof of
/// another network is FAIL, the proof never picks the network). `receipt`: an ENGINE (OpenVM) proof file of the utreexo
/// image `image_hex`, verified under its pinned `key` (engine/VERIFIER.md: the key, not the image id, authenticates it);
/// `core_image_hex`: the engine core image it rests on (node format 2: an engine zAPP proof can only rest on an engine
/// core proof, its child verified in the same VM); `now`: the checker's clock; `expect_at`: (H, hash at H as
/// `getblockhash` prints it) from the node's own header chain; `expect_tip`: the same for the core tip when it is above
/// H. The proof checks are here; everything after the journal is decoded is [`judge_verified_statement`]. A FAIL report
/// carries no statement. A RISC Zero receipt is checked with v0.27.1's verifier.
#[allow(clippy::too_many_arguments)]
pub fn verify_utreexo_on(network: &str, receipt: &[u8], image_hex: &str, key: &EngineKey, core_image_hex: &str, now: u64,
                         expect_at: Option<(u32, &str)>, expect_tip: Option<(u32, &str)>) -> UtreexoReport {
    let mut rep = UtreexoReport { checks: Vec::new(), statement: None, verdict: Verdict::Fail, identity_checked: false,
                                  network: network.to_string(), image_source: hex32(image_hex).as_ref().map(image_source) };
    let Some(img) = NETWORKS.contains(&network).then(|| Image::by_network(network)).flatten() else {
        rep.check(false, format!("the network is mainnet or signet ({network:?})"));
        return rep;
    };
    let Some(img) = img.with_method_id(&core_image_hex.to_lowercase()) else {
        rep.check(false, format!("the core image id is 64 hex digits ({core_image_hex})"));
        return rep;
    };
    let Some(id) = hex32(image_hex) else {
        rep.check(false, format!("the utreexo image id is 32 bytes of hex ({image_hex})"));
        return rep;
    };
    let Some(journal) = proven_journal(&mut rep.checks, receipt, &id, &format!("the utreexo image {image_hex}"), key) else {
        return rep;
    };
    let z: UtreexoStatement = match journal.decode() {
        Ok(z) => z,
        Err(e) => { rep.check(false, format!("the journal is a utreexo statement ({e})")); return rep; }
    };
    let receipt_ok = rep.checks.iter().all(|c| c.0);
    let mut s = judge_verified_statement(z, &id, img, now, expect_at, expect_tip);
    s.checks.splice(0..0, rep.checks);
    if !receipt_ok {
        // the journal is not proven: nothing of it may reach a caller
        s.verdict = Verdict::Fail;
    }
    if s.verdict == Verdict::Fail {
        s.statement = None;
        s.identity_checked = false;
    }
    s
}

/// The final run's journal and the check that proves it (pushed to `checks`): an ENGINE proof file of image `id`
/// verified under the pinned key. None: the proof did not verify, and its journal may not be read at all.
pub(crate) fn proven_journal(checks: &mut Vec<(bool, String)>, file: &[u8], id: &[u8; 32], what: &str, key: &EngineKey) -> Option<Journal> {
    let r = if swiftsync_verify::is_engine_proof(file) { swiftsync_verify::engine_proof_journal(file, key, id) }
            else { Err("not an engine proof file (a RISC Zero receipt is checked with v0.27.1's verifier)".to_string()) };
    match r {
        Ok(j) => {
            checks.push((true, format!("the engine proof verifies against {what} under the pinned key {}", hex(&key.pin))));
            Some(j)
        }
        Err(e) => {
            checks.push((false, format!("the engine proof verifies against {what}: {e}")));
            None
        }
    }
}

/// The verdict on a decoded statement, a pure function of it: `id` the utreexo image its receipt verified under, `img`
/// the trusted core image of the caller's network (its `network` is the one checked), `now`, `expect_at`, `expect_tip`
/// as in [`verify_utreexo_on`]. FAIL if any check fails; OK needs `expect_at` matched, the core tip matched (by
/// `expect_tip`, or T = H), Core's minimum work and the checkpoint, and a utreexo image that is not the unpinned
/// built-in one ([`image_source`]); otherwise DEMO. ⚠ It sees only the journal: its OK means nothing unless the
/// receipt was verified under `id` first, as [`verify_utreexo_on`] does (public so that tests can reach every branch).
pub fn judge_verified_statement(z: UtreexoStatement, id: &[u8; 32], img: &Image, now: u64, expect_at: Option<(u32, &str)>,
                        expect_tip: Option<(u32, &str)>) -> UtreexoReport {
    let net = img.network;
    let mut rep = UtreexoReport { checks: Vec::new(), statement: None, verdict: Verdict::Fail, identity_checked: false,
                                  network: net.to_string(), image_source: Some(image_source(id)) };
    rep.check(z.kind == KIND_UTREEXO, "a utreexo statement (kind 'ZU' 1)");
    rep.check(z.done == 1 && z.to == z.height, "the final run: it ended at H and checked the core receipt");
    // every earlier run was verified, inside the zkVM, under the id this statement commits: it must be this image
    rep.check(z.self_id == swiftsync_verify::words(id), "every run of the chain is this image (self_id)");
    rep.check(z.roots.len() as u32 == z.leaves.count_ones(), format!("{} roots for {} leaves (one per set bit)", z.roots.len(), z.leaves));
    // The one lineage check. A zero lineage: the core image proved 1..T alone, so every block carries its undo
    // commitment. A non-zero one (a handover, HANDOVER.md) is refused even when it is the image's own lineage
    // (`lineage_of(img.lineage)`): blocks before the handover were proved by another image. (Until 18.11.2026 a second
    // line also accepted the image's lineage; this check refused it first, so that line never decided anything.)
    rep.check(z.lineage == [0u8; 32], "one core image since genesis (every block 1..T carries the undo commitment)");
    match network_bits(net) {
        Some(b) => rep.check(z.network == b, format!("the proof is for {net}, the network asked for (its network word {}, {net}'s {b})", z.network)),
        None => rep.check(false, format!("the network is mainnet or signet ({net:?})")),
    };
    rep.check(swiftsync_verify::words(&img.method_id()) == z.core_id, format!("rests on the trusted {net} core image {}", img.method_id_hex));
    if !img.is_published() {
        rep.checks.push((true, swiftsync_verify::caller_image_note("core image", img.method_id_hex)));
    }
    rep.check(z.profile == img.profile, format!("core profile 0x{:x} == the {net} image's 0x{:x} ({})", z.profile, img.profile, swiftsync_verify::describe_profile(z.profile)));
    rep.check(hex32(img.genesis_anchor_hex) == Some(z.anchor_id), format!("anchored at the {net} genesis"));
    rep.check(z.max_time as u64 <= now.saturating_add(MAX_FUTURE_BLOCK_TIME), "time-too-new: no core block more than 2 h ahead of this clock");
    if let Some((h, x)) = expect_at {
        rep.identity_checked = rep.check(identity_matches(h, &x.to_lowercase(), z.height, &z.block_hash),
                                         format!("block {h} is {x} (your header chain); the roots are for block {} {}", z.height, disp(&z.block_hash)));
    }
    // a given tip is always compared, also when T = H (then it must name block H too)
    let tip_ok = match expect_tip {
        Some((h, x)) => rep.check(identity_matches(h, &x.to_lowercase(), z.core_height, &z.tip_hash),
                                  format!("the core tip is block {h} {x}; the core proof's tip is block {} {}", z.core_height, disp(&z.tip_hash))),
        None => z.core_height == z.height && rep.identity_checked,
    };
    let failed = rep.checks.iter().any(|(ok, _)| !ok);
    let enough = work_at_least(&z.cum_work, img.min_work_hex) && z.core_height >= img.checkpoint_height;
    rep.checks.push((true, (if enough { "the core chain reaches Core's minimum work and the checkpoint" } else { "the core chain is below Core's minimum work or the checkpoint" }).to_string()));
    let src = image_source(id);
    if src == IMAGE_BUILTIN_UNPINNED {
        rep.checks.push((true, format!("the utreexo image {} is the built-in one ({UTREEXO_IMAGE_LABEL}): at most DEMO", hex(id))));
    } else if src == IMAGE_CALLER {
        rep.checks.push((true, format!("the utreexo image {} is the one you named: this checker does not know its code (trust it only if you built it)", hex(id))));
    }
    rep.verdict = if failed { Verdict::Fail } else if enough && rep.identity_checked && tip_ok && src != IMAGE_BUILTIN_UNPINNED { Verdict::Ok } else { Verdict::Demo };
    rep.statement = Some(z);
    rep
}

/// libzync::facts::Network::bits for the network's name (the statement's `network` word); None for a network this
/// checker has no image for.
pub(crate) fn network_bits(net: &str) -> Option<u32> {
    match net {
        "mainnet" => Some(libzync::facts::Network::Mainnet.bits()),
        "signet" => Some(libzync::facts::Network::Signet.bits()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn floresta_source_form() {
        let z = UtreexoStatement { height: 3, leaves: 3, roots: vec![[1; 32], [0; 32]], ..Default::default() };
        assert_eq!(z.roots_hex()[1], "empty");
        let f = z.floresta_rust("mainnet");
        assert!(f.contains("leaves: 3,") && f.contains("BitcoinNodeHash::Empty,") && !f.contains("acchashes!") && !f.contains("\"empty\""), "{f}");
        let n = UtreexoStatement { height: 3, leaves: 3, roots: vec![[1; 32], [2; 32]], ..Default::default() };
        assert!(n.floresta_rust("signet").contains("acchashes![") && !n.floresta_rust("signet").contains("Empty"));
    }

    /// Bytes that are not an engine proof file: refused before any decoding (a RISC Zero receipt's bincode length
    /// prefix of 2^60, a file of zeros).
    #[test]
    fn hostile_bytes_are_refused() {
        let mut b = vec![0u8; 64];
        b[..8].copy_from_slice(&(1u64 << 60).to_le_bytes());
        let key = EngineKey { bytes: &[], pin: [0; 32] };
        for f in [&b[..], &vec![0u8; 4096][..]] {
            let r = verify_utreexo(f, &"ab".repeat(32), &key, &"cd".repeat(32), 1_800_000_000, None, None);
            assert_eq!(r.verdict, Verdict::Fail);
            assert!(r.statement.is_none());
        }
    }
}
