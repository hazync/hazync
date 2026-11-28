// Copies the parser functions out of the shipped sources (by name, from the `fn` line to its closing brace at column
// 0), so the test exercises exactly the text the guest and the zAPP guests compile. A missing function fails the build.
use std::{env, fs, path::Path};

fn item(src: &str, head: &str, file: &str) -> String {
    let start = src.find(&format!("\n{head}")).unwrap_or_else(|| panic!("{head} not found in {file}")) + 1;
    let end = src[start..].find("\n}\n").unwrap_or_else(|| panic!("end of {head} not found in {file}")) + start + 3;
    // made visible to this crate's main.rs (the only change to the copied text)
    format!("pub(crate) {}", &src[start..end])
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../../..");
    let out = Path::new(&env::var("OUT_DIR").unwrap()).to_path_buf();
    let agg_f = root.join("prover/methods/guest/src/agg_lite.rs");
    let agg = fs::read_to_string(&agg_f).unwrap();
    let cs = item(&agg, "fn compact_size(", "agg_lite.rs");
    let tx_io = item(&agg, "fn tx_io(", "agg_lite.rs");
    let pf = item(&agg, "fn prevout_fields(", "agg_lite.rs");
    fs::write(out.join("agg.rs"), format!("{cs}\n{tx_io}\n{pf}\n")).unwrap();
    // Positive controls, two broken readers the test must catch (it fails if either passes):
    //  lenient: compact sizes in any encoding up to MAX_SIZE. On the core guest's path Core parses the same bytes
    //    first and traps on a non-canonical size, so this one is EXPECTED to give no mismatch (kept as a measurement
    //    of that claim: test-txparse.sh requires it to pass);
    //  nomarker: tx_io without the BIP144 marker skip, which misreads every witness transaction Core accepts.
    let strict = "(v >= min && v <= 0x0200_0000).then_some(v as usize)";
    assert!(cs.contains(strict), "compact_size's range check changed: update the positive control");
    let lenient = cs.replace(strict, "{ let _: u64 = min; (v <= 0x0200_0000).then_some(v as usize) }");
    let marker = "if b.get(4) == Some(&0) && b.get(5) == Some(&1) { o += 2; }";
    assert!(tx_io.contains(marker), "tx_io's marker skip changed: update the positive control");
    fs::write(out.join("agg_nomarker.rs"), format!("{cs}\n{}\n{pf}\n", tx_io.replace(marker, ""))).unwrap();
    fs::write(out.join("agg_lenient.rs"), format!("{lenient}\n{tx_io}\n{pf}\n")).unwrap();
    // hints.rs and utreexo_roots.rs: the zAPP guests' stripped-transaction readers (private there).
    for (name, file) in [("hints", "libzync/src/hints.rs"), ("utreexo", "libzync/src/utreexo_roots.rs")] {
        let f = root.join(file);
        let s = fs::read_to_string(&f).unwrap();
        let mut t = String::new();
        for head in ["fn varint(", "fn len_varint(", "fn take<", "struct Tx<", "fn parse_tx<"] {
            t += &item(&s, head, file);
            t += "\n";
        }
        fs::write(out.join(format!("{name}.rs")), t).unwrap();
        println!("cargo:rerun-if-changed={}", f.display());
    }
    println!("cargo:rerun-if-changed={}", agg_f.display());
}
