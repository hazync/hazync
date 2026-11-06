//! ROADMAP H-B: the NATIVE prep. The chain state after a block, computed on the host from the previous state and the
//! block file, with no guest run and no validation (Core validated the chain; the proofs check everything again). It
//! must be byte-identical to what the product aggregate commits (coin log + counting trie + live-coin hash): block h is
//! then proved as a base step on prep's state at h-1, in any order, and a join refuses any state that is not exactly
//! the chain's (sha256d of the journal), so a prep bug costs time, never soundness.
//!
//! Every field, after the guest (prover/methods/guest/src/main.rs `aggregate`, agg_lite.rs `validate_block_lite`):
//!   - created coins: the coinbase's outputs (none at the BIP30-unspendable 91,722 / 91,812), then every other
//!     transaction's, in block order, unspendable outputs (OP_RETURN, script over 10,000 bytes) skipped; the leaf
//!     binds the parent's MTP and the coinbase / BIP30-tracked flags the block file carries (as the guest's witness);
//!   - a spend of a coin this block creates (same outpoint): in-block, never in the log; any other: external, a flip
//!     in the coin log at the coin's position, spender (height, tx position, input index within the tx);
//!   - appends: created coins not spent in the block; utxo_value += their values - the external coins' values;
//!   - the trie: marks = the external spends (key, spender), inserts = one per created outpoint (key, its in-block
//!     spender if any, value); key = sha256(script) || txid with the vout XORed in;
//!   - the live-coin hash: + each append's point, - each external spend's point (first ctr with x = sha256(leaf||ctr)
//!     on the curve, the even root), committed affine x || y in 8 little-endian limbs each, live_set 0 = the identity;
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
pub(crate) fn prep_block(prev: &ChainState, j: &serde_json::Value, smt: &mut Smt, ecmh: &mut Ecmh) -> ChainState {
    use crate::{arr, build_header_v, hx, median_u32, normalize_host, out_leaf_of, out_spendable, rev};
    let height = j["height"].as_u64().unwrap() as u32;
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

    // The coin log and the trie (the host's stores), the BIP30 tree.
    let (utxo_roots, utxo_leaves) = crate::lz::cl_apply(&ext, &appends, height);
    let trie_root = {
        let mut g = crate::trie::TS.lock().unwrap();
        let ts = g.as_mut().expect("trie store not initialised");
        let r = ts.apply(&marks, &ins).unwrap_or_else(|e| panic!("prep: block {height}: the trie store refused the update: {e:?}"));
        ts.height = height as u64;
        ts.hash = tip_hash;
        libzync::coinlog::h_of(&r)
    };
    let tracked_new: Vec<([u8; 32], u32)> = ptxs.iter().filter(|p| tx_flags(p) != 0)
        .map(|p| (p.txid, p.tx.output.iter().filter(|o| out_spendable(o.script_pubkey.as_bytes())).count() as u32))
        .filter(|t| t.1 > 0).collect();
    let grandfathered = methods::NETWORK == "mainnet" && (height == 91_842 || height == 91_880);
    crate::smt_advance_ex(smt, cb_txid, crate::cb_spendable_outputs(&coinbase), &cb_spends, grandfathered, &spend_tracked, &tracked_new);

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
        f.add(Sha256::digest(crate::handover_pin::block_leaf_preimage(&tip_hash, height, &s)).into());
        (normalize_host(f.roots), f.num_leaves)
    };
    let mut recent_times = prev.recent_times.clone();
    recent_times.push(time);
    if recent_times.len() > 11 { let e = recent_times.len() - 11; recent_times.drain(0..e); }
    let (live_set, live_hash) = ecmh.to_state();
    ChainState {
        tip_hash, utxo_roots, utxo_leaves, utxo_value,
        cum_work: add_work(&prev.cum_work, bits), height, prev_nbits: bits, prev_time: time,
        epoch_start: if height % 2016 == 0 { time } else { prev.epoch_start }, recent_times,
        live_set, live_hash, trie_root, smt_root: smt.root(),
        max_time: prev.max_time.max(time), block_roots, block_leaves,
        ..prev.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

/// `libzync-prep <lo> <hi> <json dir> <anchor.json | genesis> <out dir>`: the native prep of blocks lo..=hi from the
/// anchor (the state at lo-1; its .coinlog / .trie beside it, or genesis), writing `state_<h>.json` per block and the
/// coin log and trie at hi (every height with HAZYNC_COINLOG_SAVE_EACH=1, as prove-range: the anchors of leaf proofs).
/// HAZYNC_PREP_CHECK=<run dir>: each state's journal must be byte-identical to that run's `state_<h>.json` (a proved
/// chain); the first difference is named and the command fails.
pub(crate) fn lz_prep_cmd(lo: u32, hi: u32, dir: &str, anchor_path: &str, out: &str) {
    assert!(methods::LIBZYNC && methods::COINLOG && methods::TRIE, "libzync-prep needs the product image (coin log + trie)");
    std::fs::create_dir_all(out).unwrap();
    let mut prev: ChainState = if anchor_path == "genesis" { crate::genesis_anchor() } else {
        serde_json::from_str(&std::fs::read_to_string(anchor_path).unwrap_or_else(|e| panic!("{anchor_path}: {e}"))).unwrap()
    };
    crate::demo_block_forest(&mut prev, true);
    assert_eq!(prev.height + 1, lo, "anchor is at height {}, range starts at {lo}", prev.height);
    crate::lz::cl_init_at_start(false, anchor_path, &prev);
    crate::trie::trie_init_at_start(false, anchor_path, &prev);
    let (mut smt, replay_max) = crate::lz::lz_smt_replay(dir, lo - 1);
    if prev.max_time == 0 { prev.max_time = replay_max.max(crate::genesis_params().1); }
    assert_eq!(smt.root(), prev.smt_root, "replayed BIP30 tree at {} != the anchor's smt_root", lo - 1);
    let mut ecmh = Ecmh::from_state(prev.live_set, &prev.live_hash);
    // The first block is a base step on the anchor: its anchor_id is the digest of the anchor's journal, and the image
    // stamps its own id and profile; every later block carries them. HAZYNC_PREP_CONTINUE=1: the anchor is a state of
    // the SAME chain (a restart after a crash, from the last saved state_<h>.json), so the chain's own anchor_id carries
    // on and the states stay byte-identical to an uninterrupted run (11-06).
    let anchor_id = if std::env::var("HAZYNC_PREP_CONTINUE").as_deref() == Ok("1") { prev.anchor_id } else { crate::link_digest(&prev) };
    let check = std::env::var("HAZYNC_PREP_CHECK").ok();
    let each = std::env::var("HAZYNC_COINLOG_SAVE_EACH").as_deref() == Ok("1");
    // Disk mode (HAZYNC_COINLOG_DIR): also save every HAZYNC_PREP_SAVE_EVERY blocks (default 1,000), so the journal of
    // blocks since the last save stays bounded and a crash resumes from at most that far back (anchor: the last
    // state_<h>.json saved with its .coinlog / .trie).
    let every: u32 = if crate::lz::cl_dir().is_some() {
        std::env::var("HAZYNC_PREP_SAVE_EVERY").ok().and_then(|v| v.parse().ok()).unwrap_or(1000).max(1)
    } else { 0 };
    let t0 = std::time::Instant::now();
    for h in lo..=hi {
        let j = crate::read_block_json(dir, h);
        let mut st = prep_block(&prev, &j, &mut smt, &mut ecmh);
        // test hook: die after block h changed the stores, before any save (the window the journal covers)
        if std::env::var("HAZYNC_CRASH_AT").as_deref() == Ok(&format!("prep-after:{h}")) { std::process::exit(9); }
        if h == lo { st.anchor_id = anchor_id; }
        st.self_id = methods::METHOD_ID;
        st.profile = crate::lz::expected_profile();
        std::fs::write(format!("{out}/state_{h}.json"), serde_json::to_string(&st).unwrap()).unwrap();
        if each || h == hi || (every > 0 && h % every == 0) {
            crate::lz::cl_save(&format!("{out}/state_{h}.coinlog"), h);
            crate::trie::trie_save(&format!("{out}/state_{h}.trie"));
            // both stores saved at h: the journal of the blocks since the last save can go (crash before: a resume
            // from the last saved state undoes them; the disk trie falls back to its last flush by itself)
            crate::lz::cl_journal_commit();
        }
        if let Some(c) = &check {
            let want: ChainState = serde_json::from_str(&std::fs::read_to_string(format!("{c}/state_{h}.json"))
                .unwrap_or_else(|e| panic!("{c}/state_{h}.json: {e}"))).unwrap();
            if crate::state_journal_bytes(&st) != crate::state_journal_bytes(&want) {
                let (a, b) = (serde_json::to_value(&st).unwrap(), serde_json::to_value(&want).unwrap());
                let diff: Vec<&String> = a.as_object().unwrap().keys().filter(|k| a[k.as_str()] != b[k.as_str()]).collect();
                eprintln!("PREP_MISMATCH at {h}: fields {diff:?}");
                for k in &diff { eprintln!("  {k}: prep {} | proved {}", a[k.as_str()], b[k.as_str()]); }
                std::process::exit(1);
            }
        }
        prev = st;
    }
    println!("LZ_PREP_DONE {lo}..={hi} in {:.1} s{}", t0.elapsed().as_secs_f64(),
             if check.is_some() { "; every state's journal byte-identical to the proved run" } else { "" });
}
