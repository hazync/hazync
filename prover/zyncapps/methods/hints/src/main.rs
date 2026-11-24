//! zyncAPP "hints": proves that a SwiftSync hints file for height H is exactly right, from a verified core receipt
//! (docs/libzync/HINTS_ZAPP.md). Its own image; it never changes the core.
//!
//! Input:  core_id [u32; 8], core journal bytes, signet flag, format, H, the genesis header, a profiling step (0 = off);
//!         then per block 1..=H one frame of words: [L, k, s] ‖ the block (header ‖ count ‖ transactions without
//!         witness, L bytes, zero-padded to words) ‖ k tracked positions ‖ s status words ‖ state (8) ‖ undo (8);
//!         then [T - H] and T - H block-forest leaves (8 words each).
//! Checks: env::verify(core_id, journal); check_core_statement (with the network's pinned genesis anchor); one image since genesis (zero lineage) and not a
//!         pre-undo image, so every block 1..T has the 0x05 leaf; libzync::hints::Deriver over 1..=H; its block
//!         forest == the journal's (which binds every header, the coin state at H and every undo hash).
//! Commits: HintsStatement.
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
#[path = "../../../../../libzync/src/hints.rs"]
#[allow(dead_code)]
mod hints;

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

#[derive(Serialize, Deserialize)]
pub struct HintsStatement {
    pub kind: u32,
    pub format: u32,
    pub height: u32,
    pub block_hash: [u8; 32],
    pub hints_sha256: [u8; 32],
    pub hints_len: u64,
    pub n_counted: u64,
    pub n_unspent: u64,
    pub unspent_value: u64,
    pub undo_from: u32,
    pub undo_sha256: [u8; 32],
    pub core_id: [u32; 8],
    pub lineage: [u8; 32],
    pub anchor_id: [u8; 32],
    pub core_height: u32,
    pub tip_hash: [u8; 32],
    pub cum_work: [u8; 32],
    pub max_time: u32,
    pub profile: u32,
}

// The core's own pin file (PRE_UNDO_IMAGES: the images whose blocks have the 0x02 leaf), included, not copied (review
// H5). Including it reads the file; it does not change the core image, whose source is untouched.
#[allow(dead_code)]
mod handover_pin {
    include!("../../../../methods/guest/src/handover_pin.rs");
}

// A core chain with a handover: the lineage list is checked against the journal's digest (core_lineage.rs).
#[path = "../../core_lineage.rs"]
mod core_lineage;

/// The networks' PINNED genesis anchors (swiftsync-verify IMAGES `genesis_anchor_hex`, HANDOVER.md): checked here so the
/// statement itself says the core chain starts at genesis, not only the verifier (review H4).
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

fn main() {
    let core_id: [u32; 8] = env::read();
    let journal: Vec<u8> = env::read();
    let signet: bool = env::read();
    let format: u32 = env::read();
    let stop: u32 = env::read();
    let genesis: Vec<u8> = env::read();
    let prof: u32 = env::read();
    let core_lineage: Vec<core_lineage::Entry> = env::read();
    env::verify(core_id, &journal).expect("the core receipt does not verify under core_id");
    assert!(journal.len() % 4 == 0, "core journal is not whole words");
    let words: Vec<u32> = journal.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    let j: ChainState = risc0_zkvm::serde::from_slice(&words).expect("core journal is not a ChainState");
    let st = facts::CoreStatement { kind: j.kind, version: j.version, profile: j.profile, self_id: &j.self_id, anchor_id: &j.anchor_id, live_set: j.live_set as u64 };
    let net = if signet { facts::Network::Signet } else { facts::Network::Mainnet };
    let anchor = if signet { &SIGNET_ANCHOR } else { &MAINNET_ANCHOR };
    facts::check_core_statement(&st, &core_id, 2, net, false, anchor).unwrap_or_else(|e| panic!("core statement: {e}"));
    // every block 1..T carries the undo-data commitment: the lineage given is the journal's, and no image of the chain
    // (the core image, or one it hands over from) is one before the commitment (core_lineage.rs)
    core_lineage::check(&core_id, &j.lineage, &core_lineage).unwrap_or_else(|e| panic!("{e}"));
    assert!(!handover_pin::PRE_UNDO_IMAGES.contains(&core_id), "a pre-undo core image: its blocks carry no undo hash");
    assert!(stop >= 1 && stop <= j.height, "H must be in 1..=the core height");
    let g: [u8; 80] = genesis.as_slice().try_into().expect("the genesis header is 80 bytes");
    let bip30: &'static [(u32, [u8; 32])] = if signet { &[] } else { &hints::MAINNET_BIP30 };
    let mut d = hints::Deriver::new(format, stop, &g, bip30, false).unwrap_or_else(|e| panic!("{e}"));
    for h in 1..=stop {
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
            env::log(&alloc::format!("HINTS_CYC h={h} cycles={}", env::cycle_count()));
        }
    }
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
    assert!(r.block_leaves == j.block_leaves && r.block_roots == hints::trimmed(&j.block_roots),
            "the block forest is not the core proof's: a header, coin state or undo hash differs from what it proved");
    if stop == j.height {
        assert!(r.block_hash == j.tip_hash && r.utxo_leaves == j.utxo_leaves && r.utxo_roots == hints::trimmed(&j.utxo_roots)
                && r.unspent_value == j.utxo_value, "the coin log at H is not the core proof's");
    }
    env::commit(&HintsStatement {
        kind: hints::KIND_HINTS, format, height: stop, block_hash: r.block_hash, hints_sha256: r.hints_sha256, hints_len: r.hints_len,
        n_counted: r.n_counted, n_unspent: r.n_unspent, unspent_value: r.unspent_value, undo_from: 1, undo_sha256: r.undo_sha256,
        core_id, lineage: j.lineage, anchor_id: j.anchor_id, core_height: j.height, tip_hash: j.tip_hash, cum_work: j.cum_work,
        max_time: j.max_time, profile: j.profile,
    });
}
