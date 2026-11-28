//! The hints zAPP's ENGINE proof in its verifier (engine/VERIFIER.md): keys-z-hints's proof for H = 200 over keys-ch's
//! core proof of mainnet 1..200 (engine/results/zapp_hints; `hzengine wasm-inputs` wrote the file, key and pin), with
//! the hints and undo-hash files the host derived; then the refusals.
use swiftsync_verify::{hex32, EngineKey, Verdict};
use zapp_hints::*;

const D: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/engine");
const CORE_ID: &str = "c7fe0eb85d2d4c10532742191b059a2adcc3e24e22aa81c1e632e2ee718d2fc9";
const H200: &str = "000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320";
/// block 200's hash with its last digit changed: not block 200
const NOT_H200: &str = "000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef321";
const NOW: u64 = 1_800_000_000;

fn rd(n: &str) -> Vec<u8> {
    std::fs::read(format!("{D}/{n}")).unwrap_or_else(|e| panic!("{D}/{n}: {e}"))
}

fn txt(n: &str) -> String {
    String::from_utf8(rd(n)).unwrap().trim().to_string()
}

#[test]
fn engine_hints_200() {
    let (file, vk, id) = (rd("proof.hze"), rd("vk.bin"), txt("image_id"));
    let (hints, undo) = (rd("mainnet-200.hints"), rd("mainnet-200.undo"));
    let key = EngineKey { bytes: &vk, pin: hex32(&txt("vk.sha256")).unwrap() };
    let run = |f: &[u8], h: &[u8], u: Option<&[u8]>, id: &str, k: Option<&EngineKey>, core: &str, at: Option<(u32, &str)>| {
        verify_hints(f, h, u, id, k.unwrap(), core, &HintsAsk::default(), NOW, at, None)
    };
    let r = run(&file, &hints, Some(&undo), &id, Some(&key), CORE_ID, Some((200, H200)));
    for (ok, s) in &r.checks {
        eprintln!("  [{}] {}", if *ok { "ok" } else { "FAIL" }, &s[..s.len().min(160)]);
    }
    assert!(r.checks.iter().all(|(ok, _)| *ok));
    assert_eq!(r.verdict, Verdict::Demo, "200 blocks are below Core's minimum work");
    assert!(r.identity_checked);
    assert_eq!(r.statement.as_ref().unwrap().height, 200);
    let refused = |r: HintsReport, why: &str| assert_eq!(r.verdict, Verdict::Fail, "{why}");
    // the files: another hints file (one byte), another undo file (one byte)
    let mut h2 = hints.clone();
    *h2.last_mut().unwrap() ^= 1;
    refused(run(&file, &h2, Some(&undo), &id, Some(&key), CORE_ID, None), "a hints file that is not the proven one");
    let mut u2 = undo.clone();
    u2[0] ^= 1;
    refused(run(&file, &hints, Some(&u2), &id, Some(&key), CORE_ID, None), "an undo file that is not the proven one");
    // the chain: another block 200
    refused(run(&file, &hints, None, &id, Some(&key), CORE_ID, Some((200, NOT_H200))), "another block 200");
    // the proof: a journal bit, a proof byte, another image id, a pin that is not the key's, another core image
    let mut j = file.clone();
    let n = j.len() - 40;
    j[n] ^= 1;
    refused(run(&j, &hints, None, &id, Some(&key), CORE_ID, None), "journal changed");
    let mut p = file.clone();
    p[5000] ^= 1;
    refused(run(&p, &hints, None, &id, Some(&key), CORE_ID, None), "proof changed");
    refused(run(&file, &hints, None, CORE_ID, Some(&key), CORE_ID, None), "another image id");
    let bad = EngineKey { bytes: &vk, pin: [7u8; 32] };
    refused(run(&file, &hints, None, &id, Some(&bad), CORE_ID, None), "a pin that is not the key's");
    refused(run(&file, &hints, None, &id, Some(&key), &id, None), "another core image");
    // not an engine proof file (a RISC Zero receipt's first bytes); and a core image id that is not hex
    refused(run(&[1, 0, 0, 0, 9, 9], &hints, None, &id, Some(&key), CORE_ID, None), "not an engine proof");
    refused(run(&file, &hints, None, &id, Some(&key), "xyz", None), "a core image id that is not hex");
}

/// Review R3-4: the node's network (L1) and format (L2) are the caller's; a missing undo file is said (L3); every
/// image here is one the caller named (M1). Each of these fails on the code before the review.
#[test]
fn engine_hints_review_r3() {
    let (file, vk, id) = (rd("proof.hze"), rd("vk.bin"), txt("image_id"));
    let (hints, undo) = (rd("mainnet-200.hints"), rd("mainnet-200.undo"));
    let key = EngineKey { bytes: &vk, pin: hex32(&txt("vk.sha256")).unwrap() };
    let go = |ask: &HintsAsk, u: Option<&[u8]>| verify_hints(&file, &hints, u, &id, &key, CORE_ID, ask, NOW, Some((200, H200)), None);
    let base = go(&HintsAsk::default(), Some(&undo));
    assert_eq!(base.verdict, Verdict::Demo);
    let fmt = base.statement.as_ref().unwrap().format;
    let has = |r: &HintsReport, s: &str| r.checks.iter().any(|(ok, l)| *ok && l.contains(s));
    assert!(has(&base, "hints image") && has(&base, "is one you named, not a published one"), "{:?}", base.checks);
    assert!(has(&base, "core image") , "{:?}", base.checks);
    assert!(has(&base, "was not compared with the one your node reads: at most DEMO"), "{:?}", base.checks);
    // the node's format: the same passes (DEMO only for low work), the other one FAILs
    let same = go(&HintsAsk { network: "mainnet", format: Some(fmt) }, Some(&undo));
    assert_eq!(same.verdict, Verdict::Demo, "{:?}", same.checks);
    assert!(same.checks.iter().all(|(ok, _)| *ok));
    let other = if fmt == libzync::hints::FORMAT_BIP { libzync::hints::FORMAT_CRATE } else { libzync::hints::FORMAT_BIP };
    assert_eq!(go(&HintsAsk { network: "mainnet", format: Some(other) }, Some(&undo)).verdict, Verdict::Fail);
    // the node's network: a mainnet proof is not a signet node's
    let sig = go(&HintsAsk { network: "signet", format: Some(fmt) }, Some(&undo));
    assert_eq!(sig.verdict, Verdict::Fail);
    assert!(sig.checks.iter().any(|(ok, l)| !ok && l.contains("your node is on signet")), "{:?}", sig.checks);
    // no undo file: said, not silently skipped
    let nou = go(&HintsAsk::default(), None);
    assert!(has(&nou, "no undo-hash file given"), "{:?}", nou.checks);
    assert!(!has(&base, "no undo-hash file given"));
    // the parsers the CLI and wasm use
    assert_eq!((parse_format("bip"), parse_format("crate"), parse_format("2"), parse_format("x")),
               (Some(libzync::hints::FORMAT_BIP), Some(libzync::hints::FORMAT_CRATE), Some(libzync::hints::FORMAT_CRATE), None));
}
