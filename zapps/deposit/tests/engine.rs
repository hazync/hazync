//! The deposit zAPP on the ENGINE chain (engine/VERIFIER.md, roadmap 2.5), core proof alone: a bundle for block 170's
//! Satoshi-to-Hal transaction built from the engine's proof of mainnet 1..200 and ITS history (node format 2), checked
//! with the caller's expectations. (Bundles bound to a header-chain proof wait for the header zAPP's engine proofs in
//! the verifier.) `--features engine`.
#![cfg(feature = "engine")]
use swiftsync_verify::*;
use zapp_deposit::*;
use swiftsync_verify::EngineKey;

const E: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../swiftsync-verify/tests/data/engine200");
const T170: &str = "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16";

fn rd(p: String) -> Vec<u8> {
    std::fs::read(&p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn txid(display: &str) -> [u8; 32] {
    let mut t = hex32(display).unwrap();
    t.reverse();
    t
}

fn image() -> &'static Image {
    let id = String::from_utf8(rd(format!("{E}/image_id"))).unwrap();
    Image::by_network("mainnet").unwrap().with_method_id(id.trim()).unwrap().with_engine_key(rd(format!("{E}/vk.bin")))
}

#[test]
fn deposit_on_the_engine_chain() {
    let (core, hist) = (rd(format!("{E}/proof.hze")), rd(format!("{E}/history.bin")));
    let raw_hex = String::from_utf8(rd(format!("{}/../../experimental/libzync-bench/deposit/block_170_raw.txt", env!("CARGO_MANIFEST_DIR")))).unwrap();
    let raw: Vec<u8> = (0..raw_hex.trim().len()).step_by(2).map(|i| u8::from_str_radix(&raw_hex.trim()[i..i + 2], 16).unwrap()).collect();
    let b = make_bundle("mainnet", &core, &[], Some(&hist), &raw, 170, &txid(T170), &[], None).unwrap();
    let ask = |vout: Option<u32>, amount: Option<u64>| DepositAsk {
        txid: txid(T170), vout, amount, min_depth: 31, allow_demo: true, require_identity: false, ..Default::default()
    };
    let r = verify_deposit(&b, &ask(Some(0), Some(1_000_000_000)), "mainnet", image(), "", 1_800_000_000);
    for (ok, s) in &r.checks {
        eprintln!("  [{}] {}", if *ok { "ok" } else { "FAIL" }, &s[..s.len().min(150)]);
    }
    assert!(r.checks.iter().all(|(ok, _)| *ok), "every check");
    assert_eq!(r.verdict, Verdict::Demo, "core proof below minimum work");
    assert!(r.statement.is_some(), "allow_demo: the statement is returned");
    // refusals: another amount; deeper than the chain; a bundle whose history is the RISC Zero (format 1) run's
    assert_eq!(verify_deposit(&b, &ask(Some(0), Some(1_000_000_001)), "mainnet", image(), "", 1_800_000_000).verdict, Verdict::Fail);
    let deep = DepositAsk { min_depth: 32, ..ask(Some(0), None) };
    assert_eq!(verify_deposit(&b, &deep, "mainnet", image(), "", 1_800_000_000).verdict, Verdict::Fail, "170..200 is 31 blocks");
    let old = rd(format!("{}/../../experimental/libzync-bench/deposit/history_0_200.bin", env!("CARGO_MANIFEST_DIR")));
    let e = make_bundle("mainnet", &core, &[], Some(&old), &raw, 170, &txid(T170), &[], None);
    eprintln!("  bundle from the RISC Zero run's history: {:?}", e.as_ref().err());
}

/// Bound to the header-chain zAPP's ENGINE proof of headers 0..4,031 (zapps/headers/tests/data/engine4031): the core tip
/// (block 200) placed in that chain by its MMR path (`zyncapp headers-mmr-path`), so the deposit's depth counts the
/// proof-of-work headers above the core tip too.
#[test]
fn deposit_bound_to_engine_headers() {
    let h = concat!(env!("CARGO_MANIFEST_DIR"), "/../headers/tests/data/engine4031");
    let hrd = |n: &str| rd(format!("{h}/{n}"));
    let htxt = |n: &str| String::from_utf8(hrd(n)).unwrap();
    let (core, hist) = (rd(format!("{E}/proof.hze")), rd(format!("{E}/history.bin")));
    let raw_hex = String::from_utf8(rd(format!("{}/../../experimental/libzync-bench/deposit/block_170_raw.txt", env!("CARGO_MANIFEST_DIR")))).unwrap();
    let raw: Vec<u8> = (0..raw_hex.trim().len()).step_by(2).map(|i| u8::from_str_radix(&raw_hex.trim()[i..i + 2], 16).unwrap()).collect();
    let path: Vec<[u8; 32]> = htxt("mmr_path_200_tip4031.txt").lines().map(|l| hex32(l.trim()).unwrap()).collect();
    let b = make_bundle("mainnet", &core, &[hrd("proof.hze")], Some(&hist), &raw, 170, &txid(T170), &path, None).unwrap();
    let (vk, pin) = (hrd("vk.bin"), hex32(htxt("vk.sha256").trim()).unwrap());
    let key = EngineKey { bytes: &vk, pin };
    let hid = htxt("image_id").trim().to_string();
    let ask = DepositAsk { txid: txid(T170), vout: Some(0), amount: Some(1_000_000_000), min_depth: 3862, allow_demo: true,
                           require_identity: false, ..Default::default() };
    let r = verify_deposit_with(&b, &ask, "mainnet", image(), &hid, Some(&key), 1_800_000_000);
    for (ok, s) in r.checks.iter().filter(|(_, s)| !s.starts_with("core: ")) {
        eprintln!("  [{}] {}", if *ok { "ok" } else { "FAIL" }, &s[..s.len().min(160)]);
    }
    assert!(r.checks.iter().all(|(ok, _)| *ok));
    assert_eq!(r.verdict, Verdict::Demo);
    // 170..4,031 = 3,862 confirmations: one more is refused; and without the key the header run is refused
    let deeper = DepositAsk { min_depth: 3863, ..ask.clone() };
    assert_eq!(verify_deposit_with(&b, &deeper, "mainnet", image(), &hid, Some(&key), 1_800_000_000).verdict, Verdict::Fail);
    assert_eq!(verify_deposit(&b, &ask, "mainnet", image(), &hid, 1_800_000_000).verdict, Verdict::Fail);
}
