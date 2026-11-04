//! E1 split measurement (absence/README.md): the counting trie updated in its OWN receipt, as it would be proven in
//! parallel with the chunks; the aggregate then checks only the update list's digest. Execute mode only.
//! Input (read_slice, from the core host's HAZYNC_TRIE_DUMP): [marks, inserts, witness words], (marks + inserts) keys
//! of 16 big-endian words, then the witness (ttree::encode). Spenders and values are stand-ins: the same work.
//! Commits the new root and the list digest.
#![no_main]
extern crate alloc;
use alloc::vec::Vec;
use risc0_zkvm::guest::env;
risc0_zkvm::guest::entry!(main);

#[path = "../../../../../libzync/src/coinlog.rs"]
#[allow(dead_code)]
mod coinlog;
#[path = "../../../../../libzync/src/ttree.rs"]
#[allow(dead_code)]
mod ttree;

fn main() {
    let mut h = [0u32; 3];
    env::read_slice(&mut h);
    let (nm, ni, nw) = (h[0] as usize, h[1] as usize, h[2] as usize);
    let mut keys = alloc::vec![0u32; 16 * (nm + ni)];
    env::read_slice(&mut keys);
    let mut wit = alloc::vec![0u32; nw];
    env::read_slice(&mut wit);
    let key = |i: usize| -> ttree::Key {
        let mut k = [0u8; 64];
        for j in 0..16 { k[4 * j..4 * j + 4].copy_from_slice(&keys[16 * i + j].to_be_bytes()); }
        k
    };
    let marks: Vec<(ttree::Key, coinlog::W)> = (0..nm).map(|i| (key(i), [962_000, i as u32, 0, 0, 0, 0, 0, 0])).collect();
    let ins: Vec<(ttree::Key, Option<coinlog::W>, u64)> = (0..ni)
        .map(|i| (key(nm + i), (i % 4 == 0).then_some([962_000, i as u32, 1, 0, 0, 0, 0, 0]), 10_000 + i as u64)).collect();
    let root = ttree::apply_stream(&wit, &marks, &ins).expect("trie update");
    let d = ttree::list_digest(&marks, &ins);
    env::commit_slice(&root);
    env::commit_slice(&d);
}
