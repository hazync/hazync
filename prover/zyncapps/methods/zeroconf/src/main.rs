//! zyncAPP "zeroconf" (ROADMAP D5, txproofs/README.md §3): transaction T is valid for the NEXT block on top of a proven
//! tip, and every coin it spends is unspent at that tip. A merchant learns it before T confirms.
//!
//! Input:  core_id, the core tip journal (a ChainState), the chunk journal (a speculative chunk proved by the same core
//!         image for height tip+1 and the tip's MTP, txproofs §7 Phase A), signet flag, the record index of T in the
//!         chunk, and one coin-log path per input of T.
//! Checks: env::verify both journals under core_id (assumptions); check_core_statement; the chunk is speculative (zero
//!         hash and time), for height tip+1 and MTP(tip), all_valid; T's record is whole (its first and only run,
//!         covering every input); every input's coin leaf is unspent at the tip.
//! Commits: core_id, the tip it rests on, T's txid and wtxid, its fee, weight, sigops.
//! ⚠ Not "T WILL confirm": a conflicting transaction can still be mined first. It says T is valid now, against the
//!   proven tip, as Core's mempool acceptance would (scripts and the coins), with its inputs unspent at that tip.
//! ⚠ Inputs from unconfirmed parents are not in the log: such a T is refused.
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

#[derive(Deserialize)]
#[allow(dead_code)] // the journal's layout: every field must be read to decode it, not every one is used here
struct ChainState {
    kind: u32, version: u32, tip_hash: [u8; 32], utxo_roots: Vec<Option<[u8; 32]>>, utxo_leaves: u64, utxo_value: u64,
    cum_work: [u8; 32], height: u32, prev_nbits: u32, prev_time: u32, epoch_start: u32, recent_times: Vec<u32>,
    anchor_id: [u8; 32], self_id: [u32; 8], live_set: u32, live_hash: [u32; 16], trie_root: [u8; 32], smt_root: [u8; 32],
    max_time: u32, profile: u32, block_roots: Vec<Option<[u8; 32]>>, block_leaves: u64, lineage: [u8; 32],
}

#[derive(Serialize, Deserialize)]
pub struct ZeroConf {
    pub kind: u32,
    pub core_id: [u32; 8],
    pub lineage: [u8; 32],
    pub anchor_id: [u8; 32],
    pub tip_height: u32,
    pub tip_hash: [u8; 32],
    pub cum_work: [u8; 32],
    pub max_time: u32,
    pub profile: u32,
    pub txid: [u8; 32],
    pub wtxid: [u8; 32],
    pub fee: i64,
    pub weight: i64,
    pub sigops: i64,
}

pub const KIND_ZYNC_ZEROCONF: u32 = 0x5A43_0002;
const KIND_CHUNK_LITE: u32 = 0xC4A1_0010;

struct Rd<'a> { w: &'a [u32], i: usize }
impl Rd<'_> {
    fn u(&mut self) -> u32 { let x = *self.w.get(self.i).expect("chunk journal truncated"); self.i += 1; x }
    fn h(&mut self) -> [u8; 32] {
        let mut h = [0u8; 32];
        for c in h.chunks_exact_mut(4) { c.copy_from_slice(&self.u().to_le_bytes()); }
        h
    }
    fn i64(&mut self) -> i64 { let lo = self.u() as u64; let hi = self.u() as u64; (lo | (hi << 32)) as i64 }
}

fn words(b: &[u8]) -> Vec<u32> {
    assert!(b.len() % 4 == 0, "journal is not whole words");
    b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()
}

fn main() {
    let core_id: [u32; 8] = env::read();
    let tip_journal: Vec<u8> = env::read();
    let chunk_journal: Vec<u8> = env::read();
    let signet: bool = env::read();
    let rec_index: u32 = env::read();
    let paths: Vec<(u64, Vec<[u8; 32]>)> = env::read();
    env::verify(core_id, &tip_journal).expect("the tip receipt does not verify under core_id");
    env::verify(core_id, &chunk_journal).expect("the chunk receipt does not verify under core_id");
    let j: ChainState = risc0_zkvm::serde::from_slice(&words(&tip_journal)).expect("tip journal is not a ChainState");
    let st = facts::CoreStatement { kind: j.kind, version: j.version, profile: j.profile, self_id: &j.self_id, anchor_id: &j.anchor_id, live_set: j.live_set as u64 };
    facts::check_core_statement(&st, &core_id, 2, if signet { facts::Network::Signet } else { facts::Network::Mainnet }, false, &j.anchor_id).unwrap_or_else(|e| panic!("core statement: {e}"));
    let mut t = j.recent_times.clone();
    t.sort_unstable();
    let mtp = t[t.len() / 2];

    let cw = words(&chunk_journal);
    let mut r = Rd { w: &cw, i: 0 };
    assert!(r.u() == KIND_CHUNK_LITE, "not a chunk journal");
    assert!(r.u() == 1, "the chunk is not all valid");
    let (c_height, c_hash, c_mtp, c_time) = (r.u(), r.h(), r.u(), r.u());
    assert!(c_height == j.height + 1 && c_mtp == mtp, "the chunk is not for the block after this tip");
    assert!(c_hash == [0u8; 32] && c_time == 0, "the chunk is not speculative");
    let n_rec = r.u();
    assert!(rec_index < n_rec, "no such record");
    let coinlog_build = j.profile & facts::PROFILE_COINLOG != 0;
    let mut out = None;
    for k in 0..n_rec {
        let (txid, wtxid, _ph, start, count, first) = (r.h(), r.h(), r.h(), r.u(), r.u(), r.u());
        let mut head = None;
        if first == 1 {
            let (_hw, n_vin) = (r.u(), r.u());
            let (fee, weight, sigops) = (r.i64(), r.i64(), r.i64());
            if coinlog_build { let _ = r.i64(); }
            let n_out = r.u();
            r.i += 11 * n_out as usize;
            head = Some((n_vin, fee, weight, sigops));
        }
        let mut leaves = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let leaf = r.h();
            let f = r.u();
            if f & 1 == 1 { let _ = r.h(); }
            leaves.push((leaf, f));
        }
        if k == rec_index {
            let (n_vin, fee, weight, sigops) = head.expect("the record is not the transaction's first run");
            assert!(start == 0 && count == n_vin, "the record does not cover every input of the transaction");
            out = Some((txid, wtxid, fee, weight, sigops, leaves));
        }
    }
    assert!(r.i == cw.len(), "chunk journal has trailing words");
    let (txid, wtxid, fee, weight, sigops, leaves) = out.unwrap();
    assert!(paths.len() == leaves.len(), "one coin-log path per input");
    for ((leaf, f), (pos, path)) in leaves.iter().zip(&paths) {
        // bit 1: the coin claims the chunk's own height, i.e. an unconfirmed parent: not in the log.
        assert!(f & 2 == 0, "an input spends an unconfirmed coin");
        assert!(facts::coin_unspent(&j.utxo_roots, j.utxo_leaves, *pos, leaf, path), "an input's coin is not unspent at the tip");
    }
    env::commit(&ZeroConf {
        kind: KIND_ZYNC_ZEROCONF, core_id, lineage: j.lineage, anchor_id: j.anchor_id, tip_height: j.height,
        tip_hash: j.tip_hash, cum_work: j.cum_work, max_time: j.max_time, profile: j.profile, txid, wtxid, fee, weight, sigops,
    });
}
