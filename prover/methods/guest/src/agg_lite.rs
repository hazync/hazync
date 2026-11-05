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
//! ⚠ SwiftSync only: there is no per-input utreexo proof here. BIP30's two grandfathered duplicates follow
//! the hints-file convention: the coinbases of 91,722 and 91,812 are never indexed or accounted, and the
//! coin-txid SMT takes the overwrite at 91,842 / 91,880. Journal layout, all u32 LE words:
//!   header: KIND_CHUNK_LITE, all_valid, height, block_hash[8], mtp, block_time, n_records
//!   record: txid[8] wtxid[8] prevouts_hash[8] start count first
//!     if first: has_witness n_vin fee[2] weight[2] sigops[2] [total_out[2] with coinlog] n_out (out_leaf[8] vout value[2])*n_out
//!     per owned input: leaf[8] flags (bit0 coinbase coin, bit1 coin height == this block) [coinbase_txid[8] if bit0]

use super::*;

pub const KIND_CHUNK_LITE: u32 = 0xC4A1_0010;

/// coinlog: the block's log witness, read by the aggregate before validation (libzync/src/coinlog.rs).
static mut LOG_WITNESS: Vec<u32> = Vec::new();
pub fn set_log_witness(w: Vec<u32>) { unsafe { LOG_WITNESS = w } }

// E2: the block's trie receipt journal (guest mode 17), verified by the aggregate before this runs.
#[cfg(feature = "trie")]
static mut TRIE_J: Vec<u32> = Vec::new();
#[cfg(feature = "trie")]
pub fn set_trie_journal(w: Vec<u32>) { unsafe { TRIE_J = w } }

/// trie: a transaction's input outpoints and output scriptPubKey ranges, from its serialization (witness or not).
#[cfg(feature = "trie")]
fn tx_io(b: &[u8]) -> Option<(Vec<([u8; 32], u32)>, Vec<(usize, usize)>)> {
    fn vi(b: &[u8], o: &mut usize) -> Option<usize> {
        let f = *b.get(*o)?; *o += 1;
        let n = match f { 0xfd => 2, 0xfe => 4, 0xff => 8, x => return Some(x as usize) };
        let mut v = 0u64;
        for i in 0..n { v |= (*b.get(*o + i)? as u64) << (8 * i); }
        *o += n;
        Some(v as usize)
    }
    let mut o = 4;
    if b.get(4) == Some(&0) && b.get(5) == Some(&1) { o += 2; }
    let nin = vi(b, &mut o)?;
    let mut ins = Vec::with_capacity(nin);
    for _ in 0..nin {
        let t: [u8; 32] = b.get(o..o + 32)?.try_into().ok()?;
        let v = u32::from_le_bytes(b.get(o + 32..o + 36)?.try_into().ok()?);
        o += 36;
        let l = vi(b, &mut o)?;
        o += l + 4;
        ins.push((t, v));
    }
    let nout = vi(b, &mut o)?;
    let mut outs = Vec::with_capacity(nout);
    for _ in 0..nout {
        o += 8;
        let l = vi(b, &mut o)?;
        outs.push((o, l));
        o += l;
    }
    if o > b.len() { return None; }
    Some((ins, outs))
}

/// trie: the scriptPubKey ranges of a serialized Vec<TxOut> (a run's prevouts).
#[cfg(feature = "trie")]
fn prevout_spks(b: &[u8]) -> Option<Vec<(usize, usize)>> {
    let mut o = 0;
    let n = { let f = *b.first()?; o = 1; match f { 0xfd => { o = 3; u16::from_le_bytes(b.get(1..3)?.try_into().ok()?) as usize } 0xfe => { o = 5; u32::from_le_bytes(b.get(1..5)?.try_into().ok()?) as usize } x => x as usize } };
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        o += 8;
        let f = *b.get(o)?; o += 1;
        let l = match f { 0xfd => { let x = u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?) as usize; o += 2; x } 0xfe => { let x = u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?) as usize; o += 4; x } x => x as usize };
        v.push((o, l));
        o += l;
    }
    if o > b.len() { return None; }
    Some(v)
}

/// trie: the coin key is the txid with vout XORed into its FIRST 4 bytes, no hash. The txid is already a hash;
/// two outpoints collide only if their txids agree in 224 bits. In the first bytes, one transaction's coins at one
/// address part at the top of the coin trie (in the last bytes they shared 224 bits: a ~224-node one-child chain).
#[cfg(feature = "trie")]
fn okey(txid: &[u8; 32], vout: u32) -> [u8; 32] {
    let mut o = *txid;
    for (b, v) in o[..4].iter_mut().zip(vout.to_le_bytes()) { *b ^= v; }
    o
}
/// trie: SHA-256(scriptPubKey), one raw compression for scripts up to 55 bytes (every standard type).
#[cfg(feature = "trie")]
fn shash(spk: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    if spk.len() > 55 { return Sha256::digest(spk).into(); }
    let mut blk = [0u8; 64];
    blk[..spk.len()].copy_from_slice(spk);
    blk[spk.len()] = 0x80;
    blk[56..].copy_from_slice(&((spk.len() as u64) * 8).to_be_bytes());
    let mut st = [0x6a09e667u32, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    sha2::compress256(&mut st, &[blk.into()]);
    let mut o = [0u8; 32];
    for (c, x) in o.chunks_exact_mut(4).zip(st) { c.copy_from_slice(&x.to_be_bytes()); }
    o
}
fn hex32(h: &[u8; 32]) -> alloc::string::String { h.iter().map(|b| alloc::format!("{b:02x}")).collect() }
#[cfg(feature = "trie")]
fn hex64(h: &[u8; 64]) -> alloc::string::String { h.iter().map(|b| alloc::format!("{b:02x}")).collect() }

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

/// E2a batch 2: an open-addressing set of indices into a key array (32-byte keys as [u64; 4]), replacing sorts and
/// binary searches over 40-byte rows. Capacity a power of two >= 2n. The slot is the key's first word mixed with a
/// per-block salt (from the block hash, fixed before any transaction could be ground against it); it only places
/// entries — every probe compares the FULL key, so a collision costs a probe, never a wrong answer.
struct KeyTab { slots: Vec<u32>, mask: usize, salt: u64 }
impl KeyTab {
    fn new(n: usize, salt: u64) -> KeyTab {
        let cap = (2 * n.max(1)).next_power_of_two();
        KeyTab { slots: alloc::vec![0; cap], mask: cap - 1, salt }
    }
    #[inline(always)]
    fn slot(&self, k: &[u64; 4]) -> usize { ((k[0] ^ self.salt).wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 32) as usize & self.mask }
    /// Insert index `i` (key `keys[i]`) unless an equal key is present: None if inserted, Some(first index) if not.
    #[inline(always)]
    fn insert(&mut self, keys: &[[u64; 4]], i: u32) -> Option<u32> {
        let k = &keys[i as usize];
        let mut s = self.slot(k);
        loop {
            let v = self.slots[s];
            if v == 0 { self.slots[s] = i + 1; return None; }
            if keys[(v - 1) as usize] == *k { return Some(v - 1); }
            s = (s + 1) & self.mask;
        }
    }
    /// The index whose key equals `k`, if any.
    #[inline(always)]
    fn find(&self, keys: &[[u64; 4]], k: &[u64; 4]) -> Option<u32> {
        let mut s = self.slot(k);
        loop {
            let v = self.slots[s];
            if v == 0 { return None; }
            if keys[(v - 1) as usize] == *k { return Some(v - 1); }
            s = (s + 1) & self.mask;
        }
    }
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
    /// Per output leaf: its index in vout, and its amount.
    out_vout: Vec<u32>,
    out_val: Vec<u64>,
}

impl ChunkCtx {
    pub fn new(height: u32, flags: u32, mtp: u32, block_time: u32) -> Self {
        Self { height, flags, mtp, block_time, words: Vec::new(), n_records: 0, out: Vec::new(), out_vout: Vec::new(), out_val: Vec::new() }
    }

    /// Record one transaction run. Returns false if a per-transaction rule failed.
    #[allow(clippy::too_many_arguments)]
    pub fn record(&mut self, raw_tx: &[u8], prevouts: &[u8], idx: &[u32], ch: &[u32], cb: &[u32],
                  cmtp: &[u32], results: &[i32], leaves: &[u8],
                  wtxid: &[u8; 32], has_witness: u32) -> bool {
        use sha2::{Digest, Sha256};
        let mut ok = true;
        let k = idx.len();
        let (tp, tl) = (raw_tx.as_ptr(), raw_tx.len() as u32);
        // txid (and the output leaves, used only when this run owns input 0).
        let need = (raw_tx.len() / 8 + 1) * 32;
        if self.out.len() < need { self.out.resize(need, 0); self.out_vout.resize(need / 32, 0); self.out_val.resize(need / 32, 0); }
        let mut txid = [0u8; 32];
        // A transaction can repeat only if all its inputs can (coinbase-smt/src/bip30.rs), so its outputs are tracked
        // exactly when every input is. A tracked transaction must sit whole in one run: the host keeps it so, and the
        // aggregate refuses a transaction whose inputs are all tracked but that did not claim to be.
        let n_vin_all = unsafe { tx_vin_count(tp, tl) };
        let tracked_tx = !cb.is_empty() && cb.len() as u32 == n_vin_all && cb.iter().all(|c| c & 2 != 0);
        let n_out = unsafe { tx_out_leaves_mv(tp, tl, self.height, (tracked_tx as u32) << 1, self.mtp, self.out.as_mut_ptr(), txid.as_mut_ptr(),
                                              self.out_vout.as_mut_ptr(), self.out_val.as_mut_ptr()) };
        let ph: [u8; 32] = Sha256::digest(prevouts).into();
        let start = idx.first().copied().unwrap_or(0);
        // In 64 bits: `start` is the prover's, and a wrap here would let the run reach outside the transaction.
        if start as u64 + idx.len() as u64 > n_vin_all as u64 { ok = false; }
        for (j, i) in idx.iter().enumerate() {
            if *i as u64 != start as u64 + j as u64 { ok = false; }
        }
        let first = start == 0;
        // Only the block's first transaction may be a coinbase (Core CheckBlock bad-cb-multiple). A
        // coinbase-shaped transaction here would have fee 0 and outputs from nothing (review 2026-10-30).
        if unsafe { is_coinbase_tx(tp, tl) } != 0 { ok = false; env::log("agg-lite: core-reject: bad-cb-multiple"); }
        push_hash(&mut self.words, &txid);
        // EVERY run commits the wtxid, not only the input-0 run: the txid omits the witness, so a later
        // run of the same transaction could verify its inputs against DIFFERENT witness bytes than the
        // ones the coinbase's witness commitment covers. The aggregate requires all runs to agree.
        push_hash(&mut self.words, wtxid);
        push_hash(&mut self.words, &ph);
        self.words.push(start);
        self.words.push(k as u32);
        self.words.push(first as u32);
        // A coin cannot be spent before the block that creates it, and its flag is exactly 0 or 1. These
        // guard what Core's code below is given: Core reads both from its own coin database.
        for j in 0..k {
            if ch[j] > self.height || cb[j] > 3 { ok = false; }
        }
        // Core's own CheckTransaction, IsFinalTx, CheckTxInputs, SequenceLocks and GetTransactionSigOpCost
        // (core_tx_rules.cpp), on this run's inputs. The output arguments are set only when `first`.
        let lock_time = if self.height >= NET.csv_height { self.mtp } else { self.block_time };
        let (mut fee, mut weight, mut sigops) = (0i64, 0i64, 0i64);
        // Sizes are 32-bit here and Core's weight arithmetic assumes they are not: a transaction past 2^30 bytes would wrap
        // through CheckTransaction and GetTransactionWeight. None over MAX_BLOCK_SERIALIZED_SIZE can be in a valid block.
        if raw_tx.len() > MAX_BLOCK_WEIGHT as usize {
            env::log("core-reject: bad-blk-length");
            ok = false;
        }
        if ok && k > 0 {
            let r = unsafe { hz_tx_rules_c(tp, tl, prevouts.as_ptr(), prevouts.len() as u32, start, k as u32,
                                           ch.as_ptr(), cb.as_ptr(), cmtp.as_ptr(), self.height, self.mtp,
                                           lock_time as i64, self.flags, (self.height >= NET.csv_height) as u32,
                                           &mut fee, &mut weight, &mut sigops) };
            if r != 1 { ok = false; log_reject(); }
        } else {
            ok = false;
        }
        // trie: the transaction's inputs and output scripts, parsed once per run
        #[cfg(feature = "trie")]
        let io = tx_io(raw_tx);
        if first {
            let n_vin = unsafe { tx_vin_count(tp, tl) };
            self.words.push(has_witness | (tracked_tx as u32) << 1);
            self.words.push(n_vin);
            push_i64(&mut self.words, fee);
            push_i64(&mut self.words, weight);
            push_i64(&mut self.words, sigops);
            // coinlog: every output's amount, spendable or not, so the aggregate knows the transaction's inputs (fee + this).
            push_i64(&mut self.words, unsafe { tx_total_out(tp, tl) });
            self.words.push(n_out);
            #[cfg(feature = "trie")]
            if io.is_none() { ok = false; }
            for l in 0..n_out as usize {
                push_hash(&mut self.words, &self.out[32 * l..32 * l + 32]);
                self.words.push(self.out_vout[l]);
                push_i64(&mut self.words, self.out_val[l] as i64);
                #[cfg(feature = "trie")]
                {
                    // the output's script hash (the aggregate derives its coin key from the txid and vout it reads)
                    let v = self.out_vout[l];
                    let (so, sl) = io.as_ref().and_then(|x| x.1.get(v as usize).copied()).unwrap_or((0, 0));
                    push_hash(&mut self.words, &shash(raw_tx.get(so..so + sl).unwrap_or(&[])));
                }
            }
        }
        #[cfg(feature = "trie")]
        let (io_in, pspk) = (io.as_ref().map(|x| &x.0), prevout_spks(prevouts));
        #[cfg(feature = "trie")]
        if io_in.is_none() || pspk.is_none() { ok = false; }
        for j in 0..k {
            if results[j] != 1 { ok = false; env::log("core-reject: script"); }
            push_hash(&mut self.words, &leaves[32 * j..32 * j + 32]);
            // bit 2: tracked. bit 0: the BIP30 tree has this coin (a coinbase or tracked one); bit 1: the coin claims THIS block's height, so the aggregate must find it
            // in this block's created set (an external coin of the current height cannot exist; without this
            // it would only fail at H).
            self.words.push((cb[j] != 0) as u32 | (((ch[j] == self.height) as u32) << 1) | ((cb[j] & 2) << 1));
            if cb[j] != 0 {
                let mut t = [0u8; 32];
                if unsafe { tx_input_prevout_txid(tp, tl, idx[j], t.as_mut_ptr()) } != 1 { ok = false; }
                push_hash(&mut self.words, &t);
            }
            // trie: the spent coin's outpoint (from the transaction) and script (from the prevout the coin leaf binds)
            #[cfg(feature = "trie")]
            {
                let (t, v) = io_in.as_ref().and_then(|x| x.get(idx[j] as usize).copied()).unwrap_or(([0; 32], 0));
                push_hash(&mut self.words, &okey(&t, v));
                #[cfg(feature = "trie")]
                {
                    let (so, sl) = pspk.as_ref().and_then(|x| x.get(idx[j] as usize).copied()).unwrap_or((0, 0));
                    push_hash(&mut self.words, &shash(prevouts.get(so..so + sl).unwrap_or(&[])));
                }
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

/// Trie key words in a record: per input the coin key and the script hash; per output the script hash only (the
/// aggregate derives the coin key from the txid and vout).
const KEY_WORDS: usize = if cfg!(feature = "trie") { 16 } else { 0 };
const OUT_KEY_WORDS: usize = if cfg!(feature = "trie") { 8 } else { 0 };

/// Advances `r` past one record (the layout `ChunkCtx::record` writes) without interpreting it.
fn skip_record(r: &mut Rd) {
    r.i = r.i.saturating_add(24);
    let (_start, count, first) = (r.u(), r.u(), r.u());
    if first == 1 {
        let _ = (r.u(), r.u());
        r.i = r.i.saturating_add(6);
        { r.i = r.i.saturating_add(2); }
        let n_out = r.u() as usize;
        r.i = r.i.saturating_add(n_out.saturating_mul(11 + OUT_KEY_WORDS));
    }
    for _ in 0..count {
        if r.bad || r.i > r.w.len() { r.bad = true; return; }
        r.i = r.i.saturating_add(8);
        if r.u() & 1 == 1 { r.i = r.i.saturating_add(8); }
        r.i = r.i.saturating_add(KEY_WORDS);
    }
    if r.i > r.w.len() { r.bad = true; }
}

#[cfg(all(feature = "agg-lite", not(feature = "coinlog")))]
compile_error!("libzync's aggregate keeps its coins in the coin log: build with HAZYNC_COINLOG (on in the libzync profile)");

/// Aggregate side. Same checks, same order of SwiftSync operations, as `validate_block_staged`'s
/// aggregation path, with every per-transaction fact taken from the chunk journals.
/// `prev_trie` = the previous chain state's counting-trie root (zero without the trie).
#[inline(never)] // inlined, the aggregate ran ~1.08 M cycles dearer on 962,000 (HANDOVER.md §8)
pub fn validate_block_lite(w: &BlockWitness, mtp: u32, journals: &[Vec<u32>],
                           prev_trie: [u8; 32], order: &[(u32, u32)]) -> BlockResult {
    let mut all_ok = true;
    assert!(w.header.len() == 80, "block header must be exactly 80 bytes, got {}", w.header.len());
    let block_time = u32::from_le_bytes(w.header[68..72].try_into().unwrap());
    let block_hash = dsha256(&w.header);
    let flags = block_script_flags(w.height, &block_hash);
    // The two grandfathered BIP30 blocks are handled below with SwiftSync subtractions instead of the
    // full path's utreexo delete, so no overwrite witness is ever accepted here.
    let is_bip30_block = block_hash == BIP30_OVERWRITE_A || block_hash == BIP30_OVERWRITE_B;
    if w.bip30.is_some() { all_ok = false; }

    // Capacity up front: the bump allocator never frees, so every Vec regrowth copies AND leaks.
    let words: usize = journals.iter().map(|j| j.len()).sum();
    // Coinbase: the only transaction the aggregate parses.
    let mut output_leaves: Vec<[u8; 32]> = Vec::with_capacity(words / 9);
    #[cfg(feature = "trie")]
    let mut out_keys: Vec<([u8; 32], [u8; 32])> = Vec::with_capacity(words / 9);
    let mut out_pos: Vec<u32> = Vec::with_capacity(words / 9);   // creating tx position, parallel
    let mut out_vout: Vec<u32> = Vec::with_capacity(words / 9);  // output index in its transaction, parallel
    let mut out_val: Vec<u64> = Vec::with_capacity(words / 9);   // amount, parallel
    let mut cb_txid = [0u8; 32];
    let cb_outputs = {
        let mut buf = vec![0u8; (w.coinbase_tx.len() / 8 + 1) * 32];
        let mut cb_vout = vec![0u32; w.coinbase_tx.len() / 8 + 1];
        let mut cb_val = vec![0u64; w.coinbase_tx.len() / 8 + 1];
        // Tracked: mined before BIP34 and indicating a later height, or the repeat of a tracked coinbase. A false claim
        // of `coinbase_prior` does not verify in apply_block.
        let indicated = unsafe { hz_indicated_height(w.coinbase_tx.as_ptr(), w.coinbase_tx.len() as u32) };
        let root = w.height < BIP34_HEIGHT && indicated > w.height && indicated < bip30::REPEAT_HORIZON;
        let repeat = w.smt.coinbase_prior == Some(bip30::REPEATABLE);
        let flags = 1 | ((root || repeat) as u32) << 1;
        let n = unsafe { tx_out_leaves_mv(w.coinbase_tx.as_ptr(), w.coinbase_tx.len() as u32, w.height, flags, mtp,
                                          buf.as_mut_ptr(), cb_txid.as_mut_ptr(), cb_vout.as_mut_ptr(), cb_val.as_mut_ptr()) };
        // BIP30, the hints-file convention (Core's IsBIP30Unspendable): the coinbase coins of 91,722 and
        // 91,812 are never indexed or accounted. Their outpoints come back, and are accounted, when 91,880
        // and 91,842 duplicate them. Same final set as Core's overwrite, with no subtraction needed.
        if !NET.is_bip30_unspendable(w.height, &block_hash) {
            for l in 0..n as usize {
                let mut h = [0u8; 32];
                h.copy_from_slice(&buf[32 * l..32 * l + 32]);
                output_leaves.push(h);
                out_pos.push(0);
                out_vout.push(cb_vout[l]);
                out_val.push(cb_val[l]);
                #[cfg(feature = "trie")]
                {
                    let sh = tx_io(&w.coinbase_tx).and_then(|x| x.1.get(cb_vout[l] as usize).copied())
                        .map(|(o, n)| shash(&w.coinbase_tx[o..o + n])).unwrap_or([0; 32]);
                    out_keys.push((okey(&cb_txid, cb_vout[l]), sh));
                }
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
    // coinlog: the amount of every coin the block's transactions spend (fee + every output, per transaction).
    let mut total_in: u64 = 0;
    let mut total_weight: i64 = 0;
    let mut total_sigops: i64 = 0;
    let mut spends: Vec<([u8; 32], u32, bool, u32)> = Vec::with_capacity(words / 9);   // (leaf, spending tx position, claims this height, input index), block order
    let mut cb_spends: Vec<([u8; 32], bool)> = Vec::new();
    // trie: (coin key, script hash) per spend and per indexed output, parallel to `spends` / `output_leaves`
    #[cfg(feature = "trie")]
    let mut spend_keys: Vec<([u8; 32], [u8; 32])> = Vec::with_capacity(words / 9);
    let mut tracked_new: Vec<([u8; 32], u32)> = Vec::new();
    // The open transaction: all inputs seen so far are tracked, and whether it claimed to be.
    let (mut tr_all, mut tr_claim) = (false, false);

    // Walk every chunk's records in BLOCK order. `cur` = (txid, wtxid, prevouts hash, n_vin, next input expected).
    let mut cur: Option<([u8; 32], [u8; 32], [u8; 32], u32, u32)> = None;
    let mut tx_pos: u32 = 0;
    // Pass 1: each journal's header, and where each of its records starts. A chunk proved for this block carries its
    // hash and time. txlog (txproofs/README.md §7, Phase A): a SPECULATIVE chunk, proved before the block existed for
    // this height and MTP, carries zero hash and time. It is the same chunk whenever neither matters: from CSV on, the
    // lock time is the MTP (block time unused), and the hash enters only through the script flags, so they must be
    // the flags of this height with no hash exception.
    let mut rec_at: Vec<(usize, usize)> = Vec::new();   // (journal, word offset), journal order
    let mut rec_base: Vec<usize> = Vec::with_capacity(journals.len());
    for (ji, j) in journals.iter().enumerate() {
        let mut r = Rd { w: j, i: 0, bad: false };
        rec_base.push(rec_at.len());
        if r.u() != KIND_CHUNK_LITE { all_ok = false; continue; }
        if r.u() != 1 { all_ok = false; }                       // the chunk's all_valid
        // Read every field BEFORE comparing: a short-circuiting `||` over the cursor skipped the
        // remaining reads on the first mismatch and misaligned the rest of the journal.
        let (c_height, c_hash, c_mtp, c_time) = (r.u(), r.h(), r.u(), r.u());
        let speculative = cfg!(feature = "txlog") && c_hash == [0u8; 32] && c_time == 0
            && w.height >= NET.csv_height && block_script_flags(w.height, &[0u8; 32]) == flags;
        if c_height != w.height || c_mtp != mtp || (!speculative && (c_hash != block_hash || c_time != block_time)) {
            env::log("agg-lite: REJECT chunk context (height / block hash / mtp / time) differs");
            all_ok = false;
        }
        let n_rec = r.u();
        for _ in 0..n_rec {
            rec_at.push((ji, r.i));
            skip_record(&mut r);
            if r.bad { break; }
        }
        if r.bad || r.i != j.len() { all_ok = false; }
    }
    // Pass 2: the records in block order. Without txlog, every record in journal order (as before). With txlog the host
    // names the block's order as (journal, record) pairs: a batch of pre-proved mempool transactions holds some the
    // block does not, and those are left unread. Each record at most once; the Merkle root below binds the txids
    // walked to the block's exact list, so a missing or extra transaction still fails there.
    let mut sel: Vec<usize> = Vec::with_capacity(rec_at.len());
    if order.is_empty() {
        sel.extend(0..rec_at.len());
    } else {
        let mut used = vec![false; rec_at.len()];
        for &(ji, k) in order {
            let (ji, k) = (ji as usize, k as usize);
            let next = rec_base.get(ji + 1).copied().unwrap_or(rec_at.len());
            match rec_base.get(ji).and_then(|b| b.checked_add(k)).filter(|x| *x < next) {
                Some(x) if !used[x] => { used[x] = true; sel.push(x); }
                _ => { env::log("agg-lite: REJECT record order names a record twice or past a journal"); all_ok = false; }
            }
        }
    }
    for x in sel {
        let (ji, off) = rec_at[x];
        let mut r = Rd { w: &journals[ji], i: off, bad: false };
        {
            let (txid, wt, ph, start, count, first) = (r.h(), r.h(), r.h(), r.u(), r.u(), r.u());
            if first == 1 {
                if start != 0 { all_ok = false; }
                if let Some((_, _, _, n, next)) = cur { if next != n { all_ok = false; } }
                if cur.is_some() && tr_all != tr_claim { all_ok = false; }
                tx_pos += 1;
                let hw = r.u();
                (tr_all, tr_claim) = (true, hw & 2 != 0);
                let n_vin = r.u();
                let (fee, weight, sigops) = (r.i64(), r.i64(), r.i64());
                { let t_out = r.i64(); total_in = total_in.saturating_add((fee as u64).saturating_add(t_out as u64)); }
                let n_out = r.u();
                txids.push(txid);
                wtxids.push(wt);
                has_witness |= hw & 1 == 1;
                total_fee = total_fee.saturating_add(fee);
                total_weight = total_weight.saturating_add(weight);
                total_sigops = total_sigops.saturating_add(sigops);
                for _ in 0..n_out {
                    let l = r.h();
                    let (v, val) = (r.u(), r.i64() as u64);
                    output_leaves.push(l);
                    out_pos.push(tx_pos);
                    out_vout.push(v);
                    out_val.push(val);
                    #[cfg(feature = "trie")]
                    {
                        let ok_ = okey(&txid, v);
                        let sh = r.h();
                        out_keys.push((ok_, sh));
                    }
                }
                if tr_claim && n_out > 0 { tracked_new.push((txid, n_out)); }
                cur = Some((txid, wt, ph, n_vin, 0));
            } else {
                match cur {
                    Some((t, w2, p, _, next)) if t == txid && w2 == wt && p == ph && start == next => {}
                    _ => all_ok = false,
                }
            }
            let Some(c) = cur.as_mut() else { all_ok = false; break; };
            if count == 0 || c.4 as u64 + count as u64 > c.3 as u64 { all_ok = false; }
            c.4 = c.4.saturating_add(count);
            for k in 0..count {
                let leaf = r.h();
                let f = r.u();
                if f > 7 { all_ok = false; }
                spends.push((leaf, tx_pos, f & 2 != 0, start.wrapping_add(k)));
                #[cfg(feature = "trie")]
                let spend_key_at = spends.len() - 1;
                tr_all &= f & 4 != 0;
                // The BIP30 tree counts only TRACKED transactions' outputs (UTREEXO_FORMAT.md §2a), the only ones that can
                // ever repeat. A repeat of an untracked coinbase is refused whatever its count says (live or SPENT), so
                // decrementing it on a spend changed no verdict, and a double spend is the coin log's refusal (review
                // 11-03; it was ~1.5 M cycles per spent coinbase coin). The txid is still read, to keep the record aligned.
                if f & 1 == 1 { let t = r.h(); if f & 4 != 0 { cb_spends.push((t, true)); } }
                #[cfg(feature = "trie")]
                {
                    let ok_ = r.h();
                    let sh = r.h();
                    debug_assert!(spend_keys.len() == spend_key_at);
                    spend_keys.push((ok_, sh));
                }
            }
            if r.bad { all_ok = false; break; }
        }
    }
    // No records at all = a coinbase-only block (common before ~2011): coverage holds trivially. Safe
    // because the merkle check below binds the full txid list, so dropping every record of a block that
    // HAS other transactions still fails there.
    let covered = match cur { Some((_, _, _, n, next)) => next == n, None => true };
    if !covered { all_ok = false; }
    if cur.is_some() && tr_all != tr_claim { all_ok = false; }
    // A tracked coinbase indicating this height or above is not in the tree, so the guest stops short of it.
    if w.height >= bip30::REPEAT_HORIZON { all_ok = false; }
    if !all_ok { env::log(&format!("agg-lite: REJECT after the record walk (transactions={}, last covered={covered})", tx_pos)); }

    // The coins this block creates, by leaf: a hash table of output indices (E2a batch 2; it was a sort of 40-byte rows
    // and four binary-search passes). A repeated leaf keeps its FIRST output (lowest position), as the full path's
    // `entry().or_insert` does: outputs are inserted in block order and a present key is never replaced.
    let salt = u64::from_le_bytes(block_hash[..8].try_into().unwrap());
    let okeys: Vec<[u64; 4]> = output_leaves.iter().map(key).collect();
    let mut ctab = KeyTab::new(okeys.len(), salt);
    for k in 0..okeys.len() as u32 { let _ = ctab.insert(&okeys, k); }
    // Indexed by the coin's FIRST output index (what `find` returns).
    let mut spent = vec![false; okeys.len()];
    let find = |k: &[u64; 4]| ctab.find(&okeys, k);
    // The same external coin spent twice in one block: Core rejects it at the second spend
    // (bad-txns-inputs-missingorspent). Checked in the spend walk below with a second table (review 2026-10-30).
    let skeys: Vec<[u64; 4]> = spends.iter().map(|(l, _, _, _)| key(l)).collect();
    let mut etab = KeyTab::new(skeys.len(), salt);
    let mut ext_twice = false;
    let (mut ext, mut in_block_spent): (Vec<([u8; 32], u32, u32)>, u64) = (Vec::new(), 0);   // (leaf, tx position, input index)
    #[cfg(feature = "trie")]
    let mut ext_keys: Vec<([u8; 32], [u8; 32])> = Vec::new();
    // htree: the spender (tx position, input index) of each created coin this block also spends
    #[cfg(feature = "trie")]
    let mut in_sp: Vec<Option<(u32, u32)>> = alloc::vec![None; okeys.len()];
    for (_si, (leaf, txp, same_height, _vin)) in spends.iter().enumerate() {
        match find(&skeys[_si]) {
            Some(i) => {
                let i = i as usize;
                // In-block spend: created by a strictly earlier tx, spent at most once.
                // Core: bad-txns-inputs-missingorspent, the coin is not in the view yet or no longer.
                if out_pos[i] >= *txp { env::log("agg-lite: REJECT a coin is spent before the transaction that creates it"); all_ok = false; }
                if spent[i] { env::log("agg-lite: REJECT a coin created in this block is spent twice"); all_ok = false; }
                spent[i] = true;
                #[cfg(feature = "trie")]
                { in_sp[i] = Some((*txp, *_vin)); }
                { in_block_spent = in_block_spent.saturating_add(out_val[i]); }
            }
            None => {
                if etab.insert(&skeys, _si as u32).is_some() { ext_twice = true; }
                if *same_height {
                    env::log("agg-lite: REJECT a spend claims a coin of this height that this block does not create");
                    all_ok = false;
                }
                // coinlog: a coin from an earlier block, which must be unspent in the log at the position the prover names
                ext.push((*leaf, *txp, *_vin));
                #[cfg(feature = "trie")]
                ext_keys.push(spend_keys[_si]);
            }
        }
    }
    if ext_twice {
        env::log("agg-lite: REJECT the same external coin is spent twice in this block");
        all_ok = false;
    }
    // coinlog: every coin the block creates and does not spend is appended in block order; the earlier coins it spends
    // are flipped in the log at the positions the witness names. A collect run (no witness yet) logs both lists for the
    // host instead and commits no state.
    // E2: the counting trie's root, carried in the chain state's former hint-hash slot (an empty trie's root is zero)
    #[allow(unused_mut)]
    let mut trie_root: [u8; 32] = prev_trie;
    let (next_roots, next_leaves, survivor_value, spent_value) = {
        let mut appends: Vec<[u8; 32]> = Vec::with_capacity(output_leaves.len());
        let mut survivor_value: u64 = 0;
        for (k, leaf) in output_leaves.iter().enumerate() {
            if let Some(i) = find(&okeys[k]) { if spent[i as usize] { continue; } }
            appends.push(*leaf);
            survivor_value = survivor_value.saturating_add(out_val[k]);
        }
        let (mut roots, mut n) = (w.root_prev.roots.clone(), w.root_prev.num_leaves);
        // utxohash (instant IBD, 11-04): the chain state's multiset hash of the LIVE coins. Every coin that outlives
        // its block enters it, every earlier coin this block spends leaves it; in-block create-then-spend never
        // enters. A new node checks a downloaded UTXO set (coin leaves) against this one value. Collect runs log the
        // leaves for the host's hints (swiftsync.rs).
        #[cfg(feature = "utxohash")]
        {
            for l in &appends { swiftsync::add(l); }
            for (l, _, _) in &ext { swiftsync::sub(l); }
        }
        if swiftsync::collecting() {
            for (l, t, v) in &ext { env::log(&alloc::format!("coinlog: S {} {t} {v}", hex32(l))); }
            for l in &appends { env::log(&alloc::format!("coinlog: A {}", hex32(l))); }
        } else {
            #[allow(static_mut_refs)]
            match coinlog::decode_witness(unsafe { &LOG_WITNESS }) {
                Some((pos, sib)) if pos.len() == ext.len() => {
                    // Sorted by log position through a (position, index) list: 16-byte rows moved, not 72-byte ones.
                    let mut ord: Vec<(u64, u32)> = pos.iter().enumerate().map(|(i, p)| (*p, i as u32)).collect();
                    ord.sort_unstable();
                    let sp: Vec<coinlog::Spend> = ord.iter().map(|&(p, i)| {
                        let (l, t, v) = &ext[i as usize];
                        (p, coinlog::w_of(l), coinlog::w_of(&coinlog::spender(w.height, *t, *v)))
                    }).collect();
                    if let Err(e) = coinlog::apply(&mut roots, &mut n, &sp, &appends, &sib) {
                        env::log(&alloc::format!("agg-lite: REJECT coin log: {e:?}"));
                        all_ok = false;
                    }
                }
                _ => { env::log("agg-lite: REJECT coin log witness malformed or wrong length"); all_ok = false; }
            }
        }
        // The counting trie (libzync/src/ttree.rs). This block's update list -- every earlier coin it spends, marked with
        // its spender; every coin it creates, inserted (one spent in its own block already marked) -- is applied in the
        // trie's OWN receipt, proven beside the chunks; the aggregate recomputes the list's digest, which that receipt
        // commits. Keys are script hash ‖ coin key. A collect run logs the keys for the host instead.
        #[cfg(feature = "trie")]
        {
            let mk = |k: &([u8; 32], [u8; 32])| -> ttree::Key {
                let mut o = [0u8; 64];
                o[..32].copy_from_slice(&k.1);
                o[32..].copy_from_slice(&k.0);
                o
            };
            let mut hins: Vec<(ttree::Key, Option<coinlog::W>, u64)> = Vec::with_capacity(output_leaves.len());
            for k in 0..output_leaves.len() {
                // one entry per created key: a repeated leaf keeps its first output
                if find(&okeys[k]) != Some(k as u32) { continue; }
                hins.push((mk(&out_keys[k]), in_sp[k].map(|(t, v)| coinlog::w_of(&coinlog::spender(w.height, t, v))), out_val[k]));
            }
            let marks: Vec<(ttree::Key, coinlog::W)> = ext_keys.iter().zip(ext.iter())
                .map(|(k, (_, tx, vin))| (mk(k), coinlog::w_of(&coinlog::spender(w.height, *tx, *vin)))).collect();
            if swiftsync::collecting() {
                // the whole list, for the host's trie update (mode 17): key, then spender words / spent flag + value
                let hw = |w: &coinlog::W| -> alloc::string::String { w.iter().map(|x| alloc::format!("{x:08x}")).collect() };
                for (k, sp) in &marks { env::log(&alloc::format!("trie: R {} {}", hex64(k), hw(sp))); }
                for (k, sp, v) in &hins {
                    env::log(&alloc::format!("trie: I {} {} {} {v}", hex64(k), sp.is_some() as u32, hw(&sp.unwrap_or([0; 8]))));
                }
            } else {
                // The trie receipt must start from the chain's trie root and apply exactly this list.
                let dg = ttree::list_digest(&marks, &hins);
                #[allow(static_mut_refs)]
                let tj = unsafe { &TRIE_J };
                let prev_root = coinlog::w_of(&trie_root);
                let jd: alloc::vec::Vec<u32> = dg.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
                if tj.len() != ttree::TRIE_JOURNAL_WORDS || tj[0] != ttree::KIND_TRIE {
                    env::log("agg-lite: REJECT no trie receipt for this block"); all_ok = false;
                } else if tj[1..9] != prev_root[..] {
                    env::log("agg-lite: REJECT the trie receipt does not start from the chain's trie root"); all_ok = false;
                } else if tj[17..25] != jd[..] {
                    env::log("agg-lite: REJECT the trie receipt applies a different update list"); all_ok = false;
                } else {
                    let nr: coinlog::W = tj[9..17].try_into().unwrap();
                    trie_root = coinlog::h_of(&nr);
                }
            }
        }
        (normalize(roots), n, survivor_value, total_in.saturating_sub(in_block_spent))
    };
    swiftsync::report();

    let pow_ok = unsafe { check_pow(w.header.as_ptr()) } == 1;
    let mut mroot = [0u8; 32];
    let mut mutated = 0u8;
    unsafe { merkle_root(txids.as_ptr() as *const u8, txids.len() as u32, mroot.as_mut_ptr(), &mut mutated) };
    let merkle_ok = mroot[..] == w.header[36..68] && mutated == 0;

    let flat_wtx: &[u8] = unsafe { core::slice::from_raw_parts(wtxids.as_ptr() as *const u8, wtxids.len() * 32) };
    // BIP325: on signet the block solution in the coinbase witness commitment must satisfy the challenge.
    // Core's own CheckSignetBlockSolution, given the coinbase and the txids the merkle check just bound.
    #[cfg(feature = "signet")]
    {
        let rest: &[u8] = unsafe { core::slice::from_raw_parts(txids[1..].as_ptr() as *const u8, (txids.len() - 1) * 32) };
        if unsafe { hz_check_signet(w.header.as_ptr(), w.coinbase_tx.as_ptr(), w.coinbase_tx.len() as u32,
                                    rest.as_ptr(), (txids.len() - 1) as u32) } != 1 {
            env::log("agg-lite: REJECT signet block solution");
            all_ok = false;
        }
    }

    let ids_distinct = {
        let ids: Vec<[u64; 4]> = txids.iter().map(key).collect();
        let mut t = KeyTab::new(ids.len(), salt);
        (0..ids.len() as u32).all(|i| t.insert(&ids, i).is_none())
    };
    let spends_ok = w.smt.coinbase_txid == cb_txid
        && w.smt.coinbase_outputs == cb_outputs
        && w.smt.spends.len() == cb_spends.len()
        && w.smt.spends.iter().zip(cb_spends.iter()).all(|(s, d)| s.coinbase_txid == d.0 && s.tracked == d.1)
        && w.smt.tracked.len() == tracked_new.len()
        && w.smt.tracked.iter().zip(tracked_new.iter()).all(|(t, d)| t.txid == d.0 && t.outputs == d.1);
    // Same rule as the full path: an SMT overwrite exactly at the two grandfathered blocks.
    let overwrite_ok = w.smt.smt_overwrite.is_some() == is_bip30_block;
    let (bip30_ok, out_smt_root) = if !ids_distinct || !spends_ok || !overwrite_ok {
        (false, w.in_smt_root)
    } else {
        let u = bip30::BlockUpdate {
            coinbase_prior: w.smt.coinbase_prior,
            tracked: w.smt.tracked.iter()
                .map(|t| bip30::TrackedTx { txid: t.txid, outputs: t.outputs, prior: t.prior, proof: t.proof.clone() })
                .collect(),
            coinbase_txid: cb_txid,
            coinbase_outputs: cb_outputs,
            absence_proof: w.smt.absence_proof.clone(),
            overwrite: w.smt.smt_overwrite,
            spends: w.smt.spends.iter()
                .map(|s| bip30::Spend { coinbase_txid: s.coinbase_txid, tracked: s.tracked,
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
    if !cb_struct_ok { log_reject(); }   // Core's CheckTransaction reason (bad-cb-length, ...)
    if !cb_struct_ok || !cb_is_coinbase { all_ok = false; }
    let coinbase_val = unsafe { coinbase_value(w.coinbase_tx.as_ptr(), w.coinbase_tx.len() as u32) };
    let subsidy = unsafe { block_subsidy(w.height) };
    const MAX_MONEY: i64 = 21_000_000 * 100_000_000;
    let money_ok = (0..=MAX_MONEY).contains(&total_fee)
        && (0..=MAX_MONEY).contains(&subsidy)
        && coinbase_val >= 0
        && (subsidy as i128 + total_fee as i128) <= MAX_MONEY as i128;
    let subsidy_ok = money_ok && coinbase_val <= subsidy + total_fee;

    total_weight += unsafe { hz_weight_c(w.coinbase_tx.as_ptr(), w.coinbase_tx.len() as u32) };
    total_sigops += unsafe { hz_coinbase_sigops_c(w.coinbase_tx.as_ptr(), w.coinbase_tx.len() as u32, flags) };
    let ntx = txids.len();
    let cs: i64 = if ntx < 0xfd { 1 } else if ntx <= 0xffff { 3 } else if ntx <= 0xffff_ffff { 5 } else { 9 };
    total_weight += 4 * (80 + cs);
    // Core's own ContextualCheckBlock, lifted from validation.cpp (core_block_rules.cpp): the coinbase's finality, BIP34,
    // the witness rules and the weight limit.
    if w.coinbase_tx.len() > MAX_BLOCK_WEIGHT as usize { all_ok = false; }
    let context_ok = unsafe { hz_block_rules_c(w.header.as_ptr(), w.coinbase_tx.as_ptr(), w.coinbase_tx.len() as u32, flat_wtx.as_ptr(),
                                               wtxids.len() as u32, has_witness as u32, w.height, mtp, total_weight) } == 1;
    if !context_ok { log_reject(); }
    let (weight_ok, witness_ok, bip34_ok) = (context_ok, context_ok, context_ok);
    let sigops_ok = total_sigops <= MAX_BLOCK_SIGOPS_COST;
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
        root_next_roots: next_roots,
        root_next_leaves: next_leaves,
        trie_root,
        survivor_value,
        spent_value,
        ..Default::default()
    }
}
