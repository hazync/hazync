fn main() {
    for f in ["../../../../libzync/src/coinlog.rs", "../../../../libzync/src/ttree.rs", "../../../../coinbase-smt/src/roots.rs",
              "../../../../prover/methods/guest/src/swiftsync.rs", "../../../../prover/methods/guest/src/batch_ecdsa.rs",
              "guest/src/main.rs", "guest/Cargo.toml", "../tsi.rs", "../gcommon.rs",
              "chunk/src/main.rs", "chunk/Cargo.toml", "combine/src/main.rs", "combine/Cargo.toml",
              "bip30/src/main.rs", "bip30/Cargo.toml", "join/src/main.rs", "join/Cargo.toml", "../join_state.rs"] {
        println!("cargo:rerun-if-changed={f}");
    }
    // TS_CONTROLS=1: the negative-control build (TRUSTED_STATE.md §13): the chunk, bip30, combine and join guests read a cut mask
    // and skip the checks it names. Build it into its own target directory; its image ids differ from the real ones.
    println!("cargo:rerun-if-env-changed=TS_CONTROLS");
    println!("cargo:rustc-check-cfg=cfg(ts_controls)");
    if std::env::var("TS_CONTROLS").as_deref() == Ok("1") {
        println!("cargo:rustc-cfg=ts_controls");
        let opt = risc0_build::GuestOptionsBuilder::default().features(vec!["controls".to_string()]).build().unwrap();
        let mut m = std::collections::HashMap::new();
        for g in ["ts-chunk", "ts-bip30", "ts-combine", "ts-join"] { m.insert(g, opt.clone()); }
        risc0_build::embed_methods_with_options(m);
    } else {
        risc0_build::embed_methods();
    }
}
