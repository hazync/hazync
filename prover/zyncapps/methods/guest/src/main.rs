//! zyncAPP "coin": proves, from a verified core receipt, that a coin is unspent or was spent by (height, tx, vin) at the
//! core proof's tip (docs/libzync/ZYNCAPPS.md §4-§5). Its own image; it never changes the core.
//!
//! Input:  core_id [u32; 8], core journal bytes, signet flag, CoinQuery
//! Checks: env::verify(core_id, journal) (an assumption the host resolves with the core receipt);
//!         check_core_statement (kind, version >= 2, self_id == core_id, network, scripts checked, coin log);
//!         the coin fact against the journal's coin-log roots.
//! Commits: ZyncCoin: core_id, the core statement it rests on (lineage, anchor_id, height, tip_hash, cum_work,
//!          max_time, profile), and the answer. The final verifier pins core_id to a known core image and anchor_id
//!          to the network's genesis anchor (policy, as for the core proof itself).
#![no_main]
extern crate alloc;
use alloc::vec::Vec;
use risc0_zkvm::guest::env;
use serde::{Deserialize, Serialize};

risc0_zkvm::guest::entry!(main);

#[path = "../../../../../libzync/src/coinlog.rs"]
mod coinlog;
#[path = "../../../../../libzync/src/facts.rs"]
#[allow(dead_code)]
mod facts;

/// The core ChainState, field for field (prover/methods/guest/src/main.rs): risc0 serde reads fields in order, so every
/// field up to the last one used must be declared; later (appended) fields are ignored.
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
    ss_set: u32,
    ss_acc: [u32; 16],
    ss_hints: [u8; 32],
    smt_root: [u8; 32],
    max_time: u32,
    profile: u32,
    block_roots: Vec<Option<[u8; 32]>>,
    block_leaves: u64,
    lineage: [u8; 32],
}

#[derive(Deserialize, Serialize)]
pub struct CoinQuery {
    pub pos: u64,
    pub coin: [u8; 32],
    pub spent_by: Option<(u32, u32, u32)>,
    pub path: Vec<[u8; 32]>,
}

#[derive(Serialize, Deserialize)]
pub struct ZyncCoin {
    pub kind: u32,
    pub core_id: [u32; 8],
    pub lineage: [u8; 32],
    pub anchor_id: [u8; 32],
    pub height: u32,
    pub tip_hash: [u8; 32],
    pub cum_work: [u8; 32],
    pub max_time: u32,
    pub profile: u32,
    pub pos: u64,
    pub coin: [u8; 32],
    pub spent_by: Option<(u32, u32, u32)>,
}

pub const KIND_ZYNC_COIN: u32 = 0x5A43_0001;

fn main() {
    let core_id: [u32; 8] = env::read();
    let journal: Vec<u8> = env::read();
    let signet: bool = env::read();
    let q: CoinQuery = env::read();
    env::verify(core_id, &journal).expect("the core receipt does not verify under core_id");
    let words: Vec<u32> = journal.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    assert!(journal.len() % 4 == 0, "core journal is not whole words");
    let j: ChainState = risc0_zkvm::serde::from_slice(&words).expect("core journal is not a ChainState");
    let st = facts::CoreStatement { kind: j.kind, version: j.version, profile: j.profile, self_id: &j.self_id, anchor_id: &j.anchor_id, ss_set: j.ss_set as u64 };
    // The anchor is policy (the final verifier pins it), so the guest compares it with itself here.
    facts::check_core_statement(&st, &core_id, 2, if signet { facts::Network::Signet } else { facts::Network::Mainnet }, false, &j.anchor_id).unwrap_or_else(|e| panic!("core statement: {e}"));
    let ok = match q.spent_by {
        None => facts::coin_unspent(&j.utxo_roots, j.utxo_leaves, q.pos, &q.coin, &q.path),
        Some((h, t, v)) => h <= j.height && facts::coin_spent_by(&j.utxo_roots, j.utxo_leaves, q.pos, &q.coin, h, t, v, &q.path),
    };
    assert!(ok, "the coin is not in that state at that position");
    env::commit(&ZyncCoin {
        kind: KIND_ZYNC_COIN, core_id, lineage: j.lineage, anchor_id: j.anchor_id, height: j.height, tip_hash: j.tip_hash,
        cum_work: j.cum_work, max_time: j.max_time, profile: j.profile, pos: q.pos, coin: q.coin, spent_by: q.spent_by,
    });
}
