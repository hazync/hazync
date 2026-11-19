//! Writes the seed corpus: `cargo run --release --example mkseeds` (from fuzz/).
fn main() {
    let d = format!("{}/corpus/common_identity", env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(&d).unwrap();
    let t = zapp_common_fuzz::TIP_200;
    let seeds = [format!("--expect\0200:{t}"), format!("--expect\0200:{}", t.to_uppercase()), format!("--expect\0199:{t}"),
                 format!("--expect\0200:{t}\0--expect\0200:{t}"), "--mempool".into(), format!("--mempool-url\0https://x\0--expect\0200:{t}"),
                 format!("--bitcoin-cli\0bitcoin-cli -regtest\0--expect\0200:{t}"), format!("--other\0--expect\0200:{t}0")];
    for (i, s) in seeds.iter().enumerate() {
        std::fs::write(format!("{d}/s{i}"), s).unwrap();
        zapp_common_fuzz::identity(s.as_bytes());
    }
    println!("common_identity: seeds hold");
}
