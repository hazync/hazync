fn main() {
    for f in ["../../../../libzync/src/coinlog.rs", "../../../../libzync/src/ttree.rs", "../../../../coinbase-smt/src/roots.rs",
              "../../../../prover/methods/guest/src/swiftsync.rs", "../../../../prover/methods/guest/src/batch_ecdsa.rs"] {
        println!("cargo:rerun-if-changed={f}");
    }
    risc0_build::embed_methods();
}
