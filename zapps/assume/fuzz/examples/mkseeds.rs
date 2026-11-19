//! Writes the seed corpus of every target from the zAPP's real test inputs: `cargo run --release --example mkseeds`
//! (from fuzz/). Seeds are committed; rerunning rewrites the same files.
use zapp_assume_fuzz::*;
fn put(target: &str, name: &str, b: &[u8]) {
    let d = format!("{}/corpus/{target}", env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(format!("{d}/{name}"), b).unwrap();
}
fn side(h: &[u8], text: &str) -> Vec<u8> {
    let mut v = (h.len() as u32).to_le_bytes().to_vec();
    v.extend_from_slice(h);
    v.extend_from_slice(text.as_bytes());
    v
}
fn main() {
    let v = |f: &str| std::fs::read(format!("{}/../vectors/{f}", env!("CARGO_MANIFEST_DIR"))).unwrap();
    for f in ["utxo_199.dat", "utxo_200.dat", "utxo_1003.dat"] {
        put("assume_snapshot_decode", f, &v(f));
        put("assume_snapshot_check", f, &v(f));
    }
    put("assume_side_inputs", "all_tip", &side(HDRS, "av:tip"));
    put("assume_side_inputs", "all_170", &side(HDRS, "av:00000000d1145790a8694403d4063f323d499e655c83426834d4ce2f8dd4a2ee"));
    put("assume_side_inputs", "all_tracked", &side(HDRS, "# one coin\n0e3e2357e806b6cdb1f70b54c3a3a17b6714ee1f0e68bebb44a74b1efd512098:0\n"));
    put("assume_side_inputs", "last_51", &side(&HDRS[HDRS.len() - 80 * 51..], "av:tip"));
    put("assume_side_inputs", "none", &side(&[], "av:tip"));
    put("assume_core_receipt", "mainnet_1_200_8888d59c.rcpt", RCPT);
    let fl = std::fs::read(format!("{}/../../../swiftsync-verify/vectors/mainnet_1_200_8888d59c_flipped.rcpt", env!("CARGO_MANIFEST_DIR"))).unwrap();
    put("assume_core_receipt", "mainnet_1_200_8888d59c_flipped.rcpt", &fl);
    for t in ["assume_snapshot_decode", "assume_snapshot_check", "assume_side_inputs", "assume_core_receipt"] {
        let d = format!("{}/corpus/{t}", env!("CARGO_MANIFEST_DIR"));
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            let b = std::fs::read(&p).unwrap();
            match t {
                "assume_snapshot_decode" => snapshot_decode(&b),
                "assume_snapshot_check" => snapshot_check(&b),
                "assume_side_inputs" => side_inputs(&b),
                _ => core_receipt(&b),
            }
        }
        println!("{t}: seeds hold");
    }
}
