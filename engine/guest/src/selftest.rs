//! M1 self-test modes: a leaf commits a value; a node verifies two leaf (or node) proofs and commits their sum.
//! Journal layout: kind u32 LE | image 64 bytes (node only, zeros for a leaf) | value u64 LE.
use alloc::vec::Vec;

use crate::recursion::{read_child, verify_child, Image};

const KIND: u32 = 0x4E47_0001;

fn journal(image: &[u8; 64], value: u64) -> Vec<u8> {
    let mut j = Vec::with_capacity(76);
    j.extend_from_slice(&KIND.to_le_bytes());
    j.extend_from_slice(image);
    j.extend_from_slice(&value.to_le_bytes());
    j
}

fn value_of(j: &[u8]) -> u64 {
    assert!(j.len() == 76 && j[..4] == KIND.to_le_bytes(), "not a self-test journal");
    u64::from_le_bytes(j[68..76].try_into().unwrap())
}

pub fn leaf() {
    let v: u64 = openvm::io::read();
    // some work so the proof is not empty
    let mut x = v;
    for _ in 0..1000 { x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); }
    let _ = core::hint::black_box(x);
    crate::commit_journal(&journal(&[0u8; 64], v));
}

pub fn node() {
    let image = Image::read();
    let (lc, lj) = read_child();
    let (rc, rj) = read_child();
    verify_child(&image, &lc, &lj);
    verify_child(&image, &rc, &rj);
    // a child node must have recursed against the same image (a leaf carries zeros)
    for j in [&lj, &rj] {
        let img = &j[4..68];
        assert!(img.iter().all(|&b| b == 0) || img == image.bytes().as_slice(), "child recursed against another image");
    }
    crate::commit_journal(&journal(&image.bytes(), value_of(&lj) + value_of(&rj)));
}
