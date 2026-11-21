//! zyncAPP "utreexo": proves that Floresta's Utreexo accumulator (roots and leaf count) after block H is exactly the
//! one a Floresta node holds after validating blocks 1..H, from a verified core receipt (docs/libzync/UTREEXO_ZAPP.md).
//! Its own image; it never changes the core.
//!
//! A proof is one run or a chain of runs over consecutive block ranges. A run without a previous run starts from
//! genesis (the guest builds that state itself); a run with one verifies the previous run's receipt under `self_id`
//! (env::verify), requires its committed `self_id` to be the same and its end-state digest to be the SHA-256 of the
//! state bytes the host gives, and continues from that state. The run that ends at H checks the core receipt.
//!
//! Input:  self_id [u32; 8], core_id [u32; 8], core journal bytes (empty unless this run ends at H), signet flag, H,
//!         the genesis header, a profiling step (0 = off), `to` (the last block of this run, <= H), the previous run:
//!         [0] or [1] ‖ its journal words (Vec<u8>) ‖ its end state (Vec<u8>); then per block one frame of words:
//!         [L, k, s] ‖ the block (header ‖ count ‖ transactions without witness, L bytes, zero-padded to words) ‖ k
//!         tracked positions ‖ s status words ‖ state (8) ‖ undo (8); if `to` == H: [T - H] and T - H block-forest
//!         leaves (8 words each).
//! Checks (when `to` == H): env::verify(core_id, journal); check_core_statement (with the network's pinned genesis
//!         anchor); zero lineage and not a pre-undo image, so every block 1..T has the 0x05 leaf; the walk's block forest
//!         == the journal's (which binds every header, the coin state at H and so every spent/unspent claim).
//! Commits: UtreexoStatement.
#![no_main]
extern crate alloc;
use alloc::vec::Vec;
use risc0_zkvm::guest::env;
use serde::{Deserialize, Serialize};

risc0_zkvm::guest::entry!(main);

#[path = "../../../../../libzync/src/coinlog.rs"]
mod coinlog;
#[path = "../../../../../libzync/src/facts.rs"]
#[allow(dead_code)] // a shared libzync file (#[path], hazync#88): each includer uses its own subset
mod facts;
#[path = "../../../../../libzync/src/utreexo_roots.rs"]
#[allow(dead_code)]
mod utreexo_roots;
use utreexo_roots::Deriver;

/// The core ChainState, field for field (prover/methods/guest/src/main.rs).
#[derive(Deserialize)]
#[allow(dead_code)]
struct ChainState {
    kind: u32,
    version: u32,
    tip_hash: [u8; 32],
    utxo_roots: Vec<Option<[u8; 32]>>,
    utxo_leaves: u64,
    utxo_value: u64,
    cum_work: [u8; 32],
    height: u32,
    prev_nbits: u32,
    prev_time: u32,
    epoch_start: u32,
    recent_times: Vec<u32>,
    anchor_id: [u8; 32],
    self_id: [u32; 8],
    live_set: u32,
    live_hash: [u32; 16],
    trie_root: [u8; 32],
    smt_root: [u8; 32],
    max_time: u32,
    profile: u32,
    block_roots: Vec<Option<[u8; 32]>>,
    block_leaves: u64,
    lineage: [u8; 32],
}

/// What a run commits. `done` = 0: a run below H (only `self_id`, `network`, `height`, `to` and `state_out` mean
/// anything; the next run continues from it). `done` = 1: the run that ended at H and checked the core receipt; the
/// Utreexo fields are Floresta's `AssumeUtreexoValue` (block_hash, height, roots, leaves).
#[derive(Serialize, Deserialize, Default)]
pub struct UtreexoStatement {
    pub kind: u32,
    pub self_id: [u32; 8],
    pub network: u32,
    pub height: u32,
    pub to: u32,
    pub runs: u32,
    pub state_out: [u8; 32],
    pub done: u32,
    pub block_hash: [u8; 32],
    pub leaves: u64,
    pub roots: Vec<[u8; 32]>,
    pub n_live: u64,
    pub core_id: [u32; 8],
    pub lineage: [u8; 32],
    pub anchor_id: [u8; 32],
    pub core_height: u32,
    pub tip_hash: [u8; 32],
    pub cum_work: [u8; 32],
    pub max_time: u32,
    pub profile: u32,
}

// The core's own pin file (PRE_UNDO_IMAGES: the images whose blocks have the 0x02 leaf), included, not copied.
#[allow(dead_code)]
mod handover_pin {
    include!("../../../../methods/guest/src/handover_pin.rs");
}

/// The networks' PINNED genesis anchors (swiftsync-verify IMAGES `genesis_anchor_hex`, HANDOVER.md).
const MAINNET_ANCHOR: [u8; 32] = hex32("852c5c7e009317c2dd232dcf9b1a9e2babd58df324add461a6ff61824f390167");
const SIGNET_ANCHOR: [u8; 32] = hex32("3b67ead0618b7786b001525b68c7b088405d6a577292838361cc07e5bb938827");

const fn nib(c: u8) -> u8 {
    if c >= b'a' { c - b'a' + 10 } else { c - b'0' }
}

const fn hex32(s: &str) -> [u8; 32] {
    let b = s.as_bytes();
    let mut o = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        o[i] = nib(b[2 * i]) << 4 | nib(b[2 * i + 1]);
        i += 1;
    }
    o
}

fn words_to_bytes(w: &[u32], n: usize) -> Vec<u8> {
    let mut b: Vec<u8> = w.iter().flat_map(|x| x.to_le_bytes()).collect();
    b.truncate(n);
    b
}

fn h_of(w: &[u32]) -> [u8; 32] {
    let mut o = [0u8; 32];
    for (i, x) in w.iter().enumerate() {
        o[4 * i..4 * i + 4].copy_from_slice(&x.to_le_bytes());
    }
    o
}

fn words(b: &[u8]) -> Vec<u32> {
    assert!(b.len() % 4 == 0, "a journal is whole words");
    b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()
}

fn main() {
    let self_id: [u32; 8] = env::read();
    let core_id: [u32; 8] = env::read();
    let journal: Vec<u8> = env::read();
    let signet: bool = env::read();
    let stop: u32 = env::read();
    let genesis: Vec<u8> = env::read();
    let prof: u32 = env::read();
    let to: u32 = env::read();
    let has_prev: u32 = env::read();
    let network = if signet { facts::Network::Signet } else { facts::Network::Mainnet };
    let (mut d, runs) = if has_prev == 0 {
        let g: [u8; 80] = genesis.as_slice().try_into().expect("the genesis header is 80 bytes");
        (Deriver::new(stop, &g, !signet).unwrap_or_else(|e| panic!("{e}")), 1u32)
    } else {
        let pj: Vec<u8> = env::read();
        let ps: Vec<u8> = env::read();
        env::verify(self_id, &pj).expect("the previous run does not verify under self_id");
        let p: UtreexoStatement = risc0_zkvm::serde::from_slice(&words(&pj)).expect("the previous run's journal");
        assert!(p.kind == utreexo_roots::KIND_UTREEXO && p.self_id == self_id && p.done == 0, "the previous run is not a run of this image below H");
        assert!(p.network == network.bits() && p.height == stop, "the previous run is for another network or H");
        let sd: [u8; 32] = {
            use sha2::Digest;
            sha2::Sha256::digest(&ps).into()
        };
        assert!(sd == p.state_out, "the state given is not the previous run's end state");
        let d = Deriver::decode(&ps).unwrap_or_else(|e| panic!("{e}"));
        assert!(d.height() == p.to && d.stop() == stop, "the previous run's state does not end where its statement says");
        (d, p.runs + 1)
    };
    assert!(to > d.height() && to <= stop, "a run covers at least one block, and none above H");
    if prof > 0 {
        d.prof.clock = Some(env::cycle_count);
    }
    for h in d.height() + 1..=to {
        let mut fr = [0u32; 3];
        env::read_slice(&mut fr);
        let (l, k, s) = (fr[0] as usize, fr[1] as usize, fr[2] as usize);
        let nw = l.div_ceil(4);
        let mut body = alloc::vec![0u32; nw + k + s + 16];
        env::read_slice(&mut body);
        let block = words_to_bytes(&body[..nw], l);
        let (state, undo) = (h_of(&body[nw + k + s..nw + k + s + 8]), h_of(&body[nw + k + s + 8..]));
        d.block(&block, &body[nw..nw + k], &body[nw + k..nw + k + s], &state, &undo).unwrap_or_else(|e| panic!("block {h}: {e}"));
        if prof > 0 && h % prof == 0 {
            let p = &d.prof;
            env::log(&alloc::format!(
                "UTX_CYC h={h} cycles={} utreexo_cycles={} leaf_hashes={} leaf_blocks={} parent_hashes={} empty_adds={} log_leaves={} outputs={} inputs={} txs={} bytes={}",
                env::cycle_count(), p.utreexo_cycles, p.leaf_hashes, p.leaf_blocks, p.parent_hashes, p.empty_adds, p.log_leaves, p.outputs, p.inputs, p.txs, p.bytes
            ));
        }
    }
    let mut z = UtreexoStatement { kind: utreexo_roots::KIND_UTREEXO, self_id, network: network.bits(), height: stop, to, runs, ..Default::default() };
    if to < stop {
        z.state_out = d.digest();
        env::commit(&z);
        return;
    }
    // the run that ends at H: the core receipt, then the block forest above H
    env::verify(core_id, &journal).expect("the core receipt does not verify under core_id");
    let j: ChainState = risc0_zkvm::serde::from_slice(&words(&journal)).expect("core journal is not a ChainState");
    let st = facts::CoreStatement { kind: j.kind, version: j.version, profile: j.profile, self_id: &j.self_id, anchor_id: &j.anchor_id, live_set: j.live_set as u64 };
    let anchor = if signet { &SIGNET_ANCHOR } else { &MAINNET_ANCHOR };
    facts::check_core_statement(&st, &core_id, 2, network, false, anchor).unwrap_or_else(|e| panic!("core statement: {e}"));
    assert!(j.lineage == [0u8; 32], "the core chain has a handover: blocks below it may lack the undo commitment");
    assert!(!handover_pin::PRE_UNDO_IMAGES.contains(&core_id), "a pre-undo core image: its blocks carry no undo hash");
    assert!(stop <= j.height, "H must be at most the core height");
    let mut n = [0u32; 1];
    env::read_slice(&mut n);
    assert!(n[0] == j.height - stop, "block-forest leaves above H: want T - H");
    let mut left = n[0] as usize;
    while left > 0 {
        let c = left.min(4096);
        let mut buf = alloc::vec![0u32; 8 * c];
        env::read_slice(&mut buf);
        for x in buf.chunks_exact(8) {
            d.tail_leaf(&h_of(x)).unwrap_or_else(|e| panic!("{e}"));
        }
        left -= c;
    }
    let r = d.finish().unwrap_or_else(|e| panic!("{e}"));
    assert!(r.block_leaves == j.block_leaves && r.block_roots == utreexo_roots::trimmed(&j.block_roots),
            "the block forest is not the core proof's: a header, coin state or undo hash differs from what it proved");
    if stop == j.height {
        assert!(r.block_hash == j.tip_hash && r.utxo_leaves == j.utxo_leaves && r.utxo_roots == utreexo_roots::trimmed(&j.utxo_roots),
                "the coin log at H is not the core proof's");
    }
    z.done = 1;
    z.block_hash = r.block_hash;
    z.leaves = r.utreexo.leaves;
    z.roots = r.utreexo.roots();
    z.n_live = r.n_live;
    z.core_id = core_id;
    z.lineage = j.lineage;
    z.anchor_id = j.anchor_id;
    z.core_height = j.height;
    z.tip_hash = j.tip_hash;
    z.cum_work = j.cum_work;
    z.max_time = j.max_time;
    z.profile = j.profile;
    env::commit(&z);
}
