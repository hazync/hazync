//! The utreexo zAPP's ENGINE proofs in its verifier (engine/VERIFIER.md): the engine's base (roots after block 100)
//! and a tip step 100 -> 150 on it, both over keys-ch's core proof of mainnet 1..200, checked under their pinned keys
//! by the same statement checks as a RISC Zero receipt; then the refusals. `--features engine,tip`.
#![cfg(all(feature = "engine", feature = "tip"))]
use swiftsync_verify::{hex32, EngineKey, Verdict};
use zapp_utreexo::*;

const D: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/engine");
const BASE_ID: &str = "340f6675bafcdd893c12e0dfae0391043cc7fd8919b526ff756ec13d5d76cc95";
const TIP_ID: &str = "6ecf6f438938c0c63ed39daf089168fb607eefc806f4b28435d73d54eae09c1b";
const CORE_ID: &str = "c7fe0eb85d2d4c10532742191b059a2adcc3e24e22aa81c1e632e2ee718d2fc9";
const BASE_PIN: &str = "a966e0ebe4e542192436c292a0db4b1c1b4ac2f0208a3d985fd1f5563ecd730a";
const TIP_PIN: &str = "788f9ef42cf7431862be309450b1ad9f07558b9980a20d387bb1b4c470be4bbd";
// mainnet blocks as explorers print them
const H100: &str = "000000007bc154e0fa7ea32218a72fe2c1bb9f86cf8c9ebf9a715ed27fdb229a";
const H150: &str = "000000009ca75733b4cf527fe193b919201a2ed38c9e147a5665fdfade551f4d";
const H200: &str = "000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320";
const NOW: u64 = 1_800_000_000;

fn rd(n: &str) -> Vec<u8> {
    std::fs::read(format!("{D}/{n}")).unwrap_or_else(|e| panic!("{D}/{n}: {e}"))
}

fn show(checks: &[(bool, String)]) {
    for (ok, s) in checks {
        eprintln!("  [{}] {}", if *ok { "ok" } else { "FAIL" }, &s[..s.len().min(160)]);
    }
}

#[test]
fn engine_base_100() {
    let (file, vk) = (rd("base100.hze"), rd("base100.vk"));
    let key = EngineKey { bytes: &vk, pin: hex32(BASE_PIN).unwrap() };
    let base = |f: &[u8], id: &str, k: &EngineKey, at: Option<(u32, &str)>, tip: Option<(u32, &str)>| {
        verify_utreexo_with("mainnet", f, id, Some(k), Some(CORE_ID), NOW, at, tip)
    };
    let r = base(&file, BASE_ID, &key, Some((100, H100)), Some((200, H200)));
    show(&r.checks);
    assert!(r.checks.iter().all(|(ok, _)| *ok));
    assert_eq!(r.verdict, Verdict::Demo, "a chain of 200 blocks is below Core's minimum work");
    let z = r.statement.as_ref().unwrap();
    assert_eq!((z.height, z.core_height), (100, 200));
    assert!(r.identity_checked);
    let refused = |r: UtreexoReport, why: &str| {
        assert_eq!(r.verdict, Verdict::Fail, "{why}");
        assert!(r.statement.is_none(), "{why}: a FAIL report carries no statement");
    };
    // another block at 100, another core tip
    refused(base(&file, BASE_ID, &key, Some((100, H150)), Some((200, H200))), "another block 100");
    refused(base(&file, BASE_ID, &key, Some((100, H100)), Some((200, H150))), "another core tip");
    // the journal (the file's tail) changed
    let mut j = file.clone();
    let n = j.len() - 40;
    j[n] ^= 1;
    refused(base(&j, BASE_ID, &key, None, None), "journal changed");
    // a byte of the proof changed
    let mut p = file.clone();
    p[5000] ^= 1;
    refused(base(&p, BASE_ID, &key, None, None), "proof changed");
    // another image id, the wrong pin, another image's key
    refused(base(&file, TIP_ID, &key, None, None), "another image id");
    let bad = EngineKey { bytes: &vk, pin: hex32(TIP_PIN).unwrap() };
    refused(base(&file, BASE_ID, &bad, None, None), "a pin that is not the key's");
    let tvk = rd("step150.vk");
    let other = EngineKey { bytes: &tvk, pin: hex32(TIP_PIN).unwrap() };
    refused(base(&file, BASE_ID, &other, None, None), "the tip image's key for the base proof");
    // the tip step's proof where the base belongs (its own key: a real proof, but not a base statement of this image)
    refused(base(&rd("step150.hze"), TIP_ID, &other, None, None), "a tip proof as a base");
    // another core image trusted
    refused(verify_utreexo_with("mainnet", &file, BASE_ID, Some(&key), Some(TIP_ID), NOW, None, None), "another core image");
    // the wrong network
    refused(verify_utreexo_with("signet", &file, BASE_ID, Some(&key), Some(CORE_ID), NOW, None, None), "signet");
    // a RISC Zero verifier given the engine file
    refused(verify_utreexo_on("mainnet", &file, BASE_ID, Some(CORE_ID), NOW, None, None), "no key: read as a RISC Zero receipt");
}

#[test]
fn engine_tip_step_150() {
    let (file, vk) = (rd("step150.hze"), rd("step150.vk"));
    let key = EngineKey { bytes: &vk, pin: hex32(TIP_PIN).unwrap() };
    let tip = |f: &[u8], base: &str, at: Option<(u32, &str)>| {
        verify_utreexo_tip_with("mainnet", f, TIP_ID, Some(&key), base, Some(CORE_ID), NOW, at, Some((200, H200)))
    };
    let r = tip(&file, BASE_ID, Some((150, H150)));
    show(&r.report.checks);
    assert!(r.report.checks.iter().all(|(ok, _)| *ok));
    assert_eq!(r.report.verdict, Verdict::Demo);
    assert_eq!(r.report.statement.as_ref().unwrap().height, 150);
    assert!(r.tip.is_some());
    let refused = |r: TipReport, why: &str| {
        assert_eq!(r.report.verdict, Verdict::Fail, "{why}");
        assert!(r.report.statement.is_none() && r.tip.is_none(), "{why}");
    };
    refused(tip(&file, BASE_ID, Some((150, H100))), "another block 150");
    refused(tip(&file, TIP_ID, Some((150, H150))), "another base image");
    let mut j = file.clone();
    let n = j.len() - 40;
    j[n] ^= 1;
    refused(tip(&j, BASE_ID, None), "journal changed");
}
