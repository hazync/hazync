//! libzync: the libzync commands (engine-export, libzync-hints, libzync-prove-range, libzync-join, libzync-verify), the
//! aggregate's stream and hint block, and the coin-log / trie synthesis (cl_synth_around, cl_synth). Every guest run is
//! on the OpenVM engine (engine_backend); RISC Zero's executor and prover went with CUTOVER C8 step 4.

use bitcoin::Transaction;
use bitcoin::consensus::deserialize;
use bitcoin::hashes::Hash as _;
use hazync_coinbase_smt::Smt;
use crate::{BlockWitness, CHAIN_STATE_VERSION, ChainState, KIND_CHAIN, PackedHashes, WireStump, arr,
    build_full, demo_block_forest, genesis_anchor, genesis_params, header_hash, hex, hx, lineage_of,
    normalize_host, pinned_lineage, read_block_json, rev, sha256_host, smt_advance_ex, state_journal_bytes};
use crate::chunks::{chunk_block_hash, chunk_bounds, write_chunk_inputs};
use crate::verify::{MAX_FUTURE_BLOCK_TIME, time_ok, verifier_clock};

/// libzync: the per-block hint bundle at $HAZYNC_LZ_HINTS, if there is one for THIS block. None means
/// collect mode (the guest logs what it needs). A bundle for another block is refused, not applied.
pub(crate) fn lz_bundle(height: u32, block_hash: &[u8; 32]) -> Option<libzync::host::BlockBundle> {
    let path = std::env::var("HAZYNC_LZ_HINTS").ok()?;
    if !std::path::Path::new(&path).exists() { return None; }
    let b = libzync::host::BlockBundle::read(&path).unwrap_or_else(|e| panic!("{e}"));
    assert!(b.height == height && &b.block_hash == block_hash,
            "{path} is the hint bundle of block {} ({}), not of block {height}", b.height, hex(&b.block_hash));
    Some(b)
}

/// The profile word this host's image commits (guest `chain_profile`).
pub(crate) fn expected_profile() -> u32 {
    methods::LIBZYNC as u32 | (methods::LIBZYNC_ASSUMEVALID as u32) << 1 | ((methods::NETWORK == "regtest") as u32) << 3 | ((methods::NETWORK == "signet") as u32) << 2
        | (methods::COINLOG as u32) << 4 | (methods::TRIE as u32) << 5 | (methods::UTXOHASH as u32) << 6
        | 1 << 7   // node format 2 (COINLOG.md §2)
}

/// libzync-verify's outcomes are 0, 2 and 1 only: any panic (bad arguments, unreadable or malformed inputs)
/// is a FAIL with exit 1. Writes ignore errors, since a panic inside a hook aborts (a closed stdout pipe).
pub(crate) fn libzync_verify_fail_hook() {
    std::panic::set_hook(Box::new(|i| {
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), "LIBZYNC_VERIFY_FAIL: {}", format!("{i}").escape_debug());
        std::process::exit(1);
    }));
}
/// An image id as `method-id` prints it (the LE bytes of the words).
pub(crate) fn hex_id(id: &[u32; 8]) -> String { hex(&id.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>()) }

/// `libzync-verify <proof> [anchor.json]`: everything a user checks before trusting a Hazync chain proof. Exit 0
/// only if every check passes (2 = DEMO: a valid chain not shown to be the network's).
///
///  1. the engine proof (P.proof.json + P.journal) verifies under THIS host's engine keys (HAZYNC_ENGINE_KEYS: the
///     image for the network), and the journal is a ChainState whose self_id is that image (so every recursion step
///     ran the same guest);
///  2. anchor_id is the digest of the network's GENESIS anchor -- otherwise the chain started from a
///     state the prover asserted, and nothing below means anything (the base step is not verified);
///  3. the profile is this image's, the block forest has one leaf per block, and the fields of features the image
///     lacks (live-coin hash, trie root) are zero; time-too-new on this machine's clock.
///
/// It prints tip_hash and cum_work too: the proof is about THAT chain, and a client must check it is the chain it
/// follows (--expect / --mempool / --bitcoin-cli). If 1-3 hold, the coin log (utxo_roots, utxo_leaves) is exactly
/// the UTXO set history to H, and the trie root and live-coin hash are those of the chain to H.
///
/// With [anchor.json] (a ChainState written by a trusted run), check 2 is against that state instead:
/// for demonstration proofs that start mid-chain. The output says so.
/// The genesis anchor_id pinned per network (swiftsync-verify `Image::genesis_anchor_hex`); none for regtest.
pub(crate) fn pinned_genesis_anchor() -> Option<[u8; 32]> {
    let h = match methods::NETWORK {
        "mainnet" => "852c5c7e009317c2dd232dcf9b1a9e2babd58df324add461a6ff61824f390167",
        "signet" => "3b67ead0618b7786b001525b68c7b088405d6a577292838361cc07e5bb938827",
        _ => return None,
    };
    Some(arr(hx(h)))
}

/// Bitcoin Core v32.1's consensus.nMinimumChainWork for this network (src/kernel/chainparams.cpp at v32.1), little-endian like cum_work (0 on regtest).
pub(crate) fn min_chain_work() -> [u8; 32] {
    let h = match methods::NETWORK {
        "mainnet" => "000000000000000000000000000000000000000145ec036acc5ba740052af1a0",
        "signet" => "00000000000000000000000000000000000000000000000000001090e9dc1520",
        _ => return [0; 32],
    };
    arr(rev(hx(h)))
}

/// The height of the checkpoint the guest pins (prover/methods/guest/src/checkpoint.rs; 0 on regtest). Below it a proof
/// does not show that its chain contains that block: at best DEMO, as for low work.
pub(crate) fn checkpoint_height() -> u32 {
    match methods::NETWORK { "mainnet" => 970_000, "signet" => 325_000, _ => 0 }
}

#[cfg(test)]
mod checkpoint_tests {
    /// The host's checkpoint heights are the guest's (review 11-03: they were hard-coded with no tie).
    #[test]
    fn checkpoint_heights_match_the_guest() {
        let g = include_str!("../../methods/guest/src/checkpoint.rs");
        assert!(g.contains("Some((970_000, ["), "mainnet");
        assert!(g.contains("Some((325_000, ["), "signet");
        assert!(g.contains("pub const CHECKPOINT: Option<(u32, [u8; 32])> = None;"), "regtest");
    }
}

/// cum_work (little-endian) >= min (little-endian).
pub(crate) fn work_at_least(w: &[u8; 32], min: [u8; 32]) -> bool {
    (0..32).rev().map(|i| w[i].cmp(&min[i])).find(|o| o.is_ne()).map_or(true, |o| o.is_gt())
}

/// The block hash at `height` from an esplora-compatible API (mempool.space by default), display order.
pub(crate) fn mempool_block_hash(base: &str, height: u32) -> Result<(String, String), String> {
    let base = if base.is_empty() {
        if methods::NETWORK == "signet" { "https://mempool.space/signet/api".to_string() } else { "https://mempool.space/api".to_string() }
    } else { base.trim_end_matches('/').to_string() };
    let url = format!("{base}/block-height/{height}");
    let o = std::process::Command::new("curl").args(["-sf", "--max-time", "30", &url]).output().map_err(|e| format!("curl {url}: {e}"))?;
    if !o.status.success() { return Err(format!("{url}: request failed ({})", o.status)); }
    Ok((url, String::from_utf8_lossy(&o.stdout).trim().to_lowercase()))
}

/// The block hash at `height` from the user's own node: `<cmd> getblockhash <height>` (display order).
pub(crate) fn node_block_hash(cmd: &str, height: u32) -> Result<String, String> {
    let mut parts = cmd.split_whitespace();
    let prog = parts.next().ok_or("empty --bitcoin-cli")?;
    let o = std::process::Command::new(prog).args(parts).args(["getblockhash", &height.to_string()]).output()
        .map_err(|e| format!("{prog}: {e}"))?;
    if !o.status.success() { return Err(format!("{cmd} getblockhash {height}: {}", String::from_utf8_lossy(&o.stderr).trim())); }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_lowercase())
}

pub(crate) fn libzync_verify_cmd(receipt: &str, anchor: Option<&str>, expect: Option<&str>, mempool: Option<&str>, cli: Option<&str>) {
    libzync_verify_fail_hook(); // also installed by the dispatcher, before the arguments are read
    let mut fails = 0;
    let mut check = |ok: bool, what: &str| {
        println!("  [{}] {what}", if ok { "ok" } else { "FAIL" });
        if !ok { fails += 1; }
    };
    // `receipt` is an engine proof (P.proof.json + P.journal). The proof itself is checked by the engine (STARK against
    // its keys, public values = sha256(journal), the recorded image); everything after is the ChainState check, with
    // self_id = the engine's image id. (A RISC Zero receipt is checked with v0.27.1's verifier, its last release.)
    crate::engine_backend::require();
    let (cs, my_id): (ChainState, [u32; 8]) = {
        let bin = std::env::var("HAZYNC_ENGINE").unwrap();
        let keys = std::env::var("HAZYNC_ENGINE_KEYS").unwrap();
        let o = std::process::Command::new(&bin).args(["verify", "--dir", &keys, "--proof", receipt]).output()
            .unwrap_or_else(|e| panic!("HAZYNC_ENGINE={bin}: {e}"));
        check(o.status.success(), &format!("engine proof verifies (STARK vs {keys}/agg.vk + baseline, public values = sha256(journal), recorded image){}",
            if o.status.success() { String::new() } else { format!(": {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("")) }));
        let j = std::fs::read(format!("{receipt}.journal")).unwrap_or_else(|e| panic!("{receipt}.journal: {e}"));
        (crate::engine_backend::decode_state(&j), crate::engine_backend::image_id_words(&keys))
    };

    // A later version only appends fields (HANDOVER.md §2): this reader checks the prefix it knows.
    check(cs.kind == KIND_CHAIN && cs.version >= CHAIN_STATE_VERSION && cs.self_id == my_id,
          &format!("journal is a ChainState v{} committed by this image (kind, version, self_id)", cs.version));
    // The profile is committed by the image itself (bits from the compiled code, never from input), so the
    // proof states what was checked; it must also be what this verifier was built to accept.
    check(cs.profile == expected_profile(), &(format!("profile in the proof 0x{:x} == this verifier's 0x{:x} ({}{}{}{})",
        cs.profile, expected_profile(), if cs.profile & 1 != 0 { "libzync" } else { "canonical" },
        if cs.profile & 2 != 0 { ", ASSUMEVALID: scripts and signatures NOT checked" } else { ", scripts checked" },
        if cs.profile & 4 != 0 { ", signet" } else { ", mainnet" }, if cs.profile & 16 != 0 { ", coin log" } else { "" }) + if cs.profile & 32 != 0 { ", counting trie" } else { "" }
        + if cs.profile & 64 != 0 { ", live-coin hash" } else { "" } + if cs.profile & 128 != 0 { ", node format 2" } else { "" }));
    // time-too-new (Core ContextualCheckBlockHeader): no block more than 2 h past THIS node's clock. The proof
    // has no clock; it commits the largest timestamp and the rule is applied here, as Core applies it to
    // every block during IBD. HAZYNC_VERIFY_NOW overrides the clock for tests, and is itself a failed check.
    let (now, ovr) = verifier_clock();
    if let Some(v) = &ovr {
        check(false, &format!("this verifier's clock is real (HAZYNC_VERIFY_NOW={v} is set: test only)"));
    }
    check(time_ok(cs.max_time, now), &format!(
        "time-too-new: largest block time {} <= this clock {} + {} s", cs.max_time, now, MAX_FUTURE_BLOCK_TIME));
    // The anchor's link digest, as the guest's base step commits it: sha256d of its journal with ITS anchor_id zeroed
    // (main.rs link_digest). Hashing the state as it is gave a different value for any anchor whose own anchor_id is
    // not zero (cold review 11-06: a genuine proof from such an anchor failed here). Genesis's anchor_id is zero.
    let (mut want_anchor, what) = match anchor {
        None => (crate::link_digest(&genesis_anchor()), "genesis".to_string()),
        Some(p) => {
            let a: ChainState = serde_json::from_str(&std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{p:?}: {e}"))).unwrap_or_else(|e| panic!("{p:?}: {e}"));
            (crate::link_digest(&a), format!("the GIVEN anchor at height {} ({p:?}) -- NOT genesis", a.height))
        }
    };
    // Genesis anchors are PINNED per network (as swiftsync-verify's Image::genesis_anchor_hex): a chain keeps its
    // anchor_id across every handover, so a later image must not recompute it in its own layout (review 11-03).
    // The pin is used as it is, never re-derived here: a later layout's genesis_anchor() would differ from it by design.
    // That this layout's anchor equals the pin is checked by swiftsync-verify's unit test and genesis_anchor_id.py.
    if anchor.is_none() {
        if let Some(p) = pinned_genesis_anchor() { want_anchor = p; }
    }
    check(cs.anchor_id == want_anchor, &format!("anchor_id is {what}"));
    // Core's nMinimumChainWork (v32.1): below it, a valid chain from genesis is no evidence of the network's chain.
    let low_work = !work_at_least(&cs.cum_work, min_chain_work()) || cs.height < checkpoint_height();
    println!("  height {} {} the pinned checkpoint height {}", cs.height, if cs.height < checkpoint_height() { "<" } else { ">=" }, checkpoint_height());
    // The coin log (COINLOG.md) is sound at every block: nothing to close at H. The live-coin ECMH (instant IBD;
    // live_set / live_hash) and the counting trie's root are each zero exactly when its feature is off.
    check(methods::UTXOHASH || (cs.live_set == 0 && cs.live_hash == [0; 16]), "live-coin hash unused in this image (live_set == 0)");
    check(methods::TRIE || cs.trie_root == [0; 32], "trie root unused in this image (zero)");
    if methods::UTXOHASH { println!("  live-coin hash: {}", if cs.live_set == 0 { "the empty set".to_string() } else { format!("x {:08x}..", cs.live_hash[7]) }); }
    if methods::TRIE { println!("  trie root: {}", hex(&cs.trie_root)); }
    // As swiftsync-verify: a root exactly at each set bit of block_leaves (HANDOVER.md §6).
    let shape = (0..64).all(|i| ((cs.block_leaves >> i) & 1 == 1) == cs.block_roots.get(i).copied().flatten().is_some());
    check(cs.block_leaves == cs.height as u64 + 1 && shape, &format!("block forest has one leaf per block ({} leaves at height {})", cs.block_leaves, cs.height));
    let full = lineage_of(pinned_lineage());
    check(cs.lineage == [0; 32] || cs.lineage == full, &format!("lineage {} is this image's ({} handover(s) pinned)", hex(&cs.lineage), pinned_lineage().len()));
    println!("  chain: tip {} (display order {}), cum_work 0x{}", hex(&cs.tip_hash),
             hex(&rev(cs.tip_hash.to_vec())), hex(&rev(cs.cum_work.to_vec())));
    println!("  {}: {} coins, {} sat, roots {:?}", if methods::COINLOG { "coin log (every coin ever appended)" } else { "UTXO set at H" }, cs.utxo_leaves, cs.utxo_value,
             cs.utxo_roots.iter().map(|x| x.map(|h| hex(&h[..8]))).collect::<Vec<_>>());
    println!("  cum_work 0x{} {} Core's nMinimumChainWork 0x{}", hex(&rev(cs.cum_work.to_vec())), if low_work { "<" } else { ">=" },
             hex(&rev(min_chain_work().to_vec())));
    // Chain identity: is the proven tip THE network's block at that height? The proof alone cannot say (a valid fork
    // has its own valid tip); a source the user trusts can, in one comparison.
    let tip = hex(&rev(cs.tip_hash.to_vec()));
    let mut identity = false;
    if let Some(e) = expect {
        match e.split_once(':').and_then(|(h, x)| Some((h.parse::<u32>().ok()?, x.to_lowercase()))) {
            Some((h, x)) => { check(h == cs.height && x == tip, &format!("--expect: block {h} is {x}; the proof's tip is block {} {tip}", cs.height)); identity = true; }
            None => check(false, &format!("--expect {e:?}: want <height>:<64 hex digits>")),
        }
    }
    if let Some(b) = mempool {
        match mempool_block_hash(b, cs.height) {
            Ok((url, got)) => { check(got == tip, &format!("{url} says block {} is {got:?}; the proof's tip is {tip}", cs.height)); identity = true; }
            Err(e) => check(false, &format!("{e}: chain identity not checked")),
        }
    }
    if let Some(c) = cli {
        match node_block_hash(c, cs.height) {
            Ok(got) => { check(got == tip, &format!("your node ({c}) says block {} is {got:?}; the proof's tip is {tip}", cs.height)); identity = true; }
            Err(e) => check(false, &format!("{e}: chain identity not checked")),
        }
    }
    let id_note = if identity { String::new() } else {
        format!(" (chain identity NOT checked: compare block {} = {tip} with a source you trust, --bitcoin-cli, --mempool or --expect)", cs.height)
    };
    if fails == 0 && anchor.is_none() && low_work {
        println!("LIBZYNC_VERIFY_DEMO: a valid chain from genesis at height {}, but below Core's minimum chainwork or the pinned checkpoint for {}: not shown to be the network's chain", cs.height, methods::NETWORK);
        std::process::exit(2);
    }
    if fails == 0 && anchor.is_some() {
        // Never the OK line: the base state was the prover's (or a sweep's) assertion, not genesis, so the
        // proof says nothing about history before it (review 2).
        println!("LIBZYNC_VERIFY_DEMO: a valid chain to height {} FROM THE GIVEN ANCHOR ONLY -- not a proof from genesis", cs.height);
        std::process::exit(2);
    } else if fails == 0 {
        // The verdict line names an assumevalid profile, so a reader of this line alone (or a script) cannot
        // take a scripts-skipped proof for a full one (review 5).
        println!("LIBZYNC_VERIFY_OK: the chain and its coin log are proven at height {}{}{id_note}", cs.height,
                 if cs.profile & 2 != 0 { " (ASSUMEVALID: scripts and signatures NOT checked)" } else { "" });
    } else {
        println!("LIBZYNC_VERIFY_FAIL: {fails} check(s) failed");
        std::process::exit(1);
    }
}

pub(crate) fn write_aggregate_env(
    b: &mut impl crate::sink::Sink,
    receipts: &[impl crate::engine_backend::Proven],
    anchor: &ChainState,
    w: &BlockWitness,
) {
    write_aggregate_env_ex(b, receipts, anchor, None, w, 1);
}

/// `step` is the guest's is_base. 1: a base step on `anchor`. 0: `anchor` is the PREVIOUS block's committed chain
/// state, whose proof the caller passes to the engine as a child -- the guest verifies it recursively instead of
/// trusting it. 2: the same, with a proof of the predecessor image (a handover).
/// `prev_journal`: for steps 0 and 2, the previous proof's journal BYTES, sent as they are. Re-encoding the decoded
/// state through this host's struct only round-trips while the layouts match, which is exactly what can change at a
/// handover (review 11-03), and the guest's env::verify needs the proof's own bytes.
/// Bytes 4..36 are this host's image id (engine_backend::image_id); the engine puts its own there when it runs the
/// stream (`self_id`), so the two agree for any host set up for the image it proves.
pub(crate) fn write_aggregate_env_ex(
    b: &mut impl crate::sink::Sink,
    receipts: &[impl crate::engine_backend::Proven],
    anchor: &ChainState,
    prev_journal: Option<&[u8]>,
    w: &BlockWitness,
    step: u32,
) {
    b.write(&5u32);
    b.write(&crate::engine_backend::image_id());
    b.write(&(receipts.len() as u32));
    // #136's fix for the AGGREGATE (`agg-readslice`, part of the libzync profile): a length prefix then a
    // 4-byte-padded slice, exactly as write_chunk_inputs does.
    {
        fn padded(v: &[u8]) -> Vec<u8> {
            let mut p = v.to_vec();
            p.resize(v.len().div_ceil(4) * 4, 0);
            p
        }
        for r in receipts {
            b.write(&(r.journal_bytes().len() as u32));
            b.write_slice(&padded(r.journal_bytes()));
        }
        let sj = prev_journal.map(|j| j.to_vec()).unwrap_or_else(|| state_journal_bytes(anchor));
        b.write(&(sj.len() as u32));
        b.write_slice(&padded(&sj));
    }
    b.write(w);
    // Negative-test hook (test-only, inert unless set): send is_base = 2, which the guest must refuse.
    let raw_base = if std::env::var("HAZYNC_TEST_ISBASE2").is_ok() { 2u32 } else { step };
    b.write(&raw_base);
}

/// `engine-export <dir>`: one block (HAZYNC_BLOCK) written out for the engine's own tools (`hzengine prove-block`,
/// the cost and negative-test scripts), every execution on the ENGINE (HAZYNC_ENGINE + HAZYNC_ENGINE_KEYS): every
/// chunk's input stream (chunk_NN.in) and the journal its execution committed (chunk_NN.r0journal: the name kept from
/// the RISC Zero era, when RISC Zero's executor wrote it; it is the reference journal the aggregate's stream carries and
/// hzengine prove-block checks) with its engine cost (chunk_NN.cycles: the scheduler's size, largest first); the collect
/// run on the engine (dry: it takes no proofs); the trie update (and its split parts) executed on the engine
/// (trie.r0journal); this image's id (method_id: the engine image, as bytes 4..36 of agg.in carry it). The aggregate
/// itself is not executed here: it verifies its children in-guest and there are no proofs yet (hzengine prove-block
/// proves it with the chunk and trie proofs as children); agg.in is written for it.
pub(crate) fn engine_export(dir: &str) {
    use crate::sink::Stream;
    use crate::engine_backend::JournalOnly;
    crate::engine_backend::require();
    std::fs::create_dir_all(dir).unwrap();
    let put = |name: &str, b: &[u8]| std::fs::write(format!("{dir}/{name}"), b).unwrap_or_else(|e| panic!("{dir}/{name}: {e}"));
    let idb: Vec<u8> = crate::engine_backend::image_id().iter().flat_map(|w| w.to_le_bytes()).collect();
    put("method_id", &idb);
    *EXPORT_DIR.lock().unwrap() = Some(dir.to_string());
    *crate::engine_backend::TRIE.lock().unwrap() = None;
    let (anchor, mut w) = build_full();
    let bounds = chunk_bounds(&w, crate::chunks::nchunks_for(&w));
    let pred: u64 = crate::chunks::input_costs(&w).iter().sum();
    println!("  predicted cost {pred} ({} inputs, {} chunks); executions on the ENGINE", w.inputs.len(), bounds.len());
    put("predicted_cost", format!("{pred}\n").as_bytes());
    let mut receipts: Vec<JournalOnly> = Vec::with_capacity(bounds.len());
    for (i, &(lo, hi)) in bounds.iter().enumerate() {
        let mut st = Stream::default();
        {
            use crate::sink::Sink;
            st.write(&4u32);
            st.write(&w.height);
            st.write(&chunk_block_hash(&w.header));
        }
        write_chunk_inputs(&mut st, &w, lo, hi);
        put(&format!("chunk_{i:02}.in"), &st.0);
        let x = crate::engine_backend::with(dir, |e| e.try_execute(&format!("chunk_{i:02}"), &st.0))
            .unwrap_or_else(|e| panic!("chunk {i} execution on the engine: {e}"));
        let j = x.journal.unwrap_or_else(|| panic!("chunk {i}: exit {:?}, no journal", x.exit));
        put(&format!("chunk_{i:02}.r0journal"), &j);
        put(&format!("chunk_{i:02}.cycles"), format!("{}\n", x.cost).as_bytes());
        println!("  chunk {i:>3} [{lo}..{hi}) exported: {} bytes in, engine cost {}", st.0.len(), x.cost);
        receipts.push(JournalOnly(j));
    }
    libzync_agg_prep(&mut w);
    let mut anchor = anchor;
    if methods::COINLOG { cl_synth_around(&receipts, &mut anchor, &mut w); }
    if let Some(t) = crate::engine_backend::TRIE.lock().unwrap().as_ref() { put("trie.r0journal", &t.journal); }
    let mut st = Stream::default();
    write_aggregate_env(&mut st, &receipts, &anchor, &w);
    libzync_agg_tail(&mut st, &w);
    assert!(st.0[4..36] == idb[..], "aggregate stream: the image id is not at bytes 4..36");
    put("agg.in", &st.0);
    put("chunks", format!("{}\n", bounds.len()).as_bytes());
    println!("ENGINE_EXPORT_DONE {dir}: {} chunks, aggregate stream {} bytes (executed on the engine; the aggregate is proved by prove-block)", bounds.len(), st.0.len());
}

/// Set by engine_export: where cl_synth_around writes the collect run's stream (agg_collect.in).
static EXPORT_DIR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Coin log, single-block measurement (engine-export): a collect run on the engine for the block's spends, then a
/// modelled log around them (cl_synth) set as the anchor's coin state. A demonstration anchor: costs, not history. The
/// modelled trie update (mode 17) is executed on the engine; its journal is what the aggregate's stream carries.
pub(crate) fn cl_synth_around(receipts: &[impl crate::engine_backend::Proven], anchor: &mut ChainState, w: &mut BlockWitness) {
    let edir = EXPORT_DIR.lock().unwrap().clone().unwrap_or_else(|| ".".into());
    // collect run for the block's spends, then a modelled log around them (cl_synth)
    let hints = std::env::var("HAZYNC_LZ_HINTS").ok();
    std::env::remove_var("HAZYNC_LZ_HINTS");
    // engine-export: the collect run's own stream too (agg_collect.in), written BEFORE the engine runs it, so a block
    // refused here (a bad header or coinbase: refused before any commit) can be given to the engine's tools as well
    if let Some(dir) = EXPORT_DIR.lock().unwrap().as_ref() {
        let mut st = crate::sink::Stream::default();
        write_aggregate_env(&mut st, receipts, anchor, w);
        libzync_agg_tail(&mut st, w);
        std::fs::write(format!("{dir}/agg_collect.in"), &st.0).unwrap_or_else(|e| panic!("{dir}/agg_collect.in: {e}"));
    }
    // the collect run on the engine: DRY (it takes no proofs and can never commit), as libzync-prove-range's
    let log = {
        let mut st = crate::sink::Stream::default();
        write_aggregate_env(&mut st, receipts, anchor, w);
        libzync_agg_tail(&mut st, w);
        let x = crate::engine_backend::with(&edir, |e| e.execute_ex("agg_collect", &st.0, &[], true, true));
        assert!(x.exit == Some(libzync::wire::COLLECT_EXIT as u32), "coin-log collect run: exit {:?}, not a finished collect run", x.exit);
        x.log.into_bytes()
    };
    if let Some(h) = hints { std::env::set_var("HAZYNC_LZ_HINTS", h); }
    let (spends, appends) = cl_parse(&String::from_utf8_lossy(&log));
    let (roots, n, wit) = cl_synth(&std::env::var("HAZYNC_BLOCK").unwrap(), &spends);
    // The counting trie: the keys the collect run logged. The aggregate takes no trie witness (the trie is applied in its
    // own proof, guest mode 17); for measurement the host synthesises the trie's witness around a modelled population.
    if methods::TRIE {
        let (mut marks, ins) = crate::trie::trie_parse(&String::from_utf8_lossy(&log));
        let envu = |k: &str, d: u64| -> u64 { std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d) };
        // MEASUREMENT: the old trie is synthesised around the block's keys (HAZYNC_TTREE_ADDRS addresses, default the
        // coin log's modelled size; HAZYNC_TTREE_SUB other coins under each spent-from address, default 2), and the test
        // anchor's trie root is set to it. The prover (libzync-prove-range) serves the witness from its trie store.
        let (addrs, sub) = (envu("HAZYNC_TTREE_ADDRS", n), envu("HAZYNC_TTREE_SUB", 2));
        // Negative controls (test-only), HAZYNC_TEST_TRIE: "list" = the receipt applies a different update list (one
        // spender changed), "root" = the chain's trie root is not the receipt's old root, "none" = no receipt at all,
        // "counts" = a witness without values and counts (mode 17 must refuse it).
        let neg = std::env::var("HAZYNC_TEST_TRIE").unwrap_or_default();
        let counts = neg != "counts";
        let rem: Vec<[u8; 64]> = marks.iter().map(|x| x.0).collect();
        let ink: Vec<[u8; 64]> = ins.iter().map(|x| x.0).collect();
        let (root, tr) = libzync::trie_model::synth(&rem, &ink, addrs, sub, counts, 0x5eed + 1);
        let wit = libzync::trie_model::encode(counts, &root, &tr);
        if neg == "list" && !marks.is_empty() { marks[0].1[0] ^= 1; }
        if let Ok(d) = std::env::var("HAZYNC_ENGINE_EXPORT") {
            let mut st = crate::sink::Stream::default();
            crate::trie::trie_env(&mut st, &wit, &marks, &ins);
            std::fs::write(format!("{d}/trie.in"), &st.0).expect("engine export: trie.in");
            // HAZYNC_TRIE_SPLIT=k: also the split update (parts at depth k of the top trie + the combine), each part
            // executed on the engine too, its journal checked against the one the combine's stream carries
            if let Some(k) = std::env::var("HAZYNC_TRIE_SPLIT").ok().and_then(|v| v.parse::<u32>().ok()) {
                match crate::trie::trie_split(&wit, &marks, &ins, k) {
                    None => println!("  trie split at depth {k}: no part (left whole)"),
                    Some((parts, comb)) => {
                        for (i, (st, j)) in parts.iter().enumerate() {
                            std::fs::write(format!("{d}/trie_part_{i:02}.in"), &st.0).expect("engine export: trie part");
                            let jb: Vec<u8> = j.iter().flat_map(|x| x.to_le_bytes()).collect();
                            let (got, size) = {
                                let x = crate::engine_backend::with(&edir, |e| e.try_execute(&format!("trie_part_{i:02}"), &st.0))
                                    .unwrap_or_else(|e| panic!("trie part (mode 19) on the engine: {e}"));
                                (x.journal.unwrap_or_default(), format!("engine cost {}", x.cost))
                            };
                            assert!(got == jb, "trie part {i}: the guest's journal != the host's");
                            std::fs::write(format!("{d}/trie_part_{i:02}.r0journal"), &got).expect("engine export: part journal");
                            println!("  trie part {i}: {} bytes in, {size}", st.0.len());
                        }
                        std::fs::write(format!("{d}/trie_combine.in"), &comb.0).expect("engine export: trie_combine.in");
                        std::fs::write(format!("{d}/trie_parts"), format!("{}\n", parts.len())).expect("engine export: trie_parts");
                        println!("  trie split at depth {k}: {} parts, combine {} bytes in", parts.len(), comb.0.len());
                    }
                }
            }
        }
        {
            // the trie update executed on the engine; its journal is what the aggregate's stream carries (engine TRIE)
            let mut st = crate::sink::Stream::default();
            crate::trie::trie_env(&mut st, &wit, &marks, &ins);
            let x = crate::engine_backend::with(&edir, |e| e.try_execute("trie", &st.0)).unwrap_or_else(|e| panic!("trie update (mode 17) on the engine: {e}"));
            let tj = x.journal.unwrap_or_else(|| panic!("trie update: exit {:?}, no journal", x.exit));
            println!("  trie: {} marks, {} inserts, witness {} words; trie update executed on the engine: cost {}", marks.len(), ins.len(), wit.len(), x.cost);
            *crate::engine_backend::TRIE.lock().unwrap() = if neg == "none" { None } else {
                Some(crate::engine_backend::EngineProof { path: std::path::PathBuf::from(format!("{edir}/trie_exec")), journal: tj }) };
            anchor.trie_root = libzync::coinlog::h_of(&root);
            if neg == "root" { anchor.trie_root[0] ^= 1; }
            if !neg.is_empty() { println!("  trie: NEGATIVE CONTROL HAZYNC_TEST_TRIE={neg}"); }
        }
    }
    println!("  coinlog: {} spends of earlier coins, {} appends", spends.len(), appends.len());
    anchor.utxo_roots = roots.clone();
    anchor.utxo_leaves = n;
    // The modelled log holds the coins this block spends, so its value must cover them (utxo_value is checked).
    if anchor.utxo_value == 0 { anchor.utxo_value = 2_000_000_000_000_000; }
    w.root_prev = WireStump { roots, num_leaves: n };
    *CL_WIT.lock().unwrap() = wit;
}

/// libzync: what the aggregate is NOT sent. SwiftSync takes no inclusion proofs; agg-lite takes every
/// transaction fact from the chunk journals, so no transactions, prevouts, inputs or txids at all.
/// Shared by every aggregate stream this host writes (libzync-prove-range, libzync-hints, engine-export).
pub(crate) fn libzync_agg_prep(w: &mut BlockWitness) {
    for inp in w.inputs.iter_mut() {
        inp.proof_i.siblings = PackedHashes(Vec::new());
        inp.proof_last.siblings = PackedHashes(Vec::new());
    }
    w.txs.clear();
    w.tx_prevouts.clear();
    w.inputs.clear();
    w.txids = PackedHashes(Vec::new());
}

/// libzync: the aggregate's ONE hint block, after the witness: ECMH hints from the block's bundle
/// ($HAZYNC_LZ_HINTS; none = the guest collects, logging ECMHREC lines).
pub(crate) fn libzync_agg_tail(b: &mut impl crate::sink::Sink, w: &BlockWitness) {
    let bundle = lz_bundle(w.height, &header_hash(&w.header));
    let words = libzync::wire::AggHints {
        ecmh_collect: bundle.is_none(),
        ecmh: bundle.as_ref().map(|b| b.ecmh.as_slice()).unwrap_or(&[]),
    }.encode();
    b.write(&(words.len() as u32));
    b.write_slice(&words);
    // coin log: the block's witness (empty in a collect run, which logs what the witness must cover)
    if methods::COINLOG {
        let wit = if bundle.is_none() { Vec::new() } else { CL_WIT.lock().unwrap().clone() };
        b.write(&(wit.len() as u32));
        if !wit.is_empty() { b.write_slice(&wit); }
    }
    // txlog: the block's record order as (journal, record) pairs, from HAZYNC_TXLOG_ORDER (one "journal record" pair per
    // line); none = every record in journal order.
    if methods::TXLOG {
        let order: Vec<u32> = std::env::var("HAZYNC_TXLOG_ORDER").ok().map(|p| {
            std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}")).split_whitespace()
                .map(|x| x.parse().unwrap_or_else(|_| panic!("{p}: not a number: {x}"))).collect()
        }).unwrap_or_default();
        assert!(order.len() % 2 == 0, "HAZYNC_TXLOG_ORDER: pairs");
        b.write(&((order.len() / 2) as u32));
        if !order.is_empty() { b.write_slice(&order); }
    }
    // E2: the trie proof's journal (empty in a collect run); the proof is a child the aggregate verifies in-guest
    if methods::TRIE {
        let ej = if bundle.is_none() { None } else { crate::engine_backend::TRIE.lock().unwrap().as_ref().map(|p| p.journal.clone()) };
        let tj: Vec<u32> = ej.map(|b| b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()).unwrap_or_default();
        b.write(&(tj.len() as u32));
        if !tj.is_empty() { b.write_slice(&tj); }
    }
    match &bundle {
        Some(bd) => println!("  libzync: {} bytes of ECMH hints, no hints file", bd.ecmh.len()),
        None => println!("  libzync: ECMH COLLECT (no bundle)"),
    }
}

/// An exclusive lock on each directory a run writes in place (the out dir, HAZYNC_COINLOG_DIR, HAZYNC_TRIE_DIR), held
/// until the process exits: two runs on the same stores interleave their in-place writes and, in the trie, reuse each
/// other's freed slots (round 11). A second run fails at once instead.
pub(crate) fn lock_run_dirs(out: &str) {
    static LOCKS: std::sync::Mutex<Vec<std::fs::File>> = std::sync::Mutex::new(Vec::new());
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    // the leaf bundle directory too (review 11-14, round 2): libzync-reforest rewrites bundles there and must not race prep
    let leaves = std::env::var_os("HAZYNC_PREP_LEAF_BUNDLES").map(std::path::PathBuf::from);
    for d in [Some(std::path::PathBuf::from(out)), cl_dir(), crate::trie::trie_dir(), leaves].into_iter().flatten() {
        std::fs::create_dir_all(&d).unwrap_or_else(|e| panic!("{}: {e}", d.display()));
        // compared as real paths: the same directory given twice or by another path (round 12)
        let c = std::fs::canonicalize(&d).unwrap_or_else(|e| panic!("{}: {e}", d.display()));
        // overlapping directories are refused: a fresh coin log empties its directory, which would take the out dir's
        // files or the other store with it (and the same directory locked twice would block on itself)
        for o in &dirs {
            if c.starts_with(o) || o.starts_with(&c) {
                panic!("{} and {} overlap: give the out dir, HAZYNC_COINLOG_DIR and HAZYNC_TRIE_DIR each their own directory", o.display(), c.display());
            }
        }
        dirs.push(c);
    }
    for d in dirs {
        let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(d.join("LOCK"))
            .unwrap_or_else(|e| panic!("{}/LOCK: {e}", d.display()));
        match f.try_lock() {
            Ok(()) => LOCKS.lock().unwrap().push(f),
            Err(std::fs::TryLockError::WouldBlock) => panic!("{}: another run holds this directory (LOCK): one run per store", d.display()),
            Err(std::fs::TryLockError::Error(e)) => panic!("{}/LOCK: {e}", d.display()),
        }
    }
}

/// Holds `<dir>/LOCK` for the rest of the process, as lock_run_dirs does: refused if another run holds it.
pub(crate) fn lock_dir(d: &str) {
    static LOCKS: std::sync::Mutex<Vec<std::fs::File>> = std::sync::Mutex::new(Vec::new());
    std::fs::create_dir_all(d).unwrap_or_else(|e| panic!("{d}: {e}"));
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(format!("{d}/LOCK"))
        .unwrap_or_else(|e| panic!("{d}/LOCK: {e}"));
    match f.try_lock() {
        Ok(()) => LOCKS.lock().unwrap().push(f),
        Err(std::fs::TryLockError::WouldBlock) => panic!("{d}: another run holds this directory (LOCK): one run per store"),
        Err(std::fs::TryLockError::Error(e)) => panic!("{d}/LOCK: {e}"),
    }
}

/// Writes `bytes` to `path` so a crash leaves the old file or the new one, never a torn one: a temporary file,
/// synced, renamed over `path`, then the directory synced (review 11-06: receipts, states and history.bin were
/// plain writes, and a torn receipt beside a durable coin log made a resume impossible).
pub(crate) fn write_atomic(path: &str, bytes: &[u8]) {
    use std::io::Write as _;
    let tmp = format!("{path}.tmp");
    std::fs::File::create(&tmp).and_then(|mut f| { f.write_all(bytes)?; f.sync_all() })
        .unwrap_or_else(|e| panic!("{tmp}: {e}"));
    std::fs::rename(&tmp, path).unwrap_or_else(|e| panic!("{path}: {e}"));
    if let Some(d) = std::path::Path::new(path).parent().filter(|d| !d.as_os_str().is_empty()) {
        let _ = std::fs::File::open(d).and_then(|f| f.sync_all());
    }
}

/// A chunk's whole input stream (mode 4, height, block hash, write_chunk_inputs), as the engine takes it.
pub(crate) fn chunk_stream(w: &BlockWitness, lo: usize, hi: usize) -> Vec<u8> {
    use crate::sink::Sink;
    let mut st = crate::sink::Stream::default();
    st.write(&4u32);
    st.write(&w.height);
    st.write(&chunk_block_hash(&w.header));
    write_chunk_inputs(&mut st, w, lo, hi);
    st.0
}


/// A chunk's input range `lo..hi` (indices into the block's inputs).
pub(crate) type Span = (usize, usize);
/// One round of `resplit_on_oom`: runs the given chunks, one result each, in order.
pub(crate) type ExecRound<'a, T> = dyn FnMut(&[Span]) -> Vec<Result<T, String>> + 'a;

/// The guest ran out of heap (#794): a chunk whose inputs need more guest memory than the image has stops with this
/// exact message (openvm-platform's heap/bump.rs), at the end of the guest's log, as engine_backend::try_execute reports
/// it. Nothing else is read as out-of-memory: every other failure (a REJECT, a C abort) passes through. (Until CUTOVER
/// C8 the text of risc0-zkvm-platform's bump allocator was read too.)
pub(crate) fn is_guest_oom(msg: &str) -> bool {
    msg.contains("ERROR: Maximum memory exceeded, program terminating.")
}

/// One chunk run on the engine (collect or hinted: write_chunk_inputs decides by the bundle), failures as Err so
/// `resplit_on_oom` can re-split one that ran out of guest memory. A run that ended in env::exit is an Err too.
pub(crate) fn engine_chunk_exec(out: &str, w: &BlockWitness, h: u32, tag: &str, lo: usize, hi: usize)
    -> Result<crate::engine_backend::Exec, String> {
    let st = chunk_stream(w, lo, hi);
    let x = crate::engine_backend::with(out, |e| e.try_execute(&format!("b{h}_{tag}_{lo}_{hi}"), &st))?;
    match x.exit {
        None => Ok(x),
        Some(c) => Err(format!("chunk {lo}..{hi} {tag} run did not commit (exit {c})")),
    }
}

/// Runs `exec` over `bounds` and re-splits every chunk that ran out of guest memory (#794), until each piece runs.
///
/// - Only `is_guest_oom` triggers a re-split. Any other failure is returned as it is, so a block the guest rejects is
///   still rejected: a reject in a chunk that also ran out of memory surfaces in the piece that holds it.
/// - Sound by construction: the re-split only changes chunk WIDTHS (contiguous runs, in order), which the aggregate
///   accepts for any partition; every piece must itself execute (and later prove) in full.
/// - Deterministic: execution is, and `split` (`chunks::split_chunk`) is a function of the block.
/// - Bounded: each cut is strictly inside its chunk, so pieces shrink; a chunk `split` cannot cut (one input, or one
///   tracked transaction) that still runs out of memory is an error naming the reason, not a loop.
///
/// `exec` gets the chunks to run in one round (to run them in parallel or in turn) and returns their results in order:
/// one per chunk, or fewer when it stopped at a failure that is not out-of-memory (then that failure is the last
/// result, and is what this returns: a block that fails is not worked on further).
pub(crate) fn resplit_on_oom<T>(
    bounds: &[Span],
    split: &dyn Fn(usize, usize) -> Result<usize, &'static str>,
    exec: &mut ExecRound<'_, T>,
) -> Result<Vec<(Span, T)>, String> {
    enum Node<T> { Todo((usize, usize)), Done((usize, usize), T) }
    let mut nodes: Vec<Node<T>> = bounds.iter().map(|&b| Node::Todo(b)).collect();
    loop {
        let todo: Vec<(usize, usize)> = nodes.iter().filter_map(|n| match n { Node::Todo(b) => Some(*b), Node::Done(..) => None }).collect();
        if todo.is_empty() { break }
        let results = exec(&todo);
        assert!(results.len() == todo.len() || matches!(results.last(), Some(Err(e)) if !is_guest_oom(e)),
                "resplit_on_oom: one result per chunk, unless the round stopped at a failure");
        let mut results = results.into_iter();
        let mut next = Vec::with_capacity(nodes.len() + todo.len());
        for n in nodes {
            let (lo, hi) = match n { Node::Todo(b) => b, done => { next.push(done); continue } };
            match results.next().expect("resplit_on_oom: a round's results ended before a failure") {
                Ok(t) => next.push(Node::Done((lo, hi), t)),
                Err(e) if is_guest_oom(&e) => {
                    let why = match split(lo, hi) {
                        Ok(mid) if lo < mid && mid < hi => {
                            eprintln!("  libzync: inputs {lo}..{hi} ran out of guest memory: re-split into {lo}..{mid} and {mid}..{hi} (#794)");
                            next.push(Node::Todo((lo, mid)));
                            next.push(Node::Todo((mid, hi)));
                            continue;
                        }
                        Ok(_) => "the split gave a cut outside the chunk",
                        Err(why) => why,
                    };
                    return Err(format!(
                        "inputs {lo}..{hi} run out of guest memory and cannot be split further ({why}): the guest's bump \
                         allocator cannot hold them, only a reclaiming allocator (an image change) can (#794): {e}"));
                }
                Err(e) => return Err(e),
            }
        }
        nodes = next;
    }
    Ok(nodes.into_iter().map(|n| match n { Node::Done(b, t) => (b, t), Node::Todo(_) => unreachable!() }).collect())
}

/// libzync: `libzync-hints <out>` — the block's hint bundle (libzync::host::BlockBundle), from runs on the ENGINE
/// (HAZYNC_ENGINE + HAZYNC_ENGINE_KEYS):
///
/// - every chunk in COLLECT mode (in turn, one engine server); the guest's BATCHREC lines come from its log, and
///   R = u1·G + u2·Q is computed with libsecp (libzync::host::sig_hint);
/// - then the aggregate's DRY collect run (it takes no proofs) on those chunks' journals, for the ECMH leaves.
///
/// ⛔ The hints are untrusted input to the guest (it re-checks every one), so a bug here can only make a chunk fail,
/// never make a bad signature pass. Uses the same chunk layout as proving (HAZYNC_CHUNKS, HAZYNC_CHUNK_TARGET,
/// HAZYNC_PROFILE_LAYOUT), and records it: a bundle applied under another layout is ignored per chunk. A chunk that runs
/// out of guest memory is re-split (#794), and the bundle then records the re-split layout (a WARNING says so).
/// The same work libzync-prove-range does before it proves (its collect pass).
pub(crate) fn libzync_hints_cmd(out: &str) {
    use std::time::Instant;
    use crate::engine_backend::JournalOnly;
    crate::engine_backend::require();
    let t0 = Instant::now();
    std::env::remove_var("HAZYNC_LZ_HINTS");
    let (anchor, mut w) = build_full();
    let work = format!("{out}.work");
    let packed = chunk_bounds(&w, crate::chunks::nchunks_for(&w));
    let costs: std::cell::OnceCell<Vec<u64>> = std::cell::OnceCell::new();
    let split = |lo: usize, hi: usize| crate::chunks::split_chunk(&w.inputs, costs.get_or_init(|| crate::chunks::input_costs(&w)), lo, hi);
    let h = w.height;
    let done = resplit_on_oom(&packed, &split, &mut |todo: &[Span]| {
        let mut r = Vec::with_capacity(todo.len());
        for &(lo, hi) in todo {
            let x = engine_chunk_exec(&work, &w, h, "hint", lo, hi);
            let stop = matches!(&x, Err(e) if !is_guest_oom(e));
            r.push(x);
            if stop { break }
        }
        r
    }).unwrap_or_else(|e| panic!("{}", guest_failure("chunk execution on the engine", &e)));
    let bounds: Vec<Span> = done.iter().map(|(b, _)| *b).collect();
    if bounds != packed {
        eprintln!("WARNING: block {}: chunks ran out of guest memory and were re-split ({} chunks instead of {}): this bundle \
                   records the re-split layout (#794)", w.height, bounds.len(), packed.len());
    }
    let t_chunks = t0.elapsed().as_secs_f64();
    let mut bundle = libzync::host::BlockBundle { height: w.height, block_hash: header_hash(&w.header), ..Default::default() };
    let (mut nrec, mut nvalid) = (0usize, 0usize);
    let mut journals: Vec<JournalOnly> = Vec::with_capacity(done.len());
    for ((lo, hi), x) in &done {
        let mut blob = Vec::new();
        for rec in libzync::host::sig_records(&x.log) {
            let hint = libzync::host::sig_hint(&rec);
            nrec += 1;
            nvalid += (hint[0] == 1) as usize;
            blob.extend_from_slice(&hint);
        }
        bundle.chunks.push((*lo as u32, *hi as u32, blob));
        journals.push(JournalOnly(x.journal.clone().expect("a committed chunk has a journal")));
    }
    let t_batch = t0.elapsed().as_secs_f64();
    libzync_agg_prep(&mut w);
    let mut st = crate::sink::Stream::default();
    write_aggregate_env(&mut st, &journals, &anchor, &w);
    libzync_agg_tail(&mut st, &w);
    let x = crate::engine_backend::with(&work, |e| e.execute_ex(&format!("b{h}_agg_collect"), &st.0, &[], true, true));
    assert!(x.exit == Some(libzync::wire::COLLECT_EXIT as u32), "collect aggregate: exit {:?}, not a finished collect run", x.exit);
    let leaves = libzync::host::ecmh_leaves(&x.log);
    for leaf in &leaves { bundle.ecmh.extend_from_slice(&libzync::host::ecmh_hint(leaf)); }
    bundle.write(out).unwrap_or_else(|e| panic!("{out}: {e}"));
    let _ = std::fs::remove_dir_all(&work);
    println!("libzync-hints: block {} {} chunks on the ENGINE -> {out}", w.height, bounds.len());
    println!("  batch: {nrec} verifies, {nvalid} valid, {} left to libsecp", nrec - nvalid);
    println!("  ecmh: {} operations, {} bytes", leaves.len(), bundle.ecmh.len());
    println!("  time: chunks {t_chunks:.1} s, batch hints {:.1} s, aggregate + ecmh {:.1} s, total {:.1} s",
             t_batch - t_chunks, t0.elapsed().as_secs_f64() - t_batch, t0.elapsed().as_secs_f64());
}

/// libzync (review 2): re-cut a block's BIP30 witness against a RUNNING coinbase SMT, so a chain of blocks
/// is checked against one tree (the guest requires in_smt_root == prev.smt_root). `build_full` cuts each
/// block against a throwaway tree holding only what that block touches; this replaces it.
pub(crate) fn lz_smt_rebind(w: &mut BlockWitness, smt: &mut Smt) {
    let spends: Vec<[u8; 32]> = w.smt.spends.iter().map(|s| s.coinbase_txid).collect();
    let spend_tracked: Vec<bool> = w.smt.spends.iter().map(|s| s.tracked).collect();
    let tracked_new: Vec<([u8; 32], u32)> = w.smt.tracked.iter().map(|t| (t.txid, t.outputs)).collect();
    let grandfathered = w.smt.smt_overwrite.is_some();
    w.in_smt_root = smt.root();
    w.smt = smt_advance_ex(smt, w.smt.coinbase_txid, w.smt.coinbase_outputs, &spends, grandfathered, &spend_tracked, &tracked_new).0;
}

/// The coinbase SMT after blocks 1..=upto, rebuilt from block_<h>.json (the same advance the guest checks),
/// and the largest header timestamp among those blocks (one pass over the files).
pub(crate) fn lz_smt_replay(dir: &str, upto: u32) -> (Smt, u32) {
    let mut smt = Smt::new();
    let mut max_time = 0u32;
    // the replay runs from genesis, so it knows every tracked transaction itself (tracked.rs)
    let mut tracked: std::collections::HashSet<[u8; 32]> = std::collections::HashSet::new();
    for h in 1..=upto {
        let j = crate::read_block_json(dir, h);
        let g = methods::NETWORK == "mainnet" && (h == 91_842 || h == 91_880);
        let (a, new) = crate::tracked::annotate(&j, &smt, crate::tracked::bip34_height(), g, &|t, _, _| tracked.contains(t));
        tracked.extend(new);
        let (_, mut w) = crate::build_full_from(a);
        max_time = max_time.max(u32::from_le_bytes(w.header[68..72].try_into().unwrap()));
        lz_smt_rebind(&mut w, &mut smt);
    }
    (smt, max_time)
}

/// HAZYNC_RIG: pass the guest's verdict lines through, from a run whose stdout was captured, so the rig can compare
/// the guest's reason for a rejection with Core's.
pub(crate) fn rig_echo(log: &[u8]) {
    if std::env::var("HAZYNC_RIG").is_err() { return; }
    for l in String::from_utf8_lossy(log).lines().filter(|l| l.contains("core-reject:") || l.contains("agg-lite:")) {
        println!("{l}");
    }
}

/// A guest execution that failed, said plainly. "C abort()" is the guest dying inside Core's C++: Core threw and would
/// have caught it, and every catch the guest can reach REJECTS the block (guest build.rs REVIEWED_CATCHES; the guest has
/// no unwinding), so it is the block refused, not a crash of the prover. Other failures (a guest assert or REJECT
/// panic) pass through with their own message.
pub(crate) fn guest_failure(what: &str, e: &dyn std::fmt::Display) -> String {
    let m = e.to_string();
    if m.contains("C abort()") {
        format!("BLOCK REFUSED ({what}): the guest aborted inside Core's C++ (a thrown exception; Core rejects such a block): {m}")
    } else {
        format!("{what} failed: {m}")
    }
}

/// `libzync-join <left> <right> <out>` (ROADMAP H-B): one join (guest mode 18) of two adjacent chain proofs of this
/// image on the engine, the right one starting exactly where the left one ends (its anchor_id is the digest of the
/// left's journal); the guest verifies both in-guest and checks they are adjacent. left/right: HZE1 files (the board's)
/// or P.proof.json + P.journal pairs; out: HZE1 when it names one file (.bin, .hze), else a pair. The joined proof has
/// the left's start and the right's tip, and can itself be joined. Leaves: `libzync-prove-range h h <json>
/// state_<h-1>.json <out>` (a base step on any state), or the board's `prove-range-bridge`.
pub(crate) fn lz_join_cmd(left: &str, right: &str, out: &str) {
    use crate::sink::Sink;
    crate::engine_backend::require();
    let (l, r) = (crate::engine_backend::read_any(left), crate::engine_backend::read_any(right));
    let (ls, rs) = (crate::engine_backend::decode_state(&l.journal), crate::engine_backend::decode_state(&r.journal));
    let mut st = crate::sink::Stream::default();
    st.write(&18u32);
    st.write(&crate::engine_backend::image_id());
    st.write(&l.journal);
    st.write(&r.journal);
    let dir = std::path::Path::new(out).parent().map(|d| d.display().to_string()).unwrap_or_else(|| ".".into());
    let t = std::time::Instant::now();
    let j = crate::engine_backend::with(&dir, |e| e.prove("join", &st.0, &[&l, &r], true));
    let j = if crate::engine_backend::wants_hze(out) {
        crate::engine_backend::with(&dir, |e| e.pack(&j.path, std::path::Path::new(out)));
        for ext in ["proof.json", "journal"] { let _ = std::fs::remove_file(crate::engine_backend::with_ext(&j.path, ext)); }
        crate::engine_backend::EngineProof { path: out.into(), journal: j.journal }
    } else {
        crate::engine_backend::keep(&j, out)
    };
    let s = crate::engine_backend::decode_state(&j.journal);
    assert!(s.anchor_id == ls.anchor_id && s.height == rs.height && s.tip_hash == rs.tip_hash, "join: unexpected output state");
    use sha2::Digest;
    println!("JOURNAL_SHA256 {}", hex(&sha2::Sha256::digest(&j.journal)));
    println!("JOINED left tip {} + right tip {} -> {out} (engine), {:.1} s", ls.height, rs.height, t.elapsed().as_secs_f64());
}

/// `libzync-prove-range <lo> <hi> <json dir> <anchor.json> <out dir>`: PROVE blocks lo..=hi as one recursive chain on
/// the engine (HAZYNC_ENGINE + HAZYNC_ENGINE_KEYS; HAZYNC_ENGINE_GPUS / _PER_GPU spread the proofs). Block lo is a base
/// step on the given anchor (the committed state at lo-1, e.g. from `libzync-anchor-json`); every later block's
/// aggregate VERIFIES the previous block's proof in-guest (is_base = 0). Writes block_<h>.ovm (P.proof.json + P.journal)
/// and state_<h>.json per block, and times.tsv.
/// Per block: every chunk executed in collect mode (signature hints), then hinted (its journal); the aggregate's ECMH
/// hints learned by one DRY collect run; the chunks and the trie update proved at once; then the aggregate proved.
/// `<anchor.json>` may be the word `genesis`: the network's genesis state, built here, so lo must be 1; or `leaf` with
/// HAZYNC_LEAF_BUNDLE (one block from prep's bundle). HAZYNC_HANDOVER_PROOF: the first block is a handover step on a
/// pinned engine predecessor (engine/HANDOVER.md). HAZYNC_ENGINE_DEV=1 (a metering image): executions, no proofs (the
/// core-rules rig). Resuming a run (HAZYNC_RESUME) is not supported on the engine yet.
pub(crate) fn lz_prove_range_cmd(lo: u32, hi: u32, dir: &str, anchor_path: &str, out: &str) {
    crate::engine_backend::require();
    lock_run_dirs(out);
    use std::io::Write as _;
    use std::time::Instant;
    std::fs::create_dir_all(out).unwrap();
    assert!(std::env::var_os("HAZYNC_HANDOVER_RECEIPT").is_none(),
            "HAZYNC_HANDOVER_RECEIPT: a handover on a RISC Zero receipt is not available since CUTOVER C8 (an engine chain has no \
             RISC Zero predecessor); an engine predecessor is HAZYNC_HANDOVER_PROOF");
    assert!(std::env::var("HAZYNC_RESUME").as_deref() != Ok("1"), "HAZYNC_RESUME: resume is not supported on the engine yet");
    // HAZYNC_HANDOVER_PROOF=<P> (engine/HANDOVER.md step 2): the first block is a handover step on the pinned ENGINE
    // predecessor's proof P (P.journal, and P.bin: its plain encoding, `hzengine wasm-inputs`) from another OpenVM
    // version: the lineage entry's verifier is that predecessor's key pin and the image is built with ovm-xver, whose
    // xver checks P in-guest. The chain state comes from P's journal (the guest checks P itself); the anchor argument is
    // unused (pass the word `handover`).
    let ehandover: Option<(crate::engine_backend::EngineProof, Vec<u8>, ChainState)> = std::env::var("HAZYNC_HANDOVER_PROOF").ok().map(|p| {
        let last = pinned_lineage().last().expect("HAZYNC_HANDOVER_PROOF: this image has no predecessor pinned");
        assert!(last.verifier != crate::handover_pin::SAME_ZKVM,
                "HAZYNC_HANDOVER_PROOF: the pinned predecessor is on the SAME OpenVM (a child proof, not ovm-xver): not built yet");
        let journal = std::fs::read(format!("{p}.journal")).unwrap_or_else(|e| panic!("{p}.journal: {e}"));
        let bin = std::fs::read(format!("{p}.bin")).unwrap_or_else(|e| panic!("{p}.bin (the proof's plain encoding: hzengine wasm-inputs): {e}"));
        let st = crate::engine_backend::decode_state(&journal);
        assert!(st.kind == KIND_CHAIN && st.self_id == last.image, "{p} is not the predecessor's chain state");
        assert_eq!(st.height + 1, last.first_height, "{p}: the pin hands over at {}", last.first_height);
        (crate::engine_backend::EngineProof { path: p.into(), journal }, bin, st)
    });
    // HAZYNC_LEAF_BUNDLE=<leaf_<h>.bin> (ROADMAP H-B 8): a LEAF proof of the one block h, a base step on S(h-1) taken from
    // prep's bundle with every witness in it: no coin log, trie or BIP30 tree, and no block files but block h's. The
    // proof joins with its neighbours (libzync-join). HAZYNC_LEAF_CHECK=<dir>: an ordinary run that asserts each
    // block's witnesses are byte-identical to <dir>/leaf_<h>.bin's (the test of prep's bundles).
    let leaf: Option<crate::prep::LeafBundle> = std::env::var("HAZYNC_LEAF_BUNDLE").ok().map(|p| crate::prep::leaf_read(&p));
    let leaf_check = std::env::var("HAZYNC_LEAF_CHECK").ok();
    if let Some(b) = &leaf {
        assert!(lo == hi && b.prev.height + 1 == lo, "HAZYNC_LEAF_BUNDLE: the bundle is for block {}; prove exactly that block", b.prev.height + 1);
        crate::tracked::check_bundle_json(lo, &b.block_json);
        assert!(leaf_check.is_none(), "HAZYNC_LEAF_BUNDLE and HAZYNC_LEAF_CHECK: one or the other");
    }
    // the bundle carries the block: written into the out dir, which then serves as the block dir
    let leaf_dir: String;
    let dir = match &leaf {
        Some(b) => {
            write_atomic(&format!("{out}/block_{lo}.json"), &b.block_json);
            leaf_dir = out.to_string();
            leaf_dir.as_str()
        }
        None => dir,
    };
    let mut prev: ChainState = if let Some((_, _, st)) = &ehandover { st.clone() } else if let Some(b) = &leaf { b.prev.clone() } else if anchor_path == "genesis" { genesis_anchor() } else {
        serde_json::from_str(&std::fs::read_to_string(anchor_path).unwrap_or_else(|e| panic!("{anchor_path}: {e}"))).unwrap()
    };
    demo_block_forest(&mut prev, true);
    assert_eq!(prev.height + 1, lo, "anchor is at height {}, range starts at {lo}", prev.height);
    // history.bin starts at height 0 only on a run from genesis (libzync::history).
    let prev_is_genesis = ehandover.is_none() && leaf.is_none() && anchor_path == "genesis";
    // The counting trie and the coin log at the run's start (the trie first, review 11-12, #3: it only checks, while the
    // coin log's load rewinds its journal).
    if methods::TRIE && leaf.is_none() { crate::trie::trie_init_at_start(ehandover.is_some(), anchor_path, &prev); }
    if methods::COINLOG && leaf.is_none() { cl_init_at_start(ehandover.is_some(), anchor_path, &prev); }
    // The BIP30 tree at the anchor. With every block file 1..lo-1 present it is REPLAYED, and must equal the
    // anchor's own smt_root (a sweep anchor from genesis). Otherwise this is a demonstration anchor: the tree
    // is seeded with the coinbases the range spends from before lo, and the anchor's smt_root is SET to it --
    // written to anchor_effective.json, which is what libzync-verify must be given.
    let replay = leaf.is_none() && (1..lo).all(|h| std::path::Path::new(&format!("{dir}/block_{h}.json")).exists());
    // A handover continues a proved chain: its BIP30 tree must be replayed and equal the predecessor's, never seeded (the
    // seeding path rewrites smt_root, and the state sent would no longer be the predecessor's journal).
    assert!(replay || ehandover.is_none() || leaf.is_some(), "a handover needs every block file 1..{} to replay the BIP30 tree (or a leaf bundle)", lo - 1);
    // An anchor written before ChainState carried max_time loads it as 0. With the block files present it is
    // exact (genesis and every block to lo-1: timestamps are not monotonic, so the recent window is not enough);
    // for a demonstration anchor only the anchor's own window exists, and the result says so.
    let mut smt = if leaf.is_some() { Smt::new() } else if replay {
        let (t, replay_max) = lz_smt_replay(dir, lo - 1);
        if prev.max_time == 0 { prev.max_time = replay_max.max(genesis_params().1); }
        assert_eq!(t.root(), prev.smt_root, "replayed BIP30 tree at {} != the anchor's smt_root", lo - 1);
        t
    } else {
        // A DEMONSTRATION anchor (anchor_effective.json; a verifier reports it as DEMO): the tree is seeded with the coinbase
        // coins the range spends from before lo, read from the file's flags. With no history before lo, a tracked
        // non-coinbase coin (bit 1 alone; the fetcher's files never carry it, tracked.rs) is not seeded and the guest
        // refuses its spend. Not a production path: prep, leaf bundles and a replay from genesis derive the flags.
        let mut t = Smt::new();
        let mut counts: std::collections::BTreeMap<[u8; 32], u32> = std::collections::BTreeMap::new();
        for h in lo..=hi {
            let j = read_block_json(dir, h);
            for tx in j["txs"].as_array().unwrap() {
                let t2: Transaction = deserialize(&hx(tx["raw"].as_str().unwrap())).unwrap();
                for (i, p) in tx["prevouts"].as_array().unwrap().iter().enumerate() {
                    if p["coin_is_coinbase"].as_u64().unwrap_or(0) != 0 && p["coin_height"].as_u64().unwrap_or(0) < lo as u64 {
                        *counts.entry(t2.input[i].previous_output.txid.to_byte_array()).or_insert(0) += 1;
                    }
                }
            }
        }
        for (k, c) in counts { t.insert(k, c); }
        prev.smt_root = t.root();
        if prev.max_time == 0 {
            prev.max_time = prev.recent_times.iter().copied().chain([prev.prev_time]).max().unwrap_or(0);
            eprintln!("WARNING: anchor has no max_time; filled from its last {} timestamps only (demonstration anchor)",
                      prev.recent_times.len());
        }
        t
    };
    std::fs::write(format!("{out}/anchor_effective.json"), serde_json::to_string(&prev).unwrap()).unwrap();
    let mut handover_pending = ehandover.is_some();
    // the previous block's proof (a recursive step verifies it in-guest), and the predecessor's proof as the guest's
    // xver reads it after the tail (xver_ovm.rs): its byte length, then the bytes packed little-endian, zero-padded to a
    // word. Only for the handover block (step 2).
    let mut eprev: Option<crate::engine_backend::EngineProof> = None;
    let mut eforeign: Option<Vec<u32>> = None;
    if let Some((p, bin, _)) = ehandover {
        let mut b = bin.clone();
        b.resize(bin.len().div_ceil(4) * 4, 0);
        let mut w = vec![bin.len() as u32];
        w.extend(b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())));
        eforeign = Some(w);
        eprev = Some(p);
    }
    let first = lo;
    let mut times = std::fs::OpenOptions::new().create(true).append(true).open(format!("{out}/times.tsv")).unwrap();
    let t_all = Instant::now();
    for h in first..=hi {
        let t0 = Instant::now();
        std::env::set_var("HAZYNC_BLOCK", format!("{dir}/block_{h}.json"));
        std::env::remove_var("HAZYNC_LZ_HINTS");
        // the BIP30-tracked flags as the guest derives them (tracked.rs), from this run's BIP30 tree and coin log; a
        // leaf bundle's JSON already carries them (prep wrote it; checked when the bundle was read)
        let j = crate::read_block_json(dir, h);
        let j = if leaf.is_none() {
            let g = methods::NETWORK == "mainnet" && (h == 91_842 || h == 91_880);
            crate::tracked::annotate(&j, &smt, crate::tracked::bip34_height(), g, &|_, _, l| cl_has(l)).0
        } else { j };
        let (_, mut w) = crate::build_full_from(j);
        assert_eq!(w.height, h);
        w.root_prev = WireStump { roots: prev.utxo_roots.clone(), num_leaves: prev.utxo_leaves };
        if let Some(b) = &leaf {
            w.in_smt_root = b.in_smt_root;
            w.smt = b.smt.clone();
        } else { lz_smt_rebind(&mut w, &mut smt); }
        let check_b = leaf_check.as_ref().map(|d| crate::prep::leaf_read(&format!("{d}/leaf_{h}.bin")));
        if let Some(b) = &check_b {
            assert!(state_journal_bytes(&b.prev) == state_journal_bytes(&prev), "HAZYNC_LEAF_CHECK: leaf_{h}.bin's prev != this run's S({})", h - 1);
            assert!(b.in_smt_root == w.in_smt_root && bincode::serialize(&b.smt).unwrap() == bincode::serialize(&w.smt).unwrap(),
                    "HAZYNC_LEAF_CHECK: leaf_{h}.bin's BIP30 witness != this run's");
        }
        let mut bounds = chunk_bounds(&w, crate::chunks::nchunks_for(&w));
        // Signature hints first: every chunk executes once in collect mode, and R for each logged verify comes from
        // libsecp -- the same bundle `libzync-hints` writes. Without this every chunk would be PROVED with libsecp
        // verifying each signature itself: valid, but the unbatched cost.
        // A chunk that runs out of guest memory is re-split here (#794), and `bounds` becomes the layout that ran.
        let bp = format!("{out}/bundle_{h}.lzb");
        let mut bd = libzync::host::BlockBundle { height: h, block_hash: header_hash(&w.header), ..Default::default() };
        let sig_blob = |log: &[u8]| -> Vec<u8> {
            libzync::host::sig_records(&String::from_utf8_lossy(log)).iter().flat_map(libzync::host::sig_hint).collect()
        };
        if !bounds.is_empty() {
            let done: Vec<(Span, Vec<u8>)> = {
                // collect-mode engine runs, re-split on running out of guest memory (#794)
                let costs: std::cell::OnceCell<Vec<u64>> = std::cell::OnceCell::new();
                let split = |lo: usize, hi: usize| crate::chunks::split_chunk(&w.inputs, costs.get_or_init(|| crate::chunks::input_costs(&w)), lo, hi);
                let mut round = |todo: &[Span]| -> Vec<Result<Vec<u8>, String>> {
                    let mut res = Vec::with_capacity(todo.len());
                    for &(clo, chi) in todo {
                        let r = engine_chunk_exec(out, &w, h, "collect", clo, chi).map(|x| x.log.into_bytes());
                        let fatal = matches!(&r, Err(e) if !is_guest_oom(e));
                        res.push(r);
                        if fatal { break }
                    }
                    res
                };
                resplit_on_oom(&bounds, &split, &mut round).unwrap_or_else(|e| panic!("block {h}: {}", guest_failure("chunk collect", &e)))
            };
            bounds = done.iter().map(|(b, _)| *b).collect();
            for ((clo, chi), log) in &done {
                rig_echo(log);
                bd.chunks.push((*clo as u32, *chi as u32, sig_blob(log)));
            }
            bd.write(&bp).unwrap();
            std::env::set_var("HAZYNC_LZ_HINTS", &bp);
        }
        let t_sig = t0.elapsed().as_secs_f64();
        // Each chunk EXECUTED now with its signature hints, for its journal; the proofs come later, all at once with the
        // trie's, on every GPU (HAZYNC_ENGINE_GPUS). The collect aggregate below runs DRY on these journals.
        // A hinted run can run out of guest memory where its collect run did not: re-split the same way (#794), a piece
        // the bundle has no hints for collected first (write_chunk_inputs runs it in collect mode), then run hinted.
        let mut estreams: Vec<Vec<u8>> = Vec::new();
        let mut ejournals: Vec<crate::engine_backend::JournalOnly> = Vec::new();
        {
            let costs: std::cell::OnceCell<Vec<u64>> = std::cell::OnceCell::new();
            let split = |lo: usize, hi: usize| crate::chunks::split_chunk(&w.inputs, costs.get_or_init(|| crate::chunks::input_costs(&w)), lo, hi);
            let mut exec_one = |clo: usize, chi: usize| -> Result<(Vec<u8>, Vec<u8>), String> {
                if bd.chunk_sig(clo as u32, chi as u32).is_none() {
                    let x = engine_chunk_exec(out, &w, h, "collect", clo, chi)
                        .map_err(|e| format!("collect (the hints of re-split piece {clo}..{chi}) failed: {e}"))?;
                    rig_echo(x.log.as_bytes());
                    bd.chunks.push((clo as u32, chi as u32, sig_blob(x.log.as_bytes())));
                    bd.write(&bp).unwrap();
                    std::env::set_var("HAZYNC_LZ_HINTS", &bp);
                }
                let x = engine_chunk_exec(out, &w, h, "exec", clo, chi)?;
                Ok((chunk_stream(&w, clo, chi), x.journal.expect("chunk journal")))
            };
            let mut round = |todo: &[Span]| -> Vec<Result<(Vec<u8>, Vec<u8>), String>> {
                let mut res = Vec::with_capacity(todo.len());
                for &(clo, chi) in todo {
                    let r = exec_one(clo, chi);
                    let fatal = matches!(&r, Err(e) if !is_guest_oom(e));
                    res.push(r);
                    if fatal { break }
                }
                res
            };
            let done = resplit_on_oom(&bounds, &split, &mut round)
                .unwrap_or_else(|e| panic!("block {h}: {}", guest_failure("chunk", &e)));
            bounds = done.iter().map(|(b, _)| *b).collect();
            for (_, (st, j)) in done {
                estreams.push(st);
                ejournals.push(crate::engine_backend::JournalOnly(j));
            }
        }
        // the bundle keeps only the layout that ran (a re-split in the hinted pass leaves its parent's entry behind)
        if bd.chunks.len() != bounds.len() {
            bd.chunks.retain(|c| bounds.contains(&(c.0 as usize, c.1 as usize)));
            // in input order, so the bundle's bytes do not depend on which chunk re-split
            bd.chunks.sort_by_key(|c| (c.0, c.1));
            bd.write(&bp).unwrap();
        }
        let t_chunks = t0.elapsed().as_secs_f64();
        std::env::remove_var("HAZYNC_LZ_HINTS"); // the aggregate's collect pass below runs without ECMH hints
        libzync_agg_prep(&mut w);
        let is_base = eprev.is_none();
        let step = if handover_pending { 2 } else { is_base as u32 };
        // collect pass: the leaves the ECMH hints are for (and, for a coin-log image, the log's spends and appends)
        CL_WIT.lock().unwrap().clear();
        *crate::engine_backend::TRIE.lock().unwrap() = None;
        let mut etrie_stream: Option<Vec<u8>> = None;
        // the aggregate's stream, and its children in the guest's env::verify order: the chunks, the trie (once
        // proved), the previous block's proof (a recursive step)
        let log = {
            // DRY: on the chunks' executed journals, no proofs (it can never commit); prev's journal is in the stream
            let mut st = crate::sink::Stream::default();
            write_aggregate_env_ex(&mut st, &ejournals, &prev, eprev.as_ref().map(|p| p.journal.as_slice()), &w, step);
            libzync_agg_tail(&mut st, &w);
            if let (2, Some(words)) = (step, &eforeign) {
                use crate::sink::Sink;
                st.write(&(words.len() as u32));
                st.write_slice(words);
            }
            let x = crate::engine_backend::with(out, |e| e.execute_ex(&format!("b{h}_agg_collect"), &st.0, &[], true, true));
            assert!(x.exit == Some(libzync::wire::COLLECT_EXIT as u32), "block {h}: collect aggregate: exit {:?}, not a finished collect run", x.exit);
            let log = x.log.into_bytes();
            rig_echo(&log);
            log
        };
        for leaf in libzync::host::ecmh_leaves(&String::from_utf8_lossy(&log)) {
            bd.ecmh.extend_from_slice(&libzync::host::ecmh_hint(&leaf));
        }
        // The block's undo hash (#729): the guest's (logged by the collect aggregate) must be the host's own from the block
        // file, so a disagreement between the two implementations stops here, not in a verifier.
        let undo = crate::prep::undo_of_block_json(&read_block_json(dir, h));
        match guest_undo_line(&String::from_utf8_lossy(&log), h) {
            Some((n, u)) => assert!((n, u) == (undo.1, undo.0), "block {h}: the guest's undo hash ({n} inputs, {}) != the host's ({} inputs, {})",
                                    crate::hex(&u), undo.1, crate::hex(&undo.0)),
            None => panic!("block {h}: the collect aggregate logged no `undo: block {h}` line"),
        }
        // what a consumer reads: the block's undo-data commitment, checked guest == host (docs/libzync/UNDO.md)
        println!("  undo: block {h} inputs {} hash {} (guest == host)", undo.1, crate::hex(&undo.0));
        let cl_lists = if methods::COINLOG { cl_parse(&String::from_utf8_lossy(&log)) } else { (Vec::new(), Vec::new()) };
        if methods::COINLOG {
            let wit = match &leaf { Some(b) => b.cl_wit.clone(), None => cl_prepare(&cl_lists.0) };
            if let Some(b) = &check_b { assert!(b.cl_wit == wit, "HAZYNC_LEAF_CHECK: leaf_{h}.bin's coin-log witness != this run's"); }
            *CL_WIT.lock().unwrap() = wit;
        }
        // The trie update (mode 17), from the store's witness; the aggregate verifies its proof in-guest.
        let trie_lists = if methods::TRIE { crate::trie::trie_parse(&String::from_utf8_lossy(&log)) } else { (Vec::new(), Vec::new()) };
        if methods::TRIE {
            let wit = match &leaf { Some(b) => b.trie_wit.clone(), None => crate::trie::trie_witness(h, &trie_lists.0, &trie_lists.1, &prev) };
            if let Some(b) = &check_b { assert!(b.trie_wit == wit, "HAZYNC_LEAF_CHECK: leaf_{h}.bin's trie witness != this run's"); }
            let mut st = crate::sink::Stream::default();
            crate::trie::trie_env(&mut st, &wit, &trie_lists.0, &trie_lists.1);
            etrie_stream = Some(st.0);
        }
        bd.write(&bp).unwrap();
        std::env::set_var("HAZYNC_LZ_HINTS", &bp);
        // Every chunk and the trie update proved at once (one job per GPU at a time), each against the journal its
        // execution committed; the aggregate then verifies all of them in-guest.
        let echunks: Vec<crate::engine_backend::EngineProof> = {
            let mut jobs: Vec<crate::engine_backend::Job> = estreams.into_iter().zip(ejournals.into_iter()).enumerate()
                .map(|(ci, (st, j))| (format!("b{h}_c{ci:02}"), st, Vec::new(), false, Some(j.0))).collect();
            let nchunks = jobs.len();
            if let Some(st) = etrie_stream.take() {
                let x = crate::engine_backend::with(out, |e| e.execute(&format!("b{h}_trie_exec"), &st, &[], false));
                let tj = x.journal.unwrap_or_else(|| panic!("block {h}: trie update did not commit (exit {:?})", x.exit));
                let j = crate::trie::trie_journal_words(&tj);
                assert!(j[0] == libzync::ttree::KIND_TRIE && j[1..9] == libzync::coinlog::w_of(&prev.trie_root)[..],
                        "block {h}: the engine's trie update is not from the previous trie root");
                jobs.push((format!("b{h}_trie"), st, Vec::new(), false, Some(tj)));
            }
            let mut proofs = crate::engine_backend::prove_many(out, jobs);
            let trie = if proofs.len() > nchunks { proofs.pop() } else { None };
            *crate::engine_backend::TRIE.lock().unwrap() = trie;
            proofs
        };
        let t_hints = t0.elapsed().as_secs_f64();
        let st: ChainState = {
            let trie = crate::engine_backend::TRIE.lock().unwrap().clone();
            let ch: Vec<&crate::engine_backend::EngineProof> =
                echunks.iter().chain(trie.iter()).chain(eprev.iter().filter(|_| step == 0)).collect();
            let mut st = crate::sink::Stream::default();
            write_aggregate_env_ex(&mut st, &echunks, &prev, eprev.as_ref().map(|p| p.journal.as_slice()), &w, step);
            libzync_agg_tail(&mut st, &w);
            if let (2, Some(words)) = (step, &eforeign) {
                use crate::sink::Sink;
                st.write(&(words.len() as u32));
                st.write_slice(words);
            }
            let p = crate::engine_backend::with(out, |e| e.prove(&format!("b{h}_agg"), &st.0, &ch, true));
            let p = crate::engine_backend::keep(&p, &format!("{out}/block_{h}.ovm"));
            let st = crate::engine_backend::decode_state(&p.journal);
            assert_eq!(st.height, h);
            eprev = Some(p);
            st
        };
        write_atomic(&format!("{out}/state_{h}.json"), serde_json::to_string(&st).unwrap().as_bytes());
        // a leaf has no stores to advance (the guest checked its witnesses against S(h-1))
        if methods::COINLOG && leaf.is_none() {
            let flips = cl_commit(&cl_lists.0, &cl_lists.1, &st);
            if h == 1 && prev_is_genesis {
                hist_append(&out, &libzync::history::Rec { height: 0, hash: genesis_anchor().tip_hash, value: 0, flips: vec![], appends: vec![], undo: None });
            }
            let undo = crate::handover_pin::leaf_has_undo(&st.self_id, pinned_lineage(), h).then_some(undo.0);
            hist_append(&out, &libzync::history::Rec { height: h, hash: st.tip_hash, value: st.utxo_value, flips, appends: cl_lists.1.clone(), undo });
            // The log next to each state, so a run can be anchored on it. The last two are kept (a checkpoint copier
            // takes the second newest, which may still be complete while the newest is being written);
            // HAZYNC_COINLOG_SAVE_EACH=1 keeps every one (tests). A mainnet-sized log needs a disk-backed store instead
            // (COINLOG.md §8).
            cl_save(&format!("{out}/state_{h}.coinlog"), h);
            // test hook: die between the coin log's save and the trie's (the window the journal covers)
            if std::env::var("HAZYNC_CRASH_AT").as_deref() == Ok(&format!("after-coinlog-save:{h}")) { std::process::exit(9); }
            if h >= first + 2 && std::env::var("HAZYNC_COINLOG_SAVE_EACH").as_deref() != Ok("1") {
                let _ = std::fs::remove_file(format!("{out}/state_{}.coinlog", h - 2));
            }
        }
        // The trie: the same update list applied to the store must reach the committed root; its snapshot beside the
        // state (the last two kept, as the coin log's; HAZYNC_COINLOG_SAVE_EACH=1 keeps every one).
        if methods::TRIE && leaf.is_none() {
            crate::trie::trie_commit(&trie_lists.0, &trie_lists.1, &st);
            crate::trie::trie_save(&format!("{out}/state_{h}.trie"));
            if h >= first + 2 && std::env::var("HAZYNC_COINLOG_SAVE_EACH").as_deref() != Ok("1") {
                let _ = std::fs::remove_file(format!("{out}/state_{}.trie", h - 2));
            }
        }
        // The block's commit point: the coin log and the trie are both saved at h, so its journal record can go. A crash
        // before this line leaves the coin log's record, which undoes the block (11-06).
        if methods::COINLOG && leaf.is_none() { cl_journal_commit(); }
        let dt = t0.elapsed().as_secs_f64();
        // height, chunks, sig-hint s, chunk-prove s, ecmh-hint s, total s, kind, proof bytes
        let rbytes = std::fs::metadata(format!("{out}/block_{h}.ovm.proof.json")).map(|m| m.len()).unwrap_or(0);
        writeln!(times, "{h}\t{}\t{t_sig:.1}\t{:.1}\t{:.1}\t{dt:.1}\t{}\t{rbytes}", bounds.len(),
                 t_chunks - t_sig, t_hints - t_chunks, ["recursive", "base", "handover"][step as usize]).unwrap();
        println!("  block {h}: {} chunks, proved in {dt:.1} s ({}), forest {} live_set {}  [{:.0} s total]",
                 bounds.len(), ["verifies prev", "base on anchor", "handover: verifies predecessor"][step as usize], st.utxo_leaves, st.live_set, t_all.elapsed().as_secs_f64());
        prev = st;
        handover_pending = false;
    }
    println!("LZ_PROVE_RANGE_DONE {lo}..={hi} in {:.0} s; final proof {out}/block_{hi}.ovm{}", t_all.elapsed().as_secs_f64(),
             if crate::engine_backend::dev() { " (ENGINE DEV MODE: executions, not proofs)" } else { "" });
}

// ---- libzync coin log, host side (HAZYNC_COINLOG=1 builds; libzync/src/coinlog.rs) ----------------------------
//
// The host keeps the whole log (every node) to read off each block's witness. A collect execution of the aggregate
// logs the leaves of the earlier coins the block spends ("coinlog: S") and of the coins it appends ("coinlog: A"); the
// host names each spend's position (an unknown coin gets the end of the log, which the guest refuses), and after the
// block is proved applies the same changes and checks its roots equal the committed state's.

pub(crate) struct ClHost {
    // In memory, or on disk when HAZYNC_COINLOG_DIR is set (ROADMAP C1; libzync::host::DiskNodes).
    store: libzync::coinlog_forest::NodeStore<libzync::host::AnyNodes>,
    pos: libzync::host::AnyIndex, // unspent leaf -> position; on disk (<dir>/index.bin) with HAZYNC_COINLOG_DIR (C1b)
}
pub(crate) static CL: std::sync::Mutex<Option<ClHost>> = std::sync::Mutex::new(None);
pub(crate) static CL_WIT: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());

/// `state_<h>.json` -> `state_<h>.coinlog`; `anchor.json` -> `anchor.coinlog`.
pub(crate) fn cl_path_for(state_json: &str) -> String {
    match state_json.strip_suffix(".json") { Some(s) => format!("{s}.coinlog"), None => format!("{state_json}.coinlog") }
}

/// The directory a handover's predecessor files sit in (state_<h>.coinlog, state_<h>.trie): the one of the
/// predecessor's proof (HAZYNC_HANDOVER_PROOF). A bare file name has the parent "" (review 11-03): then ".".
pub(crate) fn handover_dir() -> String {
    let p = std::env::var("HAZYNC_HANDOVER_PROOF").expect("a handover run names its predecessor's proof (HAZYNC_HANDOVER_PROOF)");
    std::path::Path::new(&p).parent().map(|x| x.to_string_lossy().to_string()).filter(|x| !x.is_empty()).unwrap_or_else(|| ".".to_string())
}

/// The coin log for a run starting after `prev`: the handover predecessor's log, an empty log (genesis), or the anchor's.
pub(crate) fn cl_init_at_start(handover: bool, anchor_path: &str, prev: &ChainState) {
    if handover {
        let d = handover_dir();
        cl_load(&format!("{d}/state_{}.coinlog", prev.height));
    } else if anchor_path == "genesis" {
        cl_init_empty();
    } else {
        cl_load(&cl_path_for(anchor_path));
    }
    let g = CL.lock().unwrap();
    let st = &g.as_ref().unwrap().store;
    assert!(st.len() == prev.utxo_leaves && normalize_host(st.roots()) == normalize_host(prev.utxo_roots.clone()),
            "the coin log file does not match the anchor's utxo_roots / utxo_leaves");
}

/// fsync a directory, so a rename into it survives a power loss (unix; a no-op elsewhere).
pub(crate) fn cl_sync_dir(d: &std::path::Path) {
    #[cfg(unix)]
    if let Ok(f) = std::fs::File::open(d) { let _ = f.sync_all(); }
    #[cfg(not(unix))]
    let _ = d;
}

/// HAZYNC_COINLOG_DIR: the on-disk store's directory (C1). Unset: the log lives in memory.
pub(crate) fn cl_dir() -> Option<std::path::PathBuf> {
    std::env::var("HAZYNC_COINLOG_DIR").ok().filter(|v| !v.is_empty()).map(std::path::PathBuf::from)
}

/// A state_<h>.coinlog written in disk mode: this magic, then the leaf count and the roots it had (the leaves are in
/// HAZYNC_COINLOG_DIR, mutated in place, so a load can only check that the directory is still at that state).
pub(crate) const CL_DISK_MAGIC: &[u8; 8] = b"LZDISK01";
/// Version 2 of that file: the magic, the HEIGHT of the state, then as version 1 (11-06: the height says which
/// journal records postdate it).
pub(crate) const CL_DISK_MAGIC2: &[u8; 8] = b"LZDISK02";
/// One journal record (pending.bin, disk mode): this magic, the block height, then the undo body cl_undo_to reads
/// (leaf count before the block, the spends' positions and leaves, the appends). pending.bin is a SEQUENCE of these,
/// one per block applied since the last commit (cl_journal_commit), each appended and synced before its block changes
/// the store. A load undoes, newest first, every record above the saved state's height (11-06: before, the record was
/// one block, replaced each block and removed at the coin log's save, so a crash between the coin log's save and the
/// trie's, or anywhere inside a libzync-prep range, left a store no saved state matched).
pub(crate) const CL_PEND_MAGIC: &[u8; 8] = b"LZPEND03";

/// The journal records in pending.bin: (height, body), oldest first. A version-1 file (one bare body) is (None, body).
/// A version-2 record is MAGIC | height | body | SHA-256 of the three (11-06): a record whose checksum fails is TORN if
/// it is the last one (its sync never returned, so its block never changed the store: ignored) and corruption anywhere
/// else (Err: the journal cannot be trusted, nothing is undone).
fn cl_journal_records(u: &[u8]) -> Result<Vec<(Option<u64>, &[u8])>, String> {
    use sha2::{Digest, Sha256};
    if u.starts_with(b"LZPEND02") {
        return Err("an LZPEND02 journal (a pre-release format of 11-06 without checksums): rebuild the store".into());
    }
    if !u.starts_with(CL_PEND_MAGIC) { return Ok(if u.len() >= 8 { vec![(None, u)] } else { vec![] }); }
    let rd = |b: &[u8], o: usize| b.get(o..o + 8).map(|x| u64::from_le_bytes(x.try_into().unwrap()));
    let mut out = Vec::new();
    let mut o = 0;
    while o < u.len() {
        // the body: n, k, k * (8 + 32), m, m * 32; then a 32-byte checksum
        let parsed = (|| {
            if !u[o..].starts_with(CL_PEND_MAGIC) { return None; }
            let h = rd(u, o + 8)?;
            let body = u.get(o + 16..)?;
            let k = rd(body, 8)?;
            let at_m = 16usize.checked_add(40usize.checked_mul(k as usize)?)?;
            let m = rd(body, at_m)?;
            let len = at_m.checked_add(8)?.checked_add(32usize.checked_mul(m as usize)?)?;
            let end = o.checked_add(16)?.checked_add(len)?;
            let sum = u.get(end..end.checked_add(32)?)?;
            (Sha256::digest(&u[o..end]).as_slice() == sum).then_some((h, &body[..len], end + 32))
        })();
        match parsed {
            Some((h, body, next)) => { out.push((Some(h), body)); o = next; }
            // a torn LAST record: no record starts after it (records are only ever appended)
            None if !u[o + 1..].windows(8).any(|w| w == CL_PEND_MAGIC) => break,
            None => return Err(format!("a damaged record at byte {o} that is not the last one")),
        }
    }
    Ok(out)
}

/// A journal record body's leaf count before its block, and after (its appends added).
fn cl_record_span(body: &[u8]) -> (u64, u64) {
    let rd = |o: usize| u64::from_le_bytes(body[o..o + 8].try_into().unwrap());
    let k = rd(8) as usize;
    (rd(0), rd(0) + rd(16 + 40 * k))
}

/// A block's undo body, as cl_undo_to reads it: the leaf count before the block, the spends' (position, unspent leaf),
/// the appends. pending.bin's records carry it (cl_journal_append), and so do prep's reorg undo records (reorg.rs).
pub(crate) fn cl_undo_body(store_len: u64, spends: &[(u64, [u8; 32])], appends: &[[u8; 32]]) -> Vec<u8> {
    let mut b = Vec::with_capacity(24 + 40 * spends.len() + 32 * appends.len());
    b.extend_from_slice(&store_len.to_le_bytes());
    b.extend_from_slice(&(spends.len() as u64).to_le_bytes());
    for (p, l) in spends { b.extend_from_slice(&p.to_le_bytes()); b.extend_from_slice(l); }
    b.extend_from_slice(&(appends.len() as u64).to_le_bytes());
    for l in appends { b.extend_from_slice(l); }
    b
}

/// Disk mode: append this block's undo record to pending.bin and sync it, BEFORE the block changes the store.
fn cl_journal_append(d: &std::path::Path, height: u32, store_len: u64, spends: &[(u64, [u8; 32])], appends: &[[u8; 32]]) {
    cl_journal_append_body(d, height, &cl_undo_body(store_len, spends, appends));
}

fn cl_journal_append_body(d: &std::path::Path, height: u32, body: &[u8]) {
    use std::io::Write as _;
    let mut b = Vec::with_capacity(16 + body.len() + 32);
    b.extend_from_slice(CL_PEND_MAGIC);
    b.extend_from_slice(&(height as u64).to_le_bytes());
    b.extend_from_slice(body);
    { use sha2::Digest as _; let sum = sha2::Sha256::digest(&b); b.extend_from_slice(&sum); }
    let p = d.join("pending.bin");
    std::fs::OpenOptions::new().create(true).append(true).open(&p).and_then(|mut f| { f.write_all(&b)?; f.sync_data() })
        .unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    cl_sync_dir(d);
}

/// Disk mode: every store this block touched is saved (the coin log's state file AND the trie's): its undo records
/// are no longer needed. The one commit point of a block; call it after the last save.
pub(crate) fn cl_journal_commit() {
    if let Some(d) = cl_dir().filter(|d| d.join("pending.bin").exists()) {
        let _ = std::fs::remove_file(d.join("pending.bin"));
        cl_sync_dir(&d);
    }
}

/// Undoes one journaled block back to the state of `n` leaves (round 11): drop its appends from the index, cut every
/// level to `n` leaves, restore each of its spends at a position below `n`. Idempotent, and independent of how much of
/// the block (or of later blocks, or of an earlier undo) reached the disk.
fn cl_undo_to(store: &mut libzync::coinlog_forest::NodeStore<libzync::host::AnyNodes>, pos: &mut libzync::host::AnyIndex, u: &[u8], n: u64) {
    let rd = |o: usize| u64::from_le_bytes(u[o..o + 8].try_into().expect("pending.bin truncated"));
    let k = rd(8) as usize;
    let flips: Vec<(u64, [u8; 32])> = (0..k).map(|j| (rd(16 + 40 * j), u[24 + 40 * j..56 + 40 * j].try_into().unwrap())).collect();
    let o = 16 + 40 * k;
    for j in 0..rd(o) as usize {
        let l: [u8; 32] = u[o + 8 + 32 * j..o + 40 + 32 * j].try_into().unwrap();
        pos.remove(&l);
    }
    if let libzync::host::AnyNodes::Disk(dn) = &mut store.nodes {
        dn.truncate_to(n).unwrap_or_else(|e| panic!("coin log store truncate: {e}"));
    }
    for (p, l) in flips {
        if p < n {
            store.set(p, l);
            pos.insert(l, p);
        }
    }
}

/// REORG.md §6 (P3): the on-disk coin log undone IN PLACE to the state `st` (the fork F), from the journal pending.bin
/// (blocks after the last save point, if prep stopped between save points) and then `bodies`, the undo records' coin
/// log bodies of the blocks F+1..A newest first (A = the last save point). Every step is cl_undo_to with F's leaf
/// count, which is idempotent and independent of how much of any block, or of an earlier interrupted rewind, reached
/// the disk: a rewind that died part-way is simply run again. Checked against `st` at the end; pending.bin is left
/// for the save point at F to commit (cl_journal_commit).
pub(crate) fn cl_rewind(fork: u32, bodies: &[Vec<u8>], st: &ChainState) {
    let d = cl_dir().expect("a rewind needs the on-disk coin log (HAZYNC_COINLOG_DIR)");
    let mut store = libzync::coinlog_forest::NodeStore::with_nodes(libzync::host::AnyNodes::Disk(
        libzync::host::DiskNodes::open(&d).unwrap_or_else(|e| panic!("{}: {e}", d.display()))));
    let mut pos = cl_index();
    let n = st.utxo_leaves;
    let pend = d.join("pending.bin");
    // only the journal's blocks ABOVE the fork, as cl_load takes only those above its state (review 16.11, H1): a crash
    // between the trie's flush at a save point and the journal's commit leaves the records of the blocks the save point
    // already holds, and undoing those to F's leaf count would unspend coins F's state spent. A record without its
    // height (the version-1 format) cannot be placed: refused.
    let journal: Vec<Vec<u8>> = match std::fs::read(&pend) {
        Ok(u) => cl_journal_records(&u).unwrap_or_else(|e| panic!("{}: {e}: the journal cannot be trusted; nothing undone", pend.display()))
                     .into_iter().filter_map(|(h, b)| match h {
                         Some(h) => (h > fork as u64).then(|| b.to_vec()),
                         None => panic!("{}: a journal record without its height (version 1): a rewind cannot place it", pend.display()),
                     }).collect(),
        Err(_) => Vec::new(),
    };
    for (i, b) in journal.iter().rev().chain(bodies.iter()).enumerate() {
        cl_undo_to(&mut store, &mut pos, b, n);
        // test hook: die part-way through the undo, its first block's changes synced (rewind_test.sh case G)
        if i == 0 && std::env::var("HAZYNC_CRASH_AT").as_deref() == Ok("rewind-mid-coinlog") {
            if let libzync::host::AnyNodes::Disk(dn) = &store.nodes { let _ = dn.sync(); }
            if let libzync::host::AnyIndex::Disk(ix) = &pos { let _ = ix.sync(); }
            std::process::exit(9);
        }
    }
    if let libzync::host::AnyNodes::Disk(dn) = &store.nodes { dn.sync().unwrap_or_else(|e| panic!("coin log store sync: {e}")); }
    if let libzync::host::AnyIndex::Disk(ix) = &pos { ix.sync().unwrap_or_else(|e| panic!("index sync: {e}")); }
    assert!(store.len() == n && normalize_host(store.roots()) == normalize_host(st.utxo_roots.clone()),
            "rewind: the coin log undone to {} leaves does not have the fork state's roots (records not of this store?)", n);
    eprintln!("  coin log store: rewound to {n} leaves ({} journaled and {} recorded block(s) undone)", journal.len(), bodies.len());
    *CL.lock().unwrap() = Some(ClHost { store, pos });
}

/// The coin log's journal (pending.bin), checked as a rewind will read it: every record intact and with its height.
/// Returns the number of records. For the rewind to refuse BEFORE it changes anything (review 16.11).
pub(crate) fn cl_journal_check() -> Result<usize, String> {
    let Some(d) = cl_dir() else { return Ok(0) };
    let Ok(u) = std::fs::read(d.join("pending.bin")) else { return Ok(0) };
    let r = cl_journal_records(&u)?;
    if r.iter().any(|(h, _)| h.is_none()) { return Err("a record without its height (version 1): a rewind cannot place it".into()); }
    Ok(r.len())
}

/// The position index: on disk in HAZYNC_COINLOG_DIR (index.bin), else in memory.
pub(crate) fn cl_index() -> libzync::host::AnyIndex {
    match cl_dir() {
        Some(d) => libzync::host::AnyIndex::Disk(libzync::host::DiskIndex::open(&d.join("index.bin"))
            .unwrap_or_else(|e| panic!("{}/index.bin: {e}", d.display()))),
        None => libzync::host::AnyIndex::Mem(std::collections::HashMap::new()),
    }
}

pub(crate) fn cl_from_leaves(leaves: &[[u8; 32]]) -> ClHost {
    let nodes = match cl_dir() {
        Some(d) => {
            // Starting a log here replaces the store: refuse to wipe one that exists unless asked (a mainnet store is
            // days of work; review 11-06). Resume with HAZYNC_RESUME=1, or set HAZYNC_COINLOG_FRESH=1 to start over.
            assert!(!d.join("L0.bin").exists() || std::env::var("HAZYNC_COINLOG_FRESH").as_deref() == Ok("1"),
                    "{}: a coin-log store is already there; HAZYNC_RESUME=1 to continue it, HAZYNC_COINLOG_FRESH=1 to replace it", d.display());
            // the store's files only: the run's LOCK stays (round 12: wiping the directory unlinked the lock just taken,
            // so a second run could lock a fresh file and write the same store)
            if let Ok(rd) = std::fs::read_dir(&d) {
                for e in rd.flatten() {
                    if e.file_name() != "LOCK" {
                        let p = e.path();
                        let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
                    }
                }
            }
            libzync::host::AnyNodes::Disk(libzync::host::DiskNodes::open(&d).unwrap_or_else(|e| panic!("{}: {e}", d.display())))
        }
        None => libzync::host::AnyNodes::Mem(Vec::new()),
    };
    let mut store = libzync::coinlog_forest::NodeStore::with_nodes(nodes);
    let mut pos = cl_index();
    for (i, l) in leaves.iter().enumerate() {
        store.push(*l);
        pos.insert(*l, i as u64);
    }
    // The file stores each position's current leaf: a live coin's leaf, or spent(c). Indexing all of them is harmless:
    // spent(c) is a different function's output and never equals a coin leaf a block could spend.
    ClHost { store, pos }
}

pub(crate) fn cl_save(path: &str, height: u32) {
    let g = CL.lock().unwrap();
    let h = g.as_ref().expect("coin log not initialised");
    let mut b = Vec::new();
    if let libzync::host::AnyNodes::Disk(d) = &h.store.nodes {
        d.sync().unwrap_or_else(|e| panic!("coin log store sync: {e}"));
        if let libzync::host::AnyIndex::Disk(ix) = &h.pos { ix.sync().unwrap_or_else(|e| panic!("index sync: {e}")); }
        // and the directory: a new level file, or the index's grow-and-rename, must survive with the marker (round 11)
        if let Some(cd) = cl_dir() { cl_sync_dir(&cd); }
        b.extend_from_slice(CL_DISK_MAGIC2);
        b.extend_from_slice(&(height as u64).to_le_bytes());
        b.extend_from_slice(&h.store.len().to_le_bytes());
        for r in h.store.roots() { b.extend_from_slice(&r.unwrap_or([0; 32])); b.push(r.is_some() as u8); }
    } else {
        b.reserve(8 + 32 * h.store.len() as usize);
        b.extend_from_slice(&h.store.len().to_le_bytes());
        for i in 0..h.store.len() { b.extend_from_slice(&h.store.leaf(i)); }
    }
    // Atomic: a run killed mid-write must not leave a short file that a resume would then refuse.
    let tmp = format!("{path}.tmp");
    let f = std::fs::File::create(&tmp).and_then(|mut f| { use std::io::Write as _; f.write_all(&b)?; f.sync_data() });
    f.unwrap_or_else(|e| panic!("{tmp}: {e}"));
    std::fs::rename(&tmp, path).unwrap_or_else(|e| panic!("{path}: {e}"));
    if let Some(dir) = std::path::Path::new(path).parent() { cl_sync_dir(if dir.as_os_str().is_empty() { std::path::Path::new(".") } else { dir }); }
    // The journal is NOT cleared here: the trie may not be saved yet (cl_journal_commit, after the last save).
}

pub(crate) fn cl_load(path: &str) {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e} (a coin-log anchor needs its .coinlog file)"));
    assert!(b.len() >= 8, "{path}: too short to be a coin log state file");
    if b.starts_with(CL_DISK_MAGIC) || b.starts_with(CL_DISK_MAGIC2) {
        let (saved_h, body) = if b.starts_with(CL_DISK_MAGIC2) {
            assert!(b.len() >= 16, "{path}: a disk-mode state file of the wrong size");
            (Some(u64::from_le_bytes(b[8..16].try_into().unwrap())), &b[16..])
        } else { (None, &b[8..]) };
        assert!(body.len() >= 8 && (body.len() - 8) % 33 == 0, "{path}: a disk-mode state file of the wrong size");
        let d = cl_dir().unwrap_or_else(|| panic!("{path} was written in disk mode: set HAZYNC_COINLOG_DIR"));
        let store = libzync::coinlog_forest::NodeStore::with_nodes(libzync::host::AnyNodes::Disk(
            libzync::host::DiskNodes::open(&d).unwrap_or_else(|e| panic!("{}: {e}", d.display()))));
        let n = u64::from_le_bytes(body[..8].try_into().unwrap());
        let roots: Vec<Option<[u8; 32]>> = body[8..].chunks_exact(33).map(|c| (c[32] == 1).then(|| c[..32].try_into().unwrap())).collect();
        let mut store = store;
        let mut pos = cl_index();
        // A block that started changing the store after this state was saved (a crash, or a failed check): undo it.
        let pend = d.join("pending.bin");
        if let Ok(u) = std::fs::read(&pend) {
            // Undo, newest first, every block recorded after this state, even if the nodes look untouched: the index
            // may already have lost a spent coin (cl_commit removes it before setting the leaf; review round 2).
            // Repeating an undo is harmless: the truncate does nothing and the sets and inserts rewrite the same values.
            // A record at or below the state's height belongs to a block this state already includes: nothing to undo.
            let recs = cl_journal_records(&u).unwrap_or_else(|e| panic!("{}: {e}: the journal cannot be trusted; nothing undone", pend.display()));
            // The blocks after this state: the records above its height (version 2), or from the first record that
            // starts at its leaf count (version 1). Checked BEFORE anything is undone: they must start at the state,
            // chain into each other (each starts where the one before ends) and end at the store's length, so a
            // journal that does not describe this store changes nothing (round 9).
            let first = match saved_h {
                Some(sh) => recs.iter().position(|(rh, _)| rh.map_or(true, |h| h > sh)),
                None => recs.iter().position(|(_, b)| cl_record_span(b).0 == n),
            };
            if let Some(i) = first {
                let todo = &recs[i..];
                let mut at = n;
                for (rh, b) in todo {
                    let (s0, s1) = cl_record_span(b);
                    assert!(s0 == at, "{}: journal record for block {rh:?} starts at {s0} leaves, not {at}: it does not describe this store (nothing undone)", pend.display());
                    at = s1;
                }
                // The store may be anywhere from the state to the journal's end: a block partly applied, an undo partly
                // done, or (prep, power loss between save points) any mix of the journaled blocks' writes reaching the
                // disk. So every record above the state is undone BACK TO THE STATE, newest first, with no inference
                // from the store's length (round 11): every level cut to the state's leaf count (synced at the save),
                // every recorded spend below it restored (each position is spent at most once, so the restored leaf is
                // the state's), every recorded append dropped from the index. Each step is idempotent; the roots
                // check below stands guard.
                assert!((n..=at).contains(&store.len()),
                        "{}: the journal spans {n}..={at} leaves, the store has {} (nothing undone)", pend.display(), store.len());
                for (_, b) in todo.iter().rev() {
                    cl_undo_to(&mut store, &mut pos, b, n);
                    // test hook: die part-way through an undo (after the newest block), its changes synced
                    if std::env::var("HAZYNC_CRASH_AT").as_deref() == Ok("mid-undo") && todo.len() > 1 {
                        if let libzync::host::AnyNodes::Disk(dn) = &store.nodes { let _ = dn.sync(); }
                        if let libzync::host::AnyIndex::Disk(ix) = &pos { let _ = ix.sync(); }
                        std::process::exit(9);
                    }
                }
                // the undo durable BEFORE the journal goes (round 10: a power loss could lose both)
                if let libzync::host::AnyNodes::Disk(dn) = &store.nodes { dn.sync().unwrap_or_else(|e| panic!("coin log store sync: {e}")); }
                if let libzync::host::AnyIndex::Disk(ix) = &pos { ix.sync().unwrap_or_else(|e| panic!("index sync: {e}")); }

                eprintln!("  coin log store: undid {} unfinished block(s) recorded in pending.bin (back to {} leaves)", todo.len(), store.len());
            }
        }
        assert!(store.len() == n && store.roots() == roots,
                "{}: the on-disk coin log is not at the state {path} recorded (it moved on after that receipt?): rebuild it from a run from genesis (pending.bin is kept)", d.display());
        // Only now, with the store at the state, is the journal no longer needed (a failed undo above keeps it, and
        // repeating an undo is harmless).
        if pend.exists() { let _ = std::fs::remove_file(&pend); cl_sync_dir(&d); }
        // C1b: the index lives next to the nodes and was kept in step with them (written before each state file).
        *CL.lock().unwrap() = Some(ClHost { store, pos });
        return;
    }
    let n = u64::from_le_bytes(b[..8].try_into().unwrap()) as usize;
    assert_eq!(b.len(), 8 + 32 * n, "{path}: truncated");
    let leaves: Vec<[u8; 32]> = b[8..].chunks_exact(32).map(|c| c.try_into().unwrap()).collect();
    *CL.lock().unwrap() = Some(cl_from_leaves(&leaves));
}

pub(crate) fn cl_init_empty() { *CL.lock().unwrap() = Some(cl_from_leaves(&[])); }

/// One spend of an earlier coin, as the collect run logs it: (unspent leaf, spending tx position, input index).
pub(crate) type ClSpend = ([u8; 32], u32, u32);

/// The "coinlog: S <leaf hex> <tx> <vin>" and "coinlog: A <leaf hex>" lines of a collect run, in order.
pub(crate) fn cl_parse(log: &str) -> (Vec<ClSpend>, Vec<[u8; 32]>) {
    let (mut s, mut a) = (Vec::new(), Vec::new());
    for l in log.lines() {
        let Some(i) = l.find("coinlog: ") else { continue };
        let f: Vec<&str> = l[i + 9..].split_whitespace().collect();
        let h: [u8; 32] = arr(hx_bytes(f[1]));
        match f[0] {
            "S" => s.push((h, f[2].parse().expect("coinlog S: tx"), f[3].parse().expect("coinlog S: vin"))),
            "A" => a.push(h),
            k => panic!("coinlog line kind {k}"),
        }
    }
    (s, a)
}

pub(crate) fn hx_bytes(h: &str) -> Vec<u8> { (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect() }

/// The block's witness words: each spend's position in block order, and the siblings for the sorted spends.
/// Read ahead what a block's spends will read in a DISK coin log (mainnet prep, 11-07): their index slots, then, at the
/// positions found, the leaf pages and every stored level's nodes; each step's reads in parallel. In memory: nothing.
pub(crate) fn cl_prefetch(spends: &[ClSpend]) {
    let g = CL.lock().unwrap();
    let Some(h) = g.as_ref() else { return };
    if let (libzync::host::AnyIndex::Disk(ix), libzync::host::AnyNodes::Disk(nodes)) = (&h.pos, &h.store.nodes) {
        let leaves: Vec<[u8; 32]> = spends.iter().map(|s| s.0).collect();
        ix.prefetch(&leaves);
        let pos: Vec<u64> = leaves.iter().filter_map(|l| h.pos.get(l)).collect();
        nodes.prefetch(&pos);
    }
}

/// Whether the host's coin log holds `leaf` as a coin (unspent): how the BIP30-tracked flag of an earlier coin is read
/// back (tracked.rs).
pub(crate) fn cl_has(leaf: &[u8; 32]) -> bool {
    CL.lock().unwrap().as_ref().expect("coin log not initialised (the tracked flag is read from it)").pos.get(leaf).is_some()
}

pub(crate) fn cl_prepare(spends: &[ClSpend]) -> Vec<u32> {
    let g = CL.lock().unwrap();
    let h = g.as_ref().expect("coin log not initialised");
    let n = h.store.len();
    let positions: Vec<u64> = spends.iter().map(|s| h.pos.get(&s.0).unwrap_or(n)).collect();
    let mut sorted: Vec<(u64, [u8; 32])> = positions.iter().copied().zip(spends.iter().map(|s| s.0)).collect();
    sorted.sort_unstable_by_key(|x| x.0);
    let sib = h.store.witness(&sorted);
    libzync::coinlog_forest::encode_witness(&positions, &sib)
}

/// After the block is proved: the same flips and appends on the host's log, which must then equal the state.
/// Applies a proved block to the host's log; returns its flips (position, tx, vin) for history.bin.
pub(crate) fn cl_commit(spends: &[ClSpend], appends: &[[u8; 32]], st: &ChainState) -> Vec<(u64, u32, u32)> {
    let mut g = CL.lock().unwrap();
    let h = g.as_mut().expect("coin log not initialised");
    // Disk mode: write (and fsync) how to undo this block BEFORE the store changes in place, so a crash or a failed
    // check below leaves a store that cl_load can roll back to the last saved state (review 11-03).
    if let (Some(d), libzync::host::AnyNodes::Disk(_)) = (cl_dir(), &h.store.nodes) {
        let sp: Vec<(u64, [u8; 32])> = spends.iter().map(|(l, _, _)| (h.pos.get(l).unwrap_or(u64::MAX), *l)).collect();
        cl_journal_append(&d, st.height, h.store.len(), &sp, appends);
        // test hook: die with the record synced and the store not yet changed
        if std::env::var("HAZYNC_CRASH_AT").as_deref() == Ok(&format!("after-journal:{}", st.height)) { std::process::exit(9); }
    }
    let mut flips = Vec::with_capacity(spends.len());
    for (l, t, v) in spends {
        let p = h.pos.remove(l).expect("a proved spend of a coin the host's log does not hold");
        let new = libzync::coinlog::spent(l, &libzync::coinlog::spender(st.height, *t, *v));
        h.store.set(p, new);
        flips.push((p, *t, *v));
    }
    for l in appends {
        h.pos.insert(*l, h.store.len());
        h.store.push(*l);
    }
    assert_eq!(h.store.len(), st.utxo_leaves, "host coin log size != committed utxo_leaves");
    assert!(normalize_host(h.store.roots()) == normalize_host(st.utxo_roots.clone()), "host coin log roots != committed roots");
    flips
}

/// The native prep's coin-log step (prep.rs): the same flips and appends as cl_commit, returning the new roots and leaf
/// count instead of checking them against a committed state. In memory (a disk store's undo journal is cl_commit's).
/// Also returns the block's flips (position, tx, vin) in spend order, as cl_commit does: a history.bin record
/// (HAZYNC_PREP_HISTORY) is the same bytes whichever of prep or a proving run wrote it.
/// Also returns the block's undo body (cl_undo_body), the same bytes its pending.bin record carries, for prep's reorg
/// undo record.
pub(crate) fn cl_apply(spends: &[ClSpend], appends: &[[u8; 32]], height: u32) -> (Vec<Option<[u8; 32]>>, u64, Vec<(u64, u32, u32)>, Vec<u8>) {
    let mut g = CL.lock().unwrap();
    let h = g.as_mut().expect("coin log not initialised");
    let sp: Vec<(u64, [u8; 32])> = spends.iter().map(|(l, _, _)| (h.pos.get(l).unwrap_or(u64::MAX), *l)).collect();
    let body = cl_undo_body(h.store.len(), &sp, appends);
    // Disk mode: journal the block first, as cl_commit (prep advances the store in place and saves only at its save
    // points, so a crash mid-range is undone back to the last one).
    if let (Some(d), libzync::host::AnyNodes::Disk(_)) = (cl_dir(), &h.store.nodes) {
        cl_journal_append_body(&d, height, &body);
    }
    let mut flips = Vec::with_capacity(spends.len());
    for (l, t, v) in spends {
        let p = h.pos.remove(l).unwrap_or_else(|| panic!("prep: block {height} spends a coin the coin log does not hold"));
        h.store.set(p, libzync::coinlog::spent(l, &libzync::coinlog::spender(height, *t, *v)));
        flips.push((p, *t, *v));
    }
    for l in appends {
        h.pos.insert(*l, h.store.len());
        h.store.push(*l);
    }
    (h.store.roots(), h.store.len(), flips, body)
}

/// The guest's `undo: block <h> inputs <n> hash <hex>` line (agg_lite.rs) for block `h`: (inputs, hash).
pub(crate) fn guest_undo_line(log: &str, h: u32) -> Option<(u64, [u8; 32])> {
    let want = format!("undo: block {h} inputs ");
    log.lines().filter_map(|l| l.find(&want).map(|i| &l[i + want.len()..])).find_map(|rest| {
        let (n, x) = rest.split_once(" hash ")?;
        let x = x.trim();
        if x.len() != 64 || !x.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
        Some((n.parse().ok()?, crate::hx(x).try_into().ok()?))
    })
}

/// Records already in `<out>/history.bin` (libzync::history), counted once per run; None = the file does not run
/// contiguously from genesis to this run's start, so this run writes none (paths need a history from genesis).
pub(crate) static HIST_N: std::sync::Mutex<Option<Option<u64>>> = std::sync::Mutex::new(None);

/// Appends height `rec.height` to `<out>/history.bin`. A run from genesis writes height 0 first. Records past the
/// new height (a resumed or re-run range) are cut off first, so the file always ends at the newest proved height.
pub(crate) fn hist_append(out: &str, rec: &libzync::history::Rec) {
    use std::io::Write as _;
    let path = format!("{out}/history.bin");
    let mut g = HIST_N.lock().unwrap();
    if g.is_none() {
        let b = std::fs::read(&path).unwrap_or_default();
        // The longest whole prefix (a torn last append is cut off), then cut back to this run's start height.
        let (r, good, torn) = libzync::history::decode_prefix(&b);
        if good < b.len() && !torn {
            // Corruption inside the file, not a torn tail: keep it for inspection, write no history this run.
            eprintln!("  history.bin: a corrupt record after {} good ones (not a torn write); no zyncAPP history for this run", r.len());
            *g = Some(None);
            return;
        }
        if good < b.len() {
            eprintln!("  history.bin: {} trailing bytes are not a whole record (a torn write): cut off", b.len() - good);
        }
        let keep = (rec.height as usize).min(r.len());
        if keep < r.len() || good < b.len() {
            let cut: Vec<u8> = r[..keep].iter().flat_map(|x| x.encode()).collect();
            write_atomic(&path, &cut);
        }
        *g = Some(Some(keep as u64));
    }
    let Some(Some(n)) = *g else { return };
    if n != rec.height as u64 {
        eprintln!("  history.bin ends at {} records, this run is at height {}: no zyncAPP history for this run", n, rec.height);
        *g = Some(None);
        return;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    f.write_all(&rec.encode()).unwrap_or_else(|e| panic!("{path}: {e}"));
    f.sync_data().unwrap_or_else(|e| panic!("{path}: {e}"));
    *g = Some(Some(n + 1));
}

/// Gate / single-block measurement (engine-export): a coin log of the modelled size around ONE real block. Each spend's
/// position comes from its coin's real creation height (block file `coin_height`), the log size from k x cumulative
/// transactions ($HAZYNC_COINLOG_TXCOUNTS, getchaintxstats pairs; k = $HAZYNC_COINLOG_K, default 1.64), a random rank
/// inside the creation block. Untouched subtrees are random; the touched trees' old roots are what the witness folds
/// to. Cycles only: such a state is not the chain's.
pub(crate) fn cl_synth(block: &str, spends: &[ClSpend]) -> (Vec<Option<[u8; 32]>>, u64, Vec<u32>) {
    let spends: Vec<[u8; 32]> = spends.iter().map(|s| s.0).collect();
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(block).unwrap()).unwrap();
    let h = j["height"].as_u64().unwrap() as f64;
    let mut heights: Vec<f64> = Vec::new();
    for tx in j["txs"].as_array().unwrap() {
        for p in tx["prevouts"].as_array().unwrap() {
            let c = p["coin_height"].as_u64().unwrap() as f64;
            if c < h { heights.push(c); }
        }
    }
    assert_eq!(heights.len(), spends.len(), "earlier-coin spends in the block file != the aggregate's");
    let tcp = std::env::var("HAZYNC_COINLOG_TXCOUNTS").expect("HAZYNC_COINLOG_TXCOUNTS (getchaintxstats pairs)");
    let k: f64 = std::env::var("HAZYNC_COINLOG_K").ok().and_then(|v| v.parse().ok()).unwrap_or(1.64);
    let tc: Vec<(f64, f64)> = std::fs::read_to_string(&tcp).unwrap().lines()
        .map(|l| { let v: Vec<f64> = l.split_whitespace().map(|x| x.parse().unwrap()).collect(); (v[0], v[1]) }).collect();
    let cum = |x: f64| -> f64 {
        let i = tc.iter().position(|t| t.0 >= x).unwrap_or(tc.len() - 1).max(1);
        let ((h0, c0), (h1, c1)) = (tc[i - 1], tc[i]);
        k * (c0 + (c1 - c0) * (x - h0) / (h1 - h0))
    };
    let n = cum(h - 1.0) as u64;
    let rnd = |tag: &[u8], i: u64| -> [u8; 32] { sha256_host(&[tag, &i.to_le_bytes()].concat()) };
    let mut taken = std::collections::HashSet::new();
    let positions: Vec<u64> = heights.iter().enumerate().map(|(i, &c)| {
        let (a, b) = (cum(c - 1.0), cum(c).max(cum(c - 1.0) + 1.0));
        let u = u64::from_le_bytes(rnd(b"rank", i as u64)[..8].try_into().unwrap()) as f64 / u64::MAX as f64;
        let mut p = ((a + u * (b - a)) as u64).min(n - 1);
        while !taken.insert(p) { p = (p + 1) % n; }
        p
    }).collect();
    let mut sorted: Vec<(u64, [u8; 32])> = positions.iter().copied().zip(spends.iter().copied()).collect();
    sorted.sort_unstable_by_key(|x| x.0);
    // siblings: every untouched subtree on the way down, random
    fn wit(l: u32, start: u64, e: &[(u64, [u8; 32])], out: &mut Vec<[u8; 32]>, rnd: &dyn Fn(&[u8], u64) -> [u8; 32]) {
        if e.is_empty() { out.push(rnd(b"sib", start ^ ((l as u64) << 56))); return; }
        if l == 0 { return; }
        let mid = start + (1u64 << (l - 1));
        let m = e.partition_point(|x| x.0 < mid);
        wit(l - 1, start, &e[..m], out, rnd);
        wit(l - 1, mid, &e[m..], out, rnd);
    }
    let mut roots: Vec<Option<[u8; 32]>> = vec![None; 64];
    let mut sib_all = Vec::new();
    let (mut start, mut kk) = (0u64, 0usize);
    for l in (0..64u32).rev() {
        if (n >> l) & 1 == 0 { continue; }
        let end = start + (1u64 << l);
        let m = kk + sorted[kk..].partition_point(|x| x.0 < end);
        if m > kk {
            let mut sib = Vec::new();
            wit(l, start, &sorted[kk..m], &mut sib, &rnd);
            use libzync::coinlog::{w_of, h_of};
            // Old roots only: the spender never enters the old side, so any spender serves here.
            let e: Vec<libzync::coinlog_forest::Spend> = sorted[kk..m].iter().map(|(p, h)| (*p, w_of(h), [0u32; 8])).collect();
            let sw: Vec<libzync::coinlog::W> = sib.iter().map(w_of).collect();
            let (mut o, mut nw) = ([0u32; 8], [0u32; 8]);
            assert!(libzync::coinlog_forest::update(l, start, &e, &mut sw.iter(), &mut o, &mut nw));
            roots[l as usize] = Some(h_of(&o));
            sib_all.extend(sib);
        } else {
            roots[l as usize] = Some(rnd(b"root", l as u64));
        }
        kk = m;
        start = end;
    }
    while roots.last() == Some(&None) { roots.pop(); }
    println!("  coinlog SYNTH: log size {n} (k = {k}), {} spends, {} siblings", spends.len(), sib_all.len());
    (roots, n, libzync::coinlog_forest::encode_witness(&positions, &sib_all))
}

#[cfg(test)]
mod guest_failure_tests {
    use super::guest_failure;

    #[test]
    fn an_abort_reads_as_a_refused_block_and_other_failures_keep_their_message() {
        let a = guest_failure("chunk execution", &"Guest panicked: C abort()");
        assert!(a.starts_with("BLOCK REFUSED (chunk execution)") && a.contains("C abort()"), "{a}");
        let r = guest_failure("aggregate execution", &"Guest panicked: agg-lite: REJECT no trie receipt for this block");
        assert_eq!(r, "aggregate execution failed: Guest panicked: agg-lite: REJECT no trie receipt for this block");
    }
}

#[cfg(test)]
mod resplit_tests {
    use super::{is_guest_oom, resplit_on_oom};
    use crate::chunks::{split_chunk, split_cut};

    // The engine guest's out-of-memory stop (openvm-platform v2.1.0's heap/bump.rs), as engine_backend::try_execute
    // reports it. The crate builds into the GUEST, not this host, so its source is not reachable here: the test below
    // pins the OpenVM version the engine guest takes instead, and a bump of it must re-check this text.
    const OOM: &str = "engine execute of b5_collect_0_9 failed: \"VM error\"; guest log ends: ERROR: Maximum memory exceeded, program terminating.";

    fn halve(lo: usize, hi: usize) -> Result<usize, &'static str> { if hi - lo >= 2 { Ok(lo + (hi - lo) / 2) } else { Err("a single input") } }

    fn covers(pieces: &[(usize, usize)], n: usize) {
        assert_eq!(pieces.first().map(|p| p.0), Some(0));
        assert_eq!(pieces.last().map(|p| p.1), Some(n));
        assert!(pieces.windows(2).all(|w| w[0].1 == w[1].0) && pieces.iter().all(|p| p.0 < p.1), "{pieces:?}");
    }

    #[test]
    fn only_the_bump_allocator_message_is_out_of_memory() {
        assert!(is_guest_oom(OOM));
        assert!(is_guest_oom(&format!("chunk proof: {OOM}")));
        assert!(!is_guest_oom("engine execute of b5_collect_0_9 failed: \"VM error\"; guest log ends: agg-lite: REJECT x"));
        assert!(!is_guest_oom("engine execute of b5 failed: \"VM error\"; guest log ends: C abort()"));
        assert!(!is_guest_oom("Maximum memory exceeded"));
        assert!(!is_guest_oom("Out of memory"));
        // RISC Zero's bump allocator text (the RISC Zero era's guest): no executor here produces it any more
        assert!(!is_guest_oom("Guest panicked: Out of memory! You have been using the default bump allocator which does not reclaim memory."));
    }

    #[test]
    fn a_chunk_that_runs_out_of_memory_is_re_split_until_it_runs() {
        // injected: a chunk of more than 3 inputs runs out of memory
        let mut runs: Vec<(usize, usize)> = Vec::new();
        let out = resplit_on_oom(&[(0, 10), (10, 12)], &halve, &mut |todo| {
            runs.extend_from_slice(todo);
            todo.iter().map(|&(lo, hi)| if hi - lo > 3 { Err(OOM.to_string()) } else { Ok(hi - lo) }).collect()
        }).unwrap();
        let pieces: Vec<_> = out.iter().map(|(b, _)| *b).collect();
        assert_eq!(pieces, vec![(0, 2), (2, 5), (5, 7), (7, 10), (10, 12)]);
        covers(&pieces, 12);
        assert!(out.iter().all(|((lo, hi), n)| hi - lo == *n));
        // round 1: both; round 2: the halves of 0..10; round 3: the halves of each (10..12 ran once)
        assert_eq!(runs, vec![(0, 10), (10, 12), (0, 5), (5, 10), (0, 2), (2, 5), (5, 7), (7, 10)]);
    }

    #[test]
    fn a_failure_that_is_not_out_of_memory_is_never_retried() {
        let mut calls = 0;
        let r: Result<Vec<_>, _> = resplit_on_oom(&[(0, 4), (4, 8)], &halve, &mut |todo| {
            calls += 1;
            todo.iter().map(|&(lo, _)| if lo == 4 { Err("Guest panicked: C abort()".to_string()) } else { Ok(()) }).collect()
        });
        assert_eq!(r.err().as_deref(), Some("Guest panicked: C abort()"));
        assert_eq!(calls, 1);
    }

    #[test]
    fn a_reject_inside_a_chunk_that_ran_out_of_memory_still_rejects() {
        // 0..8 runs out of memory; once small enough, the piece holding input 6 rejects
        let r: Result<Vec<_>, _> = resplit_on_oom(&[(0, 8)], &halve, &mut |todo| {
            todo.iter().map(|&(lo, hi)| if hi - lo > 2 { Err(OOM.to_string()) }
                else if (lo..hi).contains(&6) { Err("Guest panicked: REJECT".to_string()) } else { Ok(()) }).collect()
        });
        assert_eq!(r.err().as_deref(), Some("Guest panicked: REJECT"));
    }

    #[test]
    fn a_single_input_that_runs_out_of_memory_fails_and_does_not_loop() {
        let mut rounds = 0;
        let r: Result<Vec<_>, _> = resplit_on_oom(&[(0, 4)], &halve, &mut |todo| {
            rounds += 1;
            assert!(rounds < 10, "looping");
            todo.iter().map(|&(lo, hi)| if (lo..hi).contains(&1) { Err(OOM.to_string()) } else { Ok(()) }).collect()
        });
        let e = r.err().unwrap();
        assert!(e.starts_with("inputs 1..2 run out of guest memory and cannot be split further (a single input)"), "{e}");
        assert!(is_guest_oom(&e), "the cause is kept: {e}");
        assert_eq!(rounds, 3);
        // a split that does not shrink the chunk is refused, not followed
        let r: Result<Vec<((usize, usize), ())>, _> = resplit_on_oom(&[(0, 4)], &|lo, _| Ok(lo), &mut |t| t.iter().map(|_| Err(OOM.to_string())).collect());
        assert!(r.err().unwrap().contains("cannot be split further (the split gave a cut outside the chunk)"));
    }

    #[test]
    fn a_round_that_stops_at_a_failure_returns_it_and_runs_nothing_more() {
        // the prove round's shape: chunks in turn, stopping at the first failure that is not out-of-memory
        let mut ran: Vec<(usize, usize)> = Vec::new();
        let r: Result<Vec<_>, _> = resplit_on_oom(&[(0, 2), (2, 4), (4, 6), (6, 8)], &halve, &mut |todo| {
            let mut out = Vec::new();
            for &(lo, hi) in todo {
                ran.push((lo, hi));
                let x = match lo { 0 => Err(OOM.to_string()), 2 => Err("proof failed: GPU fault".to_string()), _ => Ok(()) };
                let fatal = matches!(&x, Err(e) if !is_guest_oom(e));
                out.push(x);
                if fatal { break }
            }
            out
        });
        assert_eq!(r.err().as_deref(), Some("proof failed: GPU fault"));
        assert_eq!(ran, vec![(0, 2), (2, 4)], "nothing after the failure, and no re-split of 0..2");
    }

    #[test]
    fn the_oom_text_is_pinned_to_the_engine_guest_s_openvm() {
        let manifest = include_str!("../../../engine/guest/Cargo.toml.in");
        assert!(manifest.contains("openvm = { git = \"https://github.com/openvm-org/openvm.git\", tag = \"v2.1.0\""),
                "the engine guest's OpenVM moved: re-check OOM (and is_guest_oom) against openvm-platform's heap/bump.rs");
        assert!(is_guest_oom(OOM));
    }

    #[test]
    fn split_chunk_reads_the_block_s_inputs() {
        use crate::{BlockInput, PackedHash, PackedHashes, WireProof};
        let wp = || WireProof { leaf: PackedHash::default(), position: 0, siblings: PackedHashes(vec![]) };
        let inp = |tx_idx: u32, cb: u32| BlockInput { tx_idx, input_idx: 0, global_pos: 0, coin_height: 0, coin_is_coinbase: cb,
                                                     coin_mtp: 0, tx_first: 0, proof_i: wp(), proof_last: wp() };
        // tx 0: 2 plain inputs; tx 1: 4 inputs, all of tracked coins (flag bit 1, as chunk_bounds reads it); tx 2: 2 plain
        let ins: Vec<BlockInput> = vec![inp(0, 0), inp(0, 1), inp(1, 2), inp(1, 3), inp(1, 2), inp(1, 2), inp(2, 0), inp(2, 0)];
        let flat = vec![1u64; 8];
        assert_eq!(split_chunk(&ins, &flat, 0, 8), Ok(6), "the midpoint 4 is inside tracked tx 1: moved to its end");
        assert_eq!(split_chunk(&ins, &flat, 2, 6), Err("one tracked transaction"));
        assert_eq!(split_chunk(&ins, &flat, 0, 2), Ok(1), "a coinbase flag (bit 0) alone is not tracked");
        assert_eq!(split_chunk(&ins, &flat, 7, 8), Err("a single input"));
    }

    #[test]
    fn the_cut_is_the_cost_midpoint_and_never_inside_a_tracked_transaction() {
        let flat = vec![1u64; 8];
        let plain: Vec<(u32, bool)> = (0..8).map(|i| (i / 4, false)).collect();
        assert_eq!(split_cut(&flat, &plain, 0, 8), Ok(4));
        // costs weigh the cut: the heavy first input stands alone
        assert_eq!(split_cut(&[100, 1, 1, 1], &plain[..4], 0, 4), Ok(1));
        // one untracked transaction can be cut anywhere
        let one: Vec<(u32, bool)> = vec![(0, false); 8];
        assert_eq!(split_cut(&flat, &one, 0, 8), Ok(4));
        // a tracked transaction 2..7 stays whole: the cut moves to its end, else its start
        let tr: Vec<(u32, bool)> = vec![(0, false), (0, false), (1, true), (1, true), (1, true), (1, true), (1, true), (2, false)];
        assert_eq!(split_cut(&flat, &tr, 0, 8), Ok(7));
        assert_eq!(split_cut(&flat, &tr, 0, 7), Ok(2));
        // nothing to cut: one input, or one tracked transaction
        assert_eq!(split_cut(&flat, &tr, 3, 4), Err("a single input"));
        assert_eq!(split_cut(&flat, &tr, 2, 7), Err("one tracked transaction"));
        // a chunk that is not the block's
        assert_eq!(split_cut(&flat, &tr, 4, 9), Err("the chunk does not match the block's inputs"));
        assert_eq!(split_cut(&flat[..7], &tr, 0, 4), Err("the chunk does not match the block's inputs"));
        // a transaction with one untracked input is not tracked
        let mixed: Vec<(u32, bool)> = vec![(0, true), (0, true), (0, false), (0, true)];
        assert_eq!(split_cut(&flat[..4], &mixed, 0, 4), Ok(2));
        // the split pieces of a resplit cover the block
        let out = resplit_on_oom(&[(0, 8)], &|lo, hi| split_cut(&flat, &tr, lo, hi), &mut |todo| {
            todo.iter().map(|&(lo, hi)| if hi - lo > 5 && lo == 0 { Err(OOM.to_string()) } else { Ok(()) }).collect()
        }).unwrap();
        assert_eq!(out.iter().map(|(b, _)| *b).collect::<Vec<_>>(), vec![(0, 2), (2, 7), (7, 8)]);
    }
}

#[cfg(test)]
mod journal_tests {
    use super::*;

    #[test]
    fn journal_records_round_trip_and_refuse_damage() {
        let d = std::env::temp_dir().join(format!("lz-journal-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        // three blocks: 10 -> 12 -> 12 (no appends) -> 13 leaves
        cl_journal_append(&d, 5, 10, &[(3, [7; 32])], &[[1; 32], [2; 32]]);
        cl_journal_append(&d, 6, 12, &[(11, [8; 32])], &[]);
        cl_journal_append(&d, 7, 12, &[], &[[3; 32]]);
        let u = std::fs::read(d.join("pending.bin")).unwrap();
        let r = cl_journal_records(&u).unwrap();
        assert_eq!(r.iter().map(|x| x.0).collect::<Vec<_>>(), vec![Some(5), Some(6), Some(7)]);
        assert_eq!(r.iter().map(|x| cl_record_span(x.1)).collect::<Vec<_>>(), vec![(10, 12), (12, 12), (12, 13)]);
        // a torn LAST record (its sync never returned) is ignored
        let torn = &u[..u.len() - 5];
        assert_eq!(cl_journal_records(torn).unwrap().len(), 2);
        // a zero-filled tail of the last record fails its checksum: torn, ignored
        let mut z = u.clone();
        let n = z.len();
        for b in &mut z[n - 40..n - 32] { *b = 0; }
        assert_eq!(cl_journal_records(&z).unwrap().len(), 2);
        // damage in a record that is NOT the last: the journal is refused, nothing is undone from it.
        // (a) a count byte: the record's lengths no longer parse
        let mut m = u.clone();
        m[30] ^= 1;
        assert!(cl_journal_records(&m).is_err());
        // (b) a byte of record 1's spent LEAF (magic 8 | height 8 | n 8 | k 8 | pos 8 | leaf 32 ...): the lengths still
        // parse, only the checksum catches it
        let mut c = u.clone();
        c[50] ^= 1;
        assert!(cl_journal_records(&c).is_err(), "a flipped leaf byte mid-file");
        // a pre-release LZPEND02 journal is refused, not misread
        let mut old = u.clone();
        old[..8].copy_from_slice(b"LZPEND02");
        assert!(cl_journal_records(&old).is_err());
        // the record body is cl_undo_body's bytes: the reorg undo records (reorg.rs) carry the same ones
        assert_eq!(r[0].1, &cl_undo_body(10, &[(3, [7; 32])], &[[1; 32], [2; 32]])[..]);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// REORG.md §6 (P1): the coin log body an undo record carries undoes its blocks on a DISK store, newest first, back
    /// to any earlier block, after a save point (no pending.bin involved), as P3's rewind will use it.
    #[test]
    fn undo_record_bodies_undo_blocks_on_a_disk_store() {
        use libzync::coinlog::{spender, spent};
        let d = std::env::temp_dir().join(format!("lz-undo-body-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let mut store = libzync::coinlog_forest::NodeStore::with_nodes(libzync::host::AnyNodes::Disk(libzync::host::DiskNodes::open(&d).unwrap()));
        let mut pos = libzync::host::AnyIndex::Mem(std::collections::HashMap::new());
        let leaf = |i: u32| -> [u8; 32] { let mut l = [0u8; 32]; l[..4].copy_from_slice(&i.to_le_bytes()); l[31] = 1; l };
        let mut next = 0u32;
        let mut states = Vec::new();   // (leaf count, roots) after each block
        let mut bodies = Vec::new();   // each block's undo body, through an undo record's encode/decode
        let mut x = 7u64;
        let mut rnd = || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x };
        for blk in 0..40u32 {
            // spend up to 3 live coins, append up to 4
            let live: Vec<([u8; 32], u64)> = (0..store.len()).filter_map(|p| { let l = store.leaf(p); (pos.get(&l) == Some(p)).then_some((l, p)) }).collect();
            let mut sp: Vec<(u64, [u8; 32])> = Vec::new();
            for _ in 0..(rnd() % 4) {
                if live.is_empty() { break; }
                let (l, p) = live[(rnd() % live.len() as u64) as usize];
                if !sp.iter().any(|s| s.0 == p) { sp.push((p, l)); }
            }
            let ap: Vec<[u8; 32]> = (0..(rnd() % 5)).map(|_| { next += 1; leaf(next) }).collect();
            let body = cl_undo_body(store.len(), &sp, &ap);
            let rec = crate::reorg::UndoRecord { height: blk + 1, hash: [0; 32], prev: [0; 32], cl: body, trie_replaced: false,
                                                 trie_marks: vec![], trie_inserts: vec![], smt_prior: vec![] };
            bodies.push(crate::reorg::UndoRecord::decode(&rec.encode()).unwrap().cl);
            for (p, l) in &sp {
                pos.remove(l);
                store.set(*p, spent(l, &spender(blk + 1, 1, 0)));
            }
            for l in &ap { pos.insert(*l, store.len()); store.push(*l); }
            states.push((store.len(), store.roots()));
        }
        // undo 40..=26 back to the state after block 25, then 25..=11 back to the one after block 10
        for (from, to) in [(40usize, 25usize), (25, 10)] {
            let n = states[to - 1].0;
            for b in bodies[to..from].iter().rev() { cl_undo_to(&mut store, &mut pos, b, n); }
            assert_eq!((store.len(), store.roots()), states[to - 1].clone(), "undone back to block {to}");
            // the index is the state's too: every live coin found at its position
            for p in 0..store.len() { let l = store.leaf(p); if let Some(q) = pos.get(&l) { assert_eq!(q, p); } }
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
