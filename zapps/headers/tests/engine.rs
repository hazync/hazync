//! The header-chain zAPP's ENGINE proofs in its verifier (engine/VERIFIER.md): the engine's proof of mainnet headers
//! 0..4,031 (keys-hdr, proved on CPU 26.11; `hzengine wasm-inputs` wrote the file, key and pin), checked by
//! verify_headers_with under the pinned key, the tip compared with block 4,031's hash; then the refusals.
//! `--features engine`.
#![cfg(feature = "engine")]
use swiftsync_verify::{hex32, EngineKey, Verdict};
use zapp_headers::*;

const D: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/engine4031");
/// block 4,031 as explorers print it (the double SHA-256 of the 4,032nd header of the proved list)
const TIP: &str = "00000000f037ad09d0b05ee66b8c1da83030abaf909d2b1bf519c3c7d2cd3fdf";

fn rd(n: &str) -> Vec<u8> {
    std::fs::read(format!("{D}/{n}")).unwrap_or_else(|e| panic!("{D}/{n}: {e}"))
}

fn txt(n: &str) -> String {
    String::from_utf8(rd(n)).unwrap().trim().to_string()
}

#[test]
fn engine_header_chain_0_4031() {
    let (file, vk, id) = (rd("proof.hze"), rd("vk.bin"), txt("image_id"));
    let key = EngineKey { bytes: &vk, pin: hex32(&txt("vk.sha256")).unwrap() };
    let r = verify_headers_with(&[&file], &id, Some(&key), 1_800_000_000, Some((4031, TIP)), None);
    for (ok, s) in &r.checks {
        eprintln!("  [{}] {}", if *ok { "ok" } else { "FAIL" }, &s[..s.len().min(160)]);
    }
    assert!(r.checks.iter().all(|(ok, _)| *ok));
    assert_eq!(r.verdict, Verdict::Demo, "4,032 headers are below Core's minimum work");
    assert!(r.tip_checked);
    assert_eq!(r.state.as_ref().unwrap().height, 4031);
    let refused = |r: &HeadersReport, why: &str| assert_eq!(r.verdict, Verdict::Fail, "{why}");
    // another expected tip
    refused(&verify_headers_with(&[&file], &id, Some(&key), 1_800_000_000, Some((4031, &TIP.replace('f', "e"))), None), "another tip");
    // the journal (the file's tail) changed
    let mut j = file.clone();
    let n = j.len() - 20;
    j[n] ^= 1;
    refused(&verify_headers_with(&[&j], &id, Some(&key), 1_800_000_000, None, None), "journal changed");
    // another pin; another image id
    let mut pin2 = key.pin;
    pin2[0] ^= 1;
    refused(&verify_headers_with(&[&file], &id, Some(&EngineKey { bytes: &vk, pin: pin2 }), 1_800_000_000, None, None), "another pin");
    let mut id2 = id.clone().into_bytes();
    id2[0] = if id2[0] == b'0' { b'1' } else { b'0' };
    refused(&verify_headers_with(&[&file], std::str::from_utf8(&id2).unwrap(), Some(&key), 1_800_000_000, None, None), "another image");
    // the same run twice: the second does not start where the first ended
    refused(&verify_headers_with(&[&file, &file], &id, Some(&key), 1_800_000_000, None, None), "a run repeated");
    // an engine file without a key (the RISC Zero path), and a RISC Zero receipt's bytes with a key
    refused(&verify_headers(&[&file], &id, 1_800_000_000, None, None), "engine file, no key");
    refused(&verify_headers_with(&[&[1u8, 0, 0, 0, 9, 9][..]], &id, Some(&key), 1_800_000_000, None, None), "not an engine file");
}
