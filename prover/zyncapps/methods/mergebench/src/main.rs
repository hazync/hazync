//! E1 option C measurement (absence/README.md): ONE merge step of a log-structured keyed tree. Execute mode only.
//!
//! Sorted runs of (64-byte key, kind) entries, kind 0 = live coin, 1 = tombstone (a spend of a coin in an older run).
//! The step: rebuild each input run's Merkle root from its entries (in a real chain, checked against the committed root),
//! check each run is strictly sorted, merge them, cancel a live entry against a tombstone of the same key, and build
//! the output run's root. Leaf = coinlog::parent_w(61 + kind, key halves); nodes = parent_w(level, L, R), odd promoted.
//! Input: read_slice words: [run count, entries per run...], then 17 words per entry (16 key words, kind).
#![no_main]
extern crate alloc;
use alloc::vec::Vec;
use risc0_zkvm::guest::env;
risc0_zkvm::guest::entry!(main);

#[path = "../../../../../libzync/src/coinlog.rs"]
#[allow(dead_code)]
mod coinlog;
use coinlog::{parent_w, W};

type E = ([u32; 16], u32);

fn leaf(e: &E) -> W {
    let (a, b): (W, W) = (e.0[..8].try_into().unwrap(), e.0[8..].try_into().unwrap());
    let mut o = [0u32; 8];
    parent_w(61 + e.1, &a, &b, &mut o);
    o
}

fn root(es: &[E]) -> W {
    let mut lvl: Vec<W> = es.iter().map(leaf).collect();
    let mut l = 1u32;
    while lvl.len() > 1 {
        let mut nx = Vec::with_capacity(lvl.len().div_ceil(2));
        for p in lvl.chunks(2) {
            if p.len() == 2 { let mut o = [0u32; 8]; parent_w(l, &p[0], &p[1], &mut o); nx.push(o) } else { nx.push(p[0]) }
        }
        lvl = nx;
        l += 1;
    }
    lvl.first().copied().unwrap_or([0; 8])
}

fn main() {
    let c0 = env::cycle_count();
    let mut hdr = [0u32; 1];
    env::read_slice(&mut hdr);
    let mut sizes = alloc::vec![0u32; hdr[0] as usize];
    env::read_slice(&mut sizes);
    let mut runs: Vec<Vec<E>> = Vec::new();
    for &n in &sizes {
        let mut w = alloc::vec![0u32; 17 * n as usize];
        env::read_slice(&mut w);
        runs.push(w.chunks_exact(17).map(|c| (c[..16].try_into().unwrap(), c[16])).collect());
    }
    let c1 = env::cycle_count();
    let mut in_roots = Vec::new();
    for r in &runs {
        assert!(r.windows(2).all(|p| p[0].0 < p[1].0), "run not strictly sorted");
        in_roots.push(root(r));
    }
    let c2 = env::cycle_count();
    // k-way merge: min-scan over each run head's cached 64-bit key prefix (full compare only on a prefix tie)
    let total: usize = runs.iter().map(|r| r.len()).sum();
    let mut out: Vec<E> = Vec::with_capacity(total);
    let pre = |e: &E| ((e.0[0] as u64) << 32) | e.0[1] as u64;
    let mut head = alloc::vec![0usize; runs.len()];
    let mut hp: Vec<u64> = runs.iter().map(|r| r.first().map_or(u64::MAX, pre)).collect();
    let mut live: Vec<bool> = runs.iter().map(|r| !r.is_empty()).collect();
    loop {
        let mut b = usize::MAX;
        for i in 0..runs.len() {
            if !live[i] { continue; }
            if b == usize::MAX || hp[i] < hp[b] || (hp[i] == hp[b] && runs[i][head[i]].0 < runs[b][head[b]].0) { b = i; }
        }
        if b == usize::MAX { break; }
        let e = runs[b][head[b]];
        head[b] += 1;
        if head[b] < runs[b].len() { hp[b] = pre(&runs[b][head[b]]); } else { live[b] = false; }
        match out.last() {
            Some(l) if l.0 == e.0 => {
                assert!(l.1 != e.1, "same key and kind twice");
                out.pop(); // a live coin and its tombstone cancel
            }
            _ => out.push(e),
        }
    }
    let c3 = env::cycle_count();
    let out_root = root(&out);
    let c4 = env::cycle_count();
    env::log(&alloc::format!("MERGEBENCH runs {} in {} out {} read {} in_roots {} merge {} out_root {}",
        runs.len(), total, out.len(), c1 - c0, c2 - c1, c3 - c2, c4 - c3));
    env::commit_slice(&out_root);
    for r in &in_roots { env::commit_slice(r); }
}
