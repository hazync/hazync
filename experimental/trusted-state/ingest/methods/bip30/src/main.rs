//! EXPERIMENTAL Trusted State, one PIECE of the split BIP30 receipt (docs/libzync/TRUSTED_STATE.md §13.1). MEASUREMENT
//! ONLY, its own image. The BIP30 tree's subtrees at depth d for the key prefixes [p_lo, p_hi): given a contiguous run
//! of the entry list the chunks emitted (txid, live count; file order = key order) and the REPEATABLE txids in the same
//! range, it requires every key inside the range, each list strictly increasing and no key in both (a REPEATABLE txid
//! with live coins), carries the two entry chains (tsi::chain_step) from the given starting digests, and commits
//! [d, p_lo, p_hi, e_in, e_out, e_n, r_in, r_out, r_n, one root per prefix]. The combine binds the pieces: they tile
//! 0..2^d in order, the entry chain runs from zero to the chunks' smt_chain, and the counts add up.
#![no_main]
extern crate alloc;
use alloc::vec::Vec;
use risc0_zkvm::guest::env;
risc0_zkvm::guest::entry!(main);

#[path = "../../../tsi.rs"]
#[allow(dead_code)]
mod tsi;
#[path = "../../../gcommon.rs"]
#[allow(dead_code)]
mod gcommon;

use gcommon::*;
use tsi::*;

const REPEATABLE: u32 = u32::MAX - 1;

fn main() {
    let c0 = env::cycle_count();
    #[cfg(feature = "controls")]
    let cut: u32 = read_u32();
    #[cfg(not(feature = "controls"))]
    let cut: u32 = 0;
    let on = |bit: u32| cut & bit == 0;
    let d = read_u32();
    let p_lo = read_u32();
    let p_hi = read_u32();
    if on(CUT_BOUNDS) {
        assert!(d >= 1 && d <= MAX_DEPTH, "depth outside 1..{MAX_DEPTH}");
        assert!(p_lo < p_hi && p_hi <= 1u32 << d, "prefix range not inside 0..2^d");
    }
    let e_in: [u8; 32] = read_section().try_into().expect("e_in");
    let r_in: [u8; 32] = read_section().try_into().expect("r_in");
    let eb = read_section();
    let rb = read_section();
    assert!(eb.len() % 36 == 0 && rb.len() % 32 == 0, "entry lists");
    let c1 = env::cycle_count();
    let (mut ec, mut rc) = (e_in, r_in);
    let mut e: Vec<([u8; 32], u32)> = Vec::with_capacity(eb.len() / 36 + rb.len() / 32);
    let mut prev: Option<[u8; 32]> = None;
    for x in eb.chunks_exact(36) {
        let k: [u8; 32] = x[..32].try_into().unwrap();
        let v = u32::from_le_bytes(x[32..].try_into().unwrap());
        if on(CUT_ORDER) { assert!(prev.map_or(true, |p| k > p), "entries not strictly increasing"); }
        prev = Some(k);
        ec = chain_step(&ec, &k, v);
        e.push((k, v));
    }
    let ne = e.len();
    let mut rep: Vec<([u8; 32], u32)> = Vec::with_capacity(rb.len() / 32);
    let mut prev: Option<[u8; 32]> = None;
    for x in rb.chunks_exact(32) {
        let k: [u8; 32] = x.try_into().unwrap();
        if on(CUT_ORDER) { assert!(prev.map_or(true, |p| k > p), "REPEATABLE list not strictly increasing"); }
        prev = Some(k);
        rc = chain_step(&rc, &k, REPEATABLE);
        rep.push((k, REPEATABLE));
    }
    let nr = rep.len();
    // merge the two sorted lists; a key in both is a REPEATABLE txid with live coins
    let mut m: Vec<([u8; 32], u32)> = Vec::with_capacity(ne + nr);
    let (mut i, mut j) = (0, 0);
    while i < ne || j < nr {
        if j == nr || (i < ne && e[i].0 < rep[j].0) { m.push(e[i]); i += 1; } else { m.push(rep[j]); j += 1; }
    }
    if on(CUT_ORDER) { assert!(m.windows(2).all(|w| w[0].0 < w[1].0), "a key twice (a REPEATABLE txid with live coins)"); }
    if on(CUT_RANGE) {
        for x in &m { let p = prefix_of(&x.0, d); assert!(p >= p_lo && p < p_hi, "an entry outside the piece's prefix range"); }
    }
    let c2 = env::cycle_count();
    let empty = empty_sys();
    let roots = piece_roots(&m, d, p_lo, p_hi, &empty, &leaf_sys, &node_sys);
    let c3 = env::cycle_count();
    let mut out: Vec<u32> = Vec::with_capacity(PIECE_HDR_WORDS + 8 * roots.len());
    let w8 = |o: &mut Vec<u32>, b: &[u8; 32]| for k in 0..8 { o.push(u32::from_le_bytes(b[4 * k..4 * k + 4].try_into().unwrap())); };
    out.extend_from_slice(&[d, p_lo, p_hi]);
    w8(&mut out, &e_in);
    w8(&mut out, &ec);
    out.push(ne as u32);
    w8(&mut out, &r_in);
    w8(&mut out, &rc);
    out.push(nr as u32);
    for r in &roots { w8(&mut out, r); }
    env::commit_slice(&out);
    let c4 = env::cycle_count();
    env::log(&alloc::format!("TSB read {} chains+checks {} tree {} commit {} d={d} range={p_lo}..{p_hi} entries={ne} repeatable={nr} cut={cut}",
                             c1 - c0, c2 - c1, c3 - c2, c4 - c3));
}
