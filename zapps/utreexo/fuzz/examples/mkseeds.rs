//! Writes the seed corpus from the real receipts: `cargo run --release --example mkseeds` (from fuzz/).
use zapp_utreexo_fuzz::*;
fn put(target: &str, name: &str, b: &[u8]) {
    let d = format!("{}/corpus/{target}", env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(format!("{d}/{name}"), b).unwrap();
}
fn main() {
    for (n, b) in ["mainnet-2.receipt", "mainnet-3.receipt", "mainnet-3-split.receipt"].iter().zip(REAL) {
        put("utreexo_decode", n, b);
        put("utreexo_verify", n, b);
    }
    let mut hostile = vec![0u8; 64];
    hostile[..8].copy_from_slice(&(1u64 << 60).to_le_bytes());
    put("utreexo_decode", "hostile_len", &hostile);
    for (t, f) in [("utreexo_decode", decode as fn(&[u8])), ("utreexo_verify", verify)] {
        let d = format!("{}/corpus/{t}", env!("CARGO_MANIFEST_DIR"));
        for e in std::fs::read_dir(&d).unwrap() { f(&std::fs::read(e.unwrap().path()).unwrap()); }
        println!("{t}: seeds hold");
    }
    // positive: the real receipts are DEMO, not FAIL, under the questions the target asks
    for b in REAL.iter().take(2) {
        assert!([((3, H3), None), ((2, H2), Some((3, H3)))].iter().any(|(a, t)|
            zapp_utreexo::verify_utreexo(b, zapp_utreexo::UTREEXO_IMAGE_ID, None, NOW, Some(*a), *t).verdict != swiftsync_verify::Verdict::Fail));
    }
}
