//! libzync: the libzync commands (agg-exec-fake, agg-chunks, libzync-hints, libzync-prove-range,
//! libzync-verify, libzync-snark-wrap), the aggregate's env writing and hint block, the in-process
//! executor, and the coin-log / trie synthesis (cl_synth_around, cl_synth).

use bitcoin::Transaction;
use bitcoin::consensus::deserialize;
use bitcoin::hashes::Hash as _;
use hazync_coinbase_smt::Smt;
use methods::{METHOD_ELF, METHOD_ID};
use risc0_zkvm::{ExecutorEnv, ProverOpts, default_executor, default_prover};
use crate::{BlockWitness, CHAIN_STATE_VERSION, ChainState, KIND_CHAIN, PackedHashes, WireStump, arr,
    build_full, demo_block_forest, genesis_anchor, genesis_params, header_hash, hex, hx, lineage_of,
    normalize_host, pinned_lineage, prove_session_resilient, read_block_json, rev, seg_po2, sha256_host,
    smt_advance_ex, state_journal_bytes, work_u128};
use crate::chunks::{chunk_block_hash, chunk_bounds, nchunks_env, read_chunk_receipts, write_chunk_inputs};
use crate::verify::{MAX_FUTURE_BLOCK_TIME, dev_mode_env, time_ok, verifier_clock};

/// The embedded guest is a libzync guest (methods/build.rs records what it was built for), so the host
/// writes libzync hint blocks. Replaces six run-time env vars that had to match the build exactly.
pub(crate) fn agg_lite() -> bool { methods::LIBZYNC }
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

/// `libzync-snark-wrap <receipt> <out>`: compress a libzync chain receipt (succinct STARK, ~227 KB) to Groth16 (a
/// few hundred bytes of seal), for shipping to light clients. Refuses a receipt that is not a real proof of this
/// image's ChainState, and refuses to write a wrap whose journal is not byte-identical to the input's: a wrap that
/// changed the statement would be a forgery vector. Needs the risc0 Groth16 prover (x86, lots of RAM): run on a
/// proving box, not a laptop. Verify the result with `swiftsync-verify`, which accepts either receipt kind.
pub(crate) fn libzync_snark_wrap_cmd(bin: &str, out: &str) {
    use std::time::Instant;
    assert!(methods::LIBZYNC, "libzync-snark-wrap needs a libzync build");
    let r: risc0_zkvm::Receipt = bincode::deserialize(&std::fs::read(bin).unwrap_or_else(|e| panic!("{bin}: {e}")))
        .unwrap_or_else(|e| panic!("{bin}: not a receipt: {e}"));
    assert!(!matches!(r.inner, risc0_zkvm::InnerReceipt::Fake(_)), "refusing to wrap a dev-mode FakeReceipt");
    r.verify(METHOD_ID).expect("the receipt does not verify against this image");
    let cs: ChainState = r.journal.decode().expect("the journal is not a ChainState");
    assert!(cs.kind == KIND_CHAIN && cs.self_id == METHOD_ID, "not a ChainState committed by this image");
    let before = bincode::serialize(&r).map(|v| v.len()).unwrap_or(0);
    println!("=== libzync Groth16 wrap: height {} (STARK -> Groth16), input {before} bytes ===", cs.height);
    let t = Instant::now();
    let snark = default_prover().compress(&ProverOpts::groth16(), &r).expect("groth16 compress");
    snark.verify(METHOD_ID).expect("the wrapped receipt does not verify");
    assert_eq!(snark.journal.bytes, r.journal.bytes, "the wrapped journal differs from the input's -- refusing to write it");
    let bytes = bincode::serialize(&snark).unwrap();
    std::fs::write(out, &bytes).unwrap_or_else(|e| panic!("{out}: {e}"));
    println!("LIBZYNC_SNARK_WRAPPED: {out}, {} bytes ({:.0}x smaller), {:.1} s", bytes.len(),
             before as f64 / bytes.len().max(1) as f64, t.elapsed().as_secs_f64());
}

/// `libzync-verify <receipt> [anchor.json]`: everything a user checks before trusting a Hazync chain proof. Exit 0
/// only if every check passes (2 = DEMO: a valid chain not shown to be the network's).
///
///  1. the receipt verifies against THIS host's METHOD_ID (the libzync image for the network), and the
///     journal is a ChainState whose self_id is that id (so every recursion step ran the same guest);
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

/// Core v28 consensus.nMinimumChainWork for this network, little-endian like cum_work (0 on regtest).
pub(crate) fn min_chain_work() -> [u8; 32] {
    let h = match methods::NETWORK {
        "mainnet" => "000000000000000000000000000000000000000088e186b70e0862c193ec44d6",
        "signet" => "0000000000000000000000000000000000000000000000000000025dbd66e58f",
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
    let r: risc0_zkvm::Receipt = bincode::deserialize(&std::fs::read(receipt).unwrap_or_else(|e| panic!("{receipt:?}: {e}")))
        .unwrap_or_else(|e| panic!("{receipt:?}: not a receipt: {e}"));
    let mut fails = 0;
    let mut check = |ok: bool, what: &str| {
        println!("  [{}] {what}", if ok { "ok" } else { "FAIL" });
        if !ok { fails += 1; }
    };
    // A FakeReceipt "verifies" whenever RISC0_DEV_MODE is set in the verifier's environment (review 2), so
    // refuse both: a dev-mode receipt is never a proof, whatever this shell has exported.
    let fake = matches!(r.inner, risc0_zkvm::InnerReceipt::Fake(_));
    let dev = dev_mode_env();
    check(!fake, "receipt is a real proof (not a dev-mode FakeReceipt)");
    check(!dev, "RISC0_DEV_MODE is not set in this verifier's environment");
    let v = r.verify(METHOD_ID);
    check(v.is_ok(), &format!("receipt verifies against METHOD_ID {} ({} guest, {}, {})", hex_id(&METHOD_ID),
        if methods::LIBZYNC { "libzync" } else { "canonical" }, methods::NETWORK,
        if methods::LIBZYNC_ASSUMEVALID { "ASSUMEVALID: scripts and signatures NOT checked" } else { "scripts checked" }));
    let cs: ChainState = match r.journal.decode() { Ok(c) => c, Err(e) => {
        println!("  [FAIL] journal is not a ChainState: {}", format!("{e}").escape_debug());
        println!("LIBZYNC_VERIFY_FAIL: the journal does not decode as a ChainState");
        std::process::exit(1)
    } };
    // A later version only appends fields (HANDOVER.md §2): this reader checks the prefix it knows.
    check(cs.kind == KIND_CHAIN && cs.version >= CHAIN_STATE_VERSION && cs.self_id == METHOD_ID,
          &format!("journal is a ChainState v{} committed by this image (kind, version, self_id)", cs.version));
    // The profile is committed by the image itself (bits from the compiled code, never from input), so the
    // proof states what was checked; it must also be what this verifier was built to accept.
    check(cs.profile == expected_profile(), &(format!("profile in the proof 0x{:x} == this verifier's 0x{:x} ({}{}{}{})",
        cs.profile, expected_profile(), if cs.profile & 1 != 0 { "libzync" } else { "canonical" },
        if cs.profile & 2 != 0 { ", ASSUMEVALID: scripts and signatures NOT checked" } else { ", scripts checked" },
        if cs.profile & 4 != 0 { ", signet" } else { ", mainnet" }, if cs.profile & 16 != 0 { ", coin log" } else { "" }) + if cs.profile & 32 != 0 { ", counting trie" } else { "" }
        + if cs.profile & 64 != 0 { ", live-coin hash" } else { "" }));
    // time-too-new (Core ContextualCheckBlockHeader): no block more than 2 h past THIS node's clock. The proof
    // has no clock; it commits the largest timestamp and the rule is applied here, as Core applies it to
    // every block during IBD. HAZYNC_VERIFY_NOW overrides the clock for tests, and is itself a failed check.
    let (now, ovr) = verifier_clock();
    if let Some(v) = &ovr {
        check(false, &format!("this verifier's clock is real (HAZYNC_VERIFY_NOW={v} is set: test only)"));
    }
    check(time_ok(cs.max_time, now), &format!(
        "time-too-new: largest block time {} <= this clock {} + {} s", cs.max_time, now, MAX_FUTURE_BLOCK_TIME));
    let (want_anchor, what) = match anchor {
        None => (state_journal_bytes(&genesis_anchor()), "genesis".to_string()),
        Some(p) => {
            let a: ChainState = serde_json::from_str(&std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{p:?}: {e}"))).unwrap_or_else(|e| panic!("{p:?}: {e}"));
            (state_journal_bytes(&a), format!("the GIVEN anchor at height {} ({p:?}) -- NOT genesis", a.height))
        }
    };
    let mut want_anchor = bitcoin::hashes::sha256d::Hash::hash(&want_anchor).to_byte_array();
    // Genesis anchors are PINNED per network (as swiftsync-verify's Image::genesis_anchor_hex): a chain keeps its
    // anchor_id across every handover, so a later image must not recompute it in its own layout (review 11-03).
    // The pin is used as it is, never re-derived here: a later layout's genesis_anchor() would differ from it by design.
    // That this layout's anchor equals the pin is checked by swiftsync-verify's unit test and genesis_anchor_id.py.
    if anchor.is_none() {
        if let Some(p) = pinned_genesis_anchor() { want_anchor = p; }
    }
    check(cs.anchor_id == want_anchor, &format!("anchor_id is {what}"));
    // Core's nMinimumChainWork (v28): below it, a valid chain from genesis is no evidence of the network's chain.
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

// `agg-chunks`: read all chunk receipt files, aggregate into the block/chain proof.
// Progress for a long prove (hazync#145). `agg-chunks` printed its header and then nothing at all
// for 35+ minutes, so a working fold and a hung one looked identical from the outside. That is not a
// cosmetic complaint: hazync#147 wedged twice, for 76 minutes and 3h38m, and what eventually gave it
// away was `nvidia-smi` showing 0% with the process alive -- not any output from the prover.
//
// The per-segment progress now comes from `prove_session_resilient`, which drives the segment loop
// itself (#237) rather than observing `prove_session` through a `SessionEvents` hook. Same lines,
// same cadence, and every segment additionally goes through the #119 retry.
// HAZYNC_AGG_EXECUTE=1 — execute mode 5 WITHOUT proving, and report its cycles.
//
// Settles which half of the aggregate is expensive. In execute mode `env::verify` merely RECORDS an
// assumption; RESOLVING it is recursion, and that cost lands in PROVING. So this cycle count is the
// block-validation work alone, and the gap between it and the measured prove wall-clock is what the
// sixteen assumption resolutions cost.
//
// It matters because the two pull in opposite directions: more chunks parallelise proving better
// and, if resolution dominates, make the aggregate worse. HAZYNC_CHUNKS has been treated as free.
// Host-side only; needs no GPU and does not move METHOD_ID.
//
// A FUNCTION RATHER THAN AN INLINE BLOCK, because check_seg_guards.sh requires a prove call within
// 25 lines of its segment_limit_po2 guard, and inline this pushed the aggregate's prove to 38 lines
// away. The guard was present and the check was right to complain: distance is what makes it
// readable at a glance that a prove path sets its segment size.
pub(crate) fn report_agg_execute_only(mut b: risc0_zkvm::ExecutorEnvBuilder) {
    use std::time::Instant;
    let t_x = Instant::now();
    let session = default_executor().execute(b.build().unwrap(), METHOD_ELF).expect("execute mode 5");
    let cycles = session.cycles();
    println!("=== AGGREGATE, EXECUTE ONLY (no proving) ===");
    println!("  block validation cycles  {cycles}");
    // What the prover pays for: every segment padded to its power of two (user cycles + paging + padding).
    println!("  block validation padded  {} ({} segments)", session.segments.iter().map(|g| 1u64 << g.po2).sum::<u64>(), session.segments.len());
    // Same-contents check across builds: the journal embeds the image's own id, so compare dumps byte by byte and
    // expect differences only there (E2a).
    if let Ok(path) = std::env::var("HAZYNC_JOURNAL_DUMP") { std::fs::write(&path, &session.journal.bytes).expect("HAZYNC_JOURNAL_DUMP"); }
    {
        use sha2::Digest;
        println!("  journal sha256           {}", hex(&sha2::Sha256::digest(&session.journal.bytes)));
    }
    println!("  segments at po2 {}        ~{}", seg_po2(), cycles.div_ceil(1u64 << seg_po2()));
    println!("  executed in              {:.1}s", t_x.elapsed().as_secs_f64());
    println!();
    println!("  Scale: chunk 9 is 948,436,992 cycles and proved in ~915 s on a B200, so this much");
    println!("  block-validation work is roughly {:.0} s of segment proving. Whatever the FULL",
             cycles as f64 / 948_436_992.0 * 915.0);
    println!("  aggregate cost beyond that (measured: >3,300 s) is the assumption resolutions.");
}

pub(crate) fn write_aggregate_env(
    b: &mut risc0_zkvm::ExecutorEnvBuilder<'_>,
    receipts: &[risc0_zkvm::Receipt],
    anchor: &ChainState,
    w: &BlockWitness,
) {
    write_aggregate_env_ex(b, receipts, anchor, None, w, 1);
}

/// `step` is the guest's is_base. 1: a base step on `anchor`. 0: `anchor` is the PREVIOUS block's committed chain
/// state, whose receipt the caller has added as an assumption -- the guest verifies it recursively instead of trusting
/// it. 2: the same, with a receipt of the predecessor image (a handover).
/// `prev_journal`: for steps 0 and 2, the previous receipt's journal BYTES, sent as they are. Re-encoding the decoded
/// state through this host's struct only round-trips while the layouts match, which is exactly what can change at a
/// handover (review 11-03), and the guest's env::verify needs the receipt's own bytes.
pub(crate) fn write_aggregate_env_ex(
    b: &mut risc0_zkvm::ExecutorEnvBuilder<'_>,
    receipts: &[risc0_zkvm::Receipt],
    anchor: &ChainState,
    prev_journal: Option<&[u8]>,
    w: &BlockWitness,
    step: u32,
) {
    b.segment_limit_po2(seg_po2());
    for r in receipts { b.add_assumption(r.clone()); }
    b.write(&5u32).unwrap();
    b.write(&METHOD_ID).unwrap();
    b.write(&(receipts.len() as u32)).unwrap();
    // #136's fix for the AGGREGATE. Guest reads these with read_slice when built with
    // `agg-readslice`; a length prefix then a 4-byte-padded slice, exactly as write_chunk_inputs does.
    // ⛔ HAZYNC_AGG_READSLICE must hold the SAME value at BUILD and RUN time -- the guest only reads
    // this shape when compiled for it, so a mismatch desynchronises the stream rather than merely
    // losing the optimisation. Same discipline as HAZYNC_BIGINT2_ECDSA.
    if methods::LIBZYNC || std::env::var("HAZYNC_AGG_READSLICE").as_deref() == Ok("1") {
        fn padded(v: &[u8]) -> Vec<u8> {
            let mut p = v.to_vec();
            p.resize(v.len().div_ceil(4) * 4, 0);
            p
        }
        for r in receipts {
            b.write(&(r.journal.bytes.len() as u32)).unwrap();
            b.write_slice(&padded(&r.journal.bytes));
        }
        let sj = prev_journal.map(|j| j.to_vec()).unwrap_or_else(|| state_journal_bytes(anchor));
        b.write(&(sj.len() as u32)).unwrap();
        b.write_slice(&padded(&sj));
    } else {
        for r in receipts { b.write(&r.journal.bytes).unwrap(); }
        b.write(&prev_journal.map(|j| j.to_vec()).unwrap_or_else(|| state_journal_bytes(anchor))).unwrap();
    }
    b.write(w).unwrap();
    // Negative-test hook (test-only, inert unless set): send is_base = 2, which the guest must refuse.
    let raw_base = if std::env::var("HAZYNC_TEST_ISBASE2").is_ok() { 2u32 } else { step };
    b.write(&raw_base).unwrap();
}

/// libzync: `agg-exec-fake` — the mode-5 aggregate in EXECUTE mode on a laptop, with no chunk proving.
///
/// Each chunk is executed (not proved) and its claim wrapped in a FakeReceipt. In execute mode
/// `env::verify` only matches the assumption's claim; it never checks a seal, so this measures
/// exactly the block-validation work `HAZYNC_AGG_EXECUTE` measures from real receipts. With
/// RISC0_PPROF_OUT set, the LAST profile written is the aggregate's. Batch-ECDSA hints apply to the
/// chunk executions exactly as in chunk-profile.
pub(crate) fn agg_exec_fake() {
    use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt};
    let (anchor, mut w) = build_full();
    let bounds = chunk_bounds(&w, nchunks_env());
    let mut receipts = Vec::with_capacity(bounds.len());
    for (i, &(lo, hi)) in bounds.iter().enumerate() {
        let mut b = ExecutorEnv::builder();
        b.segment_limit_po2(seg_po2());
        b.write(&4u32).unwrap();
        b.write(&w.height).unwrap();
        b.write(&chunk_block_hash(&w.header)).unwrap();
        write_chunk_inputs(&mut b, &w, lo, hi);
        let s = default_executor().execute(b.build().unwrap(), METHOD_ELF).expect("exec chunk");
        let claim = s.receipt_claim.clone().expect("chunk session has no receipt claim");
        println!("  chunk {i:>3} [{lo}..{hi}) executed: {} cycles", s.cycles());
        println!("  chunk {i:>3} padded {} ({} segments)", s.segments.iter().map(|g| 1u64 << g.po2).sum::<u64>(), s.segments.len());
        receipts.push(Receipt::new(InnerReceipt::Fake(FakeReceipt::new(claim)), s.journal.bytes.clone()));
    }
    // Negative control (test-only): HAZYNC_LITE_DROP_CHUNK=i omits chunk i; coverage must fail.
    if agg_lite() {
        if let Some(i) = std::env::var("HAZYNC_LITE_DROP_CHUNK").ok().and_then(|v| v.parse::<usize>().ok()) {
            receipts.remove(i);
            println!("  agg-lite NEGATIVE CONTROL: chunk {i} dropped");
        }
    }
    libzync_agg_prep(&mut w);
    let mut anchor = anchor;
    if methods::COINLOG { cl_synth_around(&receipts, &mut anchor, &mut w); }
    let mut b = ExecutorEnv::builder();
    write_aggregate_env(&mut b, &receipts, &anchor, &w);
    libzync_agg_tail(&mut b, &w);
    report_agg_execute_only(b);
}

/// Coin log, single-block measurement (agg-exec-fake, agg-chunks): a collect run for the block's spends, then a
/// modelled log around them (cl_synth) set as the anchor's coin state. A demonstration anchor: cycles, not history.
pub(crate) fn cl_synth_around(receipts: &[risc0_zkvm::Receipt], anchor: &mut ChainState, w: &mut BlockWitness) {
    // collect run for the block's spends, then a modelled log around them (cl_synth)
    let hints = std::env::var("HAZYNC_LZ_HINTS").ok();
    std::env::remove_var("HAZYNC_LZ_HINTS");
    let mut log = Vec::new();
    {
        let mut b = ExecutorEnv::builder();
        b.stdout(&mut log);
        write_aggregate_env(&mut b, receipts, anchor, w);
        libzync_agg_tail(&mut b, w);
        let r = lz_execute(b.build().unwrap());
        drop(b);
        if let Err(e) = r { assert!(e.contains("collect run commits no chain state"), "coin-log collect run failed: {e}"); }
    }
    if let Some(h) = hints { std::env::set_var("HAZYNC_LZ_HINTS", h); }
    let (spends, appends) = cl_parse(&String::from_utf8_lossy(&log));
    let (roots, n, wit) = cl_synth(&std::env::var("HAZYNC_BLOCK").unwrap(), &spends);
    // The counting trie: the keys the collect run logged. The aggregate takes no trie witness (the trie is applied in its
    // own receipt, guest mode 17); for measurement the host synthesises the trie's witness around a modelled population.
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
        let mut b = ExecutorEnv::builder();
        crate::trie::trie_env(&mut b, &wit, &marks, &ins);
        let t = lz_execute(b.build().unwrap()).unwrap_or_else(|e| panic!("trie update (mode 17) failed: {e}"));
        println!("  trie: {} marks, {} inserts, witness {} words; trie update executed: {} cycles, padded {} ({} segments)",
                 marks.len(), ins.len(), wit.len(), t.cycles, t.padded, t.segments);
        let claim = t.receipt_claim.expect("trie session has no receipt claim");
        *TRIE_RCPT.lock().unwrap() = if neg == "none" { None } else { Some(risc0_zkvm::Receipt::new(
            risc0_zkvm::InnerReceipt::Fake(risc0_zkvm::FakeReceipt::new(claim)), t.journal.bytes.clone())) };
        anchor.trie_root = libzync::coinlog::h_of(&root);
        if neg == "root" { anchor.trie_root[0] ^= 1; }
        if !neg.is_empty() { println!("  trie: NEGATIVE CONTROL HAZYNC_TEST_TRIE={neg}"); }
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
/// Shared by `agg-exec-fake` and `agg-chunks` (the real-receipt aggregate) so both send one stream.
pub(crate) fn libzync_agg_prep(w: &mut BlockWitness) {
    if methods::LIBZYNC {
        for inp in w.inputs.iter_mut() {
            inp.proof_i.siblings = PackedHashes(Vec::new());
            inp.proof_last.siblings = PackedHashes(Vec::new());
        }
    }
    if agg_lite() {
        w.txs.clear();
        w.tx_prevouts.clear();
        w.inputs.clear();
        w.txids = PackedHashes(Vec::new());
    }
}

/// libzync: the aggregate's ONE hint block, after the witness: ECMH hints from the block's bundle
/// ($HAZYNC_LZ_HINTS; none = the guest collects, logging ECMHREC lines).
pub(crate) fn libzync_agg_tail(b: &mut risc0_zkvm::ExecutorEnvBuilder, w: &BlockWitness) {
    if !methods::LIBZYNC { return; }
    let bundle = lz_bundle(w.height, &header_hash(&w.header));
    let words = libzync::wire::AggHints {
        ecmh_collect: bundle.is_none(),
        ecmh: bundle.as_ref().map(|b| b.ecmh.as_slice()).unwrap_or(&[]),
    }.encode();
    b.write(&(words.len() as u32)).unwrap();
    b.write_slice(&words);
    // coin log: the block's witness (empty in a collect run, which logs what the witness must cover)
    if methods::COINLOG {
        let wit = if bundle.is_none() { Vec::new() } else { CL_WIT.lock().unwrap().clone() };
        b.write(&(wit.len() as u32)).unwrap();
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
        b.write(&((order.len() / 2) as u32)).unwrap();
        if !order.is_empty() { b.write_slice(&order); }
    }
    // E2: the trie receipt's journal (empty in a collect run), its receipt an assumption the aggregate verifies
    if methods::TRIE {
        let r = if bundle.is_none() { None } else { TRIE_RCPT.lock().unwrap().clone() };
        let tj: Vec<u32> = r.as_ref().map(|r| r.journal.bytes.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()).unwrap_or_default();
        b.write(&(tj.len() as u32)).unwrap();
        if !tj.is_empty() { b.write_slice(&tj); }
        if let Some(r) = r { b.add_assumption(r); }
    }
    match &bundle {
        Some(bd) => println!("  libzync: {} bytes of ECMH hints, no hints file", bd.ecmh.len()),
        None => println!("  libzync: ECMH COLLECT (no bundle)"),
    }
}

/// libzync: the guest's memory image, built ONCE per process. `default_executor().execute` goes through
/// ExecutorImpl::from_elf, which decodes the ELF and builds the image again on every call; the laptop
/// tools (hints, sweeps) execute the guest thousands of times. Same ExecutorImpl, same image, same
/// session -- only the rebuild is skipped. A profiled run (RISC0_PPROF_OUT) takes the normal path,
/// because the profiler is attached in from_elf.
pub(crate) static LZ_IMAGE: std::sync::OnceLock<risc0_binfmt::MemoryImage> = std::sync::OnceLock::new();

/// What the libzync tools read from an execution (risc0's SessionInfo is #[non_exhaustive]).
pub(crate) struct Exec {
    pub(crate) journal: risc0_zkvm::Journal,
    receipt_claim: Option<risc0_zkvm::ReceiptClaim>,
    /// user cycles, padded (proven) cycles, segments
    pub(crate) cycles: u64,
    pub(crate) padded: u64,
    pub(crate) segments: usize,
}

pub(crate) fn lz_execute(env: ExecutorEnv<'_>) -> Result<Exec, String> {
    if std::env::var("RISC0_PPROF_OUT").is_ok() {
        let s = default_executor().execute(env, METHOD_ELF).map_err(|e| e.to_string())?;
        let padded = s.segments.iter().map(|g| 1u64 << g.po2).sum();
        return Ok(Exec { cycles: s.cycles(), padded, segments: s.segments.len(), journal: s.journal, receipt_claim: s.receipt_claim });
    }
    let image = LZ_IMAGE.get_or_init(|| {
        risc0_binfmt::ProgramBinary::decode(METHOD_ELF).and_then(|b| b.to_image()).expect("guest image")
    }).clone();
    let session = risc0_zkvm::ExecutorImpl::new(env, image).map_err(|e| e.to_string())?
        .run_with_callback(|_| Ok(Box::new(risc0_zkvm::NullSegmentRef)))
        .map_err(|e| e.to_string())?;
    let claim = session.claim().map_err(|e| e.to_string())?;
    // Session::total_cycles = every segment padded to its power of two (what proving pays)
    let padded = session.total_cycles;
    Ok(Exec { cycles: session.user_cycles, padded, segments: session.segments.len(), journal: session.journal.unwrap_or_default(), receipt_claim: Some(claim) })
}

/// libzync: execute every chunk of a block at once, one thread each (HAZYNC_HINT_THREADS, default all
/// cores). Returns per chunk a FakeReceipt carrying its claim and journal, plus the guest's captured
/// stdout (BATCHREC lines in collect mode). Used by `libzync-hints` and by seg-serve's HAZYNC_AGG_EARLY.
pub(crate) fn execute_chunks_parallel(w: &BlockWitness, bounds: &[(usize, usize)], threads: usize)
    -> Vec<(risc0_zkvm::Receipt, Vec<u8>)> {
    use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt};
    let next = std::sync::atomic::AtomicUsize::new(0);
    let done: std::sync::Mutex<Vec<Option<(Receipt, Vec<u8>)>>> =
        std::sync::Mutex::new((0..bounds.len()).map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..threads.min(bounds.len()).max(1) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(&(lo, hi)) = bounds.get(i) else { break };
                let mut log = Vec::new();
                let session = {
                    let mut b = ExecutorEnv::builder();
                    b.segment_limit_po2(seg_po2());
                    b.stdout(&mut log);
                    b.write(&4u32).unwrap();
                    b.write(&w.height).unwrap();
                    b.write(&chunk_block_hash(&w.header)).unwrap();
                    write_chunk_inputs(&mut b, w, lo, hi);
                    lz_execute(b.build().unwrap()).expect("execute chunk")
                };
                let claim = session.receipt_claim.clone().expect("chunk session has no receipt claim");
                let r = Receipt::new(InnerReceipt::Fake(FakeReceipt::new(claim)), session.journal.bytes.clone());
                done.lock().unwrap()[i] = Some((r, log));
            });
        }
    });
    done.into_inner().unwrap().into_iter().map(|x| x.expect("chunk")).collect()
}

pub(crate) fn hint_threads() -> usize {
    std::env::var("HAZYNC_HINT_THREADS").ok().and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)).max(1)
}

/// libzync: `libzync-hints <out>` — the block's hint bundle (libzync::host::BlockBundle), in ONE pass.
///
/// - Every chunk executes in COLLECT mode at once, one thread each; the guest's BATCHREC lines come back
///   from each session's stdout, and R = u1·G + u2·Q is computed with libsecp (libzync::host::sig_hint).
/// - The aggregate's ECMH collect reuses those chunk executions: a collect-mode chunk commits the same
///   journal as a hinted one, so no second pass over the chunks.
/// ⛔ The hints are untrusted input to the guest (it re-checks every one), so a bug here can only make a
/// chunk fail, never make a bad signature pass. Uses the same chunk layout as proving (HAZYNC_CHUNKS,
/// HAZYNC_PROFILE_LAYOUT), and records it: a bundle applied under another layout is ignored per chunk.
pub(crate) fn libzync_hints_cmd(out: &str) {
    use risc0_zkvm::Receipt;
    use std::time::Instant;
    assert!(methods::LIBZYNC, "libzync-hints needs a libzync guest (build with HAZYNC_LIBZYNC=1)");
    let t0 = Instant::now();
    // Collect mode for every hint kind: no bundle.
    std::env::remove_var("HAZYNC_LZ_HINTS");
    let (anchor, mut w) = build_full();
    let bounds = chunk_bounds(&w, nchunks_env());
    let threads = hint_threads();
    let done = execute_chunks_parallel(&w, &bounds, threads);
    let t_chunks = t0.elapsed().as_secs_f64();

    let mut bundle = libzync::host::BlockBundle { height: w.height, block_hash: header_hash(&w.header), ..Default::default() };
    let (mut nrec, mut nvalid) = (0usize, 0usize);
    for (i, &(lo, hi)) in bounds.iter().enumerate() {
        let mut blob = Vec::new();
        for rec in libzync::host::sig_records(&String::from_utf8_lossy(&done[i].1)) {
            let h = libzync::host::sig_hint(&rec);
            nrec += 1;
            nvalid += (h[0] == 1) as usize;
            blob.extend_from_slice(&h);
        }
        bundle.chunks.push((lo as u32, hi as u32, blob));
    }
    let t_batch = t0.elapsed().as_secs_f64();

    // ECMH: run the aggregate in collect mode from the chunk claims; its log lists every leaf in order.
    libzync_agg_prep(&mut w);
    let receipts: Vec<Receipt> = done.into_iter().map(|(r, _)| r).collect();
    let mut log = Vec::new();
    {
        let mut b = ExecutorEnv::builder();
        b.stdout(&mut log);
        write_aggregate_env(&mut b, &receipts, &anchor, &w);
        libzync_agg_tail(&mut b, &w);
        // A collect aggregate checks the whole block, logs every leaf, then STOPS before committing
        // (a collect run must never yield a chain state). Any other failure is a real one.
        if let Err(e) = lz_execute(b.build().unwrap()) {
            assert!(e.contains("collect run commits no chain state"), "collect aggregate failed: {e}");
        }
    }
    let leaves = libzync::host::ecmh_leaves(&String::from_utf8_lossy(&log));
    for leaf in &leaves { bundle.ecmh.extend_from_slice(&libzync::host::ecmh_hint(leaf)); }
    bundle.write(out).unwrap_or_else(|e| panic!("{out}: {e}"));
    println!("libzync-hints: block {} {} chunks on {} threads -> {out}", w.height, bounds.len(), threads);
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
    for h in 1..=upto {
        std::env::set_var("HAZYNC_BLOCK", format!("{dir}/block_{h}.json"));
        let (_, mut w) = build_full();
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

/// `libzync-prove-range <lo> <hi> <json dir> <anchor.json> <out dir>`: PROVE blocks lo..=hi as one
/// recursive chain on this machine's prover (GPU with the cuda feature). Block lo is a base step on the
/// given anchor (the committed state at lo-1, e.g. from `libzync-anchor-json`); every
/// later block's aggregate VERIFIES the previous block's receipt (is_base = 0). Writes block_<h>.rcpt
/// (bincode Receipt) and state_<h>.json per block, and times.tsv.
/// Per block: chunks proved (succinct), the aggregate's ECMH hints learned by one collect execution,
/// then the aggregate proved (succinct, so the next block can resolve it cheaply).
/// `<anchor.json>` may be the word `genesis`: the network's genesis state, built here, so lo must be 1.
/// HAZYNC_RESUME=1: continue after the highest block_<k>.rcpt in <out dir>. The receipt is verified against this
/// image, the BIP30 tree is replayed to k and must equal the receipt's smt_root, and block k+1 verifies that receipt
/// in the guest, so a resumed chain is the same statement as an unbroken one. Needs block files 1..k.
pub(crate) fn lz_prove_range_cmd(lo: u32, hi: u32, dir: &str, anchor_path: &str, out: &str) {
    use risc0_zkvm::Receipt;
    use std::io::Write as _;
    use std::time::Instant;
    assert!(methods::LIBZYNC, "libzync-prove-range needs a libzync guest");
    std::fs::create_dir_all(out).unwrap();
    // HAZYNC_HANDOVER_RECEIPT=<receipt>: the first block is a handover step on the pinned predecessor's receipt. The
    // anchor argument is unused (pass the word `handover`): the chain state comes from the verified receipt.
    let handover: Option<(Receipt, ChainState)> = std::env::var("HAZYNC_HANDOVER_RECEIPT").ok().map(|p| {
        let last = pinned_lineage().last().expect("HAZYNC_HANDOVER_RECEIPT: this image has no predecessor pinned");
        let r: Receipt = bincode::deserialize(&std::fs::read(&p).unwrap_or_else(|e| panic!("{p}: {e}"))).unwrap_or_else(|e| panic!("{p}: {e}"));
        r.verify(last.image).unwrap_or_else(|e| panic!("{p} does not verify against the pinned predecessor: {e}"));
        let st: ChainState = r.journal.decode().expect("ChainState");
        let tamper = std::env::var("HAZYNC_TEST_HANDOVER").ok();
        if tamper.is_some() { assert!(dev_mode_env(), "HAZYNC_TEST_HANDOVER is a dev-mode test hook"); }
        else {
            assert!(st.kind == KIND_CHAIN && st.self_id == last.image, "{p} is not the predecessor's chain state");
            assert_eq!(st.height + 1, last.first_height, "{p}: the pin hands over at {}", last.first_height);
        }
        // Negative tests (experimental/libzync-bench/handover/): a dev-mode receipt for a state the predecessor never
        // committed, so one guest check at a time is reached with its assumption resolved, not stopped earlier.
        let mut st = st;
        let mut claim_id: risc0_zkvm::sha::Digest = last.image.into();
        match tamper.as_deref() {
            None | Some("height") => {}
            Some("self_id") => st.self_id[0] ^= 1,
            Some("profile") => st.profile ^= 2,
            Some("lineage") => st.lineage[0] ^= 1,
            Some("forest") => st.block_leaves -= 1,
            Some("kind") => st.kind ^= 1,
            Some("wrong_image") => claim_id = METHOD_ID.into(),
            Some("no_receipt") => {}
            Some(x) => panic!("HAZYNC_TEST_HANDOVER: unknown case {x}"),
        }
        let r = if tamper.is_some() {
            use risc0_zkvm::{FakeReceipt, InnerReceipt, ReceiptClaim};
            let j = state_journal_bytes(&st);
            Receipt::new(InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(claim_id, j.clone()))), j)
        } else { r };
        (r, st)
    });
    let mut prev: ChainState = if let Some((_, st)) = &handover { st.clone() } else if anchor_path == "genesis" { genesis_anchor() } else {
        serde_json::from_str(&std::fs::read_to_string(anchor_path).unwrap_or_else(|e| panic!("{anchor_path}: {e}"))).unwrap()
    };
    demo_block_forest(&mut prev, true);
    assert_eq!(prev.height + 1, lo, "anchor is at height {}, range starts at {lo}", prev.height);
    // history.bin starts at height 0 only on a run from genesis (libzync::history).
    let prev_is_genesis = handover.is_none() && anchor_path == "genesis";
    // The coin log at the run's start. Disk mode (HAZYNC_COINLOG_DIR) with HAZYNC_RESUME and a store already there: leave
    // it alone (loading the anchor would WIPE it, review 11-03); the resume below loads it at its receipt, and only if no
    // receipt is found is the anchor's log loaded after all.
    let disk_resume = methods::COINLOG && std::env::var("HAZYNC_RESUME").as_deref() == Ok("1")
        && cl_dir().map_or(false, |d| d.join("L0.bin").exists());
    if methods::COINLOG && !disk_resume {
        cl_init_at_start(handover.is_some(), anchor_path, &prev);
    }
    // The counting trie at the run's start (a resume below replaces it with the snapshot at its receipt).
    if methods::TRIE { crate::trie::trie_init_at_start(handover.is_some(), anchor_path, &prev); }
    // The BIP30 tree at the anchor. With every block file 1..lo-1 present it is REPLAYED, and must equal the
    // anchor's own smt_root (a sweep anchor from genesis). Otherwise this is a demonstration anchor: the tree
    // is seeded with the coinbases the range spends from before lo, and the anchor's smt_root is SET to it --
    // written to anchor_effective.json, which is what libzync-verify must be given.
    let replay = (1..lo).all(|h| std::path::Path::new(&format!("{dir}/block_{h}.json")).exists());
    // A handover continues a proved chain: its BIP30 tree must be replayed and equal the receipt's, never seeded (the
    // seeding path rewrites smt_root, and the state sent would no longer be the predecessor's journal).
    assert!(replay || handover.is_none(), "a handover needs every block file 1..{} to replay the BIP30 tree", lo - 1);
    // An anchor written before ChainState carried max_time loads it as 0. With the block files present it is
    // exact (genesis and every block to lo-1: timestamps are not monotonic, so the recent window is not enough);
    // for a demonstration anchor only the anchor's own window exists, and the result says so.
    let mut smt = if replay {
        let (t, replay_max) = lz_smt_replay(dir, lo - 1);
        if prev.max_time == 0 { prev.max_time = replay_max.max(genesis_params().1); }
        assert_eq!(t.root(), prev.smt_root, "replayed BIP30 tree at {} != the anchor's smt_root", lo - 1);
        t
    } else {
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
    let mut handover_pending = handover.is_some();
    let mut prev_rcpt: Option<Receipt> = handover.map(|(r, _)| r);
    let mut first = lo;
    if std::env::var("HAZYNC_RESUME").map(|v| v == "1").unwrap_or(false) {
        // With the coin log, only a receipt whose log file was also written (the log is saved after the receipt).
        let done = |k: &u32| std::path::Path::new(&format!("{out}/block_{k}.rcpt")).exists()
            && (!methods::COINLOG || std::path::Path::new(&format!("{out}/state_{k}.coinlog")).exists())
            && (!methods::TRIE || std::path::Path::new(&format!("{out}/state_{k}.trie")).exists());
        if let Some(k) = (lo..=hi).rev().find(done) {
            assert!(replay, "HAZYNC_RESUME needs every block file 1..{}", lo - 1);
            let bin = std::fs::read(format!("{out}/block_{k}.rcpt")).unwrap();
            let r: Receipt = bincode::deserialize(&bin).unwrap_or_else(|e| panic!("block_{k}.rcpt: {e}"));
            r.verify(METHOD_ID).unwrap_or_else(|e| panic!("block_{k}.rcpt does not verify against this image: {e}"));
            let st: ChainState = r.journal.decode().expect("ChainState");
            assert!(st.height == k && st.kind == KIND_CHAIN && st.self_id == METHOD_ID, "block_{k}.rcpt is not this chain's state at {k}");
            // The same chain as this run's anchor: an output directory reused with another anchor must not be resumed.
            // From an anchor, the chain's anchor_id is the anchor's digest; after a handover, the predecessor's anchor_id.
            let aid = if handover_pending { prev.anchor_id }
                      else { bitcoin::hashes::sha256d::Hash::hash(&state_journal_bytes(&prev)).to_byte_array() };
            assert!(st.anchor_id == aid, "block_{k}.rcpt was proved from another anchor (anchor_id {}, this run's {})", hex(&st.anchor_id), hex(&aid));
            let (t, _) = lz_smt_replay(dir, k);
            assert_eq!(t.root(), st.smt_root, "replayed BIP30 tree at {k} != the receipt's smt_root");
            // The coin log at k: the file the run left next to the receipt, checked against the receipt's own roots.
            if methods::COINLOG {
                cl_load(&format!("{out}/state_{k}.coinlog"));
                let g = CL.lock().unwrap();
                let cl = &g.as_ref().unwrap().store;
                assert!(cl.len() == st.utxo_leaves && normalize_host(cl.roots()) == normalize_host(st.utxo_roots.clone()),
                        "state_{k}.coinlog does not match block_{k}.rcpt's utxo_roots / utxo_leaves");
            }
            // The trie at k: its snapshot, checked against the receipt's trie root, height and block.
            if methods::TRIE { crate::trie::trie_load(&format!("{out}/state_{k}.trie"), &st); }
            smt = t;
            prev = st;
            prev_rcpt = Some(r);
            handover_pending = false; // a receipt of this image means the handover is already behind us
            first = k + 1;
            println!("resuming after block {k}");
        }
    }
    if disk_resume && first == lo {
        cl_init_at_start(handover_pending, anchor_path, &prev);
    }
    let mut times = std::fs::OpenOptions::new().create(true).append(true).open(format!("{out}/times.tsv")).unwrap();
    let opts = ProverOpts::succinct();
    let t_all = Instant::now();
    for h in first..=hi {
        let t0 = Instant::now();
        std::env::set_var("HAZYNC_BLOCK", format!("{dir}/block_{h}.json"));
        std::env::remove_var("HAZYNC_LZ_HINTS");
        let (_, mut w) = build_full();
        assert_eq!(w.height, h);
        w.root_prev = WireStump { roots: prev.utxo_roots.clone(), num_leaves: prev.utxo_leaves };
        lz_smt_rebind(&mut w, &mut smt);
        let bounds = chunk_bounds(&w, nchunks_env());
        // Signature hints first: every chunk executes once in collect mode (in parallel), and R for each
        // logged verify comes from libsecp -- the same bundle `libzync-hints` writes. Without this every chunk
        // would be PROVED with libsecp verifying each signature itself: valid, but the unbatched cost.
        let bp = format!("{out}/bundle_{h}.lzb");
        let mut bd = libzync::host::BlockBundle { height: h, block_hash: header_hash(&w.header), ..Default::default() };
        if !bounds.is_empty() {
            let done = execute_chunks_parallel(&w, &bounds, hint_threads());
            for (i, &(clo, chi)) in bounds.iter().enumerate() {
                rig_echo(&done[i].1);
                let blob: Vec<u8> = libzync::host::sig_records(&String::from_utf8_lossy(&done[i].1))
                    .iter().flat_map(|r| libzync::host::sig_hint(r)).collect();
                bd.chunks.push((clo as u32, chi as u32, blob));
            }
            bd.write(&bp).unwrap();
            std::env::set_var("HAZYNC_LZ_HINTS", &bp);
        }
        let t_sig = t0.elapsed().as_secs_f64();
        let mut chunks: Vec<Receipt> = Vec::with_capacity(bounds.len());
        for &(clo, chi) in &bounds {
            let mut b = ExecutorEnv::builder();
            b.segment_limit_po2(seg_po2());
            b.write(&4u32).unwrap();
            b.write(&w.height).unwrap();
            b.write(&chunk_block_hash(&w.header)).unwrap();
            write_chunk_inputs(&mut b, &w, clo, chi);
            let r = default_prover().prove_with_opts(b.build().unwrap(), METHOD_ELF, &opts).expect("prove chunk").receipt;
            r.verify(METHOD_ID).expect("chunk receipt verifies");
            chunks.push(r);
        }
        let t_chunks = t0.elapsed().as_secs_f64();
        std::env::remove_var("HAZYNC_LZ_HINTS"); // the aggregate's collect pass below runs without ECMH hints
        libzync_agg_prep(&mut w);
        let is_base = prev_rcpt.is_none();
        let step = if handover_pending { 2 } else { is_base as u32 };
        let env_for = |b: &mut risc0_zkvm::ExecutorEnvBuilder| {
            b.segment_limit_po2(seg_po2());
            if let Some(p) = &prev_rcpt {
                // Negative test: a handover with the predecessor's journal and no receipt behind it.
                if std::env::var("HAZYNC_TEST_HANDOVER").as_deref() != Ok("no_receipt") { b.add_assumption(p.clone()); }
            }
            write_aggregate_env_ex(b, &chunks, &prev, prev_rcpt.as_ref().map(|p| p.journal.bytes.as_slice()), &w, step);
            libzync_agg_tail(b, &w);
        };
        // collect pass: the leaves the ECMH hints are for (and, for a coin-log image, the log's spends and appends)
        CL_WIT.lock().unwrap().clear();
        *TRIE_RCPT.lock().unwrap() = None;
        let mut log = Vec::new();
        {
            let mut b = ExecutorEnv::builder();
            b.stdout(&mut log);
            env_for(&mut b);
            let collected = lz_execute(b.build().unwrap());
            drop(b); // releases its borrow of `log`
            rig_echo(&log);
            if let Err(e) = collected {
                assert!(e.contains("collect run commits no chain state"), "block {h}: collect aggregate failed: {e}");
            }
        }
        for leaf in libzync::host::ecmh_leaves(&String::from_utf8_lossy(&log)) {
            bd.ecmh.extend_from_slice(&libzync::host::ecmh_hint(&leaf));
        }
        let cl_lists = if methods::COINLOG { cl_parse(&String::from_utf8_lossy(&log)) } else { (Vec::new(), Vec::new()) };
        if methods::COINLOG { *CL_WIT.lock().unwrap() = cl_prepare(&cl_lists.0); }
        // The trie update (mode 17), proved from the store's witness; the aggregate verifies it as an assumption.
        let trie_lists = if methods::TRIE { crate::trie::trie_parse(&String::from_utf8_lossy(&log)) } else { (Vec::new(), Vec::new()) };
        if methods::TRIE { *TRIE_RCPT.lock().unwrap() = Some(crate::trie::trie_prove(h, &trie_lists.0, &trie_lists.1, &prev, &opts)); }
        bd.write(&bp).unwrap();
        std::env::set_var("HAZYNC_LZ_HINTS", &bp);
        let t_hints = t0.elapsed().as_secs_f64();
        let mut b = ExecutorEnv::builder();
        b.segment_limit_po2(seg_po2()); // env_for sets it too; restated here for check_seg_guards.sh's 25-line window
        env_for(&mut b);
        let r = default_prover().prove_with_opts(b.build().unwrap(), METHOD_ELF, &opts)
            .unwrap_or_else(|e| panic!("block {h}: aggregate proof failed: {e}")).receipt;
        r.verify(METHOD_ID).expect("aggregate receipt verifies");
        let st: ChainState = r.journal.decode().expect("ChainState");
        assert_eq!(st.height, h);
        std::fs::write(format!("{out}/block_{h}.rcpt"), bincode::serialize(&r).unwrap()).unwrap();
        std::fs::write(format!("{out}/state_{h}.json"), serde_json::to_string(&st).unwrap()).unwrap();
        if methods::COINLOG {
            let flips = cl_commit(&cl_lists.0, &cl_lists.1, &st);
            if h == 1 && prev_is_genesis {
                hist_append(&out, &libzync::history::Rec { height: 0, hash: genesis_anchor().tip_hash, value: 0, flips: vec![], appends: vec![] });
            }
            hist_append(&out, &libzync::history::Rec { height: h, hash: st.tip_hash, value: st.utxo_value, flips, appends: cl_lists.1.clone() });
            // The log next to each receipt, so a run can resume after it (HAZYNC_RESUME) or be anchored on it. The last
            // two are kept (a checkpoint copier takes the second newest receipt, which may still be complete while
            // the newest is being written); HAZYNC_COINLOG_SAVE_EACH=1 keeps every one (tests). A mainnet-sized log
            // needs a disk-backed store instead (COINLOG.md §8).
            cl_save(&format!("{out}/state_{h}.coinlog"));
            if h >= first + 2 && std::env::var("HAZYNC_COINLOG_SAVE_EACH").as_deref() != Ok("1") {
                let _ = std::fs::remove_file(format!("{out}/state_{}.coinlog", h - 2));
            }
        }
        // The trie: the same update list applied to the store must reach the committed root; its snapshot beside the
        // receipt (the last two kept, as the coin log's; HAZYNC_COINLOG_SAVE_EACH=1 keeps every one).
        if methods::TRIE {
            crate::trie::trie_commit(&trie_lists.0, &trie_lists.1, &st);
            crate::trie::trie_save(&format!("{out}/state_{h}.trie"));
            if h >= first + 2 && std::env::var("HAZYNC_COINLOG_SAVE_EACH").as_deref() != Ok("1") {
                let _ = std::fs::remove_file(format!("{out}/state_{}.trie", h - 2));
            }
        }
        let dt = t0.elapsed().as_secs_f64();
        // height, chunks, sig-hint s, chunk-prove s, ecmh-hint s, total s, kind, receipt bytes
        let rbytes = std::fs::metadata(format!("{out}/block_{h}.rcpt")).map(|m| m.len()).unwrap_or(0);
        writeln!(times, "{h}\t{}\t{t_sig:.1}\t{:.1}\t{:.1}\t{dt:.1}\t{}\t{rbytes}", bounds.len(),
                 t_chunks - t_sig, t_hints - t_chunks, ["recursive", "base", "handover"][step as usize]).unwrap();
        println!("  block {h}: {} chunks, proved in {dt:.1} s ({}), forest {} live_set {}  [{:.0} s total]",
                 bounds.len(), ["verifies prev", "base on anchor", "handover: verifies predecessor"][step as usize], st.utxo_leaves, st.live_set, t_all.elapsed().as_secs_f64());
        prev = st;
        prev_rcpt = Some(r);
        handover_pending = false;
    }
    println!("LZ_PROVE_RANGE_DONE {lo}..={hi} in {:.0} s; final receipt {out}/block_{hi}.rcpt", t_all.elapsed().as_secs_f64());
}

pub(crate) fn agg_chunks() {
    use std::time::Instant;
    let (anchor, mut w) = build_full();
    // The partition decides how many chunk files exist — an uneven pack can yield fewer runs than
    // HAZYNC_CHUNKS asked for, and reading a fixed count would look for a file that was never written.
    let nchunks = chunk_bounds(&w, nchunks_env()).len();
    let receipts = read_chunk_receipts(nchunks);
    println!("=== AGGREGATING {} chunk receipts for block {} ===", receipts.len(), w.height);
    let t = Instant::now();
    // libzync: same stream as agg-exec-fake (chunk_bounds above needed the full witness first).
    libzync_agg_prep(&mut w);
    let mut anchor = anchor;
    if methods::COINLOG { cl_synth_around(&receipts, &mut anchor, &mut w); }
    let mut b = ExecutorEnv::builder();
    write_aggregate_env(&mut b, &receipts, &anchor, &w);
    libzync_agg_tail(&mut b, &w);
    // Execute-only mode returns before proving; see report_agg_execute_only.
    if std::env::var("HAZYNC_AGG_EXECUTE").is_ok() { report_agg_execute_only(b); return; }

    // Prove the aggregate to SUCCINCT too: the assumptions are already succinct (cheap resolve), and a
    // succinct block proof is a single fixed-size STARK — directly composable in the chain range-fold.
    //
    // EXECUTE FIRST, then prove, rather than one opaque prove_with_opts (#145). Execution is a
    // couple of percent of the work and it yields the segment count, so the fold can say how much
    // there is to do BEFORE it starts doing it. Without that the only honest thing anyone could say
    // about a running fold was "it has not finished".
    let opts = ProverOpts::succinct();
    let server = risc0_zkvm::get_prover_server(&opts).expect("prover server");
    let ctx = risc0_zkvm::VerifierContext::default();
    let session = risc0_zkvm::ExecutorImpl::from_elf(b.build().unwrap(), METHOD_ELF)
        .unwrap().run().unwrap();
    let nseg = session.segments.len();
    println!("  {nseg} segments to prove at po2 {}", seg_po2());
    // #237: the aggregate had the SAME unprotected `prove_session` as `prove-chunk`. A #119 fault
    // here loses the whole fold, which is the most expensive single unit of work in the pipeline.
    let agg = prove_session_resilient(&server, &ctx, &session);
    agg.verify(METHOD_ID).unwrap();
    let tip: ChainState = agg.journal.decode().unwrap();
    assert!(tip.self_id == METHOD_ID, "S1: proof recursed against wrong image id");
    let out = std::env::var("HAZYNC_AGG_OUT").unwrap_or_else(|_| format!("block_{}.receipt", w.height));
    std::fs::write(&out, bincode::serialize(&agg).unwrap()).ok();
    println!(">>> BLOCK {} AGGREGATED in {:.1}s — succinct receipt VERIFIED, saved {out}.", w.height, t.elapsed().as_secs_f64());
    println!("  tip_hash {}  cum_work {}  UTXO leaves {}", hex(&tip.tip_hash), work_u128(&tip.cum_work), tip.utxo_leaves);
}

// ---- libzync coin log, host side (HAZYNC_COINLOG=1 builds; libzync/src/coinlog.rs) ----------------------------
//
// The host keeps the whole log (every node) to read off each block's witness. A collect execution of the aggregate
// logs the leaves of the earlier coins the block spends ("coinlog: S") and of the coins it appends ("coinlog: A"); the
// host names each spend's position (an unknown coin gets the end of the log, which the guest refuses), and after the
// block is proved applies the same changes and checks its roots equal the committed state's.

pub(crate) struct ClHost {
    // In memory, or on disk when HAZYNC_COINLOG_DIR is set (ROADMAP C1; libzync::host::DiskNodes).
    store: libzync::coinlog::NodeStore<libzync::host::AnyNodes>,
    pos: libzync::host::AnyIndex, // unspent leaf -> position; on disk (<dir>/index.bin) with HAZYNC_COINLOG_DIR (C1b)
}
pub(crate) static CL: std::sync::Mutex<Option<ClHost>> = std::sync::Mutex::new(None);
pub(crate) static CL_WIT: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());
/// E2: the block's trie receipt (guest mode 17) for the aggregate (measurement: executed, a FakeReceipt).
pub(crate) static TRIE_RCPT: std::sync::Mutex<Option<risc0_zkvm::Receipt>> = std::sync::Mutex::new(None);

/// `state_<h>.json` -> `state_<h>.coinlog`; `anchor.json` -> `anchor.coinlog`.
pub(crate) fn cl_path_for(state_json: &str) -> String {
    match state_json.strip_suffix(".json") { Some(s) => format!("{s}.coinlog"), None => format!("{state_json}.coinlog") }
}

/// The coin log for a run starting after `prev`: the handover receipt's log, an empty log (genesis), or the anchor's.
pub(crate) fn cl_init_at_start(handover: bool, anchor_path: &str, prev: &ChainState) {
    if handover {
        let p = std::env::var("HAZYNC_HANDOVER_RECEIPT").unwrap();
        // A bare file name has the parent "" (review 11-03): read the log next to it, in ".".
        let d = std::path::Path::new(&p).parent().map(|x| x.to_string_lossy().to_string())
            .filter(|x| !x.is_empty()).unwrap_or_else(|| ".".to_string());
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

/// Undoes one block from its pending.bin record: cut the appends (and drop them from the index), restore each spent
/// coin's unspent leaf at its position (and put it back in the index). Refuses a record that does not start at `n`.
pub(crate) fn cl_rollback(store: &mut libzync::coinlog::NodeStore<libzync::host::AnyNodes>, pos: &mut libzync::host::AnyIndex, u: &[u8], n: u64) {
    let rd = |o: usize| u64::from_le_bytes(u[o..o + 8].try_into().expect("pending.bin truncated"));
    assert!(rd(0) == n, "pending.bin is for a store of {} leaves, the state file says {n}: rebuild the store", rd(0));
    let k = rd(8) as usize;
    let mut o = 16;
    let mut flips = Vec::with_capacity(k);
    for _ in 0..k {
        let p = rd(o);
        let l: [u8; 32] = u[o + 8..o + 40].try_into().unwrap();
        flips.push((p, l));
        o += 40;
    }
    let m = rd(o) as usize;
    o += 8;
    for j in 0..m {
        let l: [u8; 32] = u[o + 32 * j..o + 32 * j + 32].try_into().unwrap();
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
            let _ = std::fs::remove_dir_all(&d);
            libzync::host::AnyNodes::Disk(libzync::host::DiskNodes::open(&d).unwrap_or_else(|e| panic!("{}: {e}", d.display())))
        }
        None => libzync::host::AnyNodes::Mem(Vec::new()),
    };
    let mut store = libzync::coinlog::NodeStore::with_nodes(nodes);
    let mut pos = cl_index();
    for (i, l) in leaves.iter().enumerate() {
        store.push(*l);
        pos.insert(*l, i as u64);
    }
    // The file stores each position's current leaf: a live coin's leaf, or spent(c). Indexing all of them is harmless:
    // spent(c) is a different function's output and never equals a coin leaf a block could spend.
    ClHost { store, pos }
}

pub(crate) fn cl_save(path: &str) {
    let g = CL.lock().unwrap();
    let h = g.as_ref().expect("coin log not initialised");
    let mut b = Vec::new();
    if let libzync::host::AnyNodes::Disk(d) = &h.store.nodes {
        d.sync().unwrap_or_else(|e| panic!("coin log store sync: {e}"));
        if let libzync::host::AnyIndex::Disk(ix) = &h.pos { ix.sync().unwrap_or_else(|e| panic!("index sync: {e}")); }
        b.extend_from_slice(CL_DISK_MAGIC);
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
    // The state is recorded: the block's undo record is no longer needed.
    if let (Some(d), libzync::host::AnyNodes::Disk(_)) = (cl_dir(), &h.store.nodes) {
        let _ = std::fs::remove_file(d.join("pending.bin"));
    }
}

pub(crate) fn cl_load(path: &str) {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e} (a coin-log anchor needs its .coinlog file)"));
    assert!(b.len() >= 8, "{path}: too short to be a coin log state file");
    if b.starts_with(CL_DISK_MAGIC) {
        assert!(b.len() >= 16 && (b.len() - 16) % 33 == 0, "{path}: a disk-mode state file of the wrong size");
        let d = cl_dir().unwrap_or_else(|| panic!("{path} was written in disk mode: set HAZYNC_COINLOG_DIR"));
        let store = libzync::coinlog::NodeStore::with_nodes(libzync::host::AnyNodes::Disk(
            libzync::host::DiskNodes::open(&d).unwrap_or_else(|e| panic!("{}: {e}", d.display()))));
        let n = u64::from_le_bytes(b[8..16].try_into().unwrap());
        let roots: Vec<Option<[u8; 32]>> = b[16..].chunks_exact(33).map(|c| (c[32] == 1).then(|| c[..32].try_into().unwrap())).collect();
        let mut store = store;
        let mut pos = cl_index();
        // A block that started changing the store after this state was saved (a crash, or a failed check): undo it.
        let pend = d.join("pending.bin");
        if let Ok(u) = std::fs::read(&pend) {
            // Always roll back when the record is for this state, even if the nodes look untouched: the index may
            // already have lost a spent coin (cl_commit removes it before setting the leaf; review round 2). Repeating
            // it is harmless: the truncate does nothing and the sets and inserts rewrite the same values.
            // A record for an older state belongs to a block that FINISHED (crash after its state file, before the
            // record's removal): nothing to undo.
            if u.len() >= 8 && u64::from_le_bytes(u[..8].try_into().unwrap()) == n {
                cl_rollback(&mut store, &mut pos, &u, n);
                eprintln!("  coin log store: undid the unfinished block recorded in pending.bin (back to {n} leaves)");
            }
            let _ = std::fs::remove_file(&pend);
            cl_sync_dir(&d);
        }
        assert!(store.len() == n && store.roots() == roots,
                "{}: the on-disk coin log is not at the state {path} recorded (it moved on after that receipt?): rebuild it from a run from genesis", d.display());
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
pub(crate) fn cl_prepare(spends: &[ClSpend]) -> Vec<u32> {
    let g = CL.lock().unwrap();
    let h = g.as_ref().expect("coin log not initialised");
    let n = h.store.len();
    let positions: Vec<u64> = spends.iter().map(|s| h.pos.get(&s.0).unwrap_or(n)).collect();
    let mut sorted: Vec<(u64, [u8; 32])> = positions.iter().copied().zip(spends.iter().map(|s| s.0)).collect();
    sorted.sort_unstable_by_key(|x| x.0);
    let sib = h.store.witness(&sorted);
    libzync::coinlog::encode_witness(&positions, &sib)
}

/// After the block is proved: the same flips and appends on the host's log, which must then equal the state.
/// Applies a proved block to the host's log; returns its flips (position, tx, vin) for history.bin.
pub(crate) fn cl_commit(spends: &[ClSpend], appends: &[[u8; 32]], st: &ChainState) -> Vec<(u64, u32, u32)> {
    let mut g = CL.lock().unwrap();
    let h = g.as_mut().expect("coin log not initialised");
    // Disk mode: write (and fsync) how to undo this block BEFORE the store changes in place, so a crash or a failed
    // check below leaves a store that cl_load can roll back to the last saved state (review 11-03).
    if let (Some(d), libzync::host::AnyNodes::Disk(_)) = (cl_dir(), &h.store.nodes) {
        let mut b = Vec::new();
        b.extend_from_slice(&h.store.len().to_le_bytes());
        b.extend_from_slice(&(spends.len() as u64).to_le_bytes());
        for (l, _, _) in spends {
            let p = h.pos.get(l).unwrap_or(u64::MAX);
            b.extend_from_slice(&p.to_le_bytes());
            b.extend_from_slice(l);
        }
        b.extend_from_slice(&(appends.len() as u64).to_le_bytes());
        for l in appends { b.extend_from_slice(l); }
        let tmp = d.join("pending.tmp");
        let f = std::fs::File::create(&tmp).and_then(|mut f| { use std::io::Write as _; f.write_all(&b)?; f.sync_data()?; Ok(f) });
        f.unwrap_or_else(|e| panic!("{}: {e}", tmp.display()));
        std::fs::rename(&tmp, d.join("pending.bin")).unwrap_or_else(|e| panic!("pending.bin: {e}"));
        cl_sync_dir(&d);
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
            std::fs::write(&path, cut).unwrap_or_else(|e| panic!("{path}: {e}"));
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

/// Gate / single-block measurement (agg-exec-fake): a coin log of the modelled size around ONE real block. Each spend's
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
            let e: Vec<libzync::coinlog::Spend> = sorted[kk..m].iter().map(|(p, h)| (*p, w_of(h), [0u32; 8])).collect();
            let sw: Vec<libzync::coinlog::W> = sib.iter().map(w_of).collect();
            let (mut o, mut nw) = ([0u32; 8], [0u32; 8]);
            assert!(libzync::coinlog::update(l, start, &e, &mut sw.iter(), &mut o, &mut nw));
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
    (roots, n, libzync::coinlog::encode_witness(&positions, &sib_all))
}
