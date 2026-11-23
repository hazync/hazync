#![no_main]
sp1_zkvm::entrypoint!(main);
use sha2::Digest;
include!("../../../common/body.rs");

pub fn main() {
    let input = sp1_zkvm::io::read_vec();
    let out = bench_run(&input);
    sp1_zkvm::io::commit_slice(&out);
}
