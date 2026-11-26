//! verify_core on an ENGINE core proof (engine/VERIFIER.md): the engine chain's proof of block 200, its image record
//! carrying the verifying key, the tip compared with an expected block. `--features engine`.
#![cfg(feature = "engine")]
use swiftsync_verify::{Image, Verdict};
use zapp_corecheck::*;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../swiftsync-verify/tests/data/engine200");
const TIP_200: &str = "000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320";

fn rd(n: &str) -> Vec<u8> {
    std::fs::read(format!("{DIR}/{n}")).unwrap_or_else(|e| panic!("{DIR}/{n}: {e}"))
}

fn image() -> &'static Image {
    let id = String::from_utf8(rd("image_id")).unwrap();
    Image::by_network("mainnet").unwrap().with_method_id(id.trim()).unwrap().with_engine_key(rd("vk.bin"))
}

#[test]
fn engine_core_proof_with_expect() {
    let (file, img) = (rd("proof.hze"), image());
    let c = verify_core(&file, img, true, None, 1_800_000_000, Some(&format!("200:{TIP_200}")));
    for (ok, s) in &c.report.checks {
        eprintln!("  [{}] {s}", if *ok { "ok" } else { "FAIL" });
    }
    assert!(c.report.checks.iter().all(|(ok, _)| *ok));
    assert_eq!(c.report.verdict, Verdict::Demo, "low work: at best demo");
    assert!(c.tip_checked, "the expected block was compared and matched");
    assert_eq!(c.image.method_id_hex, img.method_id_hex);
    // another block expected: FAIL, naming the proof's tip
    let c = verify_core(&file, img, true, None, 1_800_000_000, Some("200:00000000000000000000000000000000000000000000000000000000000000aa"));
    assert_eq!(c.report.verdict, Verdict::Fail);
    assert!(!c.tip_checked);
    // the network's RISC Zero record (no engine key): FAIL with the reason
    let c = verify_core(&file, Image::by_network("mainnet").unwrap(), false, None, 1_800_000_000, None);
    assert_eq!(c.report.verdict, Verdict::Fail);
    assert!(c.report.checks.iter().any(|(ok, s)| !ok && s.contains("no engine verifying key")));
}
