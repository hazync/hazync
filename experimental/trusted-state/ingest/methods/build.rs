fn main() {
    for f in ["../../../../libzync/src/coinlog.rs", "../../../../libzync/src/ttree.rs", "../../../../coinbase-smt/src/roots.rs",
              "../../../../prover/methods/guest/src/swiftsync.rs", "../../../../prover/methods/guest/src/batch_ecdsa.rs",
              "guest/src/main.rs", "guest/Cargo.toml", "../tsi.rs", "../gcommon.rs",
              "chunk/src/main.rs", "chunk/Cargo.toml", "combine/src/main.rs", "combine/Cargo.toml"] {
        println!("cargo:rerun-if-changed={f}");
    }
    risc0_build::embed_methods();
}
