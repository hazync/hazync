//! verify_full on a real engine proof, and each refusal (engine/wasm-verify's test, on the library).
use sha2::{Digest, Sha256};

#[test]
fn verify_full_on_a_real_proof() {
    // inputs (tests/data): `hzengine wasm-inputs` on block 130,000's aggregate proof (keys-hz24, the CPU segpar test),
    // the block journal and the keys' image.json; WASMV_INPUTS=<dir> to check another proof
    let d = std::env::var("WASMV_INPUTS").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data").to_string());
    let rd = |n: &str| std::fs::read(format!("{d}/{n}")).unwrap_or_else(|e| panic!("{d}/{n}: {e}"));
    let (vk, proof, journal) = (rd("vk.bin"), rd("proof.zst"), rd("block.journal"));
    let mut bad = proof.clone();
    let k = bad.len() / 2;
    bad[k] ^= 1;
    let img: serde_json::Value = serde_json::from_slice(&rd("image.json")).unwrap();
    let hx = |k: &str| hex::decode(img[k].as_str().unwrap()).unwrap();
    let mut pre = hx("exe");
    pre.extend(hx("vm"));
    let id: [u8; 32] = Sha256::digest(&pre).into();

    // the image id derived from the vk alone equals the one the engine recorded at build
    let v: openvm_verify_stark_host::vk::VmStarkVerifyingKey = bitcode::deserialize(&vk).unwrap();
    let (exe, vm) = hazync_engine_verify::image_of(&v);
    assert_eq!((exe, vm), (hx("exe"), hx("vm")), "image_of(vk) != image.json");
    assert_eq!(hazync_engine_verify::image_id_of(&v), id);

    assert_eq!(hazync_engine_verify::code(hazync_engine_verify::verify_full_bytes(&vk, &proof, &journal, &id)), 1, "valid proof + journal + image");
    let mut j2 = journal.clone();
    j2[100] ^= 1;
    assert_eq!(hazync_engine_verify::code(hazync_engine_verify::verify_full_bytes(&vk, &proof, &j2, &id)), 4, "journal with one bit flipped");
    let mut id2 = id;
    id2[0] ^= 1;
    assert_eq!(hazync_engine_verify::code(hazync_engine_verify::verify_full_bytes(&vk, &proof, &journal, &id2)), 5, "another image id");
    assert_eq!(hazync_engine_verify::code(hazync_engine_verify::verify_full_bytes(&vk, &proof, &journal, &id[..31])), 6, "short image id");
    let r = hazync_engine_verify::code(hazync_engine_verify::verify_full_bytes(&vk, &bad, &journal, &id));
    assert!(r == 0 || r == 3, "proof with one compressed byte flipped: {r}");
    assert_eq!(hazync_engine_verify::code(hazync_engine_verify::verify_full_bytes(&vk[1..], &proof, &journal, &id)), 2, "truncated vk");
    eprintln!("verify_full: valid 1, journal 4, image 5, short id 6, bad proof {r}, bad vk 2");
}
