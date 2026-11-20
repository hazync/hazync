//! ROADMAP H-B: the NATIVE prep. The chain state after a block, computed on the host from the previous state and the
//! block file, with no guest run and no validation (Core validated the chain; the proofs check everything again). It
//! must be byte-identical to what the product aggregate commits (coin log + counting trie + live-coin hash): block h is
//! then proved as a base step on prep's state at h-1, in any order, and a join refuses any state that is not exactly
//! the chain's (sha256d of the journal), so a prep bug costs time, never soundness.
//!
//! Every field, after the guest (prover/methods/guest/src/main.rs `aggregate`, agg_lite.rs `validate_block_lite`):
//!   - created coins: the coinbase's outputs (none at the BIP30-unspendable 91,722 / 91,812), then every other
//!     transaction's, in block order, unspendable outputs (OP_RETURN, script over 10,000 bytes) skipped; the leaf
//!     binds the parent's MTP and the coinbase / BIP30-tracked flags, the latter derived as the guest does (tracked.rs:
//!     the block files carry no tracked bit);
//!   - a spend of a coin this block creates (same outpoint): in-block, never in the log; any other: external, a flip
//!     in the coin log at the coin's position, spender (height, tx position, input index within the tx);
//!   - appends: created coins not spent in the block; utxo_value += their values - the external coins' values;
//!   - the trie: marks = the external spends (key, spender), inserts = one per created outpoint (key, its in-block
//!     spender if any, value); key = sha256(script) || txid with the vout XORed in;
//!   - the live-coin hash: + each append's point, - each external spend's point (first ctr with x = sha256(leaf||ctr)
//!     on the curve, the even root), committed affine x || y in 8 little-endian limbs each, live_set 0 = the identity;
//!   - the undo hash (#729, docs/libzync/UNDO.md): every input of every non-coinbase transaction, block order, its coin's
//!     (height, coinbase flag, amount, script) from the block file's prevouts; the block leaf commits it for every block
//!     whose proving image has the commitment (handover_pin::leaf_has_undo), else the 0x02 leaf;
//!   - BIP30 tree, block forest, chainwork, times: as the guest.

use crate::{ChainState, Smt};
use bitcoin::consensus::deserialize;
use bitcoin::hashes::Hash as _;
use bitcoin::secp256k1::{PublicKey, Secp256k1};
use bitcoin::{Transaction, TxOut};
use sha2::{Digest, Sha256};

/// The live-coin multiset hash as a point sum (None = the identity).
pub(crate) struct Ecmh(Option<PublicKey>);

impl Ecmh {
    pub(crate) fn from_state(set: u32, w: &[u32; 16]) -> Ecmh {
        if set == 0 { return Ecmh(None); }
        let mut b = [4u8; 65];
        let be = |l: &[u32]| -> [u8; 32] { let mut o = [0u8; 32]; for k in 0..8 { o[28 - 4 * k..32 - 4 * k].copy_from_slice(&l[k].to_be_bytes()); } o };
        b[1..33].copy_from_slice(&be(&w[..8]));
        b[33..].copy_from_slice(&be(&w[8..]));
        Ecmh(Some(PublicKey::from_slice(&b).expect("live_hash is not a curve point")))
    }
    /// The leaf's point: the first ctr with x = sha256(leaf || ctr LE) a curve x, the even root (libsecp's 0x02 parse).
    fn point(leaf: &[u8; 32]) -> PublicKey {
        for ctr in 0u32.. {
            let mut h = Sha256::new();
            h.update(leaf);
            h.update(ctr.to_le_bytes());
            let mut c = [2u8; 33];
            c[1..].copy_from_slice(&h.finalize());
            if let Ok(p) = PublicKey::from_slice(&c) { return p; }
        }
        unreachable!()
    }
    fn add_pt(&mut self, p: &PublicKey) {
        self.0 = match self.0 { None => Some(*p), Some(a) => a.combine(p).ok() };
    }
    pub(crate) fn add(&mut self, leaf: &[u8; 32]) { self.add_pt(&Self::point(leaf)); }
    pub(crate) fn sub(&mut self, leaf: &[u8; 32]) {
        let ctx = Secp256k1::verification_only();
        self.add_pt(&Self::point(leaf).negate(&ctx));
    }
    pub(crate) fn to_state(&self) -> (u32, [u32; 16]) {
        let Some(p) = self.0 else { return (0, [0; 16]) };
        let u = p.serialize_uncompressed();
        let limbs = |b: &[u8]| -> [u32; 8] { core::array::from_fn(|k| u32::from_be_bytes(b[28 - 4 * k..32 - 4 * k].try_into().unwrap())) };
        let mut w = [0u32; 16];
        w[..8].copy_from_slice(&limbs(&u[1..33]));
        w[8..].copy_from_slice(&limbs(&u[33..65]));
        (1, w)
    }
}

fn net() -> &'static libzync::net::Net {
    match methods::NETWORK { "signet" => &libzync::net::SIGNET, "regtest" => &libzync::net::REGTEST, _ => &libzync::net::MAINNET }
}

/// Cumulative chainwork + the work of one block at `bits` (Core's GetBlockProof), as the guest's add_work: LE 256-bit.
pub(crate) fn add_work(cum: &[u8; 32], bits: u32) -> [u8; 32] {
    let t = bitcoin::Target::from_compact(bitcoin::CompactTarget::from_consensus(bits));
    let w = if t == bitcoin::Target::ZERO { bitcoin::Work::from_le_bytes([0; 32]) } else { t.to_work() };
    (bitcoin::Work::from_le_bytes(*cum) + w).to_le_bytes()
}

/// One block: the next chain state from `prev` and the block file, advancing the host's coin log (lz::CL), trie store
/// (trie::TS), BIP30 tree (`smt`) and live-coin sum (`ecmh`).
/// One block's leaf bundle (ROADMAP H-B 8): every input a leaf proof of block h needs besides the block itself, cut by
/// libzync-prep from its stores at S(h-1). A leaf prover (`libzync-prove-range h h`, HAZYNC_LEAF_BUNDLE=<file>) then
/// needs no coin log, trie or BIP30 tree; the guest checks every witness against `prev`, so a wrong bundle is refused,
/// never proved. File: LEAF_MAGIC, bincode of this, SHA-256 of both (a torn write is refused on read).
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct LeafBundle {
    pub(crate) prev: ChainState,
    /// the BIP30 witness, as lz_smt_rebind cuts it against the running tree
    pub(crate) in_smt_root: [u8; 32],
    pub(crate) smt: crate::SmtBlockWitness,
    /// the coin-log witness (cl_prepare) and the trie witness (TrieStore::witness)
    pub(crate) cl_wit: Vec<u32>,
    pub(crate) trie_wit: Vec<u32>,
    /// block h's JSON (as read_block_json parses it), so a bundle is the leaf prover's ONLY input (the coordinator
    /// serves it as the block's witness, as the original bridge's bundle_<n>.json)
    pub(crate) block_json: Vec<u8>,
}
pub(crate) const LEAF_MAGIC: &[u8; 8] = b"LZLEAF02";

pub(crate) fn leaf_write(path: &str, b: &LeafBundle) {
    let mut v = LEAF_MAGIC.to_vec();
    v.extend(bincode::serialize(b).unwrap());
    let d: [u8; 32] = Sha256::digest(&v).into();
    v.extend_from_slice(&d);
    // tmp, synced, renamed (review 11-12, #9): "a lost bundle is cut again" holds only ABOVE the save point; after a
    // power loss a bundle at or below a just-written save point could be empty and never rewritten, and the frontier
    // stalls on it. The directory is synced once per save point (below), not per block.
    use std::io::Write as _;
    let tmp = format!("{path}.tmp");
    std::fs::File::create(&tmp).and_then(|mut f| { f.write_all(&v)?; f.sync_data() }).unwrap_or_else(|e| panic!("{tmp}: {e}"));
    std::fs::rename(&tmp, path).unwrap_or_else(|e| panic!("{path}: {e}"));
}

pub(crate) fn leaf_read(path: &str) -> LeafBundle {
    let v = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    assert!(v.len() >= 40 && &v[..8] == LEAF_MAGIC, "{path}: not a leaf bundle ({})", String::from_utf8_lossy(&LEAF_MAGIC[..]));
    let (body, d) = v.split_at(v.len() - 32);
    assert!(Sha256::digest(body)[..] == d[..], "{path}: checksum mismatch (torn or damaged)");
    // exactly leaf_write's bincode: trailing bytes refused (the bundle is network-served; the checksum covers them too)
    let mut b: LeafBundle = crate::verify::decode_exact(&body[8..]).unwrap_or_else(|e| panic!("{path}: {e}"));
    // stamped on read (docs/HANDOVER.md §5): a bundle an older image's prep wrote still carries S(h-1) as the image
    // proving block h-1 commits it. Identity with an empty lineage.
    crate::stamp(&mut b.prev);
    b
}

/// One block of native prep: the state after it, and with `leaf` its leaf bundle, the witnesses cut from the stores
/// BEFORE they apply the block.
/// Seconds spent per phase since the last PREP_PROGRESS line: prefetch, coin log apply, trie apply, rest of the block,
/// reading the block, writing the state, save points (PREP_TIMING; the rate is spent somewhere, and a guess cost a day).
static PHASES: std::sync::Mutex<[f64; 7]> = std::sync::Mutex::new([0.0; 7]);
/// The last block's coin-log changes (flips, appends), for history.bin when HAZYNC_PREP_HISTORY=1.
static LAST_HIST: std::sync::Mutex<Option<(Vec<(u64, u32, u32)>, Vec<[u8; 32]>, Option<[u8; 32]>)>> = std::sync::Mutex::new(None);

/// A block file's undo hash (docs/libzync/UNDO.md) and its number of inputs: every input of every non-coinbase
/// transaction, block order, from its prevout (amount, script) and coin_height / coin_is_coinbase (bit 0). The guest's
/// aggregate computes the same from its chunks and logs it (`undo: block <h> inputs <n> hash <hex>`).
pub(crate) fn undo_of_block_json(j: &serde_json::Value) -> ([u8; 32], u64) {
    let mut u = libzync::undo::UndoHasher::new();
    for t in j["txs"].as_array().expect("block: txs") {
        for p in t["prevouts"].as_array().expect("block: prevouts") {
            u.coin(p["coin_height"].as_u64().expect("block: coin_height") as u32,
                   p["coin_is_coinbase"].as_u64().expect("block: coin_is_coinbase") & 1 == 1,
                   p["value"].as_u64().expect("block: prevout value"), &crate::hx(p["spk"].as_str().expect("block: prevout spk")));
        }
    }
    let n = u.inputs();
    (u.finish(), n)
}

/// The forest marker of a saved state (`state_<h>.forest` beside `state_<h>.json`): its block forest has the leaves with
/// the undo-data commitment. The new prep writes it at every save point, `libzync-reforest` for the state it carried
/// over; prep continues (HAZYNC_PREP_CONTINUE=1) from a state at a height whose leaf has the commitment only with it, so
/// a run made before the commitment cannot be continued by mistake (review 11-14, round 2).
pub(crate) const FOREST_MARK: &str = "U0\n";

fn forest_mark_path(state_json: &str) -> String {
    format!("{}.forest", state_json.strip_suffix(".json").unwrap_or(state_json))
}

pub(crate) fn write_forest_mark(state_json: &str) {
    crate::lz::write_atomic(&forest_mark_path(state_json), FOREST_MARK.as_bytes());
}

/// Block `height`'s leaf: with its undo hash when the image proving it has the commitment (every block of an image
/// with an empty lineage but 0; below a pre-undo predecessor's first_height, that image's 0x02 leaf).
pub(crate) fn block_leaf_host(tip_hash: &[u8; 32], height: u32, state: &[u8; 32], undo: &[u8; 32]) -> ([u8; 32], bool) {
    let with = crate::handover_pin::leaf_has_undo(&crate::METHOD_ID, crate::pinned_lineage(), height);
    let l = if with { Sha256::digest(crate::handover_pin::block_leaf_undo_preimage(tip_hash, height, state, undo)).into() }
            else { Sha256::digest(crate::handover_pin::block_leaf_preimage(tip_hash, height, state)).into() };
    (l, with)
}

fn phase(i: usize, t: std::time::Instant) { PHASES.lock().unwrap()[i] += t.elapsed().as_secs_f64(); }

pub(crate) fn prep_block(prev: &ChainState, j: &serde_json::Value, smt: &mut Smt, ecmh: &mut Ecmh, leaf: bool) -> (ChainState, Option<LeafBundle>) {
    use crate::{arr, build_header_v, hx, median_u32, normalize_host, out_leaf_of, out_spendable, rev};
    let height = j["height"].as_u64().unwrap() as u32;
    // The BIP30-tracked flags as the guest derives them (tracked.rs): the fetcher's files carry none. Everything below
    // reads them from the annotated JSON, and a leaf bundle carries that JSON, so the leaf prover's witness has them too.
    let grandfathered = methods::NETWORK == "mainnet" && (height == 91_842 || height == 91_880);
    let (annotated, tracked) = crate::tracked::annotate(j, smt, net().bip34_height, grandfathered, &|_, _, l| crate::lz::cl_has(l));
    // every tracked transaction, for a diff against reproduce/libzync-tracked.json (173 on mainnet to 974,136)
    for t in &tracked { let mut d = *t; d.reverse(); println!("PREP_TRACKED h={height} txid={}", crate::hex(&d)); }
    let j = &annotated;
    assert_eq!(height, prev.height + 1, "prep: block {height} after state {}", prev.height);
    let bits = j["bits"].as_u64().unwrap() as u32;
    let time = j["time"].as_u64().unwrap() as u32;
    let header = build_header_v(j["version"].as_i64().unwrap_or(1) as i32, j["prev"].as_str().unwrap(),
                                &arr(rev(hx(j["merkle"].as_str().unwrap()))), time, bits, j["nonce"].as_u64().unwrap() as u32);
    let tip_hash = crate::header_hash(&header);
    assert!(header[4..36] == prev.tip_hash[..], "prep: block {height} does not extend the state's tip");
    let coinbase: Transaction = deserialize(&hx(j["coinbase_hex"].as_str().unwrap())).unwrap();
    let cb_txid = coinbase.compute_txid().to_byte_array();
    let cb_flags = j["coinbase_flags"].as_u64().unwrap_or(1) as u32;
    let cmtp = median_u32(&prev.recent_times);
    struct Ptx { tx: Transaction, txid: [u8; 32], prevouts: Vec<TxOut>, meta: Vec<(u32, u32, u32)> }
    let ptxs: Vec<Ptx> = j["txs"].as_array().unwrap().iter().map(|t| {
        let tx: Transaction = deserialize(&hx(t["raw"].as_str().unwrap())).unwrap();
        let txid = tx.compute_txid().to_byte_array();
        let (mut prevouts, mut meta) = (Vec::new(), Vec::new());
        for p in t["prevouts"].as_array().unwrap() {
            prevouts.push(TxOut { value: bitcoin::Amount::from_sat(p["value"].as_u64().unwrap()),
                                  script_pubkey: bitcoin::ScriptBuf::from_bytes(hx(p["spk"].as_str().unwrap())) });
            meta.push((p["coin_height"].as_u64().expect("prep: coin_height") as u32,
                       p["coin_is_coinbase"].as_u64().expect("prep: coin_is_coinbase") as u32,
                       p["coin_mtp"].as_u64().expect("prep: coin_mtp") as u32));
        }
        Ptx { tx, txid, prevouts, meta }
    }).collect();
    let tx_flags = |p: &Ptx| -> u32 { (!p.meta.is_empty() && p.meta.iter().all(|m| m.1 & 2 != 0)) as u32 * 2 };

    // Created coins, in the guest's output_leaves order: (outpoint, leaf, trie key, value).
    let okey = |txid: &[u8; 32], v: u32| -> [u8; 36] { let mut o = [0u8; 36]; o[..32].copy_from_slice(txid); o[32..].copy_from_slice(&v.to_le_bytes()); o };
    struct Out { op: [u8; 36], leaf: [u8; 32], key: libzync::ttree::Key, value: u64 }
    let mut outs: Vec<Out> = Vec::new();
    let unspendable_cb = net().is_bip30_unspendable(height, &tip_hash);
    let push_outs = |tx: &Transaction, txid: &[u8; 32], flags: u32, outs: &mut Vec<Out>| {
        for (v, o) in tx.output.iter().enumerate() {
            if !out_spendable(o.script_pubkey.as_bytes()) { continue; }
            outs.push(Out { op: okey(txid, v as u32), leaf: out_leaf_of(tx, txid, v, height, flags, cmtp),
                            key: libzync::trie_fact::coin_key(o.script_pubkey.as_bytes(), txid, v as u32), value: o.value.to_sat() });
        }
    };
    if !unspendable_cb { push_outs(&coinbase, &cb_txid, cb_flags, &mut outs); }
    for p in &ptxs { push_outs(&p.tx, &p.txid, tx_flags(p), &mut outs); }
    let mut first: std::collections::HashMap<[u8; 36], usize> = std::collections::HashMap::new();
    for (k, o) in outs.iter().enumerate() { first.entry(o.op).or_insert(k); }

    // Spends, in block order: in-block (a created outpoint) or external (a coin-log flip, a trie mark, ECMH -).
    let spender = |t: u32, v: u32| libzync::coinlog::w_of(&libzync::coinlog::spender(height, t, v));
    let mut in_sp: Vec<Option<(u32, u32)>> = vec![None; outs.len()];
    let (mut ext, mut marks): (Vec<crate::lz::ClSpend>, crate::trie::Marks) = (Vec::new(), Vec::new());
    let (mut spent_value, mut cb_spends, mut spend_tracked) = (0u64, Vec::new(), Vec::new());
    for (ti, p) in ptxs.iter().enumerate() {
        let txp = ti as u32 + 1;
        for (vin, inp) in p.tx.input.iter().enumerate() {
            let op = inp.previous_output;
            let ptxid = op.txid.to_byte_array();
            let (ch, cb, mtp) = p.meta[vin];
            // every spend of a BIP30-TRACKED coin or a coinbase coin moves the BIP30 tree (the guest's cb_spends: flag != 0)
            if cb != 0 { cb_spends.push(ptxid); spend_tracked.push(cb & 2 != 0); }
            if let Some(&k) = first.get(&okey(&ptxid, op.vout)) {
                in_sp[k].get_or_insert((txp, vin as u32));
                continue;
            }
            let o = &p.prevouts[vin];
            let leaf = hazync_utreexo::coin_leaf_flags(&ptxid, op.vout, o.value.to_sat(), o.script_pubkey.as_bytes(), ch, cb as u8, mtp);
            ext.push((leaf, txp, vin as u32));
            marks.push((libzync::trie_fact::coin_key(o.script_pubkey.as_bytes(), &ptxid, op.vout), spender(txp, vin as u32)));
            spent_value += o.value.to_sat();
            ecmh.sub(&leaf);
        }
    }
    let (mut appends, mut survivor_value, mut ins): (Vec<[u8; 32]>, u64, crate::trie::Inserts) = (Vec::new(), 0, Vec::new());
    for (k, o) in outs.iter().enumerate() {
        if first[&o.op] != k { continue; }
        ins.push((o.key, in_sp[k].map(|(t, v)| spender(t, v)), o.value));
        if in_sp[k].is_some() { continue; }
        appends.push(o.leaf);
        survivor_value += o.value;
        ecmh.add(&o.leaf);
    }

    // The leaf bundle's coin-log and trie witnesses: the stores at S(h-1), before this block changes them.
    let wits = leaf.then(|| (crate::lz::cl_prepare(&ext), crate::trie::trie_witness(height, &marks, &ins, prev)));
    // Disk stores at mainnet size are far larger than RAM: read ahead what the updates below will read, in parallel
    // (the storage serves ~15x more random reads in parallel than one at a time; HAZYNC_PREP_PREFETCH=0 turns it off)
    if std::env::var("HAZYNC_PREP_PREFETCH").as_deref() != Ok("0") {
        let t = std::time::Instant::now();
        crate::lz::cl_prefetch(&ext);
        if let Some(ts) = crate::trie::TS.lock().unwrap().as_ref() {
            let n = ts.prefetch(&marks, &ins);
            // the slots the update will write into (HAZYNC_PREP_PREFETCH_FREE=0 turns it off): twice the path, as a
            // margin (an insert can split a leaf into a chain)
            if std::env::var("HAZYNC_PREP_PREFETCH_FREE").as_deref() != Ok("0") { ts.prefetch_free(2 * n); }
        }
        phase(0, t);
    }
    // The coin log and the trie (the host's stores), the BIP30 tree.
    let t = std::time::Instant::now();
    let (utxo_roots, utxo_leaves, flips, cl_undo) = crate::lz::cl_apply(&ext, &appends, height);
    let undo = undo_of_block_json(j).0;
    let mut leaf_undo = None;
    phase(1, t);
    let t = std::time::Instant::now();
    let (trie_root, trie_replaced) = {
        let mut g = crate::trie::TS.lock().unwrap();
        let ts = g.as_mut().expect("trie store not initialised");
        let r = ts.apply(&marks, &ins).unwrap_or_else(|e| panic!("prep: block {height}: the trie store refused the update: {e:?}"));
        ts.height = height as u64;
        ts.hash = tip_hash;
        // Z1 (docs/libzync/ZAPP_FOLLOWER.md §4): this block's root in trie/roots.log, for a fact reader at this height
        ts.log_root();
        (libzync::coinlog::h_of(&r), ts.last_apply_replaced() == Some(true))
    };
    phase(2, t);
    let tracked_new: Vec<([u8; 32], u32)> = ptxs.iter().filter(|p| tx_flags(p) != 0)
        .map(|p| (p.txid, p.tx.output.iter().filter(|o| out_spendable(o.script_pubkey.as_bytes())).count() as u32))
        .filter(|t| t.1 > 0).collect();
    let in_smt_root = smt.root();
    let smt_w = crate::smt_advance_ex(smt, cb_txid, crate::cb_spendable_outputs(&coinbase), &cb_spends, grandfathered, &spend_tracked, &tracked_new);
    // What undoes this block in each store (REORG.md §6, P1): lz_prep_cmd writes it as <out>/reorg/u_<h>.bin.
    *crate::reorg::LAST.lock().unwrap() = Some(crate::reorg::UndoRecord {
        height, hash: tip_hash, prev: prev.tip_hash, cl: cl_undo, trie_replaced,
        trie_marks: marks.iter().map(|m| m.0).collect(), trie_inserts: ins.iter().map(|i| i.0).collect(),
        smt_prior: smt_w.1.clone(),
    });

    let utxo_value = prev.utxo_value.saturating_add(survivor_value).checked_sub(spent_value)
        .unwrap_or_else(|| panic!("prep: block {height} spends more than the log holds"));
    let utxo_roots = normalize_host(utxo_roots);
    let (block_roots, block_leaves) = {
        let mut f = hazync_utreexo::Stump::new();
        f.roots = prev.block_roots.clone();
        f.num_leaves = prev.block_leaves;
        let mut st: Vec<u8> = Vec::new();
        crate::handover_pin::utxo_state_preimage(utxo_leaves, utxo_value, &utxo_roots, &mut |b: &[u8]| st.extend_from_slice(b));
        let s: [u8; 32] = Sha256::digest(&st).into();
        let (l, with) = block_leaf_host(&tip_hash, height, &s, &undo);
        if with { leaf_undo = Some(undo); }
        f.add(l);
        (normalize_host(f.roots), f.num_leaves)
    };
    *LAST_HIST.lock().unwrap() = Some((flips, appends.clone(), leaf_undo));
    let mut recent_times = prev.recent_times.clone();
    recent_times.push(time);
    if recent_times.len() > 11 { let e = recent_times.len() - 11; recent_times.drain(0..e); }
    let (live_set, live_hash) = ecmh.to_state();
    let bundle = wits.map(|(cl_wit, trie_wit)| LeafBundle { prev: prev.clone(), in_smt_root, smt: smt_w.0, cl_wit, trie_wit,
                                                                     block_json: serde_json::to_vec(j).unwrap() });
    (ChainState {
        tip_hash, utxo_roots, utxo_leaves, utxo_value,
        cum_work: add_work(&prev.cum_work, bits), height, prev_nbits: bits, prev_time: time,
        epoch_start: if height % 2016 == 0 { time } else { prev.epoch_start }, recent_times,
        live_set, live_hash, trie_root, smt_root: smt.root(),
        max_time: prev.max_time.max(time), block_roots, block_leaves,
        ..prev.clone()
    }, bundle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_bundles_from_the_handover_height() {
        // REORG.md §19 D1: the tip run cuts a bundle for every block from X on, plus its sample heights; neither list:
        // every block (the board pass)
        let at: std::collections::BTreeSet<u32> = [170, 1000].into_iter().collect();
        assert!(wants_leaf(5, None, None) && wants_leaf(974_937, None, None));
        assert!(wants_leaf(170, Some(&at), Some(974_937)) && !wants_leaf(171, Some(&at), Some(974_937)));
        assert!(!wants_leaf(974_936, Some(&at), Some(974_937)) && wants_leaf(974_937, Some(&at), Some(974_937)));
        assert!(wants_leaf(980_000, None, Some(974_937)) && !wants_leaf(974_936, None, Some(974_937)));
        assert!(wants_leaf(1000, Some(&at), None) && !wants_leaf(1001, Some(&at), None));
    }

    #[test]
    fn genesis_block_work_and_ecmh_identity() {
        // block 1's bits: 0x1d00ffff, work 0x100010001
        let w = add_work(&[0; 32], 0x1d00ffff);
        assert_eq!(u64::from_le_bytes(w[..8].try_into().unwrap()), 0x1_0001_0001);
        let mut e = Ecmh(None);
        let l = [7u8; 32];
        e.add(&l);
        let (s, p) = e.to_state();
        assert_eq!(s, 1);
        let mut e2 = Ecmh::from_state(s, &p);
        e2.sub(&l);
        assert_eq!(e2.to_state(), (0, [0; 16]), "add then subtract is the identity");
    }
}

/// `hi` of a run that FOLLOWS the chain (`libzync-prep <lo> follow ...`, REORG.md §3 D1, the tip run): no last block.
pub(crate) const FOLLOW: u32 = u32::MAX;

/// Which blocks get a leaf bundle (with HAZYNC_PREP_LEAF_BUNDLES set): every block when neither list is given; else the
/// heights in HAZYNC_PREP_LEAF_AT (a sample) and every block from HAZYNC_PREP_LEAF_FROM on (the tip run's bundles above the
/// D1 handover height X, REORG.md §19: the board pass serves the blocks below X, the tip run every block from X).
pub(crate) fn wants_leaf(h: u32, leaf_at: Option<&std::collections::BTreeSet<u32>>, leaf_from: Option<u32>) -> bool {
    match (leaf_at, leaf_from) {
        (None, None) => true,
        (at, from) => at.is_some_and(|s| s.contains(&h)) || from.is_some_and(|f| h >= f),
    }
}

/// `libzync-prep <lo> <hi> <json dir> <anchor.json | genesis> <out dir>`: the native prep of blocks lo..=hi from the
/// anchor (the state at lo-1; its .coinlog / .trie beside it, or genesis), writing `state_<h>.json` per block and the
/// coin log and trie at hi (every height with HAZYNC_COINLOG_SAVE_EACH=1, as prove-range: the anchors of leaf proofs).
/// HAZYNC_PREP_CHECK=<run dir>: each state's journal must be byte-identical to that run's `state_<h>.json` (a proved
/// chain); the first difference is named and the command fails.
pub(crate) fn lz_prep_cmd(lo: u32, hi: u32, dir: &str, anchor_path: &str, out: &str) {
    assert!(methods::COINLOG && methods::TRIE, "libzync-prep needs the product image (coin log + trie)");
    std::fs::create_dir_all(out).unwrap();
    crate::lz::lock_run_dirs(out);
    // a reorg marker from the fetcher (REORG.md §4): nothing is applied, nothing is opened, until a person removes it
    if let Some(m) = crate::reorg::marker(dir) { crate::reorg::stop_or_rewind(dir, &format!("at the start: {} is there", m.display())); }
    let mut prev: ChainState = if anchor_path == "genesis" { crate::genesis_anchor() } else {
        serde_json::from_str(&std::fs::read_to_string(anchor_path).unwrap_or_else(|e| panic!("{anchor_path}: {e}"))).unwrap()
    };
    crate::demo_block_forest(&mut prev, true);
    assert_eq!(prev.height + 1, lo, "anchor is at height {}, range starts at {lo}", prev.height);
    // decided BEFORE any store is opened: loading from an anchor can already undo journaled blocks (round 12)
    // From a state other than genesis the run must SAY which it is (round 12: either default is wrong for someone):
    // HAZYNC_PREP_CONTINUE=1, the same chain carries on (a restart from a save point: states byte-identical to an
    // uninterrupted run); HAZYNC_PREP_BASE=1, a base step on that state (anchor_id = its link digest, as a leaf proof
    // on it commits, and as prove-range does from an anchor). The choice is printed.
    let cont = match (std::env::var("HAZYNC_PREP_CONTINUE").as_deref(), std::env::var("HAZYNC_PREP_BASE").as_deref()) {
        (Ok("1"), Ok("1")) => panic!("HAZYNC_PREP_CONTINUE and HAZYNC_PREP_BASE both set"),
        (Ok("1"), _) => true,
        (_, Ok("1")) => false,
        _ if anchor_path == "genesis" => false,
        _ => panic!("libzync-prep from the state at {}: set HAZYNC_PREP_CONTINUE=1 (the same chain, e.g. a restart from a save point) or HAZYNC_PREP_BASE=1 (a base step on it, as a leaf proof)", prev.height),
    };

    // The trie FIRST (review 11-12, #3): opening a disk trie checks its height against the anchor and changes nothing,
    // while loading the coin log undoes its journal back to the anchor and deletes the journal. A crash after the trie's
    // flush but before its marker landed left the trie at h and the scripts picking save point h-1000: in the old order
    // the coin log was rewound and its journal gone before the trie refused, and no save point matched any more.
    crate::trie::trie_init_at_start(false, anchor_path, &prev);
    crate::lz::cl_init_at_start(false, anchor_path, &prev);
    // The BIP30 tree at lo-1: from the anchor's snapshot (state_<h>.smt, written at prep's save points) when it is
    // there, so a restart needs no block files below lo (mainnet: the whole chain's JSON would not be kept); else
    // replayed from the block files 1..lo-1. Either way it must give the anchor's smt_root.
    let snap = smt_snapshot_path(anchor_path);
    let (mut smt, replay_max) = match snap.as_deref().map(std::fs::read) {
        Some(Ok(b)) => (smt_from_bytes(&b).unwrap_or_else(|e| panic!("{}: {e}", snap.as_deref().unwrap())), 0),
        _ => crate::lz::lz_smt_replay(dir, lo - 1),
    };
    if prev.max_time == 0 { prev.max_time = replay_max.max(crate::genesis_params().1); }
    assert_eq!(smt.root(), prev.smt_root, "replayed BIP30 tree at {} != the anchor's smt_root", lo - 1);
    let mut ecmh = Ecmh::from_state(prev.live_set, &prev.live_hash);
    // The first block is a base step on the anchor: its anchor_id is the digest of the anchor's journal, and the image
    // stamps its own id and profile; every later block carries them. HAZYNC_PREP_CONTINUE=1: the anchor is a state of
    // the SAME chain (a restart after a crash, from the last saved state_<h>.json), so the chain's own anchor_id carries
    // on and the states stay byte-identical to an uninterrupted run (11-06).
    if anchor_path != "genesis" {
        println!("  prep from the state at {}: {}", prev.height,
                 if cont { "CONTINUING its chain (anchor_id kept)" } else { "a base step on it (anchor_id = its link digest)" });
    }
    let anchor_id = if cont { prev.anchor_id } else { crate::link_digest(&prev) };
    // CONTINUING under a different image (11-08: a run started under the laptop build bbf82800 carried on by a host
    // embedding the published 863c9ce3): the saved state is re-stamped with this image's id and profile, so the
    // first block's leaf bundle carries the same prev a join of leaves proved by this image links to. Prep states
    // are untrusted starting points; a leaf's guest checks every witness against them either way.
    // Stamped BY ERA (docs/HANDOVER.md §5): the image that proves block prev.height under this image's lineage, which
    // is this image unless a handover is pinned above it.
    if cont && prev.height >= 1 && crate::handover_pin::leaf_has_undo(&crate::METHOD_ID, crate::pinned_lineage(), prev.height) {
        let mark = std::fs::read_to_string(forest_mark_path(anchor_path)).unwrap_or_default();
        assert!(mark == FOREST_MARK, "{anchor_path}: no forest marker ({}): its block forest may be from a prep run made before the \
                undo-data commitment, which this host cannot continue. Carry the run over with libzync-reforest first \
                (docs/libzync/UNDO.md §8)", forest_mark_path(anchor_path));
    }
    // a state at or above the first tracked coin continues only if a host that derives the tracked flags wrote it
    if anchor_path != "genesis" { crate::tracked::check_flags_mark(anchor_path, prev.height); }
    if cont {
        let was = (prev.self_id, prev.lineage, prev.profile);
        crate::stamp(&mut prev);
        if was != (prev.self_id, prev.lineage, prev.profile) {
            println!("  prep: the anchor was saved under another image or era; re-stamped (image {}, lineage {})",
                     crate::hex(&risc0_zkvm::sha::Digest::from(prev.self_id).as_bytes().try_into().unwrap_or([0u8; 32])), crate::hex(&prev.lineage));
        }
    }
    let check = std::env::var("HAZYNC_PREP_CHECK").ok();
    let history = std::env::var("HAZYNC_PREP_HISTORY").as_deref() == Ok("1");
    // HAZYNC_PREP_LEAF_BUNDLES=<dir>: also write leaf_<h>.bin per block (ROADMAP H-B 8), for leaf provers without stores
    let leaves = std::env::var("HAZYNC_PREP_LEAF_BUNDLES").ok();
    // HAZYNC_PREP_LEAF_WINDOW=<n>: the backfill's window (ROADMAP H-B, 11-07). Before writing a bundle, wait while the
    // bundle directory already holds n of them: the coordinator's side removes a bundle once its block is proved, so
    // prep runs at most n blocks ahead of the provers and the bundles on disk stay bounded (a whole chain of them would
    // be ~3-5 TB). Unset: no window (the paper's sample heights).
    let leaf_window: Option<usize> = std::env::var("HAZYNC_PREP_LEAF_WINDOW").ok().and_then(|v| v.parse().ok());
    if let Some(d) = &leaves { std::fs::create_dir_all(d).unwrap_or_else(|e| panic!("{d}: {e}")); }
    // HAZYNC_PREP_LEAF_AT=<h,h,...>: bundles only at those heights (a sample: a bundle carries its block's JSON, so
    // one per block for the whole chain would be larger than the chain)
    let leaf_at: Option<std::collections::BTreeSet<u32>> = std::env::var("HAZYNC_PREP_LEAF_AT").ok().map(|v|
        v.split(',').filter(|x| !x.trim().is_empty()).map(|x| x.trim().parse().unwrap_or_else(|_| panic!("HAZYNC_PREP_LEAF_AT: not a height: {x}"))).collect());
    // HAZYNC_PREP_LEAF_FROM=<h>: also a bundle for every block from h on (the tip run, REORG.md §19 D1)
    let leaf_from: Option<u32> = std::env::var("HAZYNC_PREP_LEAF_FROM").ok().filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().parse().unwrap_or_else(|_| panic!("HAZYNC_PREP_LEAF_FROM: not a height: {v}")));
    // `follow` (hi = FOLLOW): the run has no last block; fed by `fetch_block_rpc.py range <lo> follow`, it waits at the
    // node's tip for the next block for as long as it takes (a block can take hours), saying so every 10 minutes
    let follow = hi == FOLLOW;
    if follow { assert!(consume_env(), "libzync-prep <lo> follow: needs HAZYNC_PREP_CONSUME=1 (block files fed by the fetcher)"); }
    // HAZYNC_PREP_MIN_FREE_GB: stop (exit 3, after a save) when the coin log's filesystem has less free (default 50)
    let min_free_gb: u64 = std::env::var("HAZYNC_PREP_MIN_FREE_GB").ok().and_then(|v| v.parse().ok()).unwrap_or(50);
    // HAZYNC_PREP_REPORT_EVERY: one PREP_PROGRESS line per that many blocks (default 1,000): the rate and every
    // store's bytes, so a slice run measures what the whole chain will need
    let report_every: u32 = std::env::var("HAZYNC_PREP_REPORT_EVERY").ok().and_then(|v| v.parse().ok()).unwrap_or(1000).max(1);
    let mut t_rep = std::time::Instant::now();
    let mut h_rep = lo;
    let each = std::env::var("HAZYNC_COINLOG_SAVE_EACH").as_deref() == Ok("1");
    // Disk mode (HAZYNC_COINLOG_DIR): also save every HAZYNC_PREP_SAVE_EVERY blocks (default 1,000), so the journal of
    // blocks since the last save stays bounded and a crash resumes from at most that far back (anchor: the last
    // state_<h>.json saved with its .coinlog / .trie).
    let every: u32 = if crate::lz::cl_dir().is_some() {
        std::env::var("HAZYNC_PREP_SAVE_EVERY").ok().and_then(|v| v.parse().ok()).unwrap_or(1000).max(1)
    } else { 0 };
    // The reorg undo records (REORG.md §6, P1): u_<h>.bin for the last J blocks, J = HAZYNC_PREP_UNDO_KEEP.
    let undo_keep = crate::reorg::keep();
    let undo_dir = crate::reorg::dir(out);
    if undo_keep > 0 { std::fs::create_dir_all(&undo_dir).unwrap_or_else(|e| panic!("{}: {e}", undo_dir.display())); }
    let pruned = crate::reorg::prune_at_start(&undo_dir, lo - 1, undo_keep);
    if pruned > 0 { println!("  prep: removed {pruned} reorg undo record(s) outside ({}, {}]", (lo - 1).saturating_sub(undo_keep), lo - 1); }
    let mut undo_synced = lo - 1;
    // the window is cut only at save points (reorg::write); (pruned_to, ...] is what is on disk
    let mut pruned_to = (lo - 1).saturating_sub(undo_keep);
    // the lowest height whose record exists, for PREP_TIP.json (the fetcher marks a fork the records cannot reach)
    let mut undo_from = crate::reorg::undo_from(&undo_dir, lo - 1, undo_keep);
    let tip_from = |uf: u32, h: u32| if undo_keep > 0 { uf } else { h + 1 };
    // prep's real position (review of P2, M2): the anchor, durably, before any block
    crate::reorg::write_tip(out, lo - 1, &prev.tip_hash, undo_keep, tip_from(undo_from, lo - 1), true);
    let mut last_save = lo - 1;
    let t0 = std::time::Instant::now();
    let consume = consume_env();
    let mut consumed_from = lo;
    let wait_s: u64 = std::env::var("HAZYNC_PREP_WAIT_S").ok().and_then(|v| v.parse().ok()).unwrap_or(3600);
    for h in lo..=hi {
        // Fed by a producer (fetch_block_rpc.py range --window), wait for the block's file to arrive. A file that is
        // there but cannot be read is waited for too (review L3): the fetcher may delete it between our look and our
        // read, to fetch it again from the node's chain. Static files: read as before (a missing file is an error).
        let f = format!("{dir}/block_{h}.json");
        let t = std::time::Instant::now();
        let mut next_say = 600u64;
        let mut j = loop {
            // the fetcher stops at a reorg (REORG.md §4): so does prep, at once, rather than at a timeout
            if let Some(m) = crate::reorg::marker(dir) { crate::reorg::stop_or_rewind(dir, &format!("before block {h}: {} is there", m.display())); }
            if !consume { break crate::read_block_json(dir, h); }
            if let Some(v) = std::fs::read_to_string(&f).ok().and_then(|x| serde_json::from_str::<serde_json::Value>(&x).ok()) { break v; }
            if follow {
                // at the tip the next block is not mined yet: no timeout (a stalled fetcher shows as PREP_TIP.json
                // not moving while the node's tip does: hazync-lz-check-reorg, REORG.md §19)
                if t.elapsed().as_secs() >= next_say {
                    println!("  prep: following the tip: waited {next_say} s for block {h}");
                    next_say += 600;
                }
            } else {
                assert!(t.elapsed().as_secs() < wait_s, "{f}: not there (or not readable) after {wait_s} s (HAZYNC_PREP_WAIT_S); is the producer running?");
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        };
        phase(4, t);
        // A block that does not extend the state's tip (REORG.md §4; this was prep_block's assert, a crash loop under
        // systemd): either the file is from a branch the fetcher has not yet replaced (a fork at or above h-1: it
        // deletes and fetches it again) or the state itself is on an orphaned branch (the fetcher writes a marker).
        // Fed by the fetcher, wait for one or the other; with static block files nothing will change, so stop.
        if crate::reorg::prev_of(&j) != Some(prev.tip_hash) {
            let why = format!("block {h} ({}) does not extend the state at {}", j["prev"], prev.height);
            if !consume { crate::reorg::stop(&format!("{why}: block files from another chain")); }
            println!("  prep: {why}: a reorg? waiting for the fetcher to replace the file or write its marker");
            let t = std::time::Instant::now();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                if let Some(m) = crate::reorg::marker(dir) { crate::reorg::stop_or_rewind(dir, &format!("{why}, and {} is there", m.display())); }
                // a replaced file arrives by rename, never half written; it may be absent for a moment in between
                let again = std::fs::read_to_string(format!("{dir}/block_{h}.json")).ok().and_then(|x| serde_json::from_str::<serde_json::Value>(&x).ok());
                if let Some(x) = again.filter(|x| crate::reorg::prev_of(x) == Some(prev.tip_hash)) {
                    println!("  prep: block {h} was replaced by one that extends the state: carrying on");
                    j = x;
                    break;
                }
                if t.elapsed().as_secs() >= wait_s { crate::reorg::stop(&format!("{why} after {wait_s} s (HAZYNC_PREP_WAIT_S), and the fetcher wrote no marker")); }
            }
        }
        // A state file at h left by an EARLIER run (prep replays from its save point after a restart, and the states
        // above it stay on disk, served by the board) for ANOTHER block: the node reorged inside the replay window, and
        // the fetcher, comparing only prep's tip, cannot see it (review of P4, HIGH-1). A fork at h-1, handled as any
        // other: the marker, then a stop for the rewind (which moves those files aside and tells the board).
        crate::reorg::stale_state_fork(out, dir, h, &prev.tip_hash, &j, consume);
        let want_leaf = leaves.is_some() && wants_leaf(h, leaf_at.as_ref(), leaf_from);
        let t = std::time::Instant::now();
        let inner0 = { let p = PHASES.lock().unwrap(); p[0] + p[1] + p[2] };
        let (mut st, bundle) = prep_block(&prev, &j, &mut smt, &mut ecmh, want_leaf);
        { let mut p = PHASES.lock().unwrap(); let inner = p[0] + p[1] + p[2] - inner0; p[3] += t.elapsed().as_secs_f64() - inner; }
        let undo = crate::reorg::LAST.lock().unwrap().take().expect("prep_block leaves its undo record");
        assert!(undo.height == h && undo.hash == st.tip_hash, "prep: the undo record is not block {h}'s");
        if undo_keep > 0 { crate::reorg::write(&undo_dir, &undo); }
        // the leaf bundle carries prev = S(h-1) as this run commits it (anchor_id, self_id and profile included)
        if let (Some(d), Some(b)) = (&leaves, bundle) {
            if let Some(n) = leaf_window {
                let held = || std::fs::read_dir(d).map(|it| it.flatten().filter(|e| {
                    let f = e.file_name(); let f = f.to_string_lossy(); f.starts_with("leaf_") && f.ends_with(".bin")
                }).count()).unwrap_or(0);
                let mut said = false;
                while held() >= n {
                    if !said { println!("  prep: {n} leaf bundles waiting to be proved (HAZYNC_PREP_LEAF_WINDOW): waiting before block {h}"); said = true; }
                    std::thread::sleep(std::time::Duration::from_secs(5));
                }
            }
            leaf_write(&format!("{d}/leaf_{h}.bin"), &b);
        }
        // test hook: die after block h changed the stores, before any save (the window the journal covers)
        if std::env::var("HAZYNC_CRASH_AT").as_deref() == Ok(&format!("prep-after:{h}")) { std::process::exit(9); }
        // HAZYNC_PREP_HISTORY=1: append each block to <out>/history.bin (libzync::history), as a proving run does, so a
        // path service (zync-serve) can answer coin and trie facts at any save point without proving (ROADMAP O2).
        // Contiguous from genesis or not at all: hist_append writes nothing for a run whose history has a gap.
        if history {
            let (flips, appends, undo) = LAST_HIST.lock().unwrap().take().expect("prep_block records its changes");
            if h == 1 && anchor_path == "genesis" {
                crate::lz::hist_append(out, &libzync::history::Rec { height: 0, hash: crate::genesis_anchor().tip_hash, value: 0, flips: vec![], appends: vec![], undo: None });
            }
            crate::lz::hist_append(out, &libzync::history::Rec { height: h, hash: st.tip_hash, value: st.utxo_value, flips, appends, undo });
        }
        if h == lo { st.anchor_id = anchor_id; }
        // the image proving block h, and the lineage at h (identity with an empty lineage: METHOD_ID, zero)
        crate::stamp(&mut st);
        // atomic and synced at a save point (its state_<h>.json is the anchor a restart takes; round 9); between them
        // atomic by a rename but not synced per block (two fsyncs per block across a mainnet prep cost; round 10):
        // they are synced at the next save point instead (review of P2, M4: the fetcher compares them with the node,
        // and a torn one read as a disagreement). The bytes are the same either way.
        let save = each || h == hi || (every > 0 && h % every == 0);
        // the leaf bundles' renames since the last save point made durable before the save point that covers them
        if save { if let Some(d) = &leaves { crate::lz::cl_sync_dir(std::path::Path::new(d)); } }
        // and the undo records up to h: every record at or below a complete save point survives a power loss
        if save && undo_keep > 0 {
            if let Some(m) = crate::reorg::sync_since(&undo_dir, undo_synced, h) {
                println!("  prep: the reorg undo record of block {m} is missing: the records now reach only from {}", m + 1);
                undo_from = undo_from.max(m + 1);
            }
            undo_synced = h;
        }
        // and the state files since the last save point (written by rename, not yet synced)
        if save {
            for k in last_save + 1..h {
                if let Ok(f) = std::fs::File::open(format!("{out}/state_{k}.json")) { f.sync_data().unwrap_or_else(|e| panic!("{out}/state_{k}.json: {e}")); }
            }
        }
        let t = std::time::Instant::now();
        let json = serde_json::to_string(&st).unwrap();
        if save { crate::lz::write_atomic(&format!("{out}/state_{h}.json"), json.as_bytes()); }
        else {
            let p = format!("{out}/state_{h}.json");
            std::fs::write(format!("{p}.tmp"), json).unwrap_or_else(|e| panic!("{p}.tmp: {e}"));
            std::fs::rename(format!("{p}.tmp"), &p).unwrap_or_else(|e| panic!("{p}: {e}"));
        }
        phase(5, t);
        let t = std::time::Instant::now();
        if save {
            save_stores(out, h, &smt);
            // HAZYNC_PREP_CONSUME=1: the block files this save point covers are no longer needed (a restart starts
            // here, its BIP30 tree from state_<h>.smt), so a whole-chain prep fed by fetch_block_rpc.py range
            // --window keeps only a window of blocks on disk
            if consume {
                for k in consumed_from..=h { let _ = std::fs::remove_file(format!("{dir}/block_{k}.json")); }
                consumed_from = h + 1;
            }
            // the undo window cut here, never per block (review of P1, M3): (h - J, ...] stays
            if undo_keep > 0 {
                let to = h.saturating_sub(undo_keep);
                crate::reorg::prune_to(&undo_dir, pruned_to, to);
                pruned_to = pruned_to.max(to);
                undo_from = undo_from.max(to + 1);
            }
            last_save = h;
        }
        // prep's position for the fetcher: block h is in its stores (durable at a save point)
        crate::reorg::write_tip(out, h, &st.tip_hash, undo_keep, tip_from(undo_from, h), save);
        phase(6, t);
        // the check covers the heights the reference run has (a proved range: a slice run goes past it)
        let want_path = check.as_ref().map(|c| format!("{c}/state_{h}.json")).filter(|p| std::path::Path::new(p).exists());
        if let Some(wp) = &want_path {
            let want: ChainState = serde_json::from_str(&std::fs::read_to_string(wp).unwrap_or_else(|e| panic!("{wp}: {e}"))).unwrap();
            if crate::state_journal_bytes(&st) != crate::state_journal_bytes(&want) {
                let (a, b) = (serde_json::to_value(&st).unwrap(), serde_json::to_value(&want).unwrap());
                let diff: Vec<&String> = a.as_object().unwrap().keys().filter(|k| a[k.as_str()] != b[k.as_str()]).collect();
                eprintln!("PREP_MISMATCH at {h}: fields {diff:?}");
                for k in &diff { eprintln!("  {k}: prep {} | proved {}", a[k.as_str()], b[k.as_str()]); }
                std::process::exit(1);
            }
        }
        if h % report_every == 0 || h == hi {
            let dt = t_rep.elapsed().as_secs_f64();
            let mb = |d: Option<std::path::PathBuf>| d.map_or(0, |d| dir_bytes(&d)) as f64 / 1e6;
            // trie_nodes: nodes in use (x 64 B = the trie's real size; trie_mb also counts free slots and the growth step)
            let trie_nodes = crate::trie::TS.lock().unwrap().as_ref().map_or(0, |t| t.nodes());
            println!("PREP_PROGRESS h={h} blocks_per_s={:.2} coinlog_leaves={} coinlog_mb={:.0} trie_mb={:.0} trie_nodes={trie_nodes} out_mb={:.0} rss_mb={} elapsed_s={:.0}",
                     (h - h_rep + 1) as f64 / dt.max(1e-9), st.utxo_leaves, mb(crate::lz::cl_dir()), mb(crate::trie::trie_dir()),
                     mb(Some(out.into())), rss_mb(), t0.elapsed().as_secs_f64());
            let p = std::mem::replace(&mut *PHASES.lock().unwrap(), [0.0; 7]);
            let (ch, cm) = crate::trie::TS.lock().unwrap().as_ref().map_or((0, 0), |t| t.cache_stats());
            println!("PREP_TIMING h={h} s: prefetch={:.0} coinlog={:.0} trie={:.0} rest={:.0} read_block={:.0} write_state={:.0} save={:.0} of {:.0}; trie cache hits={ch} misses={cm}",
                     p[0], p[1], p[2], p[3], p[4], p[5], p[6], dt);
            t_rep = std::time::Instant::now();
            h_rep = h + 1;
        }
        // the free-space floor, checked at a save point (a stop there needs no undo on restart)
        if save && h != hi {
            if let Some(gb) = crate::lz::cl_dir().and_then(|d| free_gb(&d)) {
                if gb < min_free_gb {
                    println!("PREP_STOPPED_LOW_DISK at the save point {h}: {gb} GB free < HAZYNC_PREP_MIN_FREE_GB={min_free_gb}; restart from state_{h}.json with HAZYNC_PREP_CONTINUE=1");
                    std::process::exit(3);
                }
            }
        }
        prev = st;
    }
    println!("LZ_PREP_DONE {lo}..={hi} in {:.1} s{}", t0.elapsed().as_secs_f64(),
             if check.is_some() { "; every state's journal byte-identical to the proved run" } else { "" });
}

fn consume_env() -> bool { std::env::var("HAZYNC_PREP_CONSUME").as_deref() == Ok("1") }

/// A save point at h, after its state_<h>.json is written: the forest and tracked-flags markers, the BIP30 tree's
/// snapshot, the coin log's and the trie's markers (each store synced), then the journal goes. Shared by prep's loop
/// and the reorg rewind (rewind.rs), which ends with a save point at the fork.
pub(crate) fn save_stores(out: &str, h: u32, smt: &Smt) {
    // right after the state, before the stores: every save point the runner takes as complete has them
    if crate::handover_pin::leaf_has_undo(&crate::METHOD_ID, crate::pinned_lineage(), h) { write_forest_mark(&format!("{out}/state_{h}.json")); }
    crate::lz::write_atomic(&crate::tracked::flags_mark_path(&format!("{out}/state_{h}.json")), crate::tracked::FLAGS_MARK.as_bytes());
    crate::lz::write_atomic(&format!("{out}/state_{h}.smt"), &smt_to_bytes(smt));
    crate::lz::cl_save(&format!("{out}/state_{h}.coinlog"), h);
    crate::trie::trie_save(&format!("{out}/state_{h}.trie"));
    // both stores saved at h: the journal of the blocks since the last save can go (crash before: a resume
    // from the last saved state undoes them; the disk trie falls back to its last flush by itself)
    crate::lz::cl_journal_commit();
}

/// Bytes in a directory's files (one level: the stores keep their files flat).
fn dir_bytes(d: &std::path::Path) -> u64 {
    std::fs::read_dir(d).map(|it| it.flatten().filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| {
        // allocated bytes, not the apparent length (a sparse index or arena); the length off Unix
        #[cfg(unix)]
        { use std::os::unix::fs::MetadataExt; m.blocks() * 512 }
        #[cfg(not(unix))]
        { m.len() }
    }).sum()).unwrap_or(0)
}

/// This process's resident memory, MB (Linux).
fn rss_mb() -> u64 {
    std::fs::read_to_string("/proc/self/status").ok().and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:"))
        .and_then(|l| l.split_whitespace().nth(1)).and_then(|k| k.parse::<u64>().ok())).map_or(0, |k| k / 1024)
}

/// Free space on the filesystem holding `d`, GB (df; None if it cannot be read).
fn free_gb(d: &std::path::Path) -> Option<u64> {
    let o = std::process::Command::new("df").arg("-k").arg("--output=avail").arg(d).output().ok()?;
    String::from_utf8_lossy(&o.stdout).lines().nth(1)?.trim().parse::<u64>().ok().map(|k| k / 1_000_000)
}

/// The BIP30 tree's snapshot beside a state file (state_<h>.json -> state_<h>.smt); None for genesis.
fn smt_snapshot_path(anchor_path: &str) -> Option<String> {
    anchor_path.strip_suffix(".json").map(|s| format!("{s}.smt"))
}

/// "LZSMT001", the leaf count (u64 LE), then each leaf: key (32 bytes) and value (u32 LE), in key order.
fn smt_to_bytes(smt: &Smt) -> Vec<u8> {
    let mut b = b"LZSMT001".to_vec();
    b.extend_from_slice(&(smt.len() as u64).to_le_bytes());
    for (k, v) in smt.leaves() { b.extend_from_slice(k); b.extend_from_slice(&v.to_le_bytes()); }
    b
}

pub(crate) fn smt_from_bytes(b: &[u8]) -> Result<Smt, String> {
    if b.len() < 16 || &b[..8] != b"LZSMT001" { return Err("not a BIP30 tree snapshot".into()); }
    let n = u64::from_le_bytes(b[8..16].try_into().unwrap()) as usize;
    if b.len() != 16 + 36 * n { return Err(format!("{} bytes for {n} leaves", b.len())); }
    let leaves = b[16..].chunks_exact(36).map(|c| (c[..32].try_into().unwrap(), u32::from_le_bytes(c[32..].try_into().unwrap())));
    Ok(Smt::from_leaves(leaves))
}
