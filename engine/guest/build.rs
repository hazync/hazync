// The engine guest's C/C++ consensus path: Bitcoin Core v28.0 + patches 0001 0002 0009 0010 libzync/0101, Hazync's
// verify_input.cpp / core_tx_rules.cpp / core_block_rules.cpp / cshims.c, and libsecp v0.5.1 with the engine's batch +
// sqrt hooks, all laid out by engine/guest/base.sh at HZ_BASE. Same riscv32 gcc/g++ 13.2 and flags as
// prover/methods/guest/build.rs. SHA-256 (Core via core-0201, libsecp via secp-0202) calls OpenVM's
// zkvm_sha256_impl in its native state layout; there is no sys_sha_compress.
use std::process::Command;
fn lib_dir(gcc: &str, q: &str) -> String {
    let o = Command::new(gcc).args(["-march=rv32im", "-mabi=ilp32", q]).output().unwrap();
    std::path::PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()).parent().unwrap().to_string_lossy().into_owned()
}
fn main() {
    let target = std::env::var("TARGET").unwrap();
    if !target.starts_with("riscv32") { return; }
    let base = std::env::var("HZ_BASE").expect("HZ_BASE: the tree engine/guest/base.sh lays out");
    println!("cargo:rerun-if-env-changed=HZ_BASE");
    let hzc = std::env::var("CARGO_MANIFEST_DIR").unwrap();   // the copied guest dir: Hazync's own C/C++ sources
    let secp = format!("{base}/secp256k1");
    let bin = format!("{}/.risc0/cpp/bin", std::env::var("HOME").unwrap());
    let (gcc, gpp, ar) = (format!("{bin}/riscv32-unknown-elf-gcc"), format!("{bin}/riscv32-unknown-elf-g++"), format!("{bin}/riscv32-unknown-elf-gcc-ar"));
    let core = format!("{base}/bitcoin-core/src"); let shim = format!("{base}/coreshim");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_BATCH_ECDSA");
    let feat = |f: &str| std::env::var(format!("CARGO_FEATURE_{f}")).is_ok();
    // Reproducible ids (engine/FEASIBILITY.md Q15): the build machine's paths reach the program through __FILE__
    // (Core's asserts) and the generated includes. Map the base tree and the assembled tree (guest dir ../../..,
    // which also holds target/) to fixed names, as the Rust side does with --remap-path-prefix (engine.rs build).
    let tree = std::fs::canonicalize(format!("{hzc}/../../..")).unwrap().to_string_lossy().into_owned();
    let base_c = std::fs::canonicalize(&base).unwrap().to_string_lossy().into_owned();
    let maps = [format!("-ffile-prefix-map={base_c}=/hz-base"), format!("-ffile-prefix-map={tree}=/hz-tree")];

    let mut s = cc::Build::new();
    for m in &maps { s.flag(m); }
    s.compiler(&gcc).archiver(&ar).flag("-march=rv32im").flag("-mabi=ilp32").opt_level(3).warnings(false)
        .flag("-fno-strict-aliasing").flag("-ffunction-sections").flag("-fdata-sections")
        .include(&secp).include(format!("{secp}/src")).include(format!("{secp}/include"))
        .define("ECMULT_WINDOW_SIZE", "15").define("ECMULT_GEN_KB", "22")
        .define("ENABLE_MODULE_SCHNORRSIG", "1").define("ENABLE_MODULE_EXTRAKEYS", "1")
        .define("USE_EXTERNAL_DEFAULT_CALLBACKS", "1")
        .file(format!("{secp}/src/secp256k1.c")).file(format!("{secp}/src/precomputed_ecmult.c"))
        .file(format!("{secp}/src/precomputed_ecmult_gen.c")).file(format!("{hzc}/cshims.c"));   // secp callbacks + abort come from main.rs
    // The batch hooks route ECDSA/BIP340/BIP341-tweak checks to batch_ecdsa_ovm.rs; the sqrt hook lifts keys on OpenVM's
    // modular extension. The engine has no other key lift, so the sqrt hook is unconditional.
    if feat("BATCH_ECDSA") { s.define("HAZYNC_BATCH_ECDSA", "1"); }
    s.define("HAZYNC_LIFTX_OVM", "1");
    s.define("HAZYNC_SHA_OVM", "1");   // secp-0202: libsecp's SHA-256 compression on OpenVM's extension
    s.compile("secp256k1");

    let tus = ["script/interpreter.cpp", "script/script.cpp", "script/script_error.cpp", "primitives/transaction.cpp",
        "pubkey.cpp", "hash.cpp", "uint256.cpp", "crypto/sha256.cpp", "crypto/sha512.cpp", "crypto/ripemd160.cpp",
        "crypto/sha1.cpp", "crypto/hmac_sha512.cpp", "util/strencodings.cpp", "crypto/hex_base.cpp",
        "consensus/tx_check.cpp", "consensus/merkle.cpp", "arith_uint256.cpp", "pow.cpp", "chain.cpp",
        "kernel/chainparams.cpp", "primitives/block.cpp", "util/chaintype.cpp", "util/moneystr.cpp"];
    let mut b = cc::Build::new();
    for m in &maps { b.flag(m); }
    b.cpp(true).compiler(&gpp).archiver(&ar).flag("-march=rv32im").flag("-mabi=ilp32").flag("-std=c++20")
        .flag("-fexceptions").flag("-fno-rtti").opt_level(3).warnings(false).flag("-fno-strict-aliasing")
        .flag("-ffunction-sections").flag("-fdata-sections")
        .include(&shim).include(&core).include(format!("{secp}/include"));
    // Hazync's Core-side levers, as the libzync profile sets them (this driver calls
    // verify_inputs_batch directly). Each is "a cheaper way to compute the same bytes" (patches/libzync/0101).
    for d in ["HAZYNC_SHA_FASTPATH", "HAZYNC_SHA_D64_ACCEL", "HAZYNC_SHA_STREAM", "HAZYNC_HASHWRITER_BUF",
              "HAZYNC_LAZY_TXID", "HAZYNC_H160_MEMO", "HAZYNC_LEGACY_SIGHASH", "HAZYNC_D64_STREAM",
              "HAZYNC_RAW_TXID", "HAZYNC_PUBKEY_MEMO", "HAZYNC_LEAF_ONEBUF", "HAZYNC_TXDATA_LAZY", "HAZYNC_AGG_LITE", "HAZYNC_TX_ONCE"] { b.define(d, "1"); }
    if std::env::var("CARGO_FEATURE_NOSCRIPTS").is_ok() { b.define("HAZYNC_NO_SCRIPTS", "1"); }
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_NOSCRIPTS");
    for t in tus { b.file(format!("{core}/{t}")); }
    b.file(format!("{hzc}/verify_input.cpp")).file(format!("{hzc}/core_tx_rules.cpp"));
    // core_block_rules.cpp #includes Core's functions lifted VERBATIM out of validation.cpp / pow.cpp, exactly as
    // prover/methods/guest/build.rs does (same signatures, same "\n}\n" end rule).
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let validation = std::fs::read_to_string(format!("{core}/validation.cpp")).unwrap();
    let pow = std::fs::read_to_string(format!("{core}/pow.cpp")).unwrap();
    let mut inc = String::new();
    let lift = |src: &str, sig: &str, inc: &mut String| {
        let from = src.find(sig).unwrap_or_else(|| panic!("not found: {sig}"));
        let body = &src[from..]; let end = body.find("\n}\n").unwrap();
        inc.push_str(&body[..end + 3]); inc.push('\n');
    };
    lift(&pow, "unsigned int GetNextWorkRequired(const CBlockIndex* pindexLast, const CBlockHeader *pblock, const Consensus::Params& params)\n{", &mut inc);
    for sig in ["CAmount GetBlockSubsidy(int nHeight, const Consensus::Params& consensusParams)\n{",
                "static bool CheckWitnessMalleation(const CBlock& block, bool expect_witness_commitment, BlockValidationState& state)\n{",
                "static unsigned int GetBlockScriptFlags(const CBlockIndex& block_index, const ChainstateManager& chainman)\n{",
                "static bool ContextualCheckBlockHeader(const CBlockHeader& block, BlockValidationState& state, BlockManager& blockman, const ChainstateManager& chainman, const CBlockIndex* pindexPrev) EXCLUSIVE_LOCKS_REQUIRED(::cs_main)\n{",
                "static bool ContextualCheckBlock(const CBlock& block, BlockValidationState& state, const ChainstateManager& chainman, const CBlockIndex* pindexPrev)\n{"] {
        lift(&validation, sig, &mut inc);
    }
    std::fs::write(format!("{out_dir}/core_extract.inc"), inc).unwrap();
    b.include(&out_dir).file(format!("{hzc}/core_block_rules.cpp"));
    b.compile("core");
    let sd = lib_dir(&gcc, "-print-file-name=libstdc++.a"); let cd = lib_dir(&gcc, "-print-file-name=libc.a");
    let gd = lib_dir(&gcc, "-print-libgcc-file-name");
    for d in [sd, cd, gd] { println!("cargo:rustc-link-search=native={d}"); }
    for l in ["stdc++", "gcc", "c", "nosys"] { println!("cargo:rustc-link-lib=static={l}"); }
    println!("cargo:rustc-link-arg=--allow-multiple-definition");
}
