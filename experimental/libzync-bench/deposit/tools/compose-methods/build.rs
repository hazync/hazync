// The composed deposit guest (DEPOSIT_ZAPP.md §2, option (b)): a measurement build, not a published image.
fn main() {
    for f in ["libzync/src/coinlog.rs", "libzync/src/facts.rs", "libzync/src/undo.rs", "prover/zyncapps/methods/headers/src/mmr.rs"] {
        println!("cargo:rerun-if-changed=../../../../../{f}");
    }
    risc0_build::embed_methods();
}
