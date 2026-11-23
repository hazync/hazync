// OpenVM has no BIP340 Schnorr: the `schnorr` feature is deliberately absent, mode 2 is a no-op here.
use k256::Secp256k1Point;
use sha2::Digest;
openvm::init!();
include!("../../common/body.rs");

fn main() {
    let input: Vec<u8> = openvm::io::read_vec();
    let out = bench_run(&input);
    openvm::io::reveal_bytes32(out);
}
