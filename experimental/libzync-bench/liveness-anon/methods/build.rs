fn main() {
    println!("cargo:rerun-if-changed=../../../../libzync/src/coinlog.rs");
    println!("cargo:rerun-if-changed=../../../../libzync/src/facts.rs");
    risc0_build::embed_methods();
}
