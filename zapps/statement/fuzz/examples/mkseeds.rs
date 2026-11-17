//! Writes the seed corpus of every target: `cargo run --release --example mkseeds` (from fuzz/). The reference
//! statement (corpus/statement_check/signed.json) is written only when missing: the targets compare with that file.
use zapp_statement_fuzz::*;
fn put(target: &str, name: &str, b: &[u8]) {
    let d = format!("{}/corpus/{target}", env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(format!("{d}/{name}"), b).unwrap();
}
fn main() {
    if !std::path::Path::new(REFERENCE).exists() {
        put("statement_check", "signed.json", serde_json::to_string(&make_reference()).unwrap().as_bytes());
    }
    let (st, _) = reference();
    put("statement_check", "signed_pretty.json", serde_json::to_string_pretty(st).unwrap().as_bytes());
    let mut one = st.clone();
    one.entries.truncate(1);
    put("statement_check", "one_entry.json", serde_json::to_string(&one).unwrap().as_bytes());
    put("statement_fields", "asked", format!("{}\0{}", ASK.purpose.unwrap(), ASK.nonce.unwrap()).as_bytes());
    put("statement_fields", "edge_space", b"audit \0n");
    put("statement_fields", "long", format!("{}\0{}", "a".repeat(256), "b".repeat(257)).as_bytes());
    for (i, (s, m, t)) in p2pk_refs().iter().enumerate() {
        let mut b = vec![i as u8];
        b.extend_from_slice(t);
        put("statement_p2pk", &format!("ref_{i}"), &b);
        let mut f = vec![0xffu8, s.len() as u8];
        f.extend_from_slice(s);
        f.extend_from_slice(&(m.len() as u16).to_le_bytes());
        f.extend_from_slice(m.as_bytes());
        f.extend_from_slice(t);
        put("statement_p2pk", &format!("framed_{i}"), &f);
    }
    for (t, f) in [("statement_check", check as fn(&[u8])), ("statement_fields", fields), ("statement_p2pk", p2pk)] {
        let d = format!("{}/corpus/{t}", env!("CARGO_MANIFEST_DIR"));
        for e in std::fs::read_dir(&d).unwrap() { f(&std::fs::read(e.unwrap().path()).unwrap()); }
        println!("{t}: seeds hold");
    }
}
