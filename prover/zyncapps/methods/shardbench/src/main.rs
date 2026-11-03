//! E1 sharding measurement (experimental/libzync-bench/absence/README.md): ONE shard prover of a key-prefix-sharded
//! indexed tree. Execute mode only; commits nothing a chain uses.
//!
//! The design measured: each tree split into 2^s shards by the key's top bits. Chunks commit, per shard, a multiset hash
//! (sum of SHA-256(key) mod 2^256) of the keys they insert and remove; a shard prover applies its shard's keys and commits
//! the same two sums with its new roots; the aggregate checks the sums match and folds 2^s roots. This guest is the shard
//! prover: decode the witness, sum the keys (the routing check's cost), apply.
//! Input: three Vec<u32>: tree witness (itree::encode_bench), removes (16 words per key), inserts (16 words per key).
#![no_main]
extern crate alloc;
use alloc::vec::Vec;
use risc0_zkvm::guest::env;
risc0_zkvm::guest::entry!(main);

#[path = "../../../../../libzync/src/coinlog.rs"]
#[allow(dead_code)]
mod coinlog;
#[path = "../../../../../libzync/src/itree.rs"]
#[allow(dead_code)]
mod itree;

fn keys(w: &[u32]) -> Vec<itree::Key> {
    w.chunks_exact(16).map(|c| {
        let mut k = [0u8; 64];
        for (j, x) in c.iter().enumerate() { k[4 * j..4 * j + 4].copy_from_slice(&x.to_be_bytes()); }
        k
    }).collect()
}

/// Multiset hash: sum of SHA-256(key) as 256-bit little-endian limbs, mod 2^256.
fn msum(ks: &[itree::Key]) -> [u32; 8] {
    use sha2::{Digest, Sha256};
    let mut acc = [0u32; 8];
    for k in ks {
        let d: [u8; 32] = Sha256::digest(k).into();
        let mut c = 0u64;
        for i in 0..8 {
            let s = acc[i] as u64 + u32::from_le_bytes(d[4 * i..4 * i + 4].try_into().unwrap()) as u64 + c;
            acc[i] = s as u32;
            c = s >> 32;
        }
    }
    acc
}

fn main() {
    let c0 = env::cycle_count();
    let wit: Vec<u32> = env::read();
    let rem = keys(&env::read::<Vec<u32>>());
    let ins = keys(&env::read::<Vec<u32>>());
    let (mut roots, mut n, touched, sib) = itree::decode_bench(&wit).expect("witness");
    let c1 = env::cycle_count();
    let (sr, si) = (msum(&rem), msum(&ins));
    let c2 = env::cycle_count();
    itree::apply(&mut roots, &mut n, &touched, &sib, &rem, &ins).expect("apply");
    let c3 = env::cycle_count();
    env::log(&alloc::format!("SHARDBENCH removes {} inserts {} touched {} siblings {} read {} msum {} apply {}",
        rem.len(), ins.len(), touched.len(), sib.len(), c1 - c0, c2 - c1, c3 - c2));
    env::commit(&(n, roots.iter().map(|r| r.unwrap_or([0; 8])).collect::<Vec<_>>(), sr, si));
}
