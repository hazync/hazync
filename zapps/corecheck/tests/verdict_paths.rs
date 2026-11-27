//! zapp-corecheck's verdict paths (docs/libzync/ZAPP_TESTING.md §3.5, the cargo-mutants pass of 17.11.2026): an image id
//! the caller names is the network's record re-pointed at it (its parameters); a refused `expect` names the proof's tip.

use swiftsync_verify::{Image, Verdict};
use zapp_corecheck::*;

const H200: &str = "000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320";
const V: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../swiftsync-verify/vectors/");

#[test]
fn an_image_id_is_the_networks_record_re_pointed() {
    let other = "ab".repeat(32);
    let i = image_for("mainnet", &other, false, false).unwrap();
    let cur = Image::by_network("mainnet").unwrap();
    assert!(i.method_id_hex == other && i.lineage.len() == cur.lineage.len() && i.min_work_hex == cur.min_work_hex);
    assert!(std::ptr::eq(i, image_for("mainnet", &other.to_uppercase(), false, false).unwrap()), "any case, cached");
}

#[test]
fn a_refused_expect_names_the_proofs_tip() {
    let rd = |f: &str| std::fs::read(format!("{V}{f}")).unwrap();
    let id = String::from_utf8(rd("mainnet_1_200_engine.image_id")).unwrap();
    let img = with_key(image_for("mainnet", id.trim(), false, false).unwrap(), &rd("mainnet_1_200_engine.vk"), None).unwrap();
    let c = verify_core(&rd("mainnet_1_200_engine.hze"), img, None, 1_800_000_000, Some(&format!("200:{}", "00".repeat(32))));
    assert_eq!((c.report.verdict, c.tip_checked), (Verdict::Fail, false));
    let line = c.report.checks.iter().find(|x| !x.0).unwrap();
    assert!(line.1.contains(&format!("block 200 {H200}")), "{line:?}");
}
