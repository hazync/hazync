// EXPERIMENTAL Trusted State join receipt (docs/libzync/TRUSTED_STATE.md §13.3): the core guest's committed ChainState,
// field for field and in order (prover/methods/guest/src/main.rs `ChainState`, swiftsync-verify `ChainState`), so the
// join guest decodes the core journals and recomputes the core's link digest. #[path]-included by the join guest and
// by ts-anchor, whose `join-inputs` checks, on every run, that a real core journal decodes and re-encodes to the
// same words through this copy (a drifted field order would fail there).
#[allow(unused_imports)]
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

pub const KIND_CHAIN: u32 = 0xC4A1_0012;
pub const CHAIN_STATE_VERSION: u32 = 2;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct JState {
    pub kind: u32,
    pub version: u32,
    pub tip_hash: [u8; 32],
    pub utxo_roots: Vec<Option<[u8; 32]>>,
    pub utxo_leaves: u64,
    pub utxo_value: u64,
    pub cum_work: [u8; 32],
    pub height: u32,
    pub prev_nbits: u32,
    pub prev_time: u32,
    pub epoch_start: u32,
    pub recent_times: Vec<u32>,
    pub anchor_id: [u8; 32],
    pub self_id: [u32; 8],
    pub live_set: u32,
    pub live_hash: [u32; 16],
    pub trie_root: [u8; 32],
    pub smt_root: [u8; 32],
    pub max_time: u32,
    pub profile: u32,
    pub block_roots: Vec<Option<[u8; 32]>>,
    pub block_leaves: u64,
    pub lineage: [u8; 32],
}

/// The ingest (combine) journal's anchor fields, in its commit order (combine `main`), and its tail.
pub struct Ingest {
    pub h: u32, pub tip: [u8; 32], pub hash3: [u8; 32], pub magic: u32, pub coins: u64, pub value: u64, pub live_set: u32,
    pub live_hash: [u32; 16], pub trie_root: [u8; 32], pub smt_root: [u8; 32], pub smt_n: u32,
    pub utxo_roots: Vec<Option<[u8; 32]>>, pub block_roots: Vec<Option<[u8; 32]>>,
    /// after the anchor fields: [chunk image (8), N] and, for a split BIP30 tree, [bip30 image (8), P]
    pub tail: Vec<u32>,
}

pub fn decode_ingest(j: &[u32]) -> Option<Ingest> {
    let mut i = 0usize;
    let mut take = |n: usize| -> Option<Vec<u32>> { let s = j.get(i..i + n)?.to_vec(); i += n; Some(s) };
    let b32 = |w: Vec<u32>| -> [u8; 32] { let mut o = [0u8; 32]; for k in 0..8 { o[4 * k..4 * k + 4].copy_from_slice(&w[k].to_le_bytes()); } o };
    let h = take(1)?[0];
    let tip = b32(take(8)?);
    let hash3 = b32(take(8)?);
    let magic = take(1)?[0];
    let nv = take(5)?;
    let live_hash: [u32; 16] = take(16)?.try_into().ok()?;
    let trie_root = b32(take(8)?);
    let smt_root = b32(take(8)?);
    let smt_n = take(1)?[0];
    let mut rs: [Vec<Option<[u8; 32]>>; 2] = [Vec::new(), Vec::new()];
    for r in rs.iter_mut() {
        let n = take(1)?[0] as usize;
        if n > 64 { return None; }
        for _ in 0..n { let f = take(1)?[0]; r.push(match f { 1 => Some(b32(take(8)?)), 0 => None, _ => return None }); }
    }
    let [utxo_roots, block_roots] = rs;
    let tail = j[i..].to_vec();
    Some(Ingest { h, tip, hash3, magic, coins: nv[0] as u64 | (nv[1] as u64) << 32, value: nv[2] as u64 | (nv[3] as u64) << 32,
                  live_set: nv[4], live_hash, trie_root, smt_root, smt_n, utxo_roots, block_roots, tail })
}

/// The anchor A the ingest proves, as §11.3 step 1: every coin-derived field from the ingest journal, every history-free
/// header field from main@H; self_id, anchor_id, lineage zero.
pub fn rebuild_anchor(ing: &Ingest, main: &JState) -> JState {
    JState {
        kind: KIND_CHAIN, version: CHAIN_STATE_VERSION, tip_hash: ing.tip, utxo_roots: ing.utxo_roots.clone(), utxo_leaves: ing.coins,
        utxo_value: ing.value, cum_work: main.cum_work, height: ing.h, prev_nbits: main.prev_nbits, prev_time: main.prev_time,
        epoch_start: main.epoch_start, recent_times: main.recent_times.clone(), anchor_id: [0; 32], self_id: [0; 8],
        live_set: ing.live_set, live_hash: ing.live_hash, trie_root: ing.trie_root, smt_root: ing.smt_root, max_time: main.max_time,
        profile: main.profile, block_roots: ing.block_roots.clone(), block_leaves: ing.h as u64 + 1, lineage: [0; 32],
    }
}
