//! EXPERIMENTAL Trusted State chunked ingest, the COMBINE (docs/libzync/TRUSTED_STATE.md §12). MEASUREMENT ONLY, its
//! own image. Verifies N chunk receipts of the chunk image (its id an input, committed), requires their states to chain
//! (in_0 = the initial state, out_i = in_(i+1), out_last = the final state given here) and every chunk to have used the
//! header times of the headers given here, then finishes what no chunk can: hash_serialized_3 (the carried SHA-256
//! midstate, padded, hashed again), the last txid group, the BIP30 tree from its entry list (the list must hash to the
//! state's entry chain), the block forest. Commits exactly the one-shot ingest guest's journal, then the chunk image id
//! and N, so the anchor (its link digest, ts-anchor join-receipt) is the same as the one-shot ingest's.
#![no_main]
extern crate alloc;
use alloc::vec::Vec;
use risc0_zkvm::guest::env;
risc0_zkvm::guest::entry!(main);

#[path = "../../../../../../libzync/src/coinlog.rs"]
#[allow(dead_code)]
mod coinlog;
#[path = "../../../../../../coinbase-smt/src/roots.rs"]
#[allow(dead_code)]
mod smt;
#[path = "../../../tsi.rs"]
#[allow(dead_code)]
mod tsi;
#[path = "../../../gcommon.rs"]
#[allow(dead_code)]
mod gcommon;

use gcommon::*;
use tsi::sha;

const UNKNOWN_STATE_TAG: &[u8] = b"hazync trusted-state: the coin state after this block is not known (below the anchor)";
const REPEATABLE: u32 = u32::MAX - 1;

fn dsha(b: &[u8]) -> [u8; 32] { sha(&sha(b)) }

fn stump_add(roots: &mut Vec<Option<[u8; 32]>>, leaf: [u8; 32]) {
    let mut node = leaf;
    let mut h = 0;
    while h < roots.len() && roots[h].is_some() {
        let l = roots[h].take().unwrap();
        let mut p = [0u8; 65];
        p[0] = 1;
        p[1..33].copy_from_slice(&l);
        p[33..].copy_from_slice(&node);
        node = sha(&p);
        h += 1;
    }
    if h == roots.len() { roots.push(None); }
    roots[h] = Some(node);
}

fn main() {
    let c0 = env::cycle_count();
    let chunk_id: [u32; 8] = env::read();
    let n = read_u32() as usize;
    assert!(n > 0, "no chunks");
    let mut j: Vec<Vec<u32>> = Vec::with_capacity(n);
    for _ in 0..n {
        let w = read_words(26);
        env::verify(chunk_id, &w).expect("chunk receipt");
        j.push(w);
    }
    let c1 = env::cycle_count();
    let sfin = tsi::ChunkState::from_bytes(&read_section()).expect("S_final: not a canonical ChunkState");
    let hdr = read_section();
    let entries = read_section();
    let rep = read_section();
    let c1b = env::cycle_count();
    // the chain of states: no chunk dropped, repeated, reordered or overlapping
    let words = |d: [u8; 32]| -> Vec<u32> { d.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect() };
    assert!(j[0][..8] == words(tsi::ChunkState::initial().digest())[..], "chunk 0 does not start from the initial state");
    for i in 1..n { assert!(j[i][..8] == j[i - 1][8..16], "chunk {i} does not start where chunk {} ends", i - 1); }
    assert!(j[n - 1][8..16] == words(sfin.digest())[..], "the last chunk does not end at the final state");
    // headers 0..H, linked, ending at the snapshot's base block; every chunk used exactly their times
    assert!(!hdr.is_empty() && hdr.len() % 80 == 0, "headers");
    let mut hashes: Vec<[u8; 32]> = Vec::with_capacity(hdr.len() / 80);
    let mut times_b: Vec<u8> = Vec::with_capacity(hdr.len() / 20);
    for (i, h) in hdr.chunks_exact(80).enumerate() {
        if i > 0 { assert!(h[4..36] == hashes[i - 1], "header {i} does not link"); }
        hashes.push(dsha(h));
        times_b.extend_from_slice(&h[68..72]);
    }
    let h_tip = (hashes.len() - 1) as u32;
    let tip_hash = hashes[h_tip as usize];
    assert!(sfin.started == 1 && sfin.base == tip_hash, "the snapshot's base block is not the headers' tip");
    let th = words(sha(&times_b));
    for (i, x) in j.iter().enumerate() { assert!(x[16..24] == th[..], "chunk {i} used other header times"); }
    // the whole file: every coin it announces, the last group complete
    assert!(sfin.idx == sfin.coins_total && sfin.log_n == sfin.coins_total && sfin.g_left == 0, "the snapshot is not complete");
    let coins: u64 = j.iter().map(|x| x[24] as u64 | (x[25] as u64) << 32).sum();
    assert!(coins == sfin.coins_total, "chunk coin counts do not add up");
    let c2 = env::cycle_count();
    let mut fin = sfin.clone();
    fin.close_group();
    let hash3 = sha(&fin.h3.clone().finalize());
    // the BIP30 entries: the list given must be the chunks' chain, entry for entry
    assert!(entries.len() % 36 == 0 && rep.len() % 32 == 0, "entry lists");
    let mut chain = [0u8; 32];
    let mut e: Vec<([u8; 32], u32)> = Vec::with_capacity(entries.len() / 36 + rep.len() / 32);
    for x in entries.chunks_exact(36) {
        let mut p = [0u8; 68];
        p[..32].copy_from_slice(&chain);
        p[32..].copy_from_slice(x);
        chain = sha(&p);
        e.push((x[..32].try_into().unwrap(), u32::from_le_bytes(x[32..].try_into().unwrap())));
    }
    assert!(chain == fin.smt_chain && e.len() as u64 == fin.smt_n, "the BIP30 entry list is not the chunks' chain");
    e.extend(rep.chunks_exact(32).map(|r| (<[u8; 32]>::try_from(r).unwrap(), REPEATABLE)));
    e.sort_unstable_by(|x, y| x.0.cmp(&y.0));
    assert!(e.windows(2).all(|w| w[0].0 < w[1].0), "BIP30 entries: a key twice (or a REPEATABLE txid with live coins)");
    let c2b = env::cycle_count();
    let empty = tsi::empty_sys();
    let smt_root = tsi::smt_bulk(&e, 0, &empty, &tsi::leaf_sys, &tsi::node_sys);
    let c3 = env::cycle_count();
    // the block forest: H leaves with the UNKNOWN state, then H with the coin state
    let roots: Vec<Option<[u8; 32]>> = {
        let mut r: Vec<Option<[u8; 32]>> = fin.log_roots.iter().map(|x| x.map(|w| coinlog::h_of(&w))).collect();
        while r.last() == Some(&None) { r.pop(); }
        r
    };
    let mut st = Vec::new();
    st.push(4u8);
    st.extend_from_slice(&fin.idx.to_le_bytes());
    st.extend_from_slice(&fin.value.to_le_bytes());
    for r in roots.iter().rev().flatten() { st.extend_from_slice(r); }
    let st_h = sha(&st);
    let unknown = sha(UNKNOWN_STATE_TAG);
    let mut bf: Vec<Option<[u8; 32]>> = Vec::new();
    for (i, h) in hashes.iter().enumerate() {
        let mut p = [0u8; 69];
        p[0] = 2;
        p[1..33].copy_from_slice(h);
        p[33..37].copy_from_slice(&(i as u32).to_le_bytes());
        p[37..].copy_from_slice(if i as u32 == h_tip { &st_h } else { &unknown });
        stump_add(&mut bf, sha(&p));
    }
    while bf.last() == Some(&None) { bf.pop(); }
    let c4 = env::cycle_count();
    // the one-shot ingest guest's journal, then the chunk image and N
    let mut out: Vec<u32> = Vec::new();
    let w8 = |o: &mut Vec<u32>, b: &[u8; 32]| for k in 0..8 { o.push(u32::from_le_bytes(b[4 * k..4 * k + 4].try_into().unwrap())); };
    out.push(h_tip);
    w8(&mut out, &tip_hash);
    w8(&mut out, &hash3);
    out.push(u32::from_le_bytes(fin.magic));
    out.extend_from_slice(&[fin.idx as u32, (fin.idx >> 32) as u32, fin.value as u32, (fin.value >> 32) as u32, fin.live_set]);
    out.extend_from_slice(&fin.live_hash);
    w8(&mut out, &coinlog::h_of(&fin.trie_root));
    w8(&mut out, &smt_root);
    out.push(e.len() as u32);
    for rs in [&roots, &bf] {
        out.push(rs.len() as u32);
        for r in rs.iter() { match r { Some(h) => { out.push(1); w8(&mut out, h); } None => out.push(0) } }
    }
    out.extend_from_slice(&chunk_id);
    out.push(n as u32);
    env::commit_slice(&out);
    let c5 = env::cycle_count();
    env::log(&alloc::format!("TSX verify {} read {} chain+headers {} close+hash3+entries {} smt {} forest {} commit {} chunks={n} smt_n={} h={h_tip}",
                             c1 - c0, c1b - c1, c2 - c1b, c2b - c2, c3 - c2b, c4 - c3, c5 - c4, e.len()));
}
