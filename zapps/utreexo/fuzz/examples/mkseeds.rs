//! Writes the seed corpus from the engine's real base proof: `cargo run --release --example mkseeds` (from fuzz/).
use zapp_utreexo_fuzz::*;
fn put(target: &str, name: &str, b: &[u8]) {
    let d = format!("{}/corpus/{target}", env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(format!("{d}/{name}"), b).unwrap();
}
fn main() {
    put("utreexo_decode", "base100.hze", REAL);
    put("utreexo_verify", "base100.hze", REAL);
    let mut flipped = REAL.to_vec();
    flipped[4096] ^= 1;
    put("utreexo_verify", "base100_flipped.hze", &flipped);
    let mut hostile = vec![0u8; 64];
    hostile[..8].copy_from_slice(&(1u64 << 60).to_le_bytes());
    put("utreexo_decode", "hostile_len", &hostile);
    put("utreexo_decode", "magic_long_len", &[b"HZE1".as_slice(), &u32::MAX.to_le_bytes()].concat());
    for (t, f) in [("utreexo_decode", decode as fn(&[u8])), ("utreexo_verify", verify)] {
        let d = format!("{}/corpus/{t}", env!("CARGO_MANIFEST_DIR"));
        for e in std::fs::read_dir(&d).unwrap() { f(&std::fs::read(e.unwrap().path()).unwrap()); }
        println!("{t}: seeds hold");
    }
    // positive: the real proof is DEMO, not FAIL, under the questions the target asks
    let key = swiftsync_verify::EngineKey { bytes: include_bytes!("../../tests/data/engine/base100.vk"), pin: swiftsync_verify::hex32(BASE_PIN).unwrap() };
    assert_ne!(zapp_utreexo::verify_utreexo(REAL, BASE_ID, &key, CORE_ID, NOW, Some((100, H100)), Some((200, H200))).verdict,
               swiftsync_verify::Verdict::Fail);
}
