//! The zeroconf zAPP's ENGINE proof in its verifier: the fixture's T (engine/zapps/zeroconf/fixture: Core v31.1
//! accepts it at 110) proved valid for block 111 on the regtest harness's tip 110, by the production zeroconf image
//! (keys-z-zeroconf3) with REAL children: the core proof of block 110 (keys-rtch, a base step on native prep's
//! state at 109) and the speculative chunk (engine/zapps/zeroconf/real_proofs.sh). Then the refusals.
use swiftsync_verify::{hex32, ChainState, EngineKey, Image, Verdict};
use zapp_zeroconf::*;

const D: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/engine");
/// keys-z-zeroconf3: the zeroconf guest with its own kind 0x5A5A0001 (27.11.2026)
const ZC_ID: &str = "1ee5f2d7a8d10a2a0da6c6de360b92002b526651d6cfae30abbf015b5ffd88bb";
const CORE_ID: &str = "2d7401595eee8ae196702fafee49057eaf1ce8627bf79dd2b04d320e2533dc4f";
/// the harness's block 110, as getblockhash prints it (scripts/zapp-regtest/oracle/snap_110.json, Core's own)
const TIP: &str = "2d5cc9c833f072d2744d26a9354e94111e77d05669b5004f7d68641c42ae9d53";
const TXID: &str = "6c00b169f8895e0a7027c2f74753041f979f44d15015d12310913a3f140dbf38";
const NOW: u64 = 1_800_000_000;

fn rd(n: &str) -> Vec<u8> {
    std::fs::read(format!("{D}/{n}")).unwrap_or_else(|e| panic!("{D}/{n}: {e}"))
}

/// The regtest harness's core image as an engine build: zapps/corecheck's REGTEST_IMAGE (tests only) with keys-rtch's
/// id and node format 2 (profile 0xf9, as the fixture's chunk and the prep states carry it). TESTS ONLY.
fn core() -> &'static Image {
    Box::leak(Box::new(Image {
        network: "regtest",
        method_id_hex: CORE_ID,
        profile: 0xf9,
        genesis_hash_hex: "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206",
        genesis_time: 1_296_688_602,
        genesis_bits: 0x207f_ffff,
        genesis_work: 2,
        min_work_hex: "0000000000000000000000000000000000000000000000000000000000000000",
        genesis_anchor_hex: "99509fc506d022f06dfd72bb146fc894718ce8047e69e10b201e1a78d4bccf63",
        checkpoint_height: 0,
        lineage: &[],
        engine_key: None,
    }))
}

#[test]
fn engine_zeroconf_t_at_110() {
    let (file, vk) = (rd("proof.hze"), rd("vk.bin"));
    let pin = hex32(String::from_utf8(rd("vk.sha256")).unwrap().trim()).unwrap();
    let key = EngineKey { bytes: &vk, pin };
    let anchor: ChainState = serde_json::from_slice(&rd("anchor_state_109.json")).unwrap();
    let t: serde_json::Value = serde_json::from_slice(&rd("T.json")).unwrap();
    assert_eq!(t["txid"], TXID);
    let run = |f: &[u8], id: &str, k: &EngineKey, img: &Image, a: Option<&ChainState>, txid: &str, tip: Option<(u32, &str)>| {
        verify_zeroconf(f, id, k, img, a, NOW, txid, tip)
    };
    let r = run(&file, ZC_ID, &key, core(), Some(&anchor), TXID, Some((110, TIP)));
    for (ok, s) in &r.checks {
        eprintln!("  [{}] {}", if *ok { "ok" } else { "FAIL" }, &s[..s.len().min(160)]);
    }
    assert!(r.checks.iter().all(|(ok, _)| *ok));
    assert_eq!(r.verdict, Verdict::Demo, "proved from a given anchor (109), not genesis: at best DEMO");
    assert!(r.tip_checked);
    let z = r.statement.as_ref().unwrap();
    assert_eq!((z.tip_height, z.fee, z.weight, z.sigops), (110, 10_000, 1_031, 1), "T's fee, weight and sigops (Core's)");
    assert_eq!(z.wtxid_hex(), t["wtxid"].as_str().unwrap());
    assert_eq!(z.kind, KIND_ZEROCONF);
    // without a tip from the caller's own chain: DEMO, never OK
    assert_eq!(run(&file, ZC_ID, &key, core(), Some(&anchor), TXID, None).verdict, Verdict::Demo);

    let refused = |r: ZeroconfReport, why: &str| {
        assert_eq!(r.verdict, Verdict::Fail, "{why}");
        assert!(r.statement.is_none(), "{why}: a FAIL report carries no statement");
    };
    // the question: another transaction
    refused(run(&file, ZC_ID, &key, core(), Some(&anchor), &TXID.replace('6', "7"), None), "another txid");
    // the chain: another tip, the genesis anchor claimed (the proof's is 109's), another anchor state
    refused(run(&file, ZC_ID, &key, core(), Some(&anchor), TXID, Some((110, &TIP.replace('d', "e")))), "another tip");
    refused(run(&file, ZC_ID, &key, core(), None, TXID, None), "genesis claimed: the proof rests on the anchor at 109");
    let mut a2 = anchor.clone();
    a2.height += 1;
    refused(run(&file, ZC_ID, &key, core(), Some(&a2), TXID, None), "another anchor state");
    // the trusted core image: another id, another profile (the RISC Zero build's, node format 1)
    let other = Box::leak(Box::new(Image { method_id_hex: ZC_ID, ..*core() }));
    refused(run(&file, ZC_ID, &key, other, Some(&anchor), TXID, None), "another core image");
    let node1 = Box::leak(Box::new(Image { profile: 0x79, ..*core() }));
    refused(run(&file, ZC_ID, &key, node1, Some(&anchor), TXID, None), "the core profile without node format 2");
    // a core record that names no image (review R1-6 #4): refused, not compared with a zero id
    let noimg = Box::leak(Box::new(Image { method_id_hex: "", ..*core() }));
    let r = run(&file, ZC_ID, &key, noimg, Some(&anchor), TXID, None);
    assert!(r.checks.iter().any(|(ok, s)| !ok && s.contains("names an image")), "{:?}", r.checks);
    refused(r, "a core record with no image");
    // the clock: a core block more than 2 h ahead
    refused(verify_zeroconf(&file, ZC_ID, &key, core(), Some(&anchor), 1_296_000_000, TXID, None), "time-too-new");
    // the proof: a journal bit, a proof byte, another image id, a pin not the key's, another image's key
    let mut j = file.clone();
    let n = j.len() - 60;
    j[n] ^= 1;
    refused(run(&j, ZC_ID, &key, core(), Some(&anchor), TXID, None), "journal changed");
    let mut p = file.clone();
    p[5000] ^= 1;
    refused(run(&p, ZC_ID, &key, core(), Some(&anchor), TXID, None), "proof changed");
    refused(run(&file, CORE_ID, &key, core(), Some(&anchor), TXID, None), "another image id");
    let bad = EngineKey { bytes: &vk, pin: [7u8; 32] };
    refused(run(&file, ZC_ID, &bad, core(), Some(&anchor), TXID, None), "a pin that is not the key's");
    // the same proof by the image before 27.11.2026 (keys-z-zeroconf2), which committed the coin zAPP's kind: a real
    // proof under its own key and image, refused for its kind alone
    let (ok_file, ok_vk) = (rd("old_kind/proof.hze"), rd("old_kind/vk.bin"));
    let old_key = EngineKey { bytes: &ok_vk, pin: { use sha2::Digest as _; sha2::Sha256::digest(&ok_vk).into() } };
    let r = run(&ok_file, "e1e5aaf58889dedcc2fa8f02496a772a49b2387f4031d4d46946151e44b32aff", &old_key, core(), Some(&anchor), TXID, None);
    assert!(r.checks.iter().any(|(ok, s)| !ok && s.contains("kind 0x5a430002")), "{:?}", r.checks);
    refused(r, "the old shared kind");
    // not an engine proof at all
    refused(run(&rd("T.json"), ZC_ID, &key, core(), Some(&anchor), TXID, None), "not an engine proof file");
}
