// Inline the guest's batch module and its MSM module (old or current) into OUT_DIR/batch.rs, without their inner doc
// comments (include! refuses `//!`), with test probes appended inside each module so they reach private items.
use std::{env, fs, path::PathBuf};
fn strip(s: &str) -> String { s.lines().filter(|l| !l.trim_start().starts_with("//!")).collect::<Vec<_>>().join("\n") }
fn main() {
    let here = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let dir = if env::var("CARGO_FEATURE_OLD").is_ok() { here.join("old") } else { here.join("..") };
    let b = fs::read_to_string(dir.join("batch_ecdsa_ovm.rs")).unwrap();
    let m = fs::read_to_string(dir.join("msm_ovm.rs")).unwrap();
    let probe_m = fs::read_to_string(here.join("src/probe_msm.rs")).unwrap();
    let probe_b = fs::read_to_string(here.join("src/probe_batch.rs")).unwrap();
    let anchor = "#[path = \"msm_ovm.rs\"]\nmod msm;";
    assert_eq!(b.matches(anchor).count(), 1, "the msm module line moved");
    let out = strip(&b).replace(anchor, &format!("mod msm {{\n{}\n{}\n}}", strip(&m), probe_m)) + "\n" + &probe_b;
    fs::write(PathBuf::from(env::var("OUT_DIR").unwrap()).join("batch.rs"), out).unwrap();
    // the MSM's largest window size, as THIS msm_ovm.rs picks it: `(1..=<N>usize).min_by_key(|&c| cost(&bits, c))`
    let i = m.find("(1..=").expect("the window range moved") + 5;
    let n: String = m[i..].chars().take_while(|c| c.is_ascii_digit()).collect();
    println!("cargo:rustc-env=HZ_MSM_MAX_C={n}");
    for f in ["old/batch_ecdsa_ovm.rs", "old/msm_ovm.rs", "../batch_ecdsa_ovm.rs", "../msm_ovm.rs", "src/probe_msm.rs", "src/probe_batch.rs"] {
        println!("cargo:rerun-if-changed={f}");
    }
}
