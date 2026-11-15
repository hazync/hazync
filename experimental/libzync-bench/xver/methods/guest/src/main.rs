// The prover guest's cross-version verifier, unchanged (#[path]), as a handover runs it: xver::verify on the input
// words, with the accelerated Poseidon2 suite. Input: params (32 bytes), image (8 u32), journal bytes, seal words.
// Panics (exit != Halted) exactly when the handover's guest would.
use risc0_zkvm::guest::env;

#[path = "../../../../../../prover/methods/guest/src/xver/mod.rs"]
mod xver;

fn main() {
    let params: [u8; 32] = env::read();
    let image: [u32; 8] = env::read();
    let journal: Vec<u8> = env::read();
    let n: u32 = env::read();
    let mut words = vec![0u32; n as usize];
    env::read_slice(&mut words);
    let c0 = env::cycle_count();
    xver::verify(&params, image, &journal, &words, &xver::accel::suite()).expect("predecessor chain proof invalid (cross-version)");
    let c1 = env::cycle_count();
    env::commit(&(params, image, c1 - c0));
}
