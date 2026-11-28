// Review R3-1 #1: a branch-coverage copy of ../msm_ovm.rs, so the test can PROVE it reached every guarded branch of the
// curve arithmetic (the guards keep OpenVM's add_ne / double away from their unconstrained degenerate cases). The
// guest's source is not touched: this writes $OUT_DIR/msm_cov.rs with `crate::cov::hit(N);` inserted at fixed points.
// Every marker must occur exactly once, so a change to msm_ovm.rs that moves a guarded branch fails the build here
// instead of silently dropping its counter.
use std::{env, fs, path::Path};

fn main() {
    let src_path = "../msm_ovm.rs";
    println!("cargo:rerun-if-changed={src_path}");
    println!("cargo:rerun-if-changed=build.rs");
    let mut s = fs::read_to_string(src_path).expect("read ../msm_ovm.rs");
    // (marker, replacement); the counter goes on the same line, before the guarded operation
    let marks: &[(&str, &str)] = &[
        // 1: add_into on an empty accumulator (copy, no curve operation)
        ("        *a = p.clone();\n        *filled = true;\n        return;",
         "        crate::cov::hit(1); *a = p.clone();\n        *filled = true;\n        return;"),
        // 2: equal x, a = -p: the sum is the identity (no add_ne, no double)
        ("*filled = false; // a = -p", "crate::cov::hit(2); *filled = false; // a = -p"),
        // 3: equal x, a = p: the doubling of a NON-identity point
        ("a.double_assign_nonidentity::<false>(); // a = p", "crate::cov::hit(3); a.double_assign_nonidentity::<false>(); // a = p"),
        // 4: different x: add_ne (its precondition, x1 != x2, holds)
        ("a.add_ne_assign_nonidentity::<false>(p);", "crate::cov::hit(4); a.add_ne_assign_nonidentity::<false>(p);"),
        // 5: Pippenger's accumulator doubling, only when the accumulator is non-identity
        ("// SAFETY: acc is non-identity (acc_f) and the group has no point of order 2\n                unsafe { acc.double_assign_nonidentity::<false>() };",
         "// SAFETY: acc is non-identity (acc_f) and the group has no point of order 2\n                crate::cov::hit(5); unsafe { acc.double_assign_nonidentity::<false>() };"),
        // 6: Straus's accumulator doubling, same guard
        ("// SAFETY: as in msm\n                unsafe { acc.double_assign_nonidentity::<false>() };",
         "// SAFETY: as in msm\n                crate::cov::hit(6); unsafe { acc.double_assign_nonidentity::<false>() };"),
        // 7: Pippenger with every scalar zero (no window): the identity, no curve operation
        ("    if nw == 0 {\n        return id;\n    }", "    if nw == 0 {\n        crate::cov::hit(7); return id;\n    }"),
        // 8: the Straus path (n <= STRAUS_MAX)
        ("        return straus(", "        crate::cov::hit(8); return straus("),
        // 9: Pippenger with negative digits (negated points used)
        ("    let negs: Vec<P> = if any_neg {", "    if any_neg { crate::cov::hit(9); }\n    let negs: Vec<P> = if any_neg {"),
        // 10: an identity base dropped before any arithmetic
        ("    let live: Vec<usize> = (0..bases.len()).filter(|&i| bases[i] != id).collect();",
         "    let live: Vec<usize> = (0..bases.len()).filter(|&i| bases[i] != id).collect();\n    if live.len() != bases.len() { crate::cov::hit(10); }"),
    ];
    for (k, (m, r)) in marks.iter().enumerate() {
        let n = s.matches(m).count();
        assert!(n == 1, "build.rs: marker {} occurs {} times in msm_ovm.rs (must be exactly once)", k + 1, n);
        s = s.replacen(m, r, 1);
    }
    // inner doc comments cannot follow `mod x {` + include!: make them plain comments
    let s: String = s.lines().map(|l| if l.starts_with("//!") { format!("//{}", &l[3..]) } else { l.to_string() })
        .collect::<Vec<_>>().join("\n");
    let out = Path::new(&env::var("OUT_DIR").unwrap()).join("msm_cov.rs");
    fs::write(out, s).expect("write msm_cov.rs");
}
