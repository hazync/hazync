//! libzync `agg-lite`: the aggregate never parses a transaction. EXPERIMENT.
//!
//! The chunks already deserialise every non-coinbase transaction. Under this feature a chunk also
//! commits, for each transaction whose input 0 it owns, the facts the aggregate used to re-derive in
//! the SERIAL phase: txid, wtxid, fee, weight, sigop cost, input count and created-output leaves.
//! For each run of inputs it owns, it commits the spent-coin leaf and, for a coinbase spend, the
//! coinbase txid. Per-transaction rules that need no other transaction (CheckTransaction and the
//! amount rules, finality, BIP68 / maturity, script results) are decided in the chunk and folded into
//! its `all_valid`.
//!
//! Why the aggregate may trust these facts:
//! - TX BYTES: every txid goes into the merkle root, checked against the header, and every wtxid into
//!   the BIP141 commitment. A fact committed next to a txid / wtxid is a fact about exactly those bytes
//!   (same guest image, so the same code computed both).
//! - PREVOUTS: a transaction's inputs can be split across chunks, each holding its own copy of the
//!   prevouts blob. Every record carries SHA-256(prevouts) and the aggregate requires all runs of a
//!   transaction to agree, so the fee and sigops computed by the input-0 chunk used the same coins the
//!   other chunks verified. The coins themselves are bound by the SwiftSync total, as before.
//! - COVERAGE: runs must tile every transaction's inputs exactly once, in block order, against the
//!   input count the input-0 chunk committed.
//! - CONTEXT: each chunk commits the height, block hash, median-time-past and block time it used; the
//!   aggregate requires them equal to its own.
//!
//! ⚠ SwiftSync only: there is no per-input utreexo proof here. At the two BIP30-grandfathered blocks
//! the superseded coinbase coins are SUBTRACTED from the SwiftSync total (pinned old height + MTP). Journal layout, all u32 LE words:
//!   header: KIND_CHUNK_LITE, all_valid, height, block_hash[8], mtp, block_time, n_records
//!   record: txid[8] prevouts_hash[8] start count first
//!     if first: wtxid[8] has_witness n_vin fee[2] weight[2] sigops[2] n_out out_leaf[8]*n_out
//!     per owned input: leaf[8] is_coinbase [coinbase_txid[8] if is_coinbase]

use super::*;

pub const KIND_CHUNK_LITE: u32 = 0xC4A1_0010;

extern "C" {
    /// Empty verify_input.cpp's per-transaction caches and bump hz_tx_generation (round 11).
    fn hz_cache_reset();
}
/// Called before every transaction a chunk processes. Required once the allocator can reuse
/// addresses (HAZYNC_TLSF); harmless, and cheap, under the bump allocator.
pub fn begin_tx() { unsafe { hz_cache_reset() } }

/// A 32-byte hash as four u64 words: ordered and compared word by word instead of a byte-wise
/// memcmp. Only used for equality and for a consistent sort order, never as a value.
fn key(h: &[u8; 32]) -> [u64; 4] {
    let w = |i: usize| u64::from_le_bytes(h[i..i + 8].try_into().unwrap());
    [w(0), w(8), w(16), w(24)]
}

fn push_hash(v: &mut Vec<u32>, h: &[u8]) {
    for c in h.chunks_exact(4) { v.push(u32::from_le_bytes([c[0], c[1], c[2], c[3]])); }
}
fn push_i64(v: &mut Vec<u32>, x: i64) {
    v.push(x as u64 as u32);
    v.push(((x as u64) >> 32) as u32);
}

/// Chunk side: everything one transaction run contributes to the journal.
pub struct ChunkCtx {
    pub height: u32,
    pub flags: u32,
    pub mtp: u32,
    pub block_time: u32,
    pub words: Vec<u32>,
    pub n_records: u32,
    /// Output-leaf scratch, reused: a fresh zeroed `vec!` per transaction was 4x the transaction's
    /// size in memset AND (bump allocator) in never-returned pages.
    out: Vec<u8>,
}

impl ChunkCtx {
    pub fn new(height: u32, flags: u32, mtp: u32, block_time: u32) -> Self {
        Self { height, flags, mtp, block_time, words: Vec::new(), n_records: 0, out: Vec::new() }
    }

    /// Record one transaction run. Returns false if a per-transaction rule failed.
    #[allow(clippy::too_many_arguments)]
    pub fn record(&mut self, raw_tx: &[u8], prevouts: &[u8], idx: &[u32], ch: &[u32], cb: &[u32],
                  cmtp: &[u32], results: &[i32], leaves: &[u8], seqs: &[u32], version: i32,
                  wtxid: &[u8; 32], has_witness: u32) -> bool {
        use sha2::{Digest, Sha256};
        let mut ok = true;
        let k = idx.len();
        let (tp, tl) = (raw_tx.as_ptr(), raw_tx.len() as u32);
        // txid (and the output leaves, used only when this run owns input 0).
        let need = (raw_tx.len() / 8 + 1) * 32;
        if self.out.len() < need { self.out.resize(need, 0); }
        let mut txid = [0u8; 32];
        let n_out = unsafe { tx_out_leaves(tp, tl, self.height, 0, self.mtp, self.out.as_mut_ptr(), txid.as_mut_ptr()) };
        let ph: [u8; 32] = Sha256::digest(prevouts).into();
        let start = idx.first().copied().unwrap_or(0);
        for (j, i) in idx.iter().enumerate() {
            if *i != start + j as u32 { ok = false; }
        }
        let first = start == 0;
        push_hash(&mut self.words, &txid);
        push_hash(&mut self.words, &ph);
        self.words.push(start);
        self.words.push(k as u32);
        self.words.push(first as u32);
        if first {
            let mut fee: i64 = 0;
            if unsafe { check_tx(tp, tl, prevouts.as_ptr(), prevouts.len() as u32, &mut fee) } != 1 { ok = false; }
            let lock_time = if self.height >= 419_328 { self.mtp } else { self.block_time };
            if unsafe { is_final_tx(tp, tl, self.height as i64, lock_time as i64) } != 1 { ok = false; }
            let (mut weight, mut _legacy) = (0i64, 0i64);
            unsafe { tx_wu_sigops(tp, tl, &mut weight, &mut _legacy) };
            let sigops = unsafe { tx_full_sigops(tp, tl, prevouts.as_ptr(), prevouts.len() as u32, self.flags) };
            let n_vin = unsafe { tx_vin_count(tp, tl) };
            push_hash(&mut self.words, wtxid);
            self.words.push(has_witness);
            self.words.push(n_vin);
            push_i64(&mut self.words, fee);
            push_i64(&mut self.words, weight);
            push_i64(&mut self.words, sigops);
            self.words.push(n_out);
            for l in 0..n_out as usize { push_hash(&mut self.words, &self.out[32 * l..32 * l + 32]); }
        }
        // BIP68 / coinbase maturity, transcribed from `check_input_locks` (as the aggregate's own
        // from-chunk path already does), with this chunk's committed median-time-past.
        const DISABLE: u32 = 1 << 31; const TYPE: u32 = 1 << 22;
        const MASK: u32 = 0x0000_ffff; const GRAN: u32 = 9;
        for j in 0..k {
            if results[j] != 1 { ok = false; }
            if cb[j] == 1 && self.height < ch[j] + 100 { ok = false; }
            let seq = seqs[j];
            if self.height >= 419_328 && version >= 2 && (seq & DISABLE) == 0 {
                if seq & TYPE != 0 {
                    if (self.mtp as u64) < cmtp[j] as u64 + (((seq & MASK) as u64) << GRAN) { ok = false; }
                } else if self.height < ch[j] + (seq & MASK) { ok = false; }
            }
            push_hash(&mut self.words, &leaves[32 * j..32 * j + 32]);
            self.words.push(cb[j]);
            if cb[j] == 1 {
                let mut t = [0u8; 32];
                if unsafe { tx_input_prevout_txid(tp, tl, idx[j], t.as_mut_ptr()) } != 1 { ok = false; }
                push_hash(&mut self.words, &t);
            }
        }
        self.n_records += 1;
        ok
    }

    pub fn commit(self, all_valid: bool, block_hash: &[u8; 32]) {
        let mut v: Vec<u32> = Vec::with_capacity(15 + self.words.len());
        v.push(KIND_CHUNK_LITE);
        v.push(all_valid as u32);
        v.push(self.height);
        push_hash(&mut v, block_hash);
        v.push(self.mtp);
        v.push(self.block_time);
        v.push(self.n_records);
        v.extend_from_slice(&self.words);
        env::commit_slice(&v);
    }
}

/// A cursor over one chunk journal's words. Out-of-range reads return 0 and set `bad`, so a short
/// journal fails the block rather than panicking.
struct Rd<'a> { w: &'a [u32], i: usize, bad: bool }
impl Rd<'_> {
    fn u(&mut self) -> u32 {
        match self.w.get(self.i) { Some(x) => { self.i += 1; *x } None => { self.bad = true; 0 } }
    }
    fn h(&mut self) -> [u8; 32] {
        let mut h = [0u8; 32];
        for c in h.chunks_exact_mut(4) { c.copy_from_slice(&self.u().to_le_bytes()); }
        h
    }
    fn i64(&mut self) -> i64 { let lo = self.u() as u64; let hi = self.u() as u64; (lo | (hi << 32)) as i64 }
}

/// Aggregate side. Same checks, same order of SwiftSync operations, as `validate_block_staged`'s
/// aggregation path, with every per-transaction fact taken from the chunk journals.
/// `ss` = ((mode, nbits, packed bits), previous hint hash): design B's SwiftSync hint bits for this block,
/// one per INDEXED output (see below), and the running hash they extend. mode 0 = no hints file.
pub fn validate_block_lite(w: &BlockWitness, mtp: u32, journals: &[Vec<u32>],
                           ss: (&(u32, u32, Vec<u32>), [u8; 32])) -> BlockResult {
    let mut all_ok = true;
    assert!(w.header.len() == 80, "block header must be exactly 80 bytes, got {}", w.header.len());
    let block_time = u32::from_le_bytes(w.header[68..72].try_into().unwrap());
    let block_hash = dsha256(&w.header);
    let flags = block_script_flags(w.height, &block_hash);
    let lock_time = if w.height >= 419_328 { mtp } else { block_time };
    let version = i32::from_le_bytes(w.header[0..4].try_into().unwrap());
    if (version < 2 && w.height >= BIP34_HEIGHT)
        || (version < 3 && w.height >= 363_725)
        || (version < 4 && w.height >= 388_381) {
        all_ok = false;
    }
    // The two grandfathered BIP30 blocks are handled below with SwiftSync subtractions instead of the
    // full path's utreexo delete, so no overwrite witness is ever accepted here.
    let is_bip30_block = block_hash == BIP30_OVERWRITE_A || block_hash == BIP30_OVERWRITE_B;
    if w.bip30.is_some() { all_ok = false; }

    // Capacity up front: the bump allocator never frees, so every Vec regrowth copies AND leaks.
    let words: usize = journals.iter().map(|j| j.len()).sum();
    // Coinbase: the only transaction the aggregate parses.
    let mut output_leaves: Vec<[u8; 32]> = Vec::with_capacity(words / 9);
    let mut out_pos: Vec<u32> = Vec::with_capacity(words / 9);   // creating tx position, parallel
    let mut cb_txid = [0u8; 32];
    let cb_outputs = {
        let mut buf = vec![0u8; (w.coinbase_tx.len() / 8 + 1) * 32];
        let n = unsafe { tx_out_leaves(w.coinbase_tx.as_ptr(), w.coinbase_tx.len() as u32, w.height, 1, mtp,
                                       buf.as_mut_ptr(), cb_txid.as_mut_ptr()) };
        // BIP30, the hints-file convention (Core's IsBIP30Unspendable): the coinbase coins of 91,722 and
        // 91,812 are never indexed or accounted. Their outpoints come back, and are accounted, when 91,880
        // and 91,842 duplicate them. Same final set as Core's overwrite, with no subtraction needed.
        if block_hash != BIP30_UNSPENDABLE_A && block_hash != BIP30_UNSPENDABLE_B {
            for l in 0..n as usize {
                let mut h = [0u8; 32];
                h.copy_from_slice(&buf[32 * l..32 * l + 32]);
                output_leaves.push(h);
                out_pos.push(0);
            }
        }
        n
    };
    let mut txids: Vec<[u8; 32]> = Vec::with_capacity(words / 30 + 1);
    txids.push(cb_txid);
    let mut cb_wt = [0u8; 32];
    let mut has_witness = unsafe { tx_wtxid_info(w.coinbase_tx.as_ptr(), w.coinbase_tx.len() as u32, cb_wt.as_mut_ptr()) } == 1;
    let mut wtxids: Vec<[u8; 32]> = Vec::with_capacity(words / 30 + 1);
    wtxids.push([0u8; 32]);
    let mut total_fee: i64 = 0;
    let mut total_weight: i64 = 0;
    let mut total_sigops: i64 = 0;
    let mut spends: Vec<([u8; 32], u32)> = Vec::with_capacity(words / 9);   // (leaf, spending tx position), block order
    let mut cb_spends: Vec<[u8; 32]> = Vec::new();

    // Walk every chunk's records in order. `cur` = (txid, prevouts hash, n_vin, next input expected).
    let mut cur: Option<([u8; 32], [u8; 32], u32, u32)> = None;
    let mut tx_pos: u32 = 0;
    for j in journals {
        let mut r = Rd { w: j, i: 0, bad: false };
        if r.u() != KIND_CHUNK_LITE { all_ok = false; continue; }
        if r.u() != 1 { all_ok = false; }                       // the chunk's all_valid
        // Read every field BEFORE comparing: a short-circuiting `||` over the cursor skipped the
        // remaining reads on the first mismatch and misaligned the rest of the journal.
        let (c_height, c_hash, c_mtp, c_time) = (r.u(), r.h(), r.u(), r.u());
        if c_height != w.height || c_hash != block_hash || c_mtp != mtp || c_time != block_time {
            env::log("agg-lite: REJECT chunk context (height / block hash / mtp / time) differs");
            all_ok = false;
        }
        let n_rec = r.u();
        for _ in 0..n_rec {
            let (txid, ph, start, count, first) = (r.h(), r.h(), r.u(), r.u(), r.u());
            if first == 1 {
                if start != 0 { all_ok = false; }
                if let Some((_, _, n, next)) = cur { if next != n { all_ok = false; } }
                tx_pos += 1;
                let wt = r.h();
                let hw = r.u();
                let n_vin = r.u();
                let (fee, weight, sigops) = (r.i64(), r.i64(), r.i64());
                let n_out = r.u();
                txids.push(txid);
                wtxids.push(wt);
                has_witness |= hw == 1;
                total_fee = total_fee.saturating_add(fee);
                total_weight = total_weight.saturating_add(weight);
                total_sigops = total_sigops.saturating_add(sigops);
                for _ in 0..n_out {
                    let l = r.h();
                    output_leaves.push(l);
                    out_pos.push(tx_pos);
                }
                cur = Some((txid, ph, n_vin, 0));
            } else {
                match cur {
                    Some((t, p, _, next)) if t == txid && p == ph && start == next => {}
                    _ => all_ok = false,
                }
            }
            let Some(c) = cur.as_mut() else { all_ok = false; break; };
            if count == 0 || c.3 + count > c.2 { all_ok = false; }
            c.3 += count;
            for _ in 0..count {
                let leaf = r.h();
                spends.push((leaf, tx_pos));
                if r.u() == 1 { cb_spends.push(r.h()); }
            }
            if r.bad { break; }
        }
        if r.bad || r.i != j.len() { all_ok = false; }
    }
    // No records at all = a coinbase-only block (common before ~2011): coverage holds trivially. Safe
    // because the merkle check below binds the full txid list, so dropping every record of a block that
    // HAS other transactions still fails there.
    let covered = match cur { Some((_, _, n, next)) => next == n, None => true };
    if !covered { all_ok = false; }
    if !all_ok { env::log(&format!("agg-lite: REJECT after the record walk (transactions={}, last covered={covered})", tx_pos)); }

    // Spends then adds, in the same order as the full path (so the same ECMH hints apply).
    // `created` = every created leaf with its creating tx position, sorted by word key; a duplicate
    // leaf keeps its FIRST (lowest) position, as the full path's `entry().or_insert` does.
    // Design B: one hint bit per indexed output (= every entry of output_leaves, in order). 1 = still
    // UNSPENT at the hints file's stop height H.
    let (&(ss_mode, ss_nbits, ref ss_words), prev_hints) = (ss.0, ss.1);
    if ss_mode == 1 && ss_nbits as usize != output_leaves.len() {
        env::log(&format!("agg-lite: REJECT hint bits {} != indexed outputs {}", ss_nbits, output_leaves.len()));
        all_ok = false;
    }
    let bit = |k: usize| ss_mode == 1 && k < ss_nbits as usize && (ss_words[k / 32] >> (k % 32)) & 1 == 1;
    let mut created: Vec<([u64; 4], u32, u32)> =
        output_leaves.iter().zip(&out_pos).enumerate().map(|(k, (l, p))| (key(l), *p, k as u32)).collect();
    created.sort_unstable();
    created.dedup_by_key(|e| e.0);
    let mut spent = vec![false; created.len()];
    let find = |k: &[u64; 4]| created.binary_search_by(|e| e.0.cmp(k));
    for (leaf, txp) in &spends {
        match find(&key(leaf)) {
            Ok(i) => {
                // In-block spend: created by a strictly earlier tx, spent at most once.
                if created[i].1 >= *txp { all_ok = false; }
                if spent[i] { all_ok = false; }
                spent[i] = true;
                // A coin spent in its own block cannot be unspent at H: the hint lies.
                if bit(created[i].2 as usize) {
                    env::log("agg-lite: REJECT a hint marks a coin unspent-at-H that this block spends");
                    all_ok = false;
                }
            }
            Err(_) => swiftsync::sub(leaf),
        }
    }
    // Design B: survivors (bit 1) go into an append-only utreexo forest -- at H it holds exactly the UTXO
    // set, in creation order; everything else into the ECMH total, which must be the identity at H.
    let mut stump = utreexo::Stump::new(w.root_prev.roots.clone(), w.root_prev.num_leaves);
    for (k, leaf) in output_leaves.iter().enumerate() {
        if let Ok(i) = find(&key(leaf)) {
            if spent[i] { continue; }
            if bit(k) { stump.add(*leaf); } else { swiftsync::add(leaf); }
        }
    }
    swiftsync::report();
    let ss_hints = if ss_mode == 1 {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"libzync/swiftsync-hints/v1");
        h.update(prev_hints);
        h.update(w.height.to_le_bytes());
        h.update(ss_nbits.to_le_bytes());
        for x in ss_words.iter() { h.update(x.to_le_bytes()); }
        let d: [u8; 32] = h.finalize().into();
        d
    } else { prev_hints };

    let pow_ok = unsafe { check_pow(w.header.as_ptr()) } == 1;
    let mut mroot = [0u8; 32];
    let mut mutated = 0u8;
    unsafe { merkle_root(txids.as_ptr() as *const u8, txids.len() as u32, mroot.as_mut_ptr(), &mut mutated) };
    let merkle_ok = mroot[..] == w.header[36..68] && mutated == 0;

    let flat_wtx: &[u8] = unsafe { core::slice::from_raw_parts(wtxids.as_ptr() as *const u8, wtxids.len() * 32) };
    let witness_ok = if w.height >= 481_824 {
        (unsafe { check_witness_commitment(w.coinbase_tx.as_ptr(), w.coinbase_tx.len() as u32,
                                           flat_wtx.as_ptr(), wtxids.len() as u32, has_witness as u32) }) == 1
    } else {
        !has_witness
    };
    let bip34_ok = unsafe { check_bip34(w.coinbase_tx.as_ptr(), w.coinbase_tx.len() as u32, w.height) } == 1;

    let ids_distinct = {
        let mut ids: Vec<[u64; 4]> = txids.iter().map(key).collect();
        ids.sort_unstable();
        ids.windows(2).all(|p| p[0] != p[1])
    };
    let spends_ok = w.smt.coinbase_txid == cb_txid
        && w.smt.coinbase_outputs == cb_outputs
        && w.smt.spends.len() == cb_spends.len()
        && w.smt.spends.iter().zip(cb_spends.iter()).all(|(s, d)| &s.coinbase_txid == d);
    // Same rule as the full path: an SMT overwrite exactly at the two grandfathered blocks.
    let overwrite_ok = w.smt.smt_overwrite.is_some() == is_bip30_block;
    let (bip30_ok, out_smt_root) = if !ids_distinct || !spends_ok || !overwrite_ok {
        (false, w.in_smt_root)
    } else {
        let u = bip30::BlockUpdate {
            coinbase_txid: cb_txid,
            coinbase_outputs: cb_outputs,
            absence_proof: w.smt.absence_proof.clone(),
            overwrite: w.smt.smt_overwrite,
            spends: w.smt.spends.iter()
                .map(|s| bip30::Spend { coinbase_txid: s.coinbase_txid,
                                        current_count: s.current_count, proof: s.proof.clone() })
                .collect(),
        };
        match bip30::apply_block(&w.in_smt_root, &u) {
            Ok(r) => (true, r),
            Err(e) => { env::log(&format!("BIP30 transition rejected the block: {e:?}")); (false, w.in_smt_root) }
        }
    };

    let cb_empty_prevouts = [0u8; 1];
    let mut cb_fee: i64 = 0;
    let cb_struct_ok = unsafe {
        check_tx(w.coinbase_tx.as_ptr(), w.coinbase_tx.len() as u32,
                 cb_empty_prevouts.as_ptr(), cb_empty_prevouts.len() as u32, &mut cb_fee as *mut i64)
    } == 1;
    let cb_is_coinbase = unsafe { is_coinbase_tx(w.coinbase_tx.as_ptr(), w.coinbase_tx.len() as u32) } == 1;
    if !cb_struct_ok || !cb_is_coinbase { all_ok = false; }
    let coinbase_val = unsafe { coinbase_value(w.coinbase_tx.as_ptr(), w.coinbase_tx.len() as u32) };
    let subsidy = unsafe { block_subsidy(w.height) };
    const MAX_MONEY: i64 = 21_000_000 * 100_000_000;
    let money_ok = (0..=MAX_MONEY).contains(&total_fee)
        && (0..=MAX_MONEY).contains(&subsidy)
        && coinbase_val >= 0
        && (subsidy as i128 + total_fee as i128) <= MAX_MONEY as i128;
    let subsidy_ok = money_ok && coinbase_val <= subsidy + total_fee;

    let (mut cb_weight, mut _l) = (0i64, 0i64);
    unsafe { tx_wu_sigops(w.coinbase_tx.as_ptr(), w.coinbase_tx.len() as u32, &mut cb_weight, &mut _l) };
    total_weight += cb_weight;
    total_sigops += unsafe { tx_full_sigops(w.coinbase_tx.as_ptr(), w.coinbase_tx.len() as u32, core::ptr::null(), 0, flags) };
    let ntx = txids.len();
    let cs: i64 = if ntx < 0xfd { 1 } else if ntx <= 0xffff { 3 } else if ntx <= 0xffff_ffff { 5 } else { 9 };
    total_weight += 4 * (80 + cs);
    let weight_ok = total_weight <= MAX_BLOCK_WEIGHT;
    let sigops_ok = total_sigops <= MAX_BLOCK_SIGOPS_COST;
    let cb_final = unsafe {
        is_final_tx(w.coinbase_tx.as_ptr(), w.coinbase_tx.len() as u32, w.height as i64, lock_time as i64)
    } == 1;
    if !cb_final { all_ok = false; }
    if !(all_ok && pow_ok && merkle_ok && subsidy_ok && weight_ok && sigops_ok && witness_ok && bip34_ok && bip30_ok) {
        env::log(&format!("agg-lite: block INVALID all_ok={all_ok} pow={pow_ok} merkle={merkle_ok} subsidy={subsidy_ok} \
            weight={weight_ok} sigops={sigops_ok} witness={witness_ok} bip34={bip34_ok} bip30={bip30_ok}"));
    }

    BlockResult {
        total_fee, pow_ok, merkle_ok, coinbase_val, subsidy, subsidy_ok, all_ok,
        root_matches: true, weight_ok, sigops_ok, witness_ok, bip34_ok, bip30_ok, out_smt_root,
        tip_hash: block_hash,
        nbits: u32::from_le_bytes(w.header[72..76].try_into().unwrap()),
        block_time,
        root_next_roots: normalize(stump.roots.clone()),
        root_next_leaves: stump.num_leaves,
        ss_hints,
        ..Default::default()
    }
}
