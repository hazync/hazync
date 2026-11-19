//! The composed deposit statement (DEPOSIT_ZAPP.md §2, option (b)), built to MEASURE its cost: one receipt that says
//! what `swiftsync-verify deposit` checks from two. Not a published image.
//!
//! Input: the core image id and its journal, the header-chain image id and its journal (one run from genesis), the core
//!        tip's MMR path, and the deposit (block height, header, its block-forest leaf and path, the raw transaction,
//!        its position and branch, the coinbase and its branch).
//! Checks: env::verify of both journals (assumptions resolved with the two receipts); the core statement (kind, version,
//!         self_id, mainnet, scripts checked, coin log, the genesis anchor); the header run starts from the empty
//!         state; the core tip is the header MMR's leaf at the core height (the binding); the block is in the core
//!         block forest (so at or below the core tip: fully validated); the transaction is at its position by its
//!         branch, the tree depth pinned by the coinbase, and is not 64 bytes.
//! Commits: both image ids, the core tip, the header tip and work, the block, the txid and every output, the depth.
#![no_main]
extern crate alloc;
use alloc::vec::Vec;
use risc0_zkvm::guest::env;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
risc0_zkvm::guest::entry!(main);

#[path = "../../../../../../../libzync/src/coinlog.rs"]
#[allow(dead_code)]
mod coinlog;
#[path = "../../../../../../../libzync/src/facts.rs"]
#[allow(dead_code)]
mod facts;
#[path = "../../../../../../../libzync/src/undo.rs"]
#[allow(dead_code)]
mod undo;
#[path = "../../../../../../../prover/zyncapps/methods/headers/src/mmr.rs"]
#[allow(dead_code)]
mod mmr;

/// The core ChainState, field for field (prover/methods/guest/src/main.rs).
#[derive(Deserialize)]
#[allow(dead_code)]
struct ChainState {
    kind: u32, version: u32, tip_hash: [u8; 32], utxo_roots: Vec<Option<[u8; 32]>>, utxo_leaves: u64, utxo_value: u64,
    cum_work: [u8; 32], height: u32, prev_nbits: u32, prev_time: u32, epoch_start: u32, recent_times: Vec<u32>,
    anchor_id: [u8; 32], self_id: [u32; 8], live_set: u32, live_hash: [u32; 16], trie_root: [u8; 32], smt_root: [u8; 32],
    max_time: u32, profile: u32, block_roots: Vec<Option<[u8; 32]>>, block_leaves: u64, lineage: [u8; 32],
}

/// The header zAPP's state (prover/zyncapps/methods/headers/src/main.rs).
#[derive(Serialize, Deserialize, Clone)]
struct HState {
    height: i64, hash: [u8; 32], work: [u8; 32], bits: u32, time: u32, epoch_first_time: u32, recent_times: Vec<u32>,
    max_time: u32, mmr_peaks: Vec<[u8; 32]>,
}

#[derive(Deserialize)]
struct Input {
    core_id: [u32; 8],
    core_journal: Vec<u8>,
    headers_id: [u32; 8],
    header_journal: Vec<u8>,
    core_tip_mmr_path: Vec<[u8; 32]>,
    height: u32,
    header: Vec<u8>,
    state: [u8; 32],
    undo: Option<[u8; 32]>,
    forest_path: Vec<[u8; 32]>,
    tx: Vec<u8>,
    index: u32,
    branch: Vec<[u8; 32]>,
    coinbase: Vec<u8>,
    coinbase_branch: Vec<[u8; 32]>,
}

#[derive(Serialize)]
struct Statement {
    kind: u32,
    core_id: [u32; 8],
    headers_id: [u32; 8],
    core_height: u32,
    core_tip: [u8; 32],
    core_max_time: u32,
    tip_height: i64,
    tip_hash: [u8; 32],
    tip_work: [u8; 32],
    tip_max_time: u32,
    block_height: u32,
    block_hash: [u8; 32],
    txid: [u8; 32],
    index: u32,
    outputs: Vec<(u64, Vec<u8>)>,
    confirmations: u64,
    validated_confirmations: u64,
}

/// The mainnet genesis anchor_id the verifier pins (swiftsync-verify IMAGES genesis_anchor_hex).
const GENESIS_ANCHOR: [u8; 32] = [
    0x85, 0x2c, 0x5c, 0x7e, 0x00, 0x93, 0x17, 0xc2, 0xdd, 0x23, 0x2d, 0xcf, 0x9b, 0x1a, 0x9e, 0x2b,
    0xab, 0xd5, 0x8d, 0xf3, 0x24, 0xad, 0xd4, 0x61, 0xa6, 0xff, 0x61, 0x82, 0x4f, 0x39, 0x01, 0x67,
];

fn words(b: &[u8]) -> Vec<u32> {
    assert!(b.len() % 4 == 0, "a journal is whole words");
    b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()
}

fn main() {
    let i: Input = env::read();
    env::verify(i.core_id, &i.core_journal).expect("the core receipt");
    env::verify(i.headers_id, &i.header_journal).expect("the header-chain receipt");
    let j: ChainState = risc0_zkvm::serde::from_slice(&words(&i.core_journal)).expect("a ChainState");
    let st = facts::CoreStatement { kind: j.kind, version: j.version, profile: j.profile, self_id: &j.self_id, anchor_id: &j.anchor_id, live_set: j.live_set as u64 };
    facts::check_core_statement(&st, &i.core_id, 2, facts::Network::Mainnet, false, &GENESIS_ANCHOR).unwrap_or_else(|e| panic!("core: {e}"));
    let (din, h): ([u8; 32], HState) = risc0_zkvm::serde::from_slice(&words(&i.header_journal)).expect("a header state");
    let empty = HState { height: -1, hash: [0; 32], work: [0; 32], bits: 0, time: 0, epoch_first_time: 0, recent_times: Vec::new(), max_time: 0, mmr_peaks: Vec::new() };
    let ew = risc0_zkvm::serde::to_vec(&empty).unwrap();
    let ed: [u8; 32] = Sha256::digest(ew.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>()).into();
    assert!(din == ed, "the header run starts from genesis");
    assert!(h.height >= j.height as i64, "the header chain reaches the core tip");
    let leaves = h.height as u64 + 1;
    assert!(mmr::verify(&h.mmr_peaks, leaves, j.height as u64, &j.tip_hash, &i.core_tip_mmr_path), "binding: the core tip is the header chain's block there");
    let header: [u8; 80] = i.header.as_slice().try_into().expect("an 80-byte header");
    let hash = facts::header_hash(&header);
    assert!(undo::block_in_chain_any(&j.block_roots, j.block_leaves, &hash, i.height, &i.state, i.undo.as_ref(), &i.forest_path),
            "the block is in the core block forest");
    assert!(i.tx.len() != 64, "a 64-byte transaction");
    let mut o = 0;
    let view = facts::parse_tx(&i.tx, &mut o).filter(|_| o == i.tx.len()).expect("one transaction");
    let txid = facts::tx_in_block(&header, &view.stripped, i.index, &i.branch, &i.coinbase, &i.coinbase_branch).expect("the transaction is in the block");
    env::commit(&Statement {
        kind: 0x5A44_0001, core_id: i.core_id, headers_id: i.headers_id, core_height: j.height, core_tip: j.tip_hash,
        core_max_time: j.max_time, tip_height: h.height, tip_hash: h.hash, tip_work: h.work, tip_max_time: h.max_time,
        block_height: i.height, block_hash: hash, txid, index: i.index, outputs: view.outputs,
        confirmations: (h.height - i.height as i64 + 1) as u64, validated_confirmations: (j.height - i.height + 1) as u64,
    });
}
