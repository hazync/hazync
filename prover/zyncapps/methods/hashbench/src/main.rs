//! E1 calibration (experimental/libzync-bench/absence): cycles per tree-node hash in the zkVM, for the two node hashes
//! an absence tree could use. Execute mode only; commits nothing useful.
//!   mode 0: SHA-256(0x01 ‖ L ‖ R), 65 bytes = 2 compressions (the block forest's node hash)
//!   mode 1: one compression of L ‖ R from a per-level IV (the coin log's node hash, coinlog::parent)
#![no_main]
extern crate alloc;
use risc0_zkvm::guest::env;
risc0_zkvm::guest::entry!(main);

#[path = "../../../../../libzync/src/coinlog.rs"]
#[allow(dead_code)]
mod coinlog;

fn main() {
    let (mode, n): (u32, u32) = env::read();
    let mut a = [7u8; 32];
    let b = [9u8; 32];
    let c0 = env::cycle_count();
    for i in 0..n {
        a = match mode {
            0 => { use sha2::{Digest, Sha256}; let mut h = Sha256::new(); h.update([1u8]); h.update(a); h.update(b); h.finalize().into() }
            _ => coinlog::parent(1 + (i % 27), &a, &b),
        };
    }
    let c1 = env::cycle_count();
    env::log(&alloc_fmt(mode, n, c1 - c0));
    env::commit(&a);
}

fn alloc_fmt(mode: u32, n: u32, c: u64) -> String {
    format!("HASHBENCH mode {mode} n {n} cycles {c} per_hash {:.1}", c as f64 / n as f64)
}
