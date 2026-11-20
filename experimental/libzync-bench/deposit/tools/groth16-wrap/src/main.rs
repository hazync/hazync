//! `groth16-wrap <succinct.rcpt> <out.rcpt> <image id hex>`: compress a succinct receipt to RISC Zero's Groth16 receipt
//! through r0vm (`identity_p254` on the CPU, then the Docker Groth16 prover `risczero/risc0-groth16-prover`), print the
//! wall time and sizes, and verify the result against the image id. A measurement tool for DEPOSIT_ZAPP.md §6; the
//! deposit verifier does not need it (it verifies the STARK receipts).
use risc0_zkvm::{default_prover, sha::Digest, InnerReceipt, ProverOpts, Receipt};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() != 3 { eprintln!("usage: groth16-wrap <succinct.rcpt> <out.rcpt> <image id hex>"); std::process::exit(1); }
    let bytes = std::fs::read(&a[0]).expect("read receipt");
    let r: Receipt = bincode::deserialize(&bytes).expect("a receipt");
    let id: [u8; 32] = (0..32).map(|i| u8::from_str_radix(&a[2][2 * i..2 * i + 2], 16).unwrap()).collect::<Vec<_>>().try_into().unwrap();
    let id = Digest::from_bytes(id);
    r.verify(id).expect("the input receipt verifies");
    assert!(matches!(r.inner, InnerReceipt::Succinct(_)), "the input must be a succinct receipt");
    let t = std::time::Instant::now();
    let g = default_prover().compress(&ProverOpts::groth16(), &r).expect("compress to Groth16");
    let secs = t.elapsed().as_secs_f64();
    let out = bincode::serialize(&g).unwrap();
    std::fs::write(&a[1], &out).expect("write");
    let t2 = std::time::Instant::now();
    g.verify(id).expect("the Groth16 receipt verifies");
    let vsecs = t2.elapsed().as_secs_f64();
    let seal = match &g.inner { InnerReceipt::Groth16(x) => x.seal.len(), _ => panic!("not a Groth16 receipt") };
    println!("GROTH16_WRAP in {secs:.1} s: succinct {} bytes -> Groth16 receipt {} bytes (seal {} bytes, journal {} bytes); verified in {:.3} s",
             bytes.len(), out.len(), seal, g.journal.bytes.len(), vsecs);
}
