//! verify_full on a real engine proof (block 130,000's aggregate, keys-hz24; inputs from `hzengine wasm-inputs` plus
//! the block journal and the keys' image.json). WASMV_INPUTS=<dir> with vk.bin, proof.zst, proof_bad.zst,
//! block.journal, image.json; skipped when unset.
use sha2::{Digest, Sha256};

#[test]
fn verify_full_on_a_real_proof() {
    let Ok(d) = std::env::var("WASMV_INPUTS") else {
        eprintln!("WASMV_INPUTS unset: skipped");
        return;
    };
    let rd = |n: &str| std::fs::read(format!("{d}/{n}")).unwrap_or_else(|e| panic!("{d}/{n}: {e}"));
    let (vk, proof, bad, journal) = (rd("vk.bin"), rd("proof.zst"), rd("proof_bad.zst"), rd("block.journal"));
    let img: serde_json::Value = serde_json::from_slice(&rd("image.json")).unwrap();
    let hx = |k: &str| hex::decode(img[k].as_str().unwrap()).unwrap();
    let mut pre = hx("exe");
    pre.extend(hx("vm"));
    let id: [u8; 32] = Sha256::digest(&pre).into();

    // the image id derived from the vk alone equals the one the engine recorded at build
    let v: hazync_engine_verify::VerifyingKey = hazync_engine_verify::decode_vk(&vk).unwrap();
    let (exe, vm) = wasmv::image_of(&v);
    assert_eq!((exe, vm), (hx("exe"), hx("vm")), "image_of(vk) != image.json");
    assert_eq!(wasmv::image_id_of(&v), id);

    assert_eq!(wasmv::verify_full_bytes(&vk, &proof, &journal, &id), 1, "valid proof + journal + image");
    let mut j2 = journal.clone();
    j2[100] ^= 1;
    assert_eq!(wasmv::verify_full_bytes(&vk, &proof, &j2, &id), 4, "journal with one bit flipped");
    let mut id2 = id;
    id2[0] ^= 1;
    assert_eq!(wasmv::verify_full_bytes(&vk, &proof, &journal, &id2), 5, "another image id");
    assert_eq!(wasmv::verify_full_bytes(&vk, &proof, &journal, &id[..31]), 6, "short image id");
    let r = wasmv::verify_full_bytes(&vk, &bad, &journal, &id);
    assert!(r == 0 || r == 3, "proof with one encoded byte flipped: {r}");
    assert_eq!(wasmv::verify_full_bytes(&vk[1..], &proof, &journal, &id), 2, "truncated vk");
    eprintln!("verify_full: valid 1, journal 4, image 5, short id 6, bad proof {r}, bad vk 2");
}
