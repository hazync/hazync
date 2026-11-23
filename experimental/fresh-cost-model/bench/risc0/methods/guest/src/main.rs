use risc0_zkvm::guest::env;
use sha2::Digest;
use std::io::Read;
include!("../../../../common/body.rs");

fn main() {
    let mut input = Vec::new();
    env::stdin().read_to_end(&mut input).unwrap();
    let out = bench_run(&input);
    env::commit_slice(&out);
}
