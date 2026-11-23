// Compile Bitcoin Core's libsecp256k1 v0.5.1 (+ the batch hook, apply_hook.py) for the OpenVM guest with the
// SAME riscv32 gcc 13.2 + flags Hazync's risc0 guest uses (prover/methods/guest/build.rs).
fn main() {
    let secp = std::env::var("SECP_DIR").unwrap_or_else(|_| "/root/fresh-bench/secp256k1".into());
    let bin = std::env::var("RV_GCC_BIN").unwrap_or_else(|_| format!("{}/.risc0/cpp/bin", std::env::var("HOME").unwrap()));
    let target = std::env::var("TARGET").unwrap();
    println!("cargo:rerun-if-changed=csrc/glue.c");
    println!("cargo:rerun-if-changed={secp}/src");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_BATCH");
    if !target.starts_with("riscv32") { return; }
    let mut b = cc::Build::new();
    b.compiler(format!("{bin}/riscv32-unknown-elf-gcc")).archiver(format!("{bin}/riscv32-unknown-elf-ar"))
        .flag("-march=rv32im").flag("-mabi=ilp32").opt_level(3).warnings(false)
        .flag("-fno-strict-aliasing").flag("-ffunction-sections").flag("-fdata-sections")
        .include(&secp).include(format!("{secp}/src")).include(format!("{secp}/include"))
        .define("ECMULT_WINDOW_SIZE", "15").define("ECMULT_GEN_KB", "22")
        .define("ENABLE_MODULE_SCHNORRSIG", "1").define("ENABLE_MODULE_EXTRAKEYS", "1")
        .define("USE_EXTERNAL_DEFAULT_CALLBACKS", "1")
        .file(format!("{secp}/src/secp256k1.c"))
        .file(format!("{secp}/src/precomputed_ecmult.c"))
        .file(format!("{secp}/src/precomputed_ecmult_gen.c"))
        .file("csrc/glue.c");
    if std::env::var("CARGO_FEATURE_BATCH").is_ok() { b.define("HAZYNC_BATCH_ECDSA", "1"); }
    if std::env::var("CARGO_FEATURE_LIFTX").is_ok() { b.define("HAZYNC_LIFTX_OVM", "1"); }
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_LIFTX");
    b.compile("secp256k1");
}
