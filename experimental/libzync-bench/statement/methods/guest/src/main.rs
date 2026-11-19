//! Design-B PROTOTYPE (docs/libzync/STATEMENT_ZAPP.md §3): a statement of balance proved inside the zkVM, committing
//! only the total and the statement, not which scripts. Measurement only: no image id is published for it.
//!
//! Input: core_id, the core journal (a ChainState), a bench flag, purpose, nonce, a 32-byte salt, and per script
//! (scriptPubKey, counting-trie balance proof, BIP-322 simple witness). Checks: env::verify(core_id, journal) (skipped
//! under the bench flag, for a trie built by the host: a measurement of the same work, never a statement), the
//! ChainState checks of ZYNCAPPS.md §4, scripts strictly increasing, each balance proof against trie_root, the total,
//! the message, each BIP-322 signature (bip322_lite.rs). Commits: the bench flag FIRST (true = env::verify was skipped:
//! the journal proves nothing about any chain, and no verifier may ever accept it; review 15.11.2026: before this the
//! flag was not committed, so image 74698c86 could "prove" any total), core_id, lineage, anchor_id, height, tip_hash,
//! cum_work, max_time, profile, total, the number of scripts, sha256(salt || script list hash), purpose, nonce.
#![no_main]
extern crate alloc;
use alloc::{format, string::String, vec::Vec};
use risc0_zkvm::guest::env;
use serde::{Deserialize, Serialize};
risc0_zkvm::guest::entry!(main);

#[path = "../../../../../../libzync/src/coinlog.rs"]
#[allow(dead_code)]
mod coinlog;
#[path = "../../../../../../libzync/src/ttree.rs"]
#[allow(dead_code)]
mod ttree;
#[path = "../../../../../../libzync/src/trie_model.rs"]
#[allow(dead_code)]
mod trie_model;
#[path = "../../../../../../libzync/src/trie_fact.rs"]
#[allow(dead_code)]
mod trie_fact;
#[path = "../../../bip322_lite.rs"]
mod bip322_lite;

#[derive(Deserialize)]
#[allow(dead_code)]
struct ChainState {
    kind: u32, version: u32, tip_hash: [u8; 32], utxo_roots: Vec<Option<[u8; 32]>>, utxo_leaves: u64, utxo_value: u64,
    cum_work: [u8; 32], height: u32, prev_nbits: u32, prev_time: u32, epoch_start: u32, recent_times: Vec<u32>,
    anchor_id: [u8; 32], self_id: [u32; 8], live_set: u32, live_hash: [u32; 16], trie_root: [u8; 32], smt_root: [u8; 32],
    max_time: u32, profile: u32, block_roots: Vec<Option<[u8; 32]>>, block_leaves: u64, lineage: [u8; 32],
}

#[derive(Deserialize)]
struct Input {
    core_id: [u32; 8],
    journal: Vec<u8>,
    bench: bool,
    purpose: String,
    nonce: String,
    salt: [u8; 32],
    entries: Vec<(Vec<u8>, Vec<u32>, Vec<u8>)>,
}

#[derive(Serialize)]
struct Out {
    kind: u32, bench: bool, core_id: [u32; 8], lineage: [u8; 32], anchor_id: [u8; 32], height: u32, tip_hash: [u8; 32],
    cum_work: [u8; 32], max_time: u32, profile: u32, total: u64, scripts: u32, commit: [u8; 32], purpose: String, nonce: String,
}

const KIND_CHAIN: u32 = 0xC4A1_0012;
const PROFILE_ASSUMEVALID: u32 = 2;
const PROFILE_TRIE: u32 = 32;

fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }

fn main() {
    let c0 = env::cycle_count();
    let inp: Input = env::read();
    let c1 = env::cycle_count();
    if !inp.bench {
        env::verify(inp.core_id, &inp.journal).expect("core receipt");
    }
    assert!(inp.journal.len() % 4 == 0);
    let w: Vec<u32> = inp.journal.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    let cs: ChainState = risc0_zkvm::serde::from_slice(&w).expect("ChainState");
    assert!(cs.kind == KIND_CHAIN && cs.version >= 2 && cs.self_id == inp.core_id, "core statement");
    assert!(cs.profile & PROFILE_TRIE != 0 && cs.profile & PROFILE_ASSUMEVALID == 0, "profile");
    assert!(!inp.purpose.is_empty() && !inp.nonce.is_empty() && !inp.purpose.chars().chain(inp.nonce.chars()).any(|c| c.is_control()));
    let c2 = env::cycle_count();

    // the balance facts
    let root = coinlog::w_of(&cs.trie_root);
    let mut total = 0u64;
    let mut lh = sha2::Sha256::new();
    use sha2::Digest;
    for (i, (spk, proof, _)) in inp.entries.iter().enumerate() {
        if i > 0 { assert!(inp.entries[i - 1].0 < *spk, "scripts distinct and increasing"); }
        let b = trie_fact::balance(proof, &root, &trie_fact::script_hash(spk)).expect("balance proof");
        total = total.checked_add(b.map_or(0, |b| b.value)).expect("overflow");
        let n = spk.len();
        assert!(n < 0xfd);
        lh.update([n as u8]);
        lh.update(spk);
    }
    let list: [u8; 32] = lh.finalize().into();
    let commit = bip322_lite::sha256(&[&inp.salt[..], &list[..]].concat());
    let c3 = env::cycle_count();

    // the message every script signs
    let id_bytes: Vec<u8> = inp.core_id.iter().flat_map(|x| x.to_le_bytes()).collect();
    let mut tip = cs.tip_hash;
    tip.reverse();
    let net = if cs.profile & 4 != 0 { "signet" } else { "mainnet" };
    let msg = format!("Hazync private statement of balance v1\nnetwork: {net}\ncore image: {}\nblock: {} {}\nscripts: {} commit {}\ntotal: {total} sat\npurpose: {}\nnonce: {}",
                      hex(&id_bytes), cs.height, hex(&tip), inp.entries.len(), hex(&commit), inp.purpose, inp.nonce);
    let c4 = env::cycle_count();
    for (spk, _, wit) in &inp.entries {
        let stack = bip322_lite::decode_witness(wit).expect("witness");
        bip322_lite::verify_simple(spk, msg.as_bytes(), &stack).expect("BIP-322 signature");
    }
    let c5 = env::cycle_count();
    env::commit(&Out {
        kind: 0x5A43_0003, bench: inp.bench, core_id: inp.core_id, lineage: cs.lineage, anchor_id: cs.anchor_id, height: cs.height, tip_hash: cs.tip_hash,
        cum_work: cs.cum_work, max_time: cs.max_time, profile: cs.profile, total, scripts: inp.entries.len() as u32, commit,
        purpose: inp.purpose, nonce: inp.nonce,
    });
    let c6 = env::cycle_count();
    env::log(&format!("PHASES read {} verify+decode {} facts {} message {} signatures {} commit {}", c1 - c0, c2 - c1, c3 - c2, c4 - c3, c5 - c4, c6 - c5));
}
