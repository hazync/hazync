// The zyncAPP guests (docs/libzync/ZYNCAPPS.md). Separate images from the core: building them never moves the core's id.
fn main() {
    println!("cargo:rerun-if-changed=../../../libzync/src/coinlog.rs");
    println!("cargo:rerun-if-changed=../../../libzync/src/facts.rs");
    println!("cargo:rerun-if-changed=../../../libzync/src/ttree.rs");
    risc0_build::embed_methods();
}
