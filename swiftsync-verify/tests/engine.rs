//! `verify_engine` on a real engine proof (engine/VERIFIER.md step 4): the engine chain 1..200 from genesis, its proof
//! of block 200 (keys-ch: node format 2 + C heap; `hzengine wasm-inputs` wrote proof.hze, vk.bin and the key's pin),
//! checked by the same statement checks as a RISC Zero receipt; then the negatives.
#![cfg(feature = "engine")]
use swiftsync_verify::*;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/engine200");

fn rd(n: &str) -> Vec<u8> {
    std::fs::read(format!("{DIR}/{n}")).unwrap_or_else(|e| panic!("{DIR}/{n}: {e}"))
}

/// The engine image as a record: today's mainnet record with the engine image's id and the node-format-2 profile
/// (no engine image is published yet; Stage 4 adds the real record).
fn image() -> &'static Image {
    let m = Image::by_network("mainnet").unwrap();
    let id = String::from_utf8(rd("image_id")).unwrap().trim().to_string();
    Box::leak(Box::new(Image {
        network: m.network,
        method_id_hex: Box::leak(id.into_boxed_str()),
        profile: m.profile | PROFILE_NODE2,
        genesis_hash_hex: m.genesis_hash_hex,
        genesis_time: m.genesis_time,
        genesis_bits: m.genesis_bits,
        genesis_work: m.genesis_work,
        min_work_hex: m.min_work_hex,
        genesis_anchor_hex: m.genesis_anchor_hex,
        checkpoint_height: m.checkpoint_height,
        lineage: &[],
    }))
}

fn pin() -> [u8; 32] {
    hex32(String::from_utf8(rd("vk.sha256")).unwrap().trim()).unwrap()
}

fn run(file: &[u8], img: &'static Image, vk: &[u8], pin: [u8; 32]) -> Report {
    verify_engine(&Inputs { receipt: file, image: img, anchor: None, now: 1_800_000_000 }, &EngineKey { bytes: vk, pin })
}

fn failed(r: &Report) -> Vec<String> {
    r.checks.iter().filter(|(ok, _)| !ok).map(|(_, s)| s.clone()).collect()
}

#[test]
fn engine_chain_block_200() {
    let (file, vk, img) = (rd("proof.hze"), rd("vk.bin"), image());
    let r = run(&file, img, &vk, pin());
    for (ok, s) in &r.checks {
        eprintln!("  [{}] {s}", if *ok { "ok" } else { "FAIL" });
    }
    // every check passes; the verdict is Demo only because 200 blocks are far below the minimum chain work
    assert!(failed(&r).is_empty(), "failed: {:?}", failed(&r));
    assert_eq!(r.verdict, Verdict::Demo);
    assert!(r.low_work);
    let cs = r.state.as_ref().unwrap();
    assert_eq!((cs.height, cs.profile), (200, 0xf1));
    assert!(r.checks[0].1.starts_with("engine proof verifies against image"));
}

#[test]
fn engine_negatives() {
    let (file, vk, img) = (rd("proof.hze"), rd("vk.bin"), image());
    let fails = |r: &Report, want: &str| {
        assert_eq!(r.verdict, Verdict::Fail);
        let f = failed(r);
        assert!(f.iter().any(|s| s.contains(want)), "want a failed check containing {want:?}, got {f:?}");
        eprintln!("  refused: {}", f[0]);
    };
    // the journal (the file's tail) with one bit changed: the public values no longer match
    let mut j = file.clone();
    let n = j.len() - 40;
    j[n] ^= 1;
    fails(&run(&j, img, &vk, pin()), "public values are not SHA-256 of this journal");
    // a proof byte changed
    let mut p = file.clone();
    p[8 + 1000] ^= 1;
    let r = run(&p, img, &vk, pin());
    assert_eq!(r.verdict, Verdict::Fail);
    assert!(r.state.is_none(), "no statement from a proof that did not verify");
    // a key that is not the pinned one, and the right key under another pin
    let mut k = vk.clone();
    let n = k.len() - 1;
    k[n] ^= 1;
    fails(&run(&file, img, &k, pin()), "not the image's pinned key");
    let mut pin2 = pin();
    pin2[0] ^= 1;
    fails(&run(&file, img, &vk, pin2), "not the image's pinned key");
    // another image's record (today's RISC Zero mainnet image)
    fails(&run(&file, Image::by_network("mainnet").unwrap(), &vk, pin()), "not the published image's");
    // not an engine proof file: a RISC Zero receipt's first bytes, and a truncated file
    fails(&run(&[1, 0, 0, 0, 7, 7, 7, 7, 7], img, &vk, pin()), "the file is an engine proof");
    fails(&run(&file[..6], img, &vk, pin()), "the file is an engine proof");
    // and the RISC Zero front refuses an engine file without decoding it as a receipt
    assert!(is_engine_proof(&file));
    let r = verify(&Inputs { receipt: &file, image: img, anchor: None, now: 1_800_000_000 });
    assert_eq!(r.verdict, Verdict::Fail);
}
