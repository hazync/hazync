fn main() {
    // the guest #[path]-includes the prover guest's xver module: rebuild when it changes
    println!("cargo:rerun-if-changed=../../../../prover/methods/guest/src/xver");
    println!("cargo:rerun-if-changed=guest");
    risc0_build::embed_methods();
}
