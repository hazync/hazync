//! The bridge: walks the node's chain into per-block bundles, its checkpoint, the undo log and reorg
//! handling, plus the UTXO snapshot commands (dump-snapshot, snapshot-emit, snapshot-verify).

use bitcoin::consensus::{deserialize, serialize};
use bitcoin::hashes::Hash as _;
use hazync_coinbase_smt::Smt;
use hazync_utreexo::{Forest, coin_leaf};
use methods::METHOD_ID;
use serde::{Deserialize, Serialize};
use crate::{Bundle, GENESIS_BITS, GENESIS_TIME, KIND_RANGE, RangeState, build_block_carried, hex, hx,
    normalize_host, out_spendable, push_mtp, smt_advance, verify_receipt_ex, wire_stump};

// ============================================================================================
// G4 — proven assumeutxo: bind a UTXO snapshot to a range proof (#42)
// ============================================================================================
//
// Core already ships `assumeutxo`: a node adopts a UTXO set at height N on the authority of a hash
// its developers chose. This replaces that authority with a proof, the same substitution Hazync makes
// for `assumevalid`. A node that verifies the proof, rebuilds the accumulator from the snapshot, and
// finds the roots equal can start at N+1 without validating anything beneath it.
//
// TWO THINGS CORE'S SNAPSHOT DOES NOT GIVE YOU, and neither is an obstacle:
//
//   coin_mtp   The leaf commits MTP(coin_height - 1) so BIP68 time-locks are checked against Core's
//              own value, and Core's UTXO representation does not store it. It is a pure function of
//              the HEADER chain, which a node doing headers-first sync already has. Verified against
//              real mainnet data: 6/6 inputs from block 195,000 reproduce exactly.
//
//   leaf order The forest is an ORDERED array with swap-and-pop deletion, so its layout depends on
//              deletion history, not just on which coins are live. Two histories ending with the same
//              live set produce different roots. A snapshot is a set, so the order must be carried —
//              done here by EMITTING IN FOREST-POSITION ORDER, which costs zero extra bytes. The
//              records are exactly Core's content; only the sort is specified. (Core's own
//              `dumptxoutset` orders by outpoint, so the two files are not byte-identical.)
//
// Format, little-endian, no framing beyond what is written:
//   magic "HZSNAP01" | height u32 | count u64 | count x { txid[32] vout u32 value u64
//                                                        spk_len u32 spk[] height u32 coinbase u8 }
// Records appear in forest-position order: record i is the coin at accumulator position i.

pub(crate) const SNAP_MAGIC: &[u8; 8] = b"HZSNAP01";

pub(crate) fn snapshot_emit_cmd(dir: &str, out: &str) {
    let st = bridge_load_state(dir).expect("no bridge checkpoint in that directory");
    // Invert the utxo map by leaf so records can be written in forest-position order. The bridge keeps
    // `leaves` (ordered, the accumulator) and `utxo` (metadata, unordered) separately; the leaf hash is
    // the only thing linking them.
    let mut by_leaf: std::collections::HashMap<[u8; 32], (&([u8; 32], u32), &(u64, Vec<u8>, u32, bool))> =
        std::collections::HashMap::with_capacity(st.utxo.len());
    // MTP comes from the checkpoint, not from the node. `block_mtp` is indexed by absolute height and
    // already holds MTP(h-1) for every height the bridge walked, and `snapshot_verify_cmd` below reads it
    // exactly this way. `mtp_at` instead asks bitcoind, and its own comment admits the cost: 22 RPC
    // round-trips per cache miss, each one a `bitcoin-cli` process. Measured on the proof-party server at
    // height 250,000: ~450 spawns a second, still running after 14 minutes, and it fails outright unless
    // the caller happens to have the node's datadir in HAZYNC_BITCOIN_DATADIR. The data was in hand the
    // whole time — `bridge_load_state` loaded it on the line above.
    for (op, meta) in st.utxo.iter() {
        let mtp = *st.block_mtp.get(meta.2 as usize).unwrap_or_else(||
            panic!("no block_mtp for coin height {} (checkpoint has {} entries) — the checkpoint does not \
                    cover this coin, so its leaf cannot be reproduced", meta.2, st.block_mtp.len()));
        let leaf = coin_leaf(&op.0, op.1, meta.0, &meta.1, meta.2, meta.3, mtp);
        by_leaf.insert(leaf, (op, meta));
    }

    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(SNAP_MAGIC);
    buf.extend_from_slice(&st.height.to_le_bytes());
    buf.extend_from_slice(&(st.leaves.len() as u64).to_le_bytes());
    let mut missing = 0usize;
    for leaf in &st.leaves {
        match by_leaf.get(leaf) {
            Some((op, meta)) => {
                buf.extend_from_slice(&op.0);
                buf.extend_from_slice(&op.1.to_le_bytes());
                buf.extend_from_slice(&meta.0.to_le_bytes());
                buf.extend_from_slice(&(meta.1.len() as u32).to_le_bytes());
                buf.extend_from_slice(&meta.1);
                buf.extend_from_slice(&meta.2.to_le_bytes());
                buf.push(meta.3 as u8);
            }
            None => missing += 1,
        }
    }
    // A leaf with no metadata means the two halves of the bridge state disagree. Emitting a short
    // snapshot would produce a file that simply fails to verify later, with no clue why.
    assert!(missing == 0, "{missing} accumulator leaves have no UTXO metadata — bridge state is inconsistent");
    std::fs::write(out, &buf).expect("write snapshot");
    println!("wrote {out}: height {}, {} coins, {} bytes (forest-position order)",
             st.height, st.leaves.len(), buf.len());
}

/// MTP(h-1) from the header chain — the value the leaf commits. Cached, because a snapshot has many
/// coins per height and each miss is 22 RPC round-trips.
pub(crate) fn mtp_at(coin_height: u32) -> u32 {
    use std::sync::Mutex;
    static CACHE: Mutex<Option<std::collections::HashMap<u32, u32>>> = Mutex::new(None);
    let mut g = CACHE.lock().unwrap();
    let c = g.get_or_insert_with(std::collections::HashMap::new);
    if let Some(v) = c.get(&coin_height) { return *v; }
    let lo = coin_height.saturating_sub(11);
    let mut ts: Vec<u32> = Vec::new();
    for h in lo..coin_height {
        let hash = bcli(&["getblockhash", &h.to_string()]);
        let hdr = bcli(&["getblockheader", &hash]);
        let t = hdr.split("\"time\":").nth(1).and_then(|x| x.split(|ch: char| !ch.is_ascii_digit()).find(|x| !x.is_empty()))
            .and_then(|x| x.parse::<u32>().ok()).expect("header time");
        ts.push(t);
    }
    ts.sort_unstable();
    let m = ts[ts.len() / 2];
    c.insert(coin_height, m);
    m
}

pub(crate) fn snapshot_verify_cmd(snap: &str, proof: &str) {
    let b = std::fs::read(snap).expect("read snapshot");
    assert!(b.len() > 20 && &b[..8] == SNAP_MAGIC, "not a HZSNAP01 snapshot");
    let height = u32::from_le_bytes(b[8..12].try_into().unwrap());
    let count = u64::from_le_bytes(b[12..20].try_into().unwrap()) as usize;

    // Rebuild every leaf FROM THE SNAPSHOT. Taking them from the bridge would only prove the bridge
    // agrees with itself; the whole point is to check the snapshot independently.
    let mut leaves: Vec<[u8; 32]> = Vec::with_capacity(count);
    let mut p = 20usize;
    for _ in 0..count {
        let txid: [u8; 32] = b[p..p + 32].try_into().unwrap(); p += 32;
        let vout = u32::from_le_bytes(b[p..p + 4].try_into().unwrap()); p += 4;
        let value = u64::from_le_bytes(b[p..p + 8].try_into().unwrap()); p += 8;
        let sl = u32::from_le_bytes(b[p..p + 4].try_into().unwrap()) as usize; p += 4;
        let spk = b[p..p + sl].to_vec(); p += sl;
        let ch = u32::from_le_bytes(b[p..p + 4].try_into().unwrap()); p += 4;
        let cb = b[p] != 0; p += 1;
        leaves.push(coin_leaf(&txid, vout, value, &spk, ch, cb, mtp_at(ch)));
    }
    assert!(p == b.len(), "trailing bytes in snapshot: parsed {p} of {}", b.len());

    let forest = Forest::from_leaves(leaves);
    let r: risc0_zkvm::Receipt = bincode::deserialize(&std::fs::read(proof).expect("proof")).unwrap();
    let claimed = r.journal.decode::<RangeState>().ok().map(|rs| rs.self_id);
    verify_receipt_ex(&r, claimed);
    let rs: RangeState = r.journal.decode().unwrap();
    assert!(rs.self_id == METHOD_ID, "proof was made by a different guest");
    assert!(rs.kind == KIND_RANGE, "receipt is not a RangeState");

    let ok_h = rs.hi == height;
    let ok_n = rs.out_leaves == forest.leaves.len() as u64;
    let ok_r = normalize_host(rs.out_roots.clone()) == normalize_host(forest.roots());
    println!("snapshot height {height}, {} coins", forest.leaves.len());
    println!("  proof range        [{}..{}]", rs.lo, rs.hi);
    println!("  height matches     {ok_h}");
    println!("  leaf count matches {ok_n}   (snapshot {} vs proof {})", forest.leaves.len(), rs.out_leaves);
    println!("  ROOTS match        {ok_r}");
    if ok_h && ok_n && ok_r {
        println!(">>> SNAPSHOT BOUND TO PROOF — this is the UTXO set the proof attests to.");
        println!("    A node may adopt it at height {height} and validate from {} onward.", height + 1);
    } else {
        eprintln!(">>> NOT BOUND — the snapshot is not the set this proof commits to.");
        std::process::exit(1);
    }
}

pub(crate) fn bcli(args: &[&str]) -> String {
    let dd = std::env::var("HAZYNC_BITCOIN_DATADIR").unwrap_or_else(|_| "/root/.bitcoin".into());
    let o = std::process::Command::new("bitcoin-cli").arg(format!("-datadir={dd}")).args(args)
        .output().expect("run bitcoin-cli");
    if !o.status.success() { panic!("bitcoin-cli {args:?}: {}", String::from_utf8_lossy(&o.stderr)); }
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

pub(crate) type Utxo = std::collections::HashMap<([u8; 32], u32), (u64, Vec<u8>, u32, bool)>; // op -> value,spk,height,coinbase

// The block-JSON build_block_carried expects, sourcing prevouts from the UTXO map (external spends) or
// this block's own outputs (in-block spends).
pub(crate) fn bridge_block_json(block: &bitcoin::Block, height: u32, utxo: &Utxo) -> serde_json::Value {
    let hdr = &block.header;
    let mut cur: std::collections::HashMap<([u8; 32], u32), (u64, Vec<u8>)> = std::collections::HashMap::new();
    for t in &block.txdata {
        let txid = t.compute_txid().to_byte_array();
        for (v, o) in t.output.iter().enumerate() {
            cur.insert((txid, v as u32), (o.value.to_sat(), o.script_pubkey.as_bytes().to_vec()));
        }
    }
    let coinbase = &block.txdata[0];
    let txs: Vec<serde_json::Value> = block.txdata[1..].iter().map(|t| {
        let prevouts: Vec<serde_json::Value> = t.input.iter().map(|inp| {
            let op = inp.previous_output;
            let key = (op.txid.to_byte_array(), op.vout);
            if let Some((value, spk, ch, cb)) = utxo.get(&key) {
                serde_json::json!({ "value": value, "spk": hex(spk), "coin_height": ch, "coin_is_coinbase": *cb as u32 })
            } else {
                let (value, spk) = cur.get(&key).expect("prevout in neither UTXO map nor this block");
                serde_json::json!({ "value": value, "spk": hex(spk), "coin_height": height, "coin_is_coinbase": 0 })
            }
        }).collect();
        serde_json::json!({ "raw": hex(&serialize(t)), "prevouts": prevouts })
    }).collect();
    let mut prev = hdr.prev_blockhash.to_byte_array(); prev.reverse();  // -> DISPLAY order
    let mut mrk = hdr.merkle_root.to_byte_array(); mrk.reverse();
    serde_json::json!({
        "height": height, "bits": hdr.bits.to_consensus(), "time": hdr.time, "nonce": hdr.nonce,
        "version": hdr.version.to_consensus(), "prev": hex(&prev), "merkle": hex(&mrk),
        "coinbase_hex": hex(&serialize(coinbase)), "txs": txs,
    })
}

// Advance the UTXO map to mirror build_block_carried's forest transition (remove external spends, add new
// spendable outputs not spent within this block).
/// Returns (coins removed with their prior value, coins created) — the block's UTXO delta, captured
/// by the code that applies it (hazync#484). ⚠ The removals carry the FULL prior value, because
/// putting a coin back needs its amount, script, creation height and coinbase flag; the key alone
/// would restore an entry that no longer says what the coin was.
pub(crate) type UtxoDelta = (Vec<(([u8; 32], u32), (u64, Vec<u8>, u32, bool))>, Vec<([u8; 32], u32)>);

pub(crate) fn bridge_update_utxo(utxo: &mut Utxo, block: &bitcoin::Block, height: u32) -> UtxoDelta {
    let mut spent: std::collections::HashSet<([u8; 32], u32)> = std::collections::HashSet::new();
    for t in block.txdata.iter().skip(1) {
        for inp in &t.input { spent.insert((inp.previous_output.txid.to_byte_array(), inp.previous_output.vout)); }
    }
    let mut removed = Vec::new();
    for k in &spent {
        // ⚠ Only what was ACTUALLY there. A prevout created within this same block is in `spent` but
        // was never in `utxo`; recording a removal for it would re-insert a coin on undo that the
        // map never held.
        if let Some(v) = utxo.remove(k) { removed.push((*k, v)); }
    }
    let mut created = Vec::new();
    for (ti, t) in block.txdata.iter().enumerate() {
        let txid = t.compute_txid().to_byte_array();
        let is_cb = ti == 0;
        for (v, o) in t.output.iter().enumerate() {
            if !out_spendable(o.script_pubkey.as_bytes()) { continue; }
            if spent.contains(&(txid, v as u32)) { continue; }
            utxo.insert((txid, v as u32), (o.value.to_sat(), o.script_pubkey.as_bytes().to_vec(), height, is_cb));
            created.push((txid, v as u32));
        }
    }
    (removed, created)
}

// The resident forest + UTXO-metadata map + MTP/difficulty carry, checkpointed so the bridge resumes
// mid-chain instead of rebuilding from genesis. `leaves` is the live UTXO set (swap-and-pop), so at the
// tip it is ~the UTXO count, not the full history. Borrowed on save (no clone of the multi-GB state),
// owned on load; field order MUST match between the two (bincode is positional).
//
// #54 adds `smt`: the coinbase-SMT entries (txid -> unspent output count) backing BIP30
// non-membership. Stored as a sorted Vec rather than the live `Smt` because bincode is positional and
// a HashMap's iteration order is not — a checkpoint that round-tripped differently each save would be
// a state that disagrees with itself.
//
// APPENDING THIS FIELD INVALIDATES EXISTING CHECKPOINTS, deliberately. bincode is positional, so an
// old state.bin fails to deserialise and the bridge rebuilds from genesis. That is the correct
// outcome and not merely tolerable: the SMT has to be built by replaying the chain anyway — there is
// no way to derive "which coinbases still have unspent outputs" from a checkpoint that never tracked
// it — so a resume that silently kept the old state would produce a tree missing all history before
// the upgrade.
#[derive(Serialize)]
pub(crate) struct BridgeStateRef<'a> {
    height: u32, leaves: &'a Vec<[u8; 32]>, utxo: &'a Utxo,
    win: &'a Vec<u32>, block_mtp: &'a Vec<u32>, nbits: u32, time: u32, epoch_start: u32,
    smt: Vec<([u8; 32], u32)>,
}
#[derive(Deserialize)]
pub(crate) struct BridgeState {
    height: u32, leaves: Vec<[u8; 32]>, utxo: Utxo,
    win: Vec<u32>, block_mtp: Vec<u32>, nbits: u32, time: u32, epoch_start: u32,
    smt: Vec<([u8; 32], u32)>,
}

/// The hash of the block the accumulator last advanced over, stored BESIDE the checkpoint.
///
/// ⛔ NOT A FIELD ON `BridgeState`, AND THAT IS THE WHOLE POINT (hazync#484). The checkpoint is
/// bincode, so adding a field makes every existing checkpoint undeserialisable — `bridge_load_state`
/// swallows that as `None` and the bridge silently "starts from genesis". The live state.bin is
/// 19.4 GB and took roughly EIGHT DAYS to build. A format change here does not cost a migration, it
/// costs a week of walking, and the log line that announces it reads like a normal startup.
///
/// A sidecar file has none of that: an old checkpoint simply has no sidecar, which is honestly
/// reported as "cannot check" until the next checkpoint writes one.
/// Has the chain moved under the accumulator? `None` means the question cannot be answered.
///
/// ⛔ THREE OUTCOMES, NOT TWO (hazync#484). "I have no head to compare against" is not "no reorg" —
/// a check that cannot fail is the thing `finality = 100` was already hiding behind. An old
/// checkpoint with no sidecar must report that it cannot check, loudly, rather than pass.
/// Everything needed to put the bridge back exactly as it was before one block (hazync#484).
///
/// ⛔ WHY THE BRIDGE COULD NEVER FOLLOW THE TIP. Four structures advance per block — the utreexo
/// forest, the UTXO map, the coinbase SMT and the header window — and not one of them could go
/// backwards. `HAZYNC_BRIDGE_FINALITY=100` is what that costs: stay so far from the tip that no
/// reorg can reach you. The alternative is to record the inverse of each step, which is what this
/// is, and it is small: measured against a 19.4 GB checkpoint, a block's undo is under a megabyte.
///
/// ⚠ NOTHING HERE IS RECONSTRUCTED. Each field is captured by the code that did the mutation, at the
/// moment it did it. A rewind that re-derived "which coins did that block spend" would be a second
/// implementation of consensus rules, betting it agrees with the first.
#[derive(Serialize, Deserialize)]
pub(crate) struct BlockUndo {
    height: u32,
    /// The block this undoes — so a rewind can prove it is unwinding the chain it thinks it is.
    block_hash: String,
    /// Forest mutations in the order they happened; inverted newest-first.
    forest_ops: Vec<UndoForestOp>,
    /// Coins the block spent, with everything needed to put them back.
    utxo_spent: Vec<(([u8; 32], u32), (u64, Vec<u8>, u32, bool))>,
    /// Coins the block created — removing these is the inverse.
    utxo_created: Vec<([u8; 32], u32)>,
    /// Every coinbase-SMT key the block touched, with the value it held BEFORE. `None` = absent.
    /// ⚠ One shape for all three mutations (insert, remove-on-grandfather, decrement-on-spend):
    /// restoring a prior value inverts each of them without the rewind needing to know which it was.
    smt_prior: Vec<([u8; 32], Option<u32>)>,
    /// The header window as it stood before. `win` is capped at 11 entries and `block_mtp` only ever
    /// grows by one, so this is bytes, not a copy of history.
    prev_win: Vec<u32>,
    prev_nbits: u32,
    prev_time: u32,
    prev_epoch_start: u32,
}

/// `ForestOp` is defined in the accumulator, which deliberately has no serde — it is linked by the
/// guest, where every dependency is proving cost. This is its serialisable twin, converted at the
/// boundary.
#[derive(Serialize, Deserialize, Clone, Copy)]
pub(crate) enum UndoForestOp {
    Added,
    Deleted { pos: usize, leaf: [u8; 32] },
}

impl From<&hazync_utreexo::ForestOp> for UndoForestOp {
    fn from(o: &hazync_utreexo::ForestOp) -> Self {
        match *o {
            hazync_utreexo::ForestOp::Added => UndoForestOp::Added,
            hazync_utreexo::ForestOp::Deleted { pos, leaf } => UndoForestOp::Deleted { pos, leaf },
        }
    }
}

impl From<&UndoForestOp> for hazync_utreexo::ForestOp {
    fn from(o: &UndoForestOp) -> Self {
        match *o {
            UndoForestOp::Added => hazync_utreexo::ForestOp::Added,
            UndoForestOp::Deleted { pos, leaf } => hazync_utreexo::ForestOp::Deleted { pos, leaf },
        }
    }
}

/// Write one block's undo and drop anything older than `keep` blocks (hazync#484).
///
/// ⚠ ONE FILE PER HEIGHT, not an append-only log. A rewind wants a specific height and a prune wants
/// to drop the oldest; both are trivial on named files and fiddly on a single growing file that
/// would also have to be rewritten to prune.
///
/// ⚠ BEST EFFORT, DELIBERATELY. A failed undo write must not stop the bridge: the consequence is a
/// shorter rewind window, not a wrong accumulator. It is reported, not fatal.
pub(crate) fn bridge_push_undo(dir: &str, u: &BlockUndo, keep: u32) {
    let d = format!("{dir}/undo");
    if std::fs::create_dir_all(&d).is_err() { return; }
    let path = format!("{d}/{}.undo", u.height);
    let tmp = format!("{path}.tmp");
    match bincode::serialize(u) {
        Ok(bytes) => {
            if std::fs::write(&tmp, &bytes).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            } else {
                println!("bridge: ⚠ could not write undo for {} — the rewind window is shorter by one", u.height);
            }
        }
        Err(e) => println!("bridge: ⚠ could not serialise undo for {}: {e}", u.height),
    }
    // prune: heights more than `keep` behind this one can no longer be reached by a rewind
    if u.height > keep {
        let cutoff = u.height - keep;
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let n = e.file_name();
                let n = n.to_string_lossy();
                if let Some(h) = n.strip_suffix(".undo").and_then(|x| x.parse::<u32>().ok()) {
                    if h < cutoff { let _ = std::fs::remove_file(e.path()); }
                }
            }
        }
    }
}

pub(crate) fn bridge_load_undo(dir: &str, height: u32) -> Option<BlockUndo> {
    let b = std::fs::read(format!("{dir}/undo/{height}.undo")).ok()?;
    bincode::deserialize(&b).ok()
}

/// The oldest height still reachable by a rewind, or `None` if nothing is stored.
pub(crate) fn bridge_undo_floor(dir: &str) -> Option<u32> {
    let rd = std::fs::read_dir(format!("{dir}/undo")).ok()?;
    rd.flatten()
        .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".undo").and_then(|x| x.parse::<u32>().ok()))
        .min()
}

/// The highest height at which the undo log and the node still agree, or `None` (hazync#484).
///
/// ⛔ WALK DOWN UNTIL THEY AGREE, DO NOT GUESS A DEPTH. A reorg's depth is not known in advance and
/// assuming one would be a number nobody can justify. The search is bounded by the undo window
/// instead, which is the honest limit: below it there is nothing to rewind to, so a deeper search
/// could only find a fork it cannot act on.
///
/// `at` returns the node's hash for a height; `undo_at` returns the hash the accumulator walked.
/// Both are injected so the search can be driven without a node or a filesystem.
pub(crate) fn bridge_fork_point<F, G>(floor: u32, done: u32, at: F, undo_at: G) -> Option<u32>
where F: Fn(u32) -> Option<String>, G: Fn(u32) -> Option<String> {
    for h in (floor..=done).rev() {
        let ours = undo_at(h)?;          // no undo here: the window has ended, and so has the search
        let theirs = at(h)?;
        if ours == theirs {
            return Some(h);              // h is common; divergence starts at h+1
        }
    }
    None
}

/// Put the bridge back to the state it held at `target`, newest block first.
///
/// ⛔ NEWEST FIRST, AND ALL OR NOTHING. Each block's undo is the inverse of the state as it stood
/// immediately after that block, so they only compose in reverse. And a rewind that gets halfway and
/// stops leaves every structure at a height none of them agree on — worse than not starting, because
/// the checkpoint would then persist it. The range is checked for completeness BEFORE anything is
/// touched.
#[allow(clippy::too_many_arguments)]
pub(crate) fn bridge_rewind_to(
    dir: &str, target: u32, done: &mut u32,
    forest: &mut Forest, utxo: &mut Utxo, smt: &mut Smt,
    win: &mut Vec<u32>, block_mtp: &mut Vec<u32>,
    nbits: &mut u32, time: &mut u32, epoch_start: &mut u32,
) -> Result<(), String> {
    if target >= *done {
        return Err(format!("rewind target {target} is not below the current height {done}"));
    }
    for h in (target + 1)..=*done {
        if bridge_load_undo(dir, h).is_none() {
            return Err(format!(
                "no undo for height {h}: the rewind window reaches back to {:?}, and {target} is \
                 beyond it — the state must be rebuilt from a checkpoint below the fork",
                bridge_undo_floor(dir)));
        }
    }
    for h in ((target + 1)..=*done).rev() {
        let u = bridge_load_undo(dir, h).expect("checked above");
        // forest: inverses, newest-first within the block too
        let ops: Vec<hazync_utreexo::ForestOp> =
            u.forest_ops.iter().map(hazync_utreexo::ForestOp::from).collect();
        forest.undo_ops(&ops);
        // utxo: remove what the block created, restore what it spent
        for k in &u.utxo_created { utxo.remove(k); }
        for (k, v) in &u.utxo_spent { utxo.insert(*k, v.clone()); }
        // smt: put every touched key back to the value it held before
        for (k, prior) in &u.smt_prior {
            match prior {
                Some(v) => smt.insert(*k, *v),
                None => smt.remove(k),
            }
        }
        // header window
        *win = u.prev_win.clone();
        block_mtp.pop();
        *nbits = u.prev_nbits;
        *time = u.prev_time;
        *epoch_start = u.prev_epoch_start;
        *done = h - 1;
        let _ = std::fs::remove_file(format!("{dir}/undo/{h}.undo"));
    }
    Ok(())
}

pub(crate) fn bridge_reorg_seen(expect: Option<&str>, node_hash_at_done: &str) -> Option<bool> {
    let e = expect?;
    if e.len() != 64 || node_hash_at_done.len() != 64 { return None; }
    Some(!e.eq_ignore_ascii_case(node_hash_at_done))
}

pub(crate) fn bridge_save_head(dir: &str, height: u32, hash_hex: &str) {
    let tmp = format!("{dir}/state.head.tmp");
    if std::fs::write(&tmp, format!("{height} {hash_hex}\n")).is_ok() {
        let _ = std::fs::rename(&tmp, format!("{dir}/state.head"));
    }
}

pub(crate) fn bridge_load_head(dir: &str) -> Option<(u32, String)> {
    let t = std::fs::read_to_string(format!("{dir}/state.head")).ok()?;
    let mut it = t.split_whitespace();
    let h: u32 = it.next()?.parse().ok()?;
    let hash = it.next()?.to_string();
    if hash.len() != 64 { return None; }
    Some((h, hash))
}

pub(crate) fn bridge_load_state(dir: &str) -> Option<BridgeState> {
    std::fs::read(format!("{dir}/state.bin")).ok().and_then(|b| bincode::deserialize(&b).ok())
}
/// Stream the checkpoint to disk instead of building it in RAM first.
///
/// `bincode::serialize` returns a `Vec<u8>` holding the WHOLE checkpoint, and it is alive at the same
/// time as every structure it was built from. Measured on the proof-party server at 19,640,166 coins:
/// the walk itself sits flat at 5.80 GiB, then anon ramps to 7.84 GiB over eleven seconds with no file
/// on disk yet, a 1.91 GB `state.bin.tmp` appears at once, and it drops straight back — so the bridge's
/// peak was never the walk, it was this buffer. It recurs at every checkpoint (the bridge runs
/// `HAZYNC_BRIDGE_CKPT=2000`, roughly every six minutes), and at the tip's ~165M coins the buffer is
/// ~18 GiB, which is what takes a tip bridge past a 62.8 GiB box.
///
/// `serialize_into` emits the same bytes: both paths use `DefaultOptions` with `with_fixint_encoding`,
/// and `allow_trailing_bytes` (the only difference) affects deserialisation only. Verified byte-for-byte
/// on a struct with these field types. That matters because a changed encoding would make every existing
/// `state.bin` fail to load, and the resume path treats that as "rebuild from genesis" — silently.
pub(crate) fn bridge_save_state(dir: &str, st: &BridgeStateRef) {
    let tmp = format!("{dir}/state.bin.tmp");
    {
        let f = std::fs::File::create(&tmp).expect("create checkpoint");
        let mut w = std::io::BufWriter::new(f);
        bincode::serialize_into(&mut w, st).expect("write checkpoint");
        // into_inner() flushes AND surfaces the error. Dropping a BufWriter discards a failed flush,
        // which would rename a truncated checkpoint into place below and look like a clean save.
        w.into_inner().expect("flush checkpoint");
    }
    std::fs::rename(&tmp, format!("{dir}/state.bin")).expect("commit checkpoint"); // atomic: never a torn state.bin
}

/// Magic + version for the intermediate UTXO dump consumed by ghostd (see `cmd_dump_snapshot`).
pub(crate) const HZUTXO_MAGIC: &[u8; 8] = b"HZUTXO\0\0";
pub(crate) const HZUTXO_VERSION: u32 = 1;

/// Emit the bridge's UTXO set as an UNCOMPRESSED intermediate for ghostd to turn into a chainstate.
///
/// This is the emitter half of "proven assumeutxo": instead of trusting a developer-chosen snapshot
/// hash, ghostd rebuilds the accumulator from what this writes and checks its roots against the ones
/// a Hazync proof commits to.
///
/// WHY AN INTERMEDIATE RATHER THAN CORE'S SNAPSHOT FORMAT. Core serialises `Coin` COMPRESSED (varint
/// `height*2+coinbase`, `CTxOutCompressor` amount/script compression). Reproducing that byte-exactly
/// here would be a second implementation of a Core format living in a different language — the same
/// duplication the project refuses in the other direction, where ghostd delegates proof verification
/// to Rust rather than reimplementing risc0 in C++. So this writes plain fields and ghostd builds the
/// chainstate with Core's own serialisers.
///
/// WHAT IS DELIBERATELY ABSENT: the accumulator roots. A consumer must take those from the PROOF, not
/// from the file it is checking — writing them here would invite verifying the dump against itself.
///
/// `position` is the coin's index in the accumulator. It cannot be derived from the coin data: the
/// forest deletes by swap-and-shrink, so its layout is a function of the whole add/delete history,
/// not of the surviving set. Core's snapshot is txid-grouped and so cannot carry it implicitly either.
pub(crate) fn cmd_dump_snapshot(out_path: &str) {
    let dir = std::env::var("HAZYNC_BRIDGE_OUT").unwrap_or_else(|_| "/root/bridge_bundles".into());
    let st = bridge_load_state(&dir).unwrap_or_else(|| panic!("no bridge checkpoint at {dir}/state.bin"));

    // The forest holds one leaf per live coin. If these ever disagree the dump would be meaningless,
    // and silently so — a missing coin still produces a well-formed file that simply fails the root
    // check later, with nothing pointing at the cause.
    assert_eq!(st.utxo.len(), st.leaves.len(),
        "bridge checkpoint disagrees with itself: {} utxos vs {} accumulator leaves",
        st.utxo.len(), st.leaves.len());

    // Invert position->leaf into leaf->position. Leaves are coin-unique (an outpoint occurs once), so
    // a collision here means the checkpoint is corrupt rather than merely surprising.
    let mut pos_of: std::collections::HashMap<[u8; 32], u32> =
        std::collections::HashMap::with_capacity(st.leaves.len());
    for (i, leaf) in st.leaves.iter().enumerate() {
        if pos_of.insert(*leaf, i as u32).is_some() {
            panic!("duplicate accumulator leaf at position {i} — corrupt checkpoint");
        }
    }

    // Deterministic order: a dump that reordered between runs could not be diffed or reproduced.
    let mut keys: Vec<&([u8; 32], u32)> = st.utxo.keys().collect();
    keys.sort_unstable();

    let mut body: Vec<u8> = Vec::new();
    for k in &keys {
        let (value, spk, height, is_coinbase) = &st.utxo[*k];
        // Same convention the prover uses: block_mtp[h] == MTP(h-1), and a coin's mtp is that of the
        // block that created it. Indexing past the window means the checkpoint cannot describe its
        // own coins, which is a bug rather than a coin to skip.
        let coin_mtp = *st.block_mtp.get(*height as usize).unwrap_or_else(||
            panic!("no block_mtp for coin height {height} (window len {})", st.block_mtp.len()));
        let leaf = coin_leaf(&k.0, k.1, *value, spk, *height, *is_coinbase, coin_mtp);
        let pos = *pos_of.get(&leaf).unwrap_or_else(||
            panic!("coin {}:{} is not in the accumulator — checkpoint utxo/leaves disagree",
                k.0.iter().map(|b| format!("{b:02x}")).collect::<String>(), k.1));

        body.extend_from_slice(&k.0);
        body.extend_from_slice(&k.1.to_le_bytes());
        body.extend_from_slice(&value.to_le_bytes());
        body.extend_from_slice(&height.to_le_bytes());
        body.push(*is_coinbase as u8);
        body.extend_from_slice(&coin_mtp.to_le_bytes());
        body.extend_from_slice(&pos.to_le_bytes());
        body.extend_from_slice(&(spk.len() as u32).to_le_bytes());
        body.extend_from_slice(spk);
    }

    let mut out: Vec<u8> = Vec::with_capacity(body.len() + 32);
    out.extend_from_slice(HZUTXO_MAGIC);
    out.extend_from_slice(&HZUTXO_VERSION.to_le_bytes());
    out.extend_from_slice(&st.height.to_le_bytes());
    out.extend_from_slice(&(keys.len() as u64).to_le_bytes());
    out.extend_from_slice(&body);

    let tmp = format!("{out_path}.tmp");
    std::fs::write(&tmp, &out).expect("write utxo dump");
    std::fs::rename(&tmp, out_path).expect("commit utxo dump"); // atomic: never a torn dump
    println!("dump-snapshot: height {}, {} coins -> {} ({} bytes)",
        st.height, keys.len(), out_path, out.len());
}

pub(crate) fn cmd_bridge() {
    let out_dir = std::env::var("HAZYNC_BRIDGE_OUT").unwrap_or_else(|_| "/root/bridge_bundles".into());
    std::fs::create_dir_all(&out_dir).unwrap();
    // ⛔ THE DEFAULT IS NOW 0: FOLLOW THE TIP (hazync#484). It was 100, and the comment here used to
    // say the forest "never enters the re-org zone" — which was true only because it could not leave
    // it either. A resident accumulator with no rewind has exactly one defence against a reorg, and
    // that is distance; the cost was a bridge permanently ~16 hours behind the chain it exists to
    // follow, which makes "tip following" a name rather than a property.
    //
    // ⚠ 0 IS ONLY SAFE BECAUSE OF THE TWO THINGS ABOVE IT. Detection notices when the chain moves
    // under the accumulator, and the undo log puts it back. Running 0 without them would emit bundles
    // that become wrong on the first 1-block reorg — which Bitcoin produces every few weeks — with
    // nothing to notice and nothing to recover to short of a walk from genesis.
    //
    // ⚠ A reorg deeper than HAZYNC_BRIDGE_UNDO_KEEP (288, two days of chain) still halts: that is not
    // a rewind this can do, and faking it would be worse than stopping.
    let finality: u32 = std::env::var("HAZYNC_BRIDGE_FINALITY").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let ckpt_every: u32 = std::env::var("HAZYNC_BRIDGE_CKPT").ok().and_then(|s| s.parse().ok()).unwrap_or(2000);
    // How many blocks of undo to keep. 288 is two days of chain at ten minutes a block — far past any
    // reorg Bitcoin has had outside the 2013 fork, and under a megabyte a block it costs nothing.
    let undo_keep: u32 = std::env::var("HAZYNC_BRIDGE_UNDO_KEEP").ok().and_then(|s| s.parse().ok()).unwrap_or(288);
    let poll: u64 = std::env::var("HAZYNC_BRIDGE_POLL").ok().and_then(|s| s.parse().ok()).unwrap_or(30);
    let once = std::env::var("HAZYNC_BRIDGE_ONCE").is_ok();     // exit after catching up once (seeding / tests)
    let cap = std::env::var("HAZYNC_BRIDGE_TO").ok().and_then(|s| s.parse::<u32>().ok()); // optional hard height cap
    // Emit a bundle only at or above this height; below it the bridge advances STATE and writes nothing.
    //
    // Reaching tip state by walking writes a bundle per block on the way, and that is the binding
    // constraint: measured 731 GB at height 381,318, and bytes/input x real input counts projects
    // 5.70 TB to reach 700,000 and 11.82 TB to reach the tip, against 4.64 TB free. The walk runs out
    // of disk around 620-650k — long before it runs out of time. Bundles for heights the board cannot
    // reach for months are dead weight; set this to the tip height to catch up silently and then emit
    // while tip-following. Default 0 keeps today's behaviour exactly.
    //
    // A skipped height is not inert on the coordinator: `bundle_path` returns None, so the bulk witness
    // endpoint reports it as missing and a sponsorship counts it as `waiting` indefinitely. Keep this
    // above anything the board can reach (highest proved height was 230,000 on 2026-09-16), and rebuild
    // a skipped range the way #347 already describes — copy a checkpoint below it into a scratch
    // HAZYNC_BRIDGE_OUT and replay with HAZYNC_BRIDGE_TO + HAZYNC_BRIDGE_ONCE.
    let emit_from: u32 = std::env::var("HAZYNC_BRIDGE_EMIT_FROM").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    if emit_from > 0 {
        // Say it out loud at startup: a silently empty bundle dir is indistinguishable from a broken
        // bridge, and this is the one setting that makes the bridge do less than its name implies.
        println!("bridge: EMIT_FROM={emit_from} — heights below it advance state only, NO bundle written");
    }

    // Resume from the last checkpoint, or start fresh at genesis.
    // A checkpoint written before #54 has no `smt` field, so bincode refuses it and this falls through
    // to genesis. That is intended — see the note on BridgeState. The log line below says so out loud,
    // because "starting from genesis" after an upgrade otherwise looks like data loss.
    let (mut forest, mut utxo, mut win, mut block_mtp, mut nbits, mut time, mut epoch_start, mut done, mut smt) =
        if let Some(st) = bridge_load_state(&out_dir) {
            println!("bridge: resuming from checkpoint @ height {} ({} utxos, {} leaves, {} coinbase-SMT entries)",
                st.height, st.utxo.len(), st.leaves.len(), st.smt.len());
            (Forest::from_leaves(st.leaves), st.utxo, st.win, st.block_mtp, st.nbits, st.time, st.epoch_start,
             st.height, Smt::from_entries(st.smt))
        } else {
            if std::path::Path::new(&format!("{out_dir}/state.bin")).exists() {
                println!("bridge: checkpoint present but not readable under this build — \
                          rebuilding from genesis (expected on the #54 upgrade: the coinbase SMT \
                          cannot be derived from a checkpoint that never tracked it)");
            }
            println!("bridge: no checkpoint — starting from genesis");
            (Forest::new(), Utxo::new(), vec![GENESIS_TIME], vec![GENESIS_TIME],
             GENESIS_BITS, GENESIS_TIME, GENESIS_TIME, 0u32, Smt::new())
        };
    let mut last_ckpt = done;

    // ⛔ THE ACCUMULATOR IS RESIDENT AND CANNOT BE REWOUND, SO IT MUST KNOW WHEN THE CHAIN MOVED
    // UNDER IT (hazync#484). Nothing checked this: the loop asks the node for whatever block is at
    // each height and walks it, so a reorg at or below `done` leaves the forest, UTXO set and
    // coinbase SMT carrying state from the abandoned chain while the bundles claim heights on the
    // new one. `finality = 100` has been standing in for the check — it makes the window so deep
    // that no real reorg reaches it — but the failure it hides is SILENT, which is the wrong shape
    // for a rare event.
    //
    // `head` is the hash of the block at `done`. Within a run it is tracked as we go; across a
    // restart it comes from the sidecar. Where there is no sidecar (a checkpoint written before this
    // existed) the answer is "cannot check", said out loud, rather than a check that always passes.
    let mut head: Option<String> = match bridge_load_head(&out_dir) {
        Some((h, hash)) if h == done => Some(hash),
        Some((h, _)) => {
            println!("bridge: ⚠ head sidecar is for height {h} but the checkpoint is at {done} — \
                      reorg detection is OFF until the next checkpoint");
            None
        }
        None if done > 0 => {
            println!("bridge: ⚠ no head sidecar beside this checkpoint — reorg detection is OFF \
                      until the next checkpoint writes one");
            None
        }
        None => None,
    };
    let mut halted = false;

    loop {
        let tip: u32 = bcli(&["getblockcount"]).parse().expect("getblockcount");

        // ⚠ ASK BEFORE ADVANCING, EVERY PASS. One `getblockhash` against a loop that otherwise does
        // thousands of RPCs; the cost is nothing and it is the only moment the mismatch is cheap to
        // act on.
        if let Some(expect) = head.clone() {
            if done > 0 && done <= tip {
                let now_hash = bcli(&["getblockhash", &done.to_string()]);
                if bridge_reorg_seen(Some(&expect), &now_hash) == Some(true) {
                    println!("bridge: ⛔ REORG AT OR BELOW HEIGHT {done}");
                    println!("bridge:    the accumulator advanced over {expect}");
                    println!("bridge:    the node now has            {now_hash}");

                    // ── find the fork point ────────────────────────────────────────────────────
                    // ⛔ WALK DOWN UNTIL THE CHAINS AGREE, do not guess a depth. The undo log is the
                    // limit of how far we can go, so the search is bounded by it rather than by a
                    // number somebody picked: below the window there is nothing to rewind to and a
                    // deeper search would only find a fork it cannot act on.
                    let floor = bridge_undo_floor(&out_dir).unwrap_or(done).max(1);
                    let fork = bridge_fork_point(
                        floor, done,
                        |h| { let x = bcli(&["getblockhash", &h.to_string()]);
                              if x.len() == 64 { Some(x) } else { None } },
                        |h| bridge_load_undo(&out_dir, h).map(|u| u.block_hash));
                    match fork {
                        Some(common) => {
                            // ⚠ CAPTURED BEFORE THE REWIND. `bridge_rewind_to` moves `done` down to
                            // `common`, so anything computed from it afterwards describes a height
                            // that no longer applies — the stale-bundle range in particular, which
                            // would collapse to a single file.
                            let was = done;
                            println!("bridge:    chains agree at {common}; rewinding {} block(s)",
                                     was - common);
                            match bridge_rewind_to(&out_dir, common, &mut done,
                                                   &mut forest, &mut utxo, &mut smt,
                                                   &mut win, &mut block_mtp,
                                                   &mut nbits, &mut time, &mut epoch_start) {
                                Ok(()) => {
                                    // ⛔ THE BUNDLES FOR THE ABANDONED BLOCKS MUST GO. They describe a
                                    // chain that no longer exists, and a prover polling the directory
                                    // cannot tell them from live work. The board re-proves the
                                    // replacements once the bridge re-emits at those heights.
                                    let mut dropped = 0;
                                    for h in (common + 1)..=was {
                                        let bp = format!("{out_dir}/bundle_{h}.json");
                                        if std::fs::remove_file(&bp).is_ok() { dropped += 1; }
                                    }
                                    // the head must describe where we actually are now
                                    head = bridge_load_undo(&out_dir, done)
                                        .map(|u| u.block_hash)
                                        .or_else(|| {
                                            let h = bcli(&["getblockhash", &done.to_string()]);
                                            if h.len() == 64 { Some(h) } else { None }
                                        });
                                    bridge_save_state(&out_dir, &BridgeStateRef {
                                        height: done, leaves: &forest.leaves, utxo: &utxo, win: &win,
                                        block_mtp: &block_mtp, nbits, time, epoch_start,
                                        smt: smt.entries() });
                                    if let Some(hh) = head.as_deref() { bridge_save_head(&out_dir, done, hh); }
                                    last_ckpt = done;
                                    println!("bridge:    ✅ rewound to {done}, dropped {dropped} stale \
                                              bundle(s); re-advancing on the new chain");
                                }
                                Err(e) => {
                                    println!("bridge:    ⛔ rewind refused: {e}");
                                    halted = true;
                                }
                            }
                        }
                        None => {
                            // ⚠ DEEPER THAN THE UNDO WINDOW. Not a rewind we can do, and not one to
                            // fake: halting keeps the accumulator honest and says what it would take.
                            println!("bridge:    ⛔ the fork is deeper than the undo window \
                                      (oldest undo: {floor}). HALTING — the state must be rebuilt \
                                      from a checkpoint below the fork, or from genesis. Raise \
                                      HAZYNC_BRIDGE_UNDO_KEEP if this recurs.");
                            halted = true;
                        }
                    }
                }
            }
        }
        // ⛔ HALT, DO NOT CRASH. `Restart=always` would turn an exit into a flap — restart, resume
        // from the same checkpoint, detect the same reorg, exit — and a flapping unit is how an
        // alert gets muted. The process stays up, emits nothing, and says why on every pass, so the
        // progress check and the journal both show a bridge that has deliberately stopped.
        if halted {
            std::thread::sleep(std::time::Duration::from_secs(poll.max(30)));
            continue;
        }

        let mut target = tip.saturating_sub(finality);
        if let Some(c) = cap { target = target.min(c); }
        if target > done {
            println!("bridge: advancing {}..={target} (node tip {tip}, finality {finality}) -> {out_dir}", done + 1);
            for h in (done + 1)..=target {
                let hash = bcli(&["getblockhash", &h.to_string()]);
                let raw = bcli(&["getblock", &hash, "0"]);
                let block: bitcoin::Block = deserialize(&hx(&raw)).expect("parse block");
                let j = bridge_block_json(&block, h, &utxo);
                let s = wire_stump(&forest);
                let in_recent = win.clone();
                let (in_nbits, in_time, in_epoch_start) = (nbits, time, epoch_start);
                let in_tip = block.header.prev_blockhash.to_byte_array(); // internal order = tip of h-1
                // ⚠ BEFORE push_mtp, which drains the front of `win` once it passes 11 — after it,
                // the element that fell off is gone and the window cannot be restored.
                let (prev_win, prev_nbits, prev_time, prev_epoch_start) =
                    (win.clone(), nbits, time, epoch_start);
                push_mtp(&j, &mut win, &mut block_mtp);
                // The forest records its own mutations for this block; `take_journal` collects them
                // below, which also turns recording back off.
                forest.begin_journal();
                let w = build_block_carried(&mut forest, &j, &block_mtp);

                // #54 — advance the coinbase SMT, in the SAME order the guest's apply_block does:
                // check-then-insert the new coinbase, then decrement each spent coinbase output. The
                // guest's proofs are sequenced against intermediate roots, so a different order here
                // produces proofs it will refuse. That coupling is documented in bip30.rs and is the
                // reason this block sits immediately after build_block_carried rather than anywhere
                // more convenient.
                let smt_root_in: [u8; 32] = smt.root();
                let smt_witness = {
                    let cb = block.txdata[0].compute_txid().to_byte_array();
                    // Count only SPENDABLE outputs: an OP_RETURN coinbase output can never be spent,
                    // so counting it would leave the entry permanently nonzero and reject-valid a
                    // legal duplicate for ever. Matches the accumulator's out_spendable rule exactly.
                    let nout = block.txdata[0].output.iter()
                        .filter(|o| out_spendable(o.script_pubkey.as_bytes())).count() as u32;
                    // `utxo` still holds the PRE-block state here, which is what tells us whether a
                    // spent coin was a coinbase output.
                    let mut cb_spends: Vec<[u8; 32]> = Vec::new();
                    for tx in block.txdata.iter().skip(1) {
                        for inp in &tx.input {
                            let key = (inp.previous_output.txid.to_byte_array(), inp.previous_output.vout);
                            if matches!(utxo.get(&key), Some((_, _, _, true))) { cb_spends.push(key.0); }
                        }
                    }
                    smt_advance(&mut smt, cb, nout, &cb_spends, h == 91842 || h == 91880)
                };
                let (smt_witness, smt_prior) = smt_witness;
                let mut w = w;
                w.in_smt_root = smt_root_in;
                w.smt = smt_witness;
                let (utxo_spent, utxo_created) = bridge_update_utxo(&mut utxo, &block, h);
                // ⛔ EVERYTHING THIS BLOCK CHANGED, CAPTURED BY WHOEVER CHANGED IT (hazync#484).
                // Assembled here only because this is where the four structures' deltas meet; not one
                // of them is re-derived. `prev_*` were snapshotted above, before `push_mtp` moved
                // them, and `block_mtp` needs nothing recorded since undoing it is a single pop.
                let undo = BlockUndo {
                    height: h,
                    block_hash: hash.clone(),
                    forest_ops: forest.take_journal().iter().map(UndoForestOp::from).collect(),
                    utxo_spent, utxo_created, smt_prior,
                    prev_win, prev_nbits, prev_time, prev_epoch_start,
                };
                bridge_push_undo(&out_dir, &undo, undo_keep);
                // Below EMIT_FROM the state still advances — forest, utxo, smt, mtp window are all
                // updated above this point, and the checkpoint below still persists them. Only the
                // per-block file is skipped, so a later run can emit any skipped height by replaying
                // from a checkpoint below it. Building the Bundle is also skipped: serialising it is
                // the expensive part (bundles average 8.3 MB at this height) and nothing else reads it.
                if h >= emit_from {
                    let bundle = Bundle { height: h, in_tip, in_roots: s.roots, in_leaves: s.num_leaves,
                        in_nbits, in_time, in_epoch_start, in_recent, witness: w };
                    // atomic bundle write too — a prover polling the dir never reads a half-written bundle
                    let bp = format!("{out_dir}/bundle_{h}.json");
                    std::fs::write(format!("{bp}.tmp"), serde_json::to_vec(&bundle).unwrap()).unwrap();
                    std::fs::rename(format!("{bp}.tmp"), &bp).unwrap();
                }
                let bt = block.header.time; nbits = block.header.bits.to_consensus(); time = bt;
                if h % 2016 == 0 { epoch_start = bt; }
                done = h;
                // ⚠ The hash we ACTUALLY walked, not the one the node offers next time round. This is
                // the whole basis of the check: `hash` came from `getblockhash(h)` at the top of this
                // iteration, and the accumulator has now absorbed that exact block.
                head = Some(hash.clone());
                if done - last_ckpt >= ckpt_every {
                    bridge_save_state(&out_dir, &BridgeStateRef { height: done, leaves: &forest.leaves,
                        utxo: &utxo, win: &win, block_mtp: &block_mtp, nbits, time, epoch_start,
                        smt: smt.entries() });
                    // ⚠ AFTER the checkpoint, never before. The sidecar claims "state.bin is at this
                    // height with this hash"; written first, a crash in between would leave it
                    // describing a checkpoint that does not exist, and the next start would compare
                    // the node against a height the accumulator never reached.
                    if let Some(hh) = head.as_deref() { bridge_save_head(&out_dir, done, hh); }
                    last_ckpt = done;
                    println!("bridge: checkpoint @ {done} ({} utxos, {} leaves)", utxo.len(), forest.leaves.len());
                }
                // "emitted" would be a lie below EMIT_FROM, and a progress line that misreports what the
                // process is doing is how a silently empty bundle dir goes unnoticed for hours.
                if h % 5000 == 0 {
                    let what = if h >= emit_from { "emitted" } else { "advanced, no bundle," };
                    println!("bridge: {what} through {h}/{target}");
                }
            }
            // checkpoint on catch-up so a one-shot run and each tip-follow cycle persist their progress
            if done > last_ckpt {
                bridge_save_state(&out_dir, &BridgeStateRef { height: done, leaves: &forest.leaves,
                    utxo: &utxo, win: &win, block_mtp: &block_mtp, nbits, time, epoch_start,
                        smt: smt.entries() });
                if let Some(hh) = head.as_deref() { bridge_save_head(&out_dir, done, hh); }
                last_ckpt = done;
            }
            println!("bridge: caught up to {done} (node tip {tip})");
        }
        if once { break; }
        std::thread::sleep(std::time::Duration::from_secs(poll)); // follow the tip
    }
}

#[cfg(test)]
mod bridge_undo_tests {
    //! A rewind must restore EVERY structure, not most of them (hazync#484).
    //!
    //! ⛔ WHY THIS IS THE TEST THAT MATTERS. Four things advance per block — forest, UTXO map,
    //! coinbase SMT, header window — and a rewind that restores three of them produces a bridge that
    //! looks healthy and emits bundles built on a state no chain ever had. The failure would surface
    //! days later as a proof the guest refuses, with nothing pointing back here. So every case walks
    //! forward, snapshots all four, walks further, rewinds, and compares all four.
    use super::*;

    fn key(n: u64) -> [u8; 32] {
        let mut k = [0u8; 32];
        k[..8].copy_from_slice(&n.to_le_bytes());
        k
    }

    /// The four structures, as a comparable whole.
    fn snapshot(forest: &Forest, utxo: &Utxo, smt: &Smt, win: &[u32], mtp: &[u32],
                nbits: u32, time: u32, epoch: u32)
        -> (Vec<Option<[u8; 32]>>, usize, usize, [u8; 32], Vec<u32>, usize, u32, u32, u32) {
        (forest.roots(), forest.leaves.len(), utxo.len(), smt.root(),
         win.to_vec(), mtp.len(), nbits, time, epoch)
    }

    /// Advance all four the way the bridge does, and return the block's undo record.
    fn step(h: u32, forest: &mut Forest, utxo: &mut Utxo, smt: &mut Smt,
            win: &mut Vec<u32>, mtp: &mut Vec<u32>,
            nbits: &mut u32, time: &mut u32, epoch: &mut u32) -> BlockUndo {
        let (prev_win, prev_nbits, prev_time, prev_epoch_start) = (win.clone(), *nbits, *time, *epoch);
        mtp.push(*time);
        win.push(1_700_000_000 + h);
        if win.len() > 11 { let n = win.len() - 11; win.drain(0..n); }

        forest.begin_journal();
        // a block's worth of forest work: spend two old coins, create three new ones
        let mut utxo_spent = Vec::new();
        let mut spent_positions: Vec<usize> = Vec::new();
        for s in 0..2u64 {
            if forest.leaves.len() > 4 {
                let pos = ((h as u64 * 7 + s * 13) as usize) % forest.leaves.len();
                if spent_positions.contains(&pos) { continue; }
                spent_positions.push(pos);
                let k = (key(pos as u64), 0u32);
                if let Some(v) = utxo.remove(&k) { utxo_spent.push((k, v)); }
                forest.delete(pos);
            }
        }
        let mut utxo_created = Vec::new();
        for c in 0..3u64 {
            let leaf_id = h as u64 * 1000 + c;
            let mut lb = [0u8; 32];
            lb[..8].copy_from_slice(&leaf_id.to_le_bytes());
            forest.add(hazync_utreexo::hash_leaf(&lb));
            let k = (key(leaf_id), 0u32);
            utxo.insert(k, (5_000_000_000, vec![0x51], h, c == 0));
            utxo_created.push(k);
        }

        // smt: insert this block's coinbase, decrement one earlier one
        let mut smt_prior = Vec::new();
        let cb = key(900_000 + h as u64);
        smt_prior.push((cb, smt.get(&cb)));
        smt.insert(cb, 2);
        if h > 3 {
            let old = key(900_000 + h as u64 - 3);
            if let Some(cur) = smt.get(&old) {
                smt_prior.push((old, Some(cur)));
                if cur > 0 { smt.insert(old, cur - 1); }
            }
        }
        *nbits = 0x1d00_ffff + h;
        *time = 1_700_000_000 + h;
        if h % 2016 == 0 { *epoch = *time; }

        BlockUndo {
            height: h,
            block_hash: format!("{h:064x}"),
            forest_ops: forest.take_journal().iter().map(UndoForestOp::from).collect(),
            utxo_spent, utxo_created, smt_prior,
            prev_win, prev_nbits, prev_time, prev_epoch_start,
        }
    }

    fn fresh() -> (Forest, Utxo, Smt, Vec<u32>, Vec<u32>, u32, u32, u32) {
        let mut forest = Forest::new();
        let mut utxo = Utxo::new();
        for i in 0..40u64 {
            let mut lb = [0u8; 32];
            lb[..8].copy_from_slice(&i.to_le_bytes());
            forest.add(hazync_utreexo::hash_leaf(&lb));
            utxo.insert((key(i), 0), (1000 + i, vec![0x51], 1, false));
        }
        (forest, utxo, Smt::new(), vec![1_700_000_000], vec![1_700_000_000],
         0x1d00_ffff, 1_700_000_000, 1_700_000_000)
    }

    #[test]
    fn a_rewind_restores_all_four_structures() {
        let d = std::env::temp_dir().join(format!("hzundo{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let dir = d.to_str().unwrap();

        let (mut forest, mut utxo, mut smt, mut win, mut mtp, mut nbits, mut time, mut epoch) = fresh();
        let mut done = 0u32;
        for h in 1..=6u32 {
            let u = step(h, &mut forest, &mut utxo, &mut smt, &mut win, &mut mtp,
                         &mut nbits, &mut time, &mut epoch);
            bridge_push_undo(dir, &u, 288);
            done = h;
        }
        let mark = snapshot(&forest, &utxo, &smt, &win, &mtp, nbits, time, epoch);

        for h in 7..=12u32 {
            let u = step(h, &mut forest, &mut utxo, &mut smt, &mut win, &mut mtp,
                         &mut nbits, &mut time, &mut epoch);
            bridge_push_undo(dir, &u, 288);
            done = h;
        }
        assert_ne!(snapshot(&forest, &utxo, &smt, &win, &mtp, nbits, time, epoch), mark,
                   "the walk must actually have changed something, or the rewind proves nothing");

        bridge_rewind_to(dir, 6, &mut done, &mut forest, &mut utxo, &mut smt,
                         &mut win, &mut mtp, &mut nbits, &mut time, &mut epoch)
            .expect("rewind");
        assert_eq!(done, 6);
        assert_eq!(snapshot(&forest, &utxo, &smt, &win, &mtp, nbits, time, epoch), mark,
                   "ROOTS, UTXO count, SMT root and the header window must all come back");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_rewind_past_the_window_refuses_and_changes_nothing() {
        // ⛔ ALL OR NOTHING. A partial rewind leaves four structures at four different heights and
        // the next checkpoint persists that. The range is checked before anything is touched.
        let d = std::env::temp_dir().join(format!("hzundo2{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let dir = d.to_str().unwrap();
        let (mut forest, mut utxo, mut smt, mut win, mut mtp, mut nbits, mut time, mut epoch) = fresh();
        let mut done = 0u32;
        for h in 1..=5u32 {
            let u = step(h, &mut forest, &mut utxo, &mut smt, &mut win, &mut mtp,
                         &mut nbits, &mut time, &mut epoch);
            bridge_push_undo(dir, &u, 2);        // keep only the last 2
            done = h;
        }
        let before = snapshot(&forest, &utxo, &smt, &win, &mtp, nbits, time, epoch);
        let r = bridge_rewind_to(dir, 1, &mut done, &mut forest, &mut utxo, &mut smt,
                                 &mut win, &mut mtp, &mut nbits, &mut time, &mut epoch);
        assert!(r.is_err(), "a rewind beyond the retained window must refuse");
        assert_eq!(done, 5, "and must not have moved the height");
        assert_eq!(snapshot(&forest, &utxo, &smt, &win, &mtp, nbits, time, epoch), before,
                   "and must not have touched a single structure");
        std::fs::remove_dir_all(&d).ok();
    }

    /// ⛔ THE FORK SEARCH IS WHERE A REORG GOES WRONG QUIETLY. Rewinding one block too few leaves
    /// the accumulator holding an abandoned block; one too many is merely wasteful, but a search
    /// that returns the WRONG height looks exactly like a successful recovery.
    #[test]
    fn the_fork_point_is_the_highest_height_the_chains_still_share() {
        // ours 1..=10; the node forked after 7, so 8,9,10 differ
        let ours = |h: u32| Some(format!("{h:064x}"));
        let node = |h: u32| Some(if h <= 7 { format!("{h:064x}") } else { format!("{:064x}", h + 500) });
        assert_eq!(bridge_fork_point(1, 10, node, ours), Some(7));
    }

    #[test]
    fn a_one_block_reorg_finds_the_block_below_it() {
        // ⚠ THE COMMON CASE, and the one FINALITY=100 was really guarding against. Bitcoin produces
        // these every few weeks.
        let ours = |h: u32| Some(format!("{h:064x}"));
        let node = |h: u32| Some(if h < 10 { format!("{h:064x}") } else { "ff".repeat(32) });
        assert_eq!(bridge_fork_point(1, 10, node, ours), Some(9));
    }

    #[test]
    fn no_divergence_at_all_reports_the_head() {
        let same = |h: u32| Some(format!("{h:064x}"));
        assert_eq!(bridge_fork_point(1, 10, same, same), Some(10));
    }

    #[test]
    fn a_fork_deeper_than_the_window_is_not_found_and_not_guessed() {
        // ⛔ THE ONE THAT MUST NOT RETURN A NUMBER. Below the undo window there is nothing to rewind
        // to; answering anyway would hand the caller a fork point it cannot act on, and the rewind
        // would then "succeed" against state that was never restored.
        let ours = |h: u32| Some(format!("{h:064x}"));
        let node = |_h: u32| Some("ff".repeat(32));        // differs everywhere in range
        assert_eq!(bridge_fork_point(5, 10, node, ours), None);
    }

    #[test]
    fn a_missing_undo_ends_the_search_rather_than_skipping_past_it() {
        // ⚠ A HOLE IN THE LOG IS NOT A MISMATCH, and the difference only shows when the chains DO
        // agree below the hole. Skipping past it would return 7 — a fork point the rewind can never
        // reach, because undoing 10..8 needs the undo at 8 that is missing. Stopping at the hole
        // reports "cannot", which is the truth.
        let ours = |h: u32| if h == 8 { None } else { Some(format!("{h:064x}")) };
        let node = |h: u32| Some(if h <= 7 { format!("{h:064x}") } else { "ff".repeat(32) });
        assert_eq!(bridge_fork_point(1, 10, node, ours), None,
                   "a hole must end the search, not be walked past to an unreachable fork point");
    }

    #[test]
    fn the_ring_prunes_and_keeps_exactly_what_it_promises() {
        let d = std::env::temp_dir().join(format!("hzundo3{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let dir = d.to_str().unwrap();
        let (mut forest, mut utxo, mut smt, mut win, mut mtp, mut nbits, mut time, mut epoch) = fresh();
        for h in 1..=20u32 {
            let u = step(h, &mut forest, &mut utxo, &mut smt, &mut win, &mut mtp,
                         &mut nbits, &mut time, &mut epoch);
            bridge_push_undo(dir, &u, 5);
        }
        assert_eq!(bridge_undo_floor(dir), Some(15), "keep=5 at height 20 retains 15..20");
        assert!(bridge_load_undo(dir, 20).is_some());
        assert!(bridge_load_undo(dir, 14).is_none(), "older than the window is gone");
        std::fs::remove_dir_all(&d).ok();
    }
}

#[cfg(test)]
mod bridge_reorg_tests {
    use super::{bridge_reorg_seen, bridge_save_head, bridge_load_head};

    const A: &str = "00000000000000000001a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f6";
    const B: &str = "000000000000000000029f8e7d6c5b4a39281706f5e4d3c2b1a09f8e7d6c5b4a";

    /// ⛔ "CANNOT CHECK" IS ITS OWN ANSWER. Folding it into "no reorg" is how `finality = 100` got to
    /// stand in for a check for this long: a guard that cannot fail is not a guard.
    #[test]
    fn no_head_means_cannot_check_not_all_clear() {
        assert_eq!(bridge_reorg_seen(None, A), None, "no head must NOT read as 'no reorg'");
        assert_eq!(bridge_reorg_seen(Some("short"), A), None, "a malformed head cannot answer");
        assert_eq!(bridge_reorg_seen(Some(A), "short"), None, "a malformed node hash cannot answer");
    }

    #[test]
    fn same_block_is_no_reorg_and_a_different_one_is() {
        assert_eq!(bridge_reorg_seen(Some(A), A), Some(false));
        assert_eq!(bridge_reorg_seen(Some(A), B), Some(true), "a changed hash at `done` IS a reorg");
        // ⚠ bitcoind prints lowercase; a future caller normalising differently must not read as a reorg
        assert_eq!(bridge_reorg_seen(Some(&A.to_uppercase()), A), Some(false));
    }

    #[test]
    fn the_head_sidecar_round_trips_and_a_missing_one_is_none() {
        let d = std::env::temp_dir().join(format!("hzhead{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let dir = d.to_str().unwrap();
        assert_eq!(bridge_load_head(dir), None, "no sidecar yet");
        bridge_save_head(dir, 968_111, A);
        assert_eq!(bridge_load_head(dir), Some((968_111u32, A.to_string())));
        // a truncated sidecar is unreadable, not a silent pass
        std::fs::write(format!("{dir}/state.head"), "968111\n").unwrap();
        assert_eq!(bridge_load_head(dir), None, "a sidecar with no hash cannot answer");
        std::fs::remove_dir_all(&d).ok();
    }

    /// ⛔⛔ THE CHECKPOINT FORMAT MUST NOT CHANGE. `bridge_load_state` swallows a deserialize failure
    /// as `None`, and the bridge then announces "no checkpoint — starting from genesis" and walks
    /// from block 0. The live state.bin is 19.4 GB and took about EIGHT DAYS to build, so adding a
    /// field to `BridgeState` does not cost a migration, it costs a week — and it looks like a normal
    /// startup in the log. This is why the head lives in a sidecar.
    #[test]
    fn the_head_is_not_a_field_on_the_checkpoint() {
        let src = include_str!("bridge.rs");
        let start = src.find("struct BridgeState {").expect("BridgeState");
        let body = &src[start..start + src[start..].find('}').unwrap()];
        for forbidden in ["head", "hash", "block_hash"] {
            assert!(!body.contains(forbidden),
                    "BridgeState gained a `{forbidden}` field — every existing checkpoint becomes \
                     unreadable and the bridge silently restarts from genesis");
        }
    }
}
