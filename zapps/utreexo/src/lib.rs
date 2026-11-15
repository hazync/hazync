//! The Utreexo roots zyncAPP's checks (docs/libzync/UTREEXO_ZAPP.md §6): what a Utreexo node (Floresta) checks before
//! it starts from proven roots instead of a hard-coded `AssumeUtreexoValue`.
//!
//! The final run's receipt proves: after block `height` (hash `block_hash`) of the chain that a core receipt `core_id`
//! proved valid from genesis (scripts checked), Floresta's accumulator holds `leaves` leaves with these `roots`. The
//! earlier runs and the core receipt are assumptions of the final receipt, so its verification covers them. What the
//! zyncAPP could not know (which core images to trust, the genesis anchor, minimum work, the clock, and that block
//! `height` is the node's own) is checked here, with the core verifier's own tables (swiftsync-verify as a library).
use risc0_zkvm::sha::Digest;
use serde::{Deserialize, Serialize};
use swiftsync_verify::{hex, hex32, identity_matches, lineage_of, receipt_kind, work_at_least, Image, Verdict, MAX_FUTURE_BLOCK_TIME, PROFILE_SIGNET};

/// The utreexo image id this checker trusts by default. ⚠ A local build (hazync-coord, 15.11.2026, the prototype on
/// feat/zapp-utreexo), NOT a reproducible build in reproduce/LIBZYNC_IDS: nobody should rely on it yet.
pub const UTREEXO_IMAGE_ID: &str = "27bea7da7c9c2c528a0c9b6e6d22f5d06c5b2c87bcd9d58c360ffecaf5424fb9";
/// How [`UTREEXO_IMAGE_ID`] is labelled wherever it is printed, until it is pinned.
pub const UTREEXO_IMAGE_LABEL: &str = "local build, not reproducible, not published";

/// The largest receipt this checker decodes. A succinct receipt here is about 225 kB; the limit bounds what a hostile
/// file can make bincode reserve (bincode 1 reads a Vec length from the input and reserves before reading the items).
/// ⚠ The core verifier's own decode (swiftsync-verify `verify`, `bincode::deserialize` at src/lib.rs:463) has no such
/// bound; not changed here (core code).
pub const MAX_RECEIPT_BYTES: usize = 16 << 20;

/// `bincode::deserialize`'s encoding (fixed-width integers, trailing bytes allowed) with a byte limit.
pub fn decode_receipt(receipt: &[u8]) -> Result<risc0_zkvm::Receipt, String> {
    use bincode::Options;
    if receipt.len() > MAX_RECEIPT_BYTES {
        return Err(format!("{} bytes, over the {MAX_RECEIPT_BYTES}-byte limit", receipt.len()));
    }
    bincode::DefaultOptions::new().with_fixint_encoding().allow_trailing_bytes().with_limit(MAX_RECEIPT_BYTES as u64)
        .deserialize(receipt).map_err(|e| e.to_string())
}

/// The statement's kind word ('ZU', version 1), as libzync::utreexo_roots::KIND_UTREEXO.
pub const KIND_UTREEXO: u32 = libzync::utreexo_roots::KIND_UTREEXO;

/// The guest's statement, field for field and in order (risc0 serde).
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
    /// Floresta's `AssumeUtreexoValue` (floresta-chain src/pruned_utreexo/chainparams.rs) as Rust source. Floresta's
    /// `acchashes!` only takes hex literals (an "empty" literal does not parse), so with an Empty root the list is
    /// written out with an explicit `BitcoinNodeHash::Empty` instead.
    pub fn floresta_rust(&self) -> String {
        let has_empty = self.roots.iter().any(|r| *r == [0u8; 32]);
        let roots = if has_empty {
            let items: Vec<String> = self.roots.iter().map(|r| if *r == [0u8; 32] { "        BitcoinNodeHash::Empty,".to_string() }
                                                    else { format!("        \"{}\".parse::<BitcoinNodeHash>().unwrap(),", hex(r)) }).collect();
            format!("vec![\n{}\n    ]", items.join("\n"))
        } else {
            let items: Vec<String> = self.roots_hex().iter().map(|r| format!("        \"{r}\",")).collect();
            format!("acchashes![\n{}\n    ]\n    .to_vec()", items.join("\n"))
        };
        format!("AssumeUtreexoValue {{\n    height: {},\n    block_hash: bhash!(\"{}\"),\n    roots: {roots},\n    leaves: {},\n}}",
                self.height, self.block_hash_hex(), self.leaves)
    }

    /// "mainnet" or "signet", from the statement's network word.
    pub fn network_name(&self) -> &'static str {
        if self.network == network_bits("signet") { "signet" } else { "mainnet" }
    }
}

fn disp(b: &[u8; 32]) -> String {
    b.iter().rev().map(|x| format!("{x:02x}")).collect()
}

pub struct UtreexoReport {
    pub checks: Vec<(bool, String)>,
    pub statement: Option<UtreexoStatement>,
    pub verdict: Verdict,
    /// `expect_at` was given and matched: block `height` is the node's
    pub identity_checked: bool,
}

impl UtreexoReport {
    fn check(&mut self, ok: bool, what: impl Into<String>) -> bool {
        self.checks.push((ok, what.into()));
        ok
    }
}

/// Verifies the final run's receipt. `image_hex`: the utreexo image id ([`UTREEXO_IMAGE_ID`] by default);
/// `core_image_hex`: a core image to trust instead of the network's built-in one; `now`: the checker's clock;
/// `expect_at`: (H, hash at H as `getblockhash` prints it) from the node's own header chain; `expect_tip`: the same for
/// the core tip when it is above H. OK needs `expect_at` (and `expect_tip` when T > H), minimum work and the checkpoint;
/// without them a valid proof is DEMO.
pub fn verify_utreexo(receipt: &[u8], image_hex: &str, core_image_hex: Option<&str>, now: u64, expect_at: Option<(u32, &str)>,
                      expect_tip: Option<(u32, &str)>) -> UtreexoReport {
    let mut rep = UtreexoReport { checks: Vec::new(), statement: None, verdict: Verdict::Fail, identity_checked: false };
    if !rep.check(!swiftsync_verify::dev_mode_env(), "RISC0_DEV_MODE is not set (unset it to verify)") {
        return rep;
    }
    let Some(id) = hex32(image_hex) else {
        rep.check(false, format!("the utreexo image id is 32 bytes of hex ({image_hex})"));
        return rep;
    };
    if let Err(e) = receipt_kind(receipt) {
        rep.check(false, format!("the receipt is a single succinct or Groth16 proof ({e})"));
        return rep;
    }
    let r: risc0_zkvm::Receipt = match decode_receipt(receipt) {
        Ok(r) => r,
        Err(e) => { rep.check(false, format!("the receipt decodes ({e})")); return rep; }
    };
    rep.check(!matches!(r.inner, risc0_zkvm::InnerReceipt::Fake(_)), "the receipt is a real proof");
    let v = r.verify(Digest::from_bytes(id));
    rep.check(v.is_ok(), format!("the receipt verifies against the utreexo image {image_hex}{}", v.err().map(|e| format!(": {e}")).unwrap_or_default()));
    let z: UtreexoStatement = match r.journal.decode() {
        Ok(z) => z,
        Err(e) => { rep.check(false, format!("the journal is a utreexo statement ({e})")); return rep; }
    };
    rep.check(z.kind == KIND_UTREEXO, "a utreexo statement (kind 'ZU' 1)");
    rep.check(z.done == 1 && z.to == z.height, "the final run: it ended at H and checked the core receipt");
    // every earlier run was verified, inside the zkVM, under the id this statement commits: it must be this image
    rep.check(z.self_id == swiftsync_verify::words(&Digest::from_bytes(id)), "every run of the chain is this image (self_id)");
    rep.check(z.roots.len() as u32 == z.leaves.count_ones(), format!("{} roots for {} leaves (one per set bit)", z.roots.len(), z.leaves));
    rep.check(z.lineage == [0u8; 32], "one core image since genesis (every block 1..T carries the undo commitment)");
    let net = if z.profile & PROFILE_SIGNET != 0 { "signet" } else { "mainnet" };
    rep.check(z.network == network_bits(net), format!("the statement's network is the core's ({net})"));
    let Some(mut img) = Image::by_network(net) else {
        rep.check(false, "a built-in image for this network");
        return rep;
    };
    if let Some(h) = core_image_hex {
        match img.with_method_id(&h.to_lowercase()) { Some(i) => img = i, None => { rep.check(false, "--core-image is 64 hex digits"); return rep; } }
    }
    rep.check(swiftsync_verify::words(&img.method_id()) == z.core_id, format!("rests on the trusted {net} core image {}", img.method_id_hex));
    rep.check(z.profile == img.profile, format!("core profile 0x{:x} == the image's 0x{:x} ({})", z.profile, img.profile, swiftsync_verify::describe_profile(z.profile)));
    rep.check(z.lineage == [0u8; 32] || z.lineage == lineage_of(img.lineage), "the core lineage is the image's");
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
    rep.verdict = if failed { Verdict::Fail } else if enough && rep.identity_checked && tip_ok { Verdict::Ok } else { Verdict::Demo };
    rep.statement = Some(z);
    rep
}

/// libzync::facts::Network::bits for the network's name (the statement's `network` word).
fn network_bits(net: &str) -> u32 {
    if net == "signet" { libzync::facts::Network::Signet.bits() } else { libzync::facts::Network::Mainnet.bits() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rd(f: &str) -> Option<Vec<u8>> { std::fs::read(format!("{}/vectors/{f}", env!("CARGO_MANIFEST_DIR"))).ok() }
    fn now() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() }
    const H3: &str = "0000000082b5015589a3fdf2d4baff403e6f0be035a5d9742c1cae6295464449";
    const H2: &str = "000000006a625f06636b8bb6ac7b960a8d03705d1ace08b1a19da3fdcc99ddbd";

    #[test]
    fn floresta_source_form() {
        let z = UtreexoStatement { height: 3, leaves: 3, roots: vec![[1; 32], [0; 32]], ..Default::default() };
        assert_eq!(z.roots_hex()[1], "empty");
        let f = z.floresta_rust();
        assert!(f.contains("leaves: 3,") && f.contains("BitcoinNodeHash::Empty,") && !f.contains("acchashes!") && !f.contains("\"empty\""), "{f}");
        let n = UtreexoStatement { height: 3, leaves: 3, roots: vec![[1; 32], [2; 32]], ..Default::default() };
        assert!(n.floresta_rust().contains("acchashes![") && !n.floresta_rust().contains("Empty"));
    }

    #[test]
    fn hostile_receipt_bytes() {
        // a bincode Vec length of 2^60 at the front: refused by the limit, not reserved
        let mut b = vec![0u8; 64];
        b[..8].copy_from_slice(&(1u64 << 60).to_le_bytes());
        assert!(decode_receipt(&b).is_err());
        assert!(decode_receipt(&vec![0u8; MAX_RECEIPT_BYTES + 1]).unwrap_err().contains("limit"));
    }

    /// The real receipts (vectors/, proved on CPU): DEMO (3 blocks are below minimum work), and refused when the
    /// height, hash or image differ.
    #[test]
    fn real_receipts() {
        let Some(r3) = rd("mainnet-3.receipt") else { panic!("vectors/mainnet-3.receipt missing") };
        let ok = verify_utreexo(&r3, UTREEXO_IMAGE_ID, None, now(), Some((3, H3)), None);
        assert_eq!(ok.verdict, Verdict::Demo, "{:?}", ok.checks);
        assert!(ok.identity_checked);
        assert_eq!(verify_utreexo(&r3, UTREEXO_IMAGE_ID, None, now(), Some((2, H3)), None).verdict, Verdict::Fail);
        assert_eq!(verify_utreexo(&r3, UTREEXO_IMAGE_ID, None, now(), Some((3, H2)), None).verdict, Verdict::Fail);
        assert_eq!(verify_utreexo(&r3, &"ab".repeat(32), None, now(), Some((3, H3)), None).verdict, Verdict::Fail);
        // T = H: a given tip is still compared
        assert_eq!(verify_utreexo(&r3, UTREEXO_IMAGE_ID, None, now(), Some((3, H3)), Some((3, H3))).verdict, Verdict::Demo);
        assert_eq!(verify_utreexo(&r3, UTREEXO_IMAGE_ID, None, now(), Some((3, H3)), Some((3, H2))).verdict, Verdict::Fail);
        assert_eq!(verify_utreexo(&r3, UTREEXO_IMAGE_ID, None, now(), Some((3, H3)), Some((2, H2))).verdict, Verdict::Fail);
        let Some(r2) = rd("mainnet-2.receipt") else { panic!("vectors/mainnet-2.receipt missing") };
        assert_eq!(verify_utreexo(&r2, UTREEXO_IMAGE_ID, None, now(), Some((2, H2)), Some((3, H3))).verdict, Verdict::Demo);
        assert_eq!(verify_utreexo(&r2, UTREEXO_IMAGE_ID, None, now(), Some((2, H2)), Some((3, H2))).verdict, Verdict::Fail);
        if let Some(s) = rd("mainnet-3-split.receipt") {
            let v = verify_utreexo(&s, UTREEXO_IMAGE_ID, None, now(), Some((3, H3)), None);
            assert_eq!(v.verdict, Verdict::Demo, "{:?}", v.checks);
            assert_eq!(v.statement.as_ref().unwrap().runs, 2);
            assert_eq!(v.statement.unwrap().roots, ok.statement.unwrap().roots, "one run and two runs give the same roots");
        }
    }
}
