//! Writes the seed corpus of every target from the zAPP's real test inputs: `cargo run --release --example mkseeds`
//! (from fuzz/). Seeds are committed; rerunning rewrites the same files.
use zapp_deposit_fuzz::*;
fn put(target: &str, name: &str, b: &[u8]) {
    let d = format!("{}/corpus/{target}", env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(format!("{d}/{name}"), b).unwrap();
}
fn main() {
    for (i, (asks, b)) in refs().bundles.iter().enumerate() {
        for a in asks {
            let j = serde_json::json!({"ask": a, "bundle": b}).to_string();
            assert!(gives_statement(j.as_bytes()), "reference bundle {i} gives no statement for ask {a}");
            put("deposit_bundle", &format!("b{i}_ask{a}.json"), j.as_bytes());
        }
    }
    let m = env!("CARGO_MANIFEST_DIR");
    let block = swiftsync_verify::unhex(std::fs::read_to_string(format!("{m}/../../../experimental/libzync-bench/deposit/block_170_raw.txt")).unwrap().trim()).unwrap();
    for (i, t) in libzync::facts::parse_block(&block).unwrap().1.iter().enumerate() { put("deposit_tx", &format!("b170_{i}"), &t.stripped); }
    let seg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(format!("{m}/../../../experimental/libzync-bench/headers/tx/tx_seg.json")).unwrap()).unwrap();
    let t = zapp_headers::TxInclusion::from_json(seg["txid"].as_str().unwrap(), &seg).unwrap();
    put("deposit_tx", "seg_witness", &t.raw);
    put("deposit_tx", "seg_coinbase", &t.coinbase);
    put("deposit_tx", "opret_pushes", &[0x6a, 0x02, 1, 2, 0x4c, 0x01, 3, 0x4d, 0x01, 0x00, 9, 0x4e, 0x01, 0, 0, 0, 7, 0x00]);
    for (t, f) in [("deposit_bundle", bundle as fn(&[u8])), ("deposit_tx", tx)] {
        let d = format!("{m}/corpus/{t}");
        for e in std::fs::read_dir(&d).unwrap() { f(&std::fs::read(e.unwrap().path()).unwrap()); }
        println!("{t}: seeds hold");
    }
}
