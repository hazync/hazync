//! zapp-corecheck's verdict paths (docs/libzync/ZAPP_TESTING.md §3.5, the cargo-mutants pass of 17.11.2026): the image a
//! caller names by the id of a built-in image is THAT image (its own lineage and parameters), not the network's current
//! image re-pointed at the id; a refused `expect` names the proof's tip.

use swiftsync_verify::{Image, Verdict};
use zapp_corecheck::*;

const ID_8888: &str = "8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4";
const H200: &str = "000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320";

#[test]
fn a_built_in_image_by_id_is_the_tables_own() {
    let table = Image::by_id("mainnet", ID_8888).unwrap();
    assert!(std::ptr::eq(image_for("mainnet", ID_8888, false, false).unwrap(), table));
    assert!(std::ptr::eq(image_for("mainnet", &ID_8888.to_uppercase(), false, false).unwrap(), table), "any case");
    // an id the table does not list: the network's current image re-pointed at it
    let other = "ab".repeat(32);
    let i = image_for("mainnet", &other, false, false).unwrap();
    let cur = Image::by_network("mainnet").unwrap();
    assert!(i.method_id_hex == other && i.lineage.len() == cur.lineage.len() && i.min_work_hex == cur.min_work_hex);
}

#[test]
fn a_refused_expect_names_the_proofs_tip() {
    let r = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../swiftsync-verify/vectors/mainnet_1_200_8888d59c.rcpt")).unwrap();
    let img = image_for("mainnet", ID_8888, false, false).unwrap();
    let c = verify_core(&r, img, false, None, 1_800_000_000, Some(&format!("200:{}", "00".repeat(32))));
    assert_eq!((c.report.verdict, c.tip_checked), (Verdict::Fail, false));
    let line = c.report.checks.iter().find(|x| !x.0).unwrap();
    assert!(line.1.contains(&format!("block 200 {H200}")), "{line:?}");
}
