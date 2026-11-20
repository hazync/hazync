//! zyncAPP "utreexo-tip": one STEP of the utreexo zyncAPP's tip mode (docs/libzync/UTREEXO_ZAPP.md §10). From the
//! roots after block P that the previous receipt proves (a BASE receipt of the utreexo image, a walk from genesis, or
//! an earlier step of this image), it applies blocks P+1..H to Floresta's accumulator exactly as a Floresta node does
//! (rustreexo 0.7.0 `Stump::modify`, `step.rs`) and proves the roots after H. Its own image; it never changes the core
//! image nor the base utreexo image.
//!
//! Input:  self_id, base_id, core_id ([u32; 8] each), core journal bytes, signet flag, profiling step (0 = off), H,
//!         prev kind (1 = a base receipt under base_id, 2 = a step under self_id), the previous receipt's journal bytes;
//!         per block P+1..H one frame of words: [L, S, Q] ‖ the block (header ‖ count ‖ transactions without witness,
//!         L bytes) ‖ the spent coins (S bytes, step.rs `SpentCoin`) ‖ the batch proof of its deletions (Q bytes,
//!         step.rs `encode_proof`), each zero-padded to words; then [T - H] ‖ T - H headers (20 words each), the link
//!         from block H to the core tip T.
//! Checks: the previous receipt (env::verify) and its statement; every block follows the previous one, starting at the
//!         previous statement's block; the link reaches the core tip hash; env::verify(core_id, journal);
//!         check_core_statement with the network's pinned genesis anchor. The core receipt proves the chain 1..T valid
//!         from genesis; the hash links put blocks P+1..H on it (and so the previous statement's block P).
//! Commits: TipStatement.
#![no_main]
extern crate alloc;
use alloc::vec::Vec;
use risc0_zkvm::guest::env;
use serde::{Deserialize, Serialize};

risc0_zkvm::guest::entry!(main);

#[path = "../../../../../libzync/src/coinlog.rs"]
#[allow(dead_code)]
mod coinlog;
#[path = "../../../../../libzync/src/facts.rs"]
#[allow(dead_code)] // a shared libzync file (#[path], hazync#88): each includer uses its own subset
mod facts;
#[path = "../../../../../libzync/src/utreexo_roots.rs"]
#[allow(dead_code)]
mod utreexo_roots;
mod step;

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

/// The base statement (prover/zyncapps/methods/utreexo, kind 'ZU' 1), field for field.
#[derive(Deserialize)]
#[allow(dead_code)]
struct BaseStatement {
    kind: u32,
    self_id: [u32; 8],
    network: u32,
    height: u32,
    to: u32,
    runs: u32,
    state_out: [u8; 32],
    done: u32,
    block_hash: [u8; 32],
    leaves: u64,
    roots: Vec<[u8; 32]>,
    n_live: u64,
    core_id: [u32; 8],
    lineage: [u8; 32],
    anchor_id: [u8; 32],
    core_height: u32,
    tip_hash: [u8; 32],
    cum_work: [u8; 32],
    max_time: u32,
    profile: u32,
}

/// What a step commits ('ZU' 2). The Utreexo fields (block_hash, height, roots, leaves) are Floresta's
/// `AssumeUtreexoValue` after block `height`; `base_*` name the walk from genesis the chain of steps starts from; the
/// core fields are THIS step's core receipt (the chain 1..core_height, which contains block `height`).
#[derive(Serialize, Deserialize, Default)]
pub struct TipStatement {
    pub kind: u32,
    pub self_id: [u32; 8],
    pub base_id: [u32; 8],
    pub network: u32,
    pub base_height: u32,
    pub base_core_id: [u32; 8],
    pub steps: u32,
    pub from: u32,
    pub height: u32,
    pub block_hash: [u8; 32],
    pub leaves: u64,
    pub roots: Vec<[u8; 32]>,
    pub core_id: [u32; 8],
    pub lineage: [u8; 32],
    pub anchor_id: [u8; 32],
    pub core_height: u32,
    pub tip_hash: [u8; 32],
    pub cum_work: [u8; 32],
    pub max_time: u32,
    pub profile: u32,
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

fn words(b: &[u8]) -> Vec<u32> {
    assert!(b.len() % 4 == 0, "a journal is whole words");
    b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()
}

fn main() {
    let self_id: [u32; 8] = env::read();
    let base_id: [u32; 8] = env::read();
    let core_id: [u32; 8] = env::read();
    let journal: Vec<u8> = env::read();
    let signet: bool = env::read();
    let prof: u32 = env::read();
    let stop: u32 = env::read();
    let prev_kind: u32 = env::read();
    let pj: Vec<u8> = env::read();
    let network = if signet { facts::Network::Signet } else { facts::Network::Mainnet };

    // the roots after block P, from the previous receipt
    let mut z = TipStatement { kind: step::KIND_UTREEXO_TIP, self_id, base_id, network: network.bits(), ..Default::default() };
    let (leaves, roots) = match prev_kind {
        1 => {
            env::verify(base_id, &pj).expect("the base receipt does not verify under base_id");
            let p: BaseStatement = risc0_zkvm::serde::from_slice(&words(&pj)).expect("the base receipt's journal");
            assert!(p.kind == utreexo_roots::KIND_UTREEXO && p.self_id == base_id && p.done == 1 && p.to == p.height,
                    "the base is not a final run of the base image");
            assert!(p.network == network.bits(), "the base is for another network");
            z.base_height = p.height;
            z.base_core_id = p.core_id;
            z.steps = 1;
            z.from = p.height;
            z.block_hash = p.block_hash;
            (p.leaves, p.roots)
        }
        2 => {
            env::verify(self_id, &pj).expect("the previous step does not verify under self_id");
            let p: TipStatement = risc0_zkvm::serde::from_slice(&words(&pj)).expect("the previous step's journal");
            assert!(p.kind == step::KIND_UTREEXO_TIP && p.self_id == self_id && p.base_id == base_id, "the previous step is not a step of this image from this base image");
            assert!(p.network == network.bits(), "the previous step is for another network");
            z.base_height = p.base_height;
            z.base_core_id = p.base_core_id;
            z.steps = p.steps + 1;
            z.from = p.height;
            z.block_hash = p.block_hash;
            (p.leaves, p.roots)
        }
        _ => panic!("prev kind: 1 (base) or 2 (step)"),
    };
    assert!(stop > z.from, "a step covers at least one block");
    let mut acc = step::stump(leaves, &roots).unwrap_or_else(|e| panic!("{e}"));
    let tag = utreexo_roots::tag_state();
    let mut c = step::Counts::default();
    let mut last = z.block_hash;
    let (mut cyc_leaf, mut cyc_modify) = (0u64, 0u64);
    for h in z.from + 1..=stop {
        let mut fr = [0u32; 3];
        env::read_slice(&mut fr);
        let (l, s, q) = (fr[0] as usize, fr[1] as usize, fr[2] as usize);
        let (lw, sw, qw) = (l.div_ceil(4), s.div_ceil(4), q.div_ceil(4));
        let mut body = alloc::vec![0u32; lw + sw + qw];
        env::read_slice(&mut body);
        let block = words_to_bytes(&body[..lw], l);
        let spent = words_to_bytes(&body[lw..lw + sw], s);
        let proof = step::decode_proof(&words_to_bytes(&body[lw + sw..], q)).unwrap_or_else(|e| panic!("block {h}: {e}"));
        let t0 = if prof > 0 { env::cycle_count() } else { 0 };
        let ch = step::block_changes(&tag, &block, &last, h, &spent, &mut c).unwrap_or_else(|e| panic!("block {h}: {e}"));
        let t1 = if prof > 0 { env::cycle_count() } else { 0 };
        acc = step::apply(&acc, &ch, &proof, &mut c).unwrap_or_else(|e| panic!("block {h}: {e}"));
        if prof > 0 {
            let t2 = env::cycle_count();
            cyc_leaf += t1 - t0;
            cyc_modify += t2 - t1;
            if h % prof == 0 || h == stop {
                env::log(&alloc::format!(
                    "UTX_TIP_CYC h={h} cycles={} parse_and_leaves={cyc_leaf} modify={cyc_modify} blocks={} txs={} adds={} dels={} proof_hashes={} leaf_blocks={} bytes={}",
                    env::cycle_count(), c.blocks, c.txs, c.adds, c.dels, c.proof_hashes, c.leaf_blocks, c.bytes
                ));
            }
        }
        last = ch.hash;
    }
    z.height = stop;
    z.block_hash = last;
    z.leaves = acc.leaves;
    z.roots = step::roots_of(&acc);

    // the core receipt, and the link from block H to its tip
    env::verify(core_id, &journal).expect("the core receipt does not verify under core_id");
    let j: ChainState = risc0_zkvm::serde::from_slice(&words(&journal)).expect("core journal is not a ChainState");
    let st = facts::CoreStatement { kind: j.kind, version: j.version, profile: j.profile, self_id: &j.self_id, anchor_id: &j.anchor_id, live_set: j.live_set as u64 };
    let anchor = if signet { &SIGNET_ANCHOR } else { &MAINNET_ANCHOR };
    facts::check_core_statement(&st, &core_id, 2, network, false, anchor).unwrap_or_else(|e| panic!("core statement: {e}"));
    assert!(stop <= j.height, "H must be at most the core height");
    let mut n = [0u32; 1];
    env::read_slice(&mut n);
    assert!(n[0] == j.height - stop, "link headers above H: want T - H");
    let mut left = n[0] as usize;
    while left > 0 {
        let k = left.min(1024);
        let mut buf = alloc::vec![0u32; 20 * k];
        env::read_slice(&mut buf);
        for x in buf.chunks_exact(20) {
            last = step::link_header(&words_to_bytes(x, 80), &last).unwrap_or_else(|e| panic!("{e}"));
        }
        left -= k;
    }
    assert!(last == j.tip_hash, "the link from block H does not end at the core tip");
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
