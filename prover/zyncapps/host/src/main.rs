//! `zyncapp`: proves and verifies zyncAPP receipts (docs/libzync/ZYNCAPPS.md §8).
//!
//!   zyncapp prove-coin <core receipt> <fact.json from zync-path coin, at = tip> <out receipt> [--signet] [--preimage <coin.json>] [--execute]
//!   zyncapp verify-coin <zyncapp receipt>
//!   zyncapp prove-zeroconf <core tip receipt> <speculative chunk receipt> <record index> <paths.json> <out> [--signet]
//!       paths.json: [[position, [sibling hex, ...]], ...], one per input of the transaction, from zync-path coin
//!   zyncapp verify-zeroconf <zyncapp receipt>
//!   verify-* take [--core-image <hex>] [--expect <h>:<hash>] [--mempool]: the core statement is judged like the core
//!   verifier's (trusted image, genesis anchor, minimum work and checkpoint -> else DEMO, time, chain identity)
//!   zyncapp headers-prove <headers file> <out receipt> [--after <previous headers receipt>] [--elf <guest>] [--execute]
//!   zyncapp headers-mmr-path <headers file from 0> <height> [--tip <height>]   (the MMR path for zapp-headers --path)
//!   zyncapp hints-prove <block json dir> <H> <core receipt | state_<T>.json> <out dir> [--format 1|2] [--signet] [--execute] [--prof N]
//!       the SwiftSync hints file for H (docs/libzync/HINTS_ZAPP.md), its undo-hash file and (without --execute) the
//!       receipt. A state JSON (prep's) instead of a receipt: execute only, the core receipt an unresolved assumption.
//!   zyncapp hints-verify <hints receipt> <hints file> [--undo <file>] [--expect <H>:<hash>] [--expect-tip <T>:<hash>] [--core-image <hex>] [--mempool]
//!   zyncapp utreexo-prove <block json dir> <H> <core receipt | state_<T>.json> <out dir> [--split N] [--signet] [--execute]
//!       [--prof N] [--ref <utreexo-ref log>] [--elf <guest>]
//!       Floresta's Utreexo accumulator after H (docs/libzync/UTREEXO_ZAPP.md), in runs of at most N blocks chained by
//!       their state digest. A state JSON (prep's) instead of a receipt: execute only, the core receipt an unresolved
//!       assumption. --ref: the roots must equal those utreexo-ref (Floresta's own code) printed for H.
//!       The receipts are checked by zapps/utreexo (`zapp-utreexo verify`), the zAPP's own module.
//!   zyncapp utreexo-tip-prove <block json dir> <previous receipt | previous .journal> <H> <core receipt | state_<T>.json>
//!       <out receipt> [--signet] [--execute] [--prof N] [--elf <tip guest>] [--base-id <hex>] [--cache <file>] [--ref <log>]
//!       [--headers <file>]   (the link H+1..T as "<height> <header hex>" lines; default: from the block files)
//!       [--cache-out <file>] (where the replay's state at H is saved; default: over --cache)
//!       One STEP of the tip mode (UTREEXO_ZAPP.md §10): from the roots the previous receipt proves (a base receipt of
//!       utreexo-prove, or an earlier step) to the roots after H, block by block as Floresta does (rustreexo
//!       Stump::modify with batch proofs from a native MemForest), linked by headers H+1..T to the core tip. A
//!       .journal file (written by --execute) instead of a receipt: execute only, an unresolved assumption. --cache: the
//!       native replay's state (MemForest and live coins) at the previous height, read if it matches, written at H.
//!   zyncapp core-info <core receipt | state_<T>.json>     the core statement's height, tip hash and image (unchecked)
//!   zyncapp utreexo-count <block_<h>.json ...>   per block: Utreexo adds, leaf SHA-512 blocks, inputs (cost model)
//!   zyncapp id
//!   zyncapp bench-trie <dump file>                                          (E1 split: the trie update in its own receipt)
//!
//! The core receipt is an ASSUMPTION of the zyncAPP receipt (risc0 composition): the zyncAPP receipt's verification
//! covers the core proof. The verifier still pins the committed core image id and anchor (policy).
use risc0_zkvm::{default_prover, ExecutorEnv, Receipt};
use serde::{Deserialize, Serialize};
mod hints_host;
extern crate alloc;
mod utreexo_host;
mod utreexo_tip_host;
/// The base derivation (libzync), under the name the shared tip step (#[path] below) uses for it.
mod utreexo_roots {
    pub use libzync::utreexo_roots::*;
}
#[path = "../../methods/utreexo-tip/src/step.rs"]
mod utreexo_tip_step;
use zyncapp_methods::{ZYNCAPP_HINTS_ELF, ZYNCAPP_HINTS_ID, ZYNCAPP_UTREEXO_TIP_ELF, ZYNCAPP_UTREEXO_TIP_ID, ZYNCAPP_UTREEXO_ELF, ZYNCAPP_UTREEXO_ID, ZYNCAPP_COIN_ELF, ZYNCAPP_COIN_ID, ZYNCAPP_ZEROCONF_ELF, ZYNCAPP_ZEROCONF_ID, ZYNCAPP_HASHBENCH_ELF, ZYNCAPP_TRIEBENCH_ELF, ZYNCAPP_HEADERS_ELF, ZYNCAPP_HEADERS_ID};

#[derive(Serialize, Deserialize, Debug)]
struct ZeroConf {
    kind: u32, core_id: [u32; 8], lineage: [u8; 32], anchor_id: [u8; 32], tip_height: u32, tip_hash: [u8; 32],
    cum_work: [u8; 32], max_time: u32, profile: u32, txid: [u8; 32], wtxid: [u8; 32], fee: i64, weight: i64, sigops: i64,
}

/// The core statement a zyncAPP receipt rests on, judged by the SAME policy as the core verifier (SWIFTSYNC_PROOFS.md
/// steps 3a, 3b, 4, 6, 7): the core image must be the network's trusted one (or `--core-image <hex>`), the anchor the
/// network's pinned genesis anchor, the largest block time not in the future; below minimum work or the checkpoint the
/// answer is DEMO (exit 2); `--expect h:hash` / `--mempool` compare the tip with a source the user trusts. A zyncAPP
/// guest takes core_id as INPUT (so it survives handovers), so without this a receipt resting on any image would pass.
struct Core<'a> { core_id: &'a [u32; 8], anchor_id: &'a [u8; 32], lineage: &'a [u8; 32], profile: u32, height: u32,
                  tip_hash: &'a [u8; 32], cum_work: &'a [u8; 32], max_time: u32 }

/// Ok(true) = OK, Ok(false) = DEMO; Err = FAIL.
fn core_policy(c: &Core, a: &[String]) -> Result<bool, String> {
    use swiftsync_verify::{hex32, identity_matches, parse_expect, work_at_least, words, Image, MAX_FUTURE_BLOCK_TIME, PROFILE_SIGNET};
    let flag = |f: &str| a.iter().position(|x| x == f).and_then(|i| a.get(i + 1)).cloned();
    let net = if c.profile & PROFILE_SIGNET != 0 { "signet" } else { "mainnet" };
    let mut img = Image::by_network(net).ok_or("no built-in image for this network")?;
    if let Some(h) = flag("--core-image") {
        img = img.with_method_id(&h.to_lowercase()).ok_or("--core-image: not 64 hex digits")?;
    }
    if words(&img.method_id()) != *c.core_id {
        return Err(format!("rests on core image {}, not the trusted {net} core image {}", id_hex(c.core_id), img.method_id_hex));
    }
    // The core verifier's lineage rule: zero (one image since genesis) or exactly the image's pinned handovers.
    if *c.lineage != [0u8; 32] && *c.lineage != swiftsync_verify::lineage_of(img.lineage) {
        return Err(format!("lineage {} is not this core image's", hex(c.lineage)));
    }
    if hex32(img.genesis_anchor_hex) != Some(*c.anchor_id) {
        return Err(format!("anchor_id {} is not the {net} genesis anchor", hex(c.anchor_id)));
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    if c.max_time as u64 > now + MAX_FUTURE_BLOCK_TIME {
        return Err("time-too-new: the core chain has a block more than 2 h in the future".into());
    }
    let mut tip = *c.tip_hash;
    tip.reverse();
    let tip = hex(&tip);
    let mut checked = false;
    if let Some(e) = flag("--expect") {
        let (h, x) = parse_expect(&e).ok_or("--expect: want <height>:<64 hex digits>")?;
        if !identity_matches(h, &x, c.height, c.tip_hash) {
            return Err(format!("--expect: block {h} is {x}; the core tip is block {} {tip}", c.height));
        }
        checked = true;
    }
    if a.iter().any(|x| x == "--mempool") {
        let base = if net == "signet" { "https://mempool.space/signet/api" } else { "https://mempool.space/api" };
        let url = format!("{base}/block-height/{}", c.height);
        let o = std::process::Command::new("curl").args(["-sf", "--max-time", "30", &url]).output().map_err(|e| format!("curl: {e}"))?;
        let got = String::from_utf8_lossy(&o.stdout).trim().to_lowercase();
        if !o.status.success() || !identity_matches(c.height, &got, c.height, c.tip_hash) {
            return Err(format!("{url} says block {} is {got:?}; the core tip is {tip}", c.height));
        }
        checked = true;
    }
    if !checked {
        // A valid chain with enough work can still be a stale branch: an answer about it needs the tip compared with
        // the network, so without that it is at most DEMO (adversarial review, 11-03).
        println!("  chain identity NOT checked: compare block {} = {tip} with a source you trust (--expect, --mempool)", c.height);
    }
    Ok(checked && work_at_least(c.cum_work, img.min_work_hex) && c.height >= img.checkpoint_height)
}

fn verdict(r: Result<bool, String>, ok_line: String) -> ! {
    match r {
        Ok(true) => { println!("{ok_line}"); std::process::exit(0) }
        Ok(false) => {
            println!("ZYNCAPP_DEMO: {ok_line} -- but the core chain is not shown to be the network's (below minimum work or the checkpoint, or its tip not compared with the network)");
            std::process::exit(2)
        }
        Err(e) => { println!("ZYNCAPP_FAIL: {e}"); std::process::exit(1) }
    }
}

#[derive(Deserialize)]
#[allow(dead_code)] // the journal's layout: every field must be read to decode it, not every one is used here
struct ChainState {
    kind: u32, version: u32, tip_hash: [u8; 32], utxo_roots: Vec<Option<[u8; 32]>>, utxo_leaves: u64, utxo_value: u64,
    cum_work: [u8; 32], height: u32, prev_nbits: u32, prev_time: u32, epoch_start: u32, recent_times: Vec<u32>,
    anchor_id: [u8; 32], self_id: [u32; 8], #[serde(alias = "ss_set")] live_set: u32,
    #[serde(alias = "ss_acc")] live_hash: [u32; 16], #[serde(alias = "ss_hints")] trie_root: [u8; 32], smt_root: [u8; 32],
    max_time: u32, profile: u32, block_roots: Vec<Option<[u8; 32]>>, block_leaves: u64, lineage: [u8; 32],
}

/// zync-path's CoinFact (swiftsync-verify), the fields this zyncAPP uses; `preimage` is the coin's own fields, which the
/// coin's owner supplies (in the fact, or `--preimage <file>`): the guest computes the leaf from them (version 2).
#[derive(Deserialize)]
struct CoinFact { at: u32, pos: u64, coin: String, spent_by: Option<(u32, u32, u32)>, path: Vec<String>,
                  #[serde(default)] preimage: Option<swiftsync_verify::CoinPreimage> }

/// The guest's CoinFields: txid in INTERNAL byte order.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct CoinFields { txid: [u8; 32], vout: u32, value: u64, script_pubkey: Vec<u8>, height: u32, coinbase: bool, tracked: bool, creation_mtp: u32 }

#[derive(Serialize, Deserialize)]
struct CoinQuery { pos: u64, fields: CoinFields, spent_by: Option<(u32, u32, u32)>, path: Vec<[u8; 32]> }

#[derive(Serialize, Deserialize, Debug)]
struct ZyncCoin {
    kind: u32, core_id: [u32; 8], lineage: [u8; 32], anchor_id: [u8; 32], height: u32, tip_hash: [u8; 32],
    cum_work: [u8; 32], max_time: u32, profile: u32, pos: u64, coin: [u8; 32], fields: CoinFields, spent_by: Option<(u32, u32, u32)>,
}
const KIND_ZYNC_COIN: u32 = 0x5A43_0002;

fn die(m: &str) -> ! { eprintln!("zyncapp: {m}"); std::process::exit(1) }
fn h32(s: &str) -> [u8; 32] {
    let v: Vec<u8> = (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2).unwrap_or("zz"), 16).unwrap_or_else(|_| die("bad hex"))).collect();
    v.try_into().unwrap_or_else(|_| die("not 32 bytes"))
}
fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }
fn id_hex(w: &[u32; 8]) -> String { hex(&w.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>()) }

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a.first().map(|s| s.as_str()) {
        Some("id") => {
            println!("ZYNCAPP_COIN_ID {}", id_hex(&ZYNCAPP_COIN_ID));
            println!("ZYNCAPP_ZEROCONF_ID {}", id_hex(&ZYNCAPP_ZEROCONF_ID));
            println!("ZYNCAPP_HEADERS_ID {}", id_hex(&ZYNCAPP_HEADERS_ID));
            println!("ZYNCAPP_HINTS_ID {}", id_hex(&ZYNCAPP_HINTS_ID));
            println!("ZYNCAPP_UTREEXO_ID {}", id_hex(&ZYNCAPP_UTREEXO_ID));
            println!("ZYNCAPP_UTREEXO_TIP_ID {}", id_hex(&ZYNCAPP_UTREEXO_TIP_ID));
        }
        Some("hints-prove") if a.len() >= 5 => hints_prove(&a),
        Some("hints-verify") if a.len() >= 3 => hints_verify(&a),
        Some("utreexo-prove") if a.len() >= 5 => utreexo_prove(&a),
        Some("utreexo-tip-prove") if a.len() >= 6 => utreexo_tip_prove(&a),
        Some("core-info") if a.len() == 2 => {
            // the core statement's height and tip (for a follower choosing H), from a receipt or a state JSON; not a check
            let (cs, _, r) = load_core(&a[1], true);
            println!("CORE height={} tip={} image={}{}", cs.height, disp(&cs.tip_hash), id_hex(&cs.self_id),
                     if r.is_none() { " (a state JSON)" } else { "" });
        }
        Some("utreexo-count") if a.len() >= 2 => {
            // Per block (block_<h>.json files): what the Utreexo part of the walk would hash if every coin it adds were
            // still live at H (an upper bound on its leaf work), and the inputs a replay-with-proofs design would delete.
            for p in &a[1..] {
                let (dir, file) = p.rsplit_once('/').unwrap_or((".", p));
                let h: u32 = file.trim_start_matches("block_").trim_end_matches(".json").parse().unwrap_or_else(|_| die("block_<h>.json"));
                let b = utreexo_host::read_block(dir, h);
                let mut made = std::collections::HashSet::new();
                let mut spent = std::collections::HashSet::new();
                let (mut outs, mut unsp, mut adds, mut blocks, mut ins, mut ins_in_block) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
                for (ti, t) in b.txs.iter().enumerate() {
                    if ti > 0 { for i in &t.inputs { ins += 1; if made.contains(i) { ins_in_block += 1; spent.insert(*i); } } }
                    for v in 0..t.outputs.len() { made.insert((b.txids[ti], v as u32)); }
                }
                for (ti, t) in b.txs.iter().enumerate() {
                    for (v, (_, spk)) in t.outputs.iter().enumerate() {
                        outs += 1;
                        if libzync::utreexo_roots::unspendable_script(spk) { unsp += 1; continue; }
                        if spent.contains(&(b.txids[ti], v as u32)) { continue; }
                        adds += 1;
                        blocks += libzync::utreexo_roots::leaf_blocks(spk.len());
                    }
                }
                println!("UTREEXO_COUNT height={h} txs={} bytes_stripped={} outputs={outs} unspendable={unsp} adds={adds} leaf_blocks_if_all_live={blocks} inputs={ins} inputs_spending_this_block={ins_in_block}",
                         b.txs.len(), b.stripped.len());
            }
        }
        Some("headers-exec") if a.len() >= 2 => {
            // K1: the header-chain zAPP, EXECUTED (cycles, no proof) over a file of "height hex80" lines, as
            // fetched from a node. A file starting at 0 runs from genesis; otherwise its first 11 headers seed the state
            // (times, hash, bits) and the run starts at the first retarget boundary after them, with the chainwork there
            // given as a third argument (hex, from getblockheader). Prints the end hash and chainwork for the node check.
            let lines: Vec<(i64, Vec<u8>)> = std::fs::read_to_string(&a[1]).unwrap_or_else(|e| die(&e.to_string())).lines()
                .filter(|l| !l.trim().is_empty()).map(|l| {
                    let mut f = l.split_whitespace();
                    let h: i64 = f.next().unwrap().parse().unwrap();
                    let x = f.next().unwrap();
                    (h, (0..x.len()).step_by(2).map(|i| u8::from_str_radix(&x[i..i + 2], 16).unwrap()).collect())
                }).collect();
            let dsha = |b: &[u8]| -> [u8; 32] { use sha2::Digest; sha2::Sha256::digest(sha2::Sha256::digest(b)).into() };
            #[derive(serde::Serialize)]
            struct HState { height: i64, hash: [u8; 32], work: [u8; 32], bits: u32, time: u32, epoch_first_time: u32, recent_times: Vec<u32>, max_time: u32, mmr_peaks: Vec<[u8; 32]> }
            let (st, run): (HState, &[(i64, Vec<u8>)]) = if lines[0].0 == 0 {
                (HState { height: -1, hash: [0; 32], work: [0; 32], bits: 0, time: 0, epoch_first_time: 0, recent_times: vec![], max_time: 0, mmr_peaks: vec![] }, &lines[..])
            } else {
                // the run starts just AFTER a retarget boundary: the boundary header is in the seed and is its epoch's first
                // time, so the next retarget in the run is computed from real timestamps
                let b = lines.iter().position(|(h, _)| h % 2016 == 0).unwrap_or_else(|| die("no retarget boundary in the file"));
                let start = b + 1;
                assert!(start >= 11, "need 11 headers up to the boundary");
                let seed = &lines[start - 11..start];
                let last = &seed[10].1;
                let le = |b: &[u8]| u32::from_le_bytes(b.try_into().unwrap());
                let wh = a.get(2).unwrap_or_else(|| die("headers-exec <file> <chainwork hex at the retarget boundary header>"));
                let mut work = [0u8; 32];
                let wb: Vec<u8> = (0..64).step_by(2).map(|i| u8::from_str_radix(&format!("{:0>64}", wh)[i..i + 2], 16).unwrap()).collect();
                for (i, b) in wb.iter().rev().enumerate() { work[i] = *b; }
                // the MMR peaks below the seed are unknown without every hash from genesis: placeholder peaks of the
                // right count (one per set bit of height + 1), so the cycles are a real run's; the end MMR is not real
                (HState { height: seed[10].0, hash: dsha(last), work, bits: le(&last[72..76]), time: le(&last[68..72]), epoch_first_time: le(&lines[b].1[68..72]),
                          recent_times: seed.iter().map(|(_, h)| le(&h[68..72])).collect(), max_time: seed.iter().map(|(_, h)| le(&h[68..72])).max().unwrap(),
                          mmr_peaks: vec![[0; 32]; ((seed[10].0 + 1) as u64).count_ones() as usize] }, &lines[start..])
            };
            let words: Vec<u32> = run.iter().flat_map(|(_, h)| h.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect::<Vec<_>>()).collect();
            let env = ExecutorEnv::builder().write(&st).unwrap().write(&(run.len() as u32)).unwrap().write_slice(&words).build().unwrap();
            let t = std::time::Instant::now();
            let info = risc0_zkvm::default_executor().execute(env, ZYNCAPP_HEADERS_ELF).unwrap_or_else(|e| die(&e.to_string()));
            #[derive(serde::Deserialize)]
            struct HOut { height: i64, hash: [u8; 32], work: [u8; 32], bits: u32, time: u32, epoch_first_time: u32, recent_times: Vec<u32>, max_time: u32, mmr_peaks: Vec<[u8; 32]> }
            let (_d, out): ([u8; 32], HOut) = info.journal.decode().unwrap_or_else(|e| die(&e.to_string()));
            let _ = (out.bits, out.time, out.epoch_first_time, &out.recent_times, out.max_time, &out.mmr_peaks);
            let disp = |b: &[u8; 32]| -> String { b.iter().rev().map(|x| format!("{x:02x}")).collect() };
            println!("HEADERS_EXEC {} headers ({}..{}) {} cycles = {:.0} per header, {} segments, executed in {:.1} s",
                     run.len(), run[0].0, out.height, info.cycles(), info.cycles() as f64 / run.len() as f64, info.segments.len(), t.elapsed().as_secs_f64());
            println!("END height {} hash {} chainwork {}", out.height, disp(&out.hash), disp(&out.work));
            println!("HEADERS_ID {}", id_hex(&ZYNCAPP_HEADERS_ID));
        }
        Some("headers-prove") if a.len() >= 3 => {
            // K1 / N3: PROVE the header chain over a file of "height hex80" lines (as fetched from a node), as one
            // succinct receipt. Without --after the file must start at genesis (height 0) and the run starts from the
            // empty state; with --after <receipt> the run starts from the exact state that receipt ended in, and takes
            // the file's headers above it, so new headers are a short run chained to the last one.
            // --execute: execute only (cycles, no proof). Verify with `zapp-headers <receipt>...`.
            // --elf <file>: prove with this guest ELF (the published one, reproduce/LIBZYNC_IDS) instead of the embedded
            // one, so a GPU box builds the host with RISC0_SKIP_BUILD=1 and needs no guest toolchain; its image id is printed.
            let elf: Vec<u8> = match a.iter().position(|x| x == "--elf").and_then(|i| a.get(i + 1)) {
                Some(p) => std::fs::read(p).unwrap_or_else(|e| die(&format!("--elf: {e}"))),
                None => ZYNCAPP_HEADERS_ELF.to_vec(),
            };
            if elf.is_empty() { die("no headers guest: built with RISC0_SKIP_BUILD, so give --elf <zyncapp-headers.bin>"); }
            let elf_id = risc0_zkvm::compute_image_id(&elf).unwrap_or_else(|e| die(&format!("image id: {e}")));
            use zapp_headers::HeaderState;
            let lines: Vec<(i64, Vec<u8>)> = std::fs::read_to_string(&a[1]).unwrap_or_else(|e| die(&e.to_string())).lines()
                .filter(|l| !l.trim().is_empty()).map(|l| {
                    let mut f = l.split_whitespace();
                    let h: i64 = f.next().and_then(|x| x.parse().ok()).unwrap_or_else(|| die("a line without a height"));
                    let x = f.next().unwrap_or_else(|| die("a line without a header"));
                    let b: Vec<u8> = (0..x.len()).step_by(2).map(|i| u8::from_str_radix(x.get(i..i + 2).unwrap_or("zz"), 16).unwrap_or_else(|_| die("bad hex"))).collect();
                    if b.len() != 80 { die(&format!("header {h} is {} bytes, not 80", b.len())); }
                    (h, b)
                }).collect();
            let after = a.iter().position(|x| x == "--after").and_then(|i| a.get(i + 1));
            let st: HeaderState = match after {
                None => HeaderState::empty(),
                Some(p) => {
                    let r: Receipt = bincode::deserialize(&std::fs::read(p).unwrap_or_else(|e| die(&e.to_string()))).unwrap_or_else(|e| die(&e.to_string()));
                    let (_d, s): ([u8; 32], HeaderState) = r.journal.decode().unwrap_or_else(|e| die(&format!("--after: not a headers receipt: {e}")));
                    s
                }
            };
            let run: Vec<&(i64, Vec<u8>)> = lines.iter().filter(|(h, _)| *h > st.height).collect();
            if run.is_empty() { die("no headers above the starting state"); }
            for (k, (h, _)) in run.iter().enumerate() {
                if *h != st.height + 1 + k as i64 { die(&format!("the file is not consecutive from {}: found {h}", st.height + 1)); }
            }
            let words: Vec<u32> = run.iter().flat_map(|(_, h)| h.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect::<Vec<_>>()).collect();
            // the MMR the guest must end with, built natively by the same code (zapp_headers::mmr = the guest's
            // src/mmr.rs): a guest that disagrees is refused before anything is written
            let mut want_peaks = st.mmr_peaks.clone();
            for (k, (_, h)) in run.iter().enumerate() {
                use sha2::Digest;
                zapp_headers::mmr::append(&mut want_peaks, (st.height + 1) as u64 + k as u64, &sha2::Sha256::digest(sha2::Sha256::digest(h)).into());
            }
            let mmr_check = |out: &HeaderState| if out.mmr_peaks != want_peaks {
                die(&format!("the guest's MMR ({} peaks, root {}) is not the native one (root {})", out.mmr_peaks.len(), out.mmr_root_hex(),
                             hex(&zapp_headers::mmr::root(&want_peaks))));
            };
            let env = ExecutorEnv::builder().write(&st).unwrap().write(&(run.len() as u32)).unwrap().write_slice(&words).build().unwrap();
            let t = std::time::Instant::now();
            if a.iter().any(|x| x == "--execute") {
                let info = risc0_zkvm::default_executor().execute(env, &elf).unwrap_or_else(|e| die(&e.to_string()));
                let (_d, out): ([u8; 32], HeaderState) = info.journal.decode().unwrap_or_else(|e| die(&e.to_string()));
                mmr_check(&out);
                println!("HEADERS_EXEC {} headers ({}..{}) {} cycles, {} segments, {:.1} s; END {} {} chainwork {} MMR root {}", run.len(), st.height + 1, out.height,
                         info.cycles(), info.segments.len(), t.elapsed().as_secs_f64(), out.height, out.hash_hex(), out.work_hex(), out.mmr_root_hex());
            } else {
                let r = default_prover().prove_with_opts(env, &elf, &risc0_zkvm::ProverOpts::succinct())
                    .unwrap_or_else(|e| die(&format!("prove: {e}"))).receipt;
                let (_d, out): ([u8; 32], HeaderState) = r.journal.decode().unwrap_or_else(|e| die(&e.to_string()));
                mmr_check(&out);
                std::fs::write(&a[2], bincode::serialize(&r).unwrap()).unwrap_or_else(|e| die(&e.to_string()));
                println!("HEADERS_PROVED {} headers ({}..{}) in {:.1} s -> {}; END {} {} chainwork {} MMR root {}", run.len(), st.height + 1, out.height,
                         t.elapsed().as_secs_f64(), a[2], out.height, out.hash_hex(), out.work_hex(), out.mmr_root_hex());
            }
            println!("HEADERS_ID {}", hex(elf_id.as_bytes()));
        }
        Some("headers-mmr-path") if a.len() >= 3 => {
            // The MMR inclusion path of block <height> in the header chain proved to the file's last header (or to
            // --tip <height>, the proven tip, when the file goes further): the file of "height hex80" lines must start
            // at genesis and be consecutive. Prints the path to stdout, one 64-hex hash per line, lowest first (raw byte
            // order; nothing for a one-block peak), the form `zapp-headers --include <height>:<hash>
            // --path <file>` reads; the block, the leaf count and the MMR root go to stderr.
            let want: u64 = a[2].parse().unwrap_or_else(|_| die("headers-mmr-path <headers file> <height> [--tip <height>]"));
            let tip: Option<i64> = a.iter().position(|x| x == "--tip").and_then(|i| a.get(i + 1))
                .map(|x| x.parse().unwrap_or_else(|_| die("--tip: not a height")));
            let mut hashes: Vec<[u8; 32]> = Vec::new();
            for l in std::fs::read_to_string(&a[1]).unwrap_or_else(|e| die(&e.to_string())).lines().filter(|l| !l.trim().is_empty()) {
                let mut f = l.split_whitespace();
                let h: i64 = f.next().and_then(|x| x.parse().ok()).unwrap_or_else(|| die("a line without a height"));
                if tip.is_some_and(|t| h > t) { break; }
                if h != hashes.len() as i64 { die(&format!("the file is not consecutive from genesis: found {h} at line {}", hashes.len() + 1)); }
                let x = f.next().unwrap_or_else(|| die("a line without a header"));
                let b: Vec<u8> = (0..x.len()).step_by(2).map(|i| u8::from_str_radix(x.get(i..i + 2).unwrap_or("zz"), 16).unwrap_or_else(|_| die("bad hex"))).collect();
                if b.len() != 80 { die(&format!("header {h} is {} bytes, not 80", b.len())); }
                if let Some(prev) = hashes.last() { if b[4..36] != prev[..] { die(&format!("header {h} does not link to {}", h - 1)); } }
                use sha2::Digest;
                hashes.push(sha2::Sha256::digest(sha2::Sha256::digest(&b)).into());
            }
            if let Some(t) = tip { if hashes.len() as i64 != t + 1 { die(&format!("the file ends at {}, below --tip {t}", hashes.len() as i64 - 1)); } }
            let path = zapp_headers::mmr::path(&hashes, want)
                .unwrap_or_else(|| die(&format!("block {want} is not in 0..={}", hashes.len() as i64 - 1)));
            for p in &path { println!("{}", hex(p)); }
            let disp: String = hashes[want as usize].iter().rev().map(|x| format!("{x:02x}")).collect();
            eprintln!("MMR_PATH block {want} {disp}: {} hashes; {} leaves (tip {}), MMR root {}", path.len(), hashes.len(),
                      hashes.len() - 1, hex(&zapp_headers::mmr::root(&zapp_headers::mmr::peaks_of(&hashes))));
        }
        Some("bench-hash") => {
            // E1 calibration, execute mode (no proof): cycles per node hash for both hash shapes.
            for mode in [0u32, 1] {
                let env = ExecutorEnv::builder().write(&(mode, 20_000u32)).unwrap().build().unwrap();
                let info = risc0_zkvm::default_executor().execute(env, ZYNCAPP_HASHBENCH_ELF).unwrap_or_else(|e| die(&e.to_string()));
                println!("mode {mode}: total {} cycles over 20000 hashes (user cycles; see HASHBENCH log line for the loop alone)", info.cycles());
            }
        }
        Some("bench-trie") if a.len() >= 2 => {
            // E1 split (absence/README.md): the trie update alone, from the core host's HAZYNC_TRIE_DUMP file.
            let b = std::fs::read(&a[1]).unwrap_or_else(|e| die(&format!("{}: {e}", a[1])));
            let w: Vec<u32> = b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
            let env = ExecutorEnv::builder().write_slice(&w).build().unwrap();
            let t = std::time::Instant::now();
            let info = risc0_zkvm::default_executor().execute(env, ZYNCAPP_TRIEBENCH_ELF).unwrap_or_else(|e| die(&e.to_string()));
            println!("BENCH_TRIE total {} cycles, {} segments, executed in {:.1} s", info.cycles(), info.segments.len(), t.elapsed().as_secs_f64());
        }
        Some("prove-zeroconf") if a.len() >= 6 => {
            let load = |p: &str| -> Receipt { bincode::deserialize(&std::fs::read(p).unwrap_or_else(|e| die(&e.to_string())))
                .unwrap_or_else(|e| die(&format!("{p}: {e}"))) };
            let (tip, chunk) = (load(&a[1]), load(&a[2]));
            let cs: ChainState = tip.journal.decode().unwrap_or_else(|e| die(&format!("tip journal: {e}")));
            let idx: u32 = a[3].parse().unwrap_or_else(|_| die("record index?"));
            let raw: Vec<(u64, Vec<String>)> = serde_json::from_slice(&std::fs::read(&a[4]).unwrap_or_else(|e| die(&e.to_string())))
                .unwrap_or_else(|e| die(&format!("paths.json: {e}")));
            let paths: Vec<(u64, Vec<[u8; 32]>)> = raw.into_iter().map(|(p, v)| (p, v.iter().map(|x| h32(x)).collect())).collect();
            let signet = a.iter().any(|x| x == "--signet");
            let env = ExecutorEnv::builder()
                .add_assumption(tip.clone()).add_assumption(chunk.clone())
                .write(&cs.self_id).unwrap()
                .write(&tip.journal.bytes).unwrap()
                .write(&chunk.journal.bytes).unwrap()
                .write(&signet).unwrap()
                .write(&idx).unwrap()
                .write(&paths).unwrap()
                .build().unwrap();
            let r = default_prover().prove(env, ZYNCAPP_ZEROCONF_ELF).unwrap_or_else(|e| die(&format!("prove: {e}"))).receipt;
            std::fs::write(&a[5], bincode::serialize(&r).unwrap()).unwrap_or_else(|e| die(&e.to_string()));
            println!("zyncapp zeroconf receipt written: {}", a[5]);
        }
        Some("verify-zeroconf") if a.len() >= 2 => {
            let r: Receipt = bincode::deserialize(&std::fs::read(&a[1]).unwrap_or_else(|e| die(&e.to_string())))
                .unwrap_or_else(|e| die(&format!("receipt: {e}")));
            if matches!(r.inner, risc0_zkvm::InnerReceipt::Fake(_)) { die("a dev-mode FakeReceipt is not a proof"); }
            r.verify(ZYNCAPP_ZEROCONF_ID).unwrap_or_else(|e| die(&format!("does not verify: {e}")));
            let z: ZeroConf = r.journal.decode().unwrap_or_else(|e| die(&format!("journal: {e}")));
            if z.kind != 0x5A43_0002 { die("not a zyncAPP zeroconf statement"); }
            let (mut t, mut id) = (z.tip_hash, z.txid);
            t.reverse();
            id.reverse();
            println!("  rests on core image {} with anchor_id {}", id_hex(&z.core_id), hex(&z.anchor_id));
            println!("  ⚠ valid now, not final: a conflicting transaction can still confirm first");
            let c = Core { core_id: &z.core_id, anchor_id: &z.anchor_id, lineage: &z.lineage, profile: z.profile, height: z.tip_height,
                           tip_hash: &z.tip_hash, cum_work: &z.cum_work, max_time: z.max_time };
            verdict(core_policy(&c, &a), format!("ZYNCAPP_ZEROCONF_OK: tx {} is valid for block {} on tip {} and spends only coins unspent there (fee {} sat)",
                     hex(&id), z.tip_height + 1, hex(&t), z.fee));
        }
        Some("prove-coin") if a.len() >= 4 => {
            let core: Receipt = bincode::deserialize(&std::fs::read(&a[1]).unwrap_or_else(|e| die(&e.to_string())))
                .unwrap_or_else(|e| die(&format!("core receipt: {e}")));
            let cs: ChainState = core.journal.decode().unwrap_or_else(|e| die(&format!("core journal: {e}")));
            let f: CoinFact = serde_json::from_slice(&std::fs::read(&a[2]).unwrap_or_else(|e| die(&e.to_string())))
                .unwrap_or_else(|e| die(&format!("fact: {e}")));
            if f.at != cs.height { die("this zyncAPP answers at the core proof's tip: make the fact with at = tip"); }
            let pre: swiftsync_verify::CoinPreimage = match (&f.preimage, a.iter().position(|x| x == "--preimage").and_then(|i| a.get(i + 1))) {
                (_, Some(p)) => serde_json::from_slice(&std::fs::read(p).unwrap_or_else(|e| die(&e.to_string())))
                    .unwrap_or_else(|e| die(&format!("--preimage: {e}"))),
                (Some(p), None) => p.clone(),
                (None, None) => die("the coin zyncAPP proves a statement about the COIN: give its fields (`preimage` in the fact, or --preimage <file.json>: txid, vout, value, script_pubkey, height, coinbase, tracked, creation_mtp)"),
            };
            let mut txid = h32(&pre.txid);
            txid.reverse(); // explorers print txids reversed
            let spk: Vec<u8> = (0..pre.script_pubkey.len()).step_by(2)
                .map(|i| u8::from_str_radix(pre.script_pubkey.get(i..i + 2).unwrap_or("zz"), 16).unwrap_or_else(|_| die("script_pubkey: bad hex"))).collect();
            let fields = CoinFields { txid, vout: pre.vout, value: pre.value, script_pubkey: spk, height: pre.height, coinbase: pre.coinbase,
                                      tracked: pre.tracked, creation_mtp: pre.creation_mtp };
            // the same leaf the guest will compute, checked here so a wrong preimage fails before any proving
            let leaf = libzync::facts::coin_leaf(&fields.txid, fields.vout, fields.value, &fields.script_pubkey, fields.height, fields.coinbase, fields.tracked, fields.creation_mtp);
            if leaf != h32(&f.coin) { die("the preimage's coin leaf is not the fact's coin: wrong fields for this coin"); }
            let q = CoinQuery { pos: f.pos, fields, spent_by: f.spent_by, path: f.path.iter().map(|x| h32(x)).collect() };
            let signet = a.iter().any(|x| x == "--signet");
            let env = ExecutorEnv::builder()
                .add_assumption(core.clone())
                .write(&cs.self_id).unwrap()
                .write(&core.journal.bytes).unwrap()
                .write(&signet).unwrap()
                .write(&q).unwrap()
                .build().unwrap();
            if a.iter().any(|x| x == "--execute") {
                // the guest EXECUTED, every check in it run, no proof (tests against a dev-mode core receipt)
                let info = risc0_zkvm::default_executor().execute(env, ZYNCAPP_COIN_ELF).unwrap_or_else(|e| die(&format!("execute: {e}")));
                let z: ZyncCoin = info.journal.decode().unwrap_or_else(|e| die(&format!("journal: {e}")));
                let mut txid = z.fields.txid;
                txid.reverse();
                println!("EXECUTED (not a proof): kind {:#x} coin {}:{} value {} created {} leaf {} pos {} spent_by {:?} at core height {}; {} cycles",
                         z.kind, hex(&txid), z.fields.vout, z.fields.value, z.fields.height, hex(&z.coin), z.pos, z.spent_by, z.height,
                         info.cycles());
                return;
            }
            let r = default_prover().prove(env, ZYNCAPP_COIN_ELF).unwrap_or_else(|e| die(&format!("prove: {e}"))).receipt;
            std::fs::write(&a[3], bincode::serialize(&r).unwrap()).unwrap_or_else(|e| die(&e.to_string()));
            println!("zyncapp coin receipt written: {}", a[3]);
        }
        Some("inspect-coin") if a.len() >= 2 => {
            // the journal of a coin receipt, NOT verified (dev-mode test runs; never an answer: verify-coin is)
            let r: Receipt = bincode::deserialize(&std::fs::read(&a[1]).unwrap_or_else(|e| die(&e.to_string())))
                .unwrap_or_else(|e| die(&format!("receipt: {e}")));
            let z: ZyncCoin = r.journal.decode().unwrap_or_else(|e| die(&format!("journal: {e}")));
            let mut txid = z.fields.txid;
            txid.reverse();
            println!("UNVERIFIED journal: kind {:#x} coin {}:{} value {} height {} leaf {} pos {} spent_by {:?} at core height {}",
                     z.kind, hex(&txid), z.fields.vout, z.fields.value, z.fields.height, hex(&z.coin), z.pos, z.spent_by, z.height);
        }
        Some("verify-coin") if a.len() >= 2 => {
            let r: Receipt = bincode::deserialize(&std::fs::read(&a[1]).unwrap_or_else(|e| die(&e.to_string())))
                .unwrap_or_else(|e| die(&format!("receipt: {e}")));
            if matches!(r.inner, risc0_zkvm::InnerReceipt::Fake(_)) { die("a dev-mode FakeReceipt is not a proof"); }
            r.verify(ZYNCAPP_COIN_ID).unwrap_or_else(|e| die(&format!("does not verify: {e}")));
            let z: ZyncCoin = r.journal.decode().unwrap_or_else(|e| die(&format!("journal: {e}")));
            if z.kind == 0x5A43_0001 { die("a version-1 coin statement (the leaf only, not the coin's fields): re-prove with this zyncapp"); }
            if z.kind != KIND_ZYNC_COIN { die("not a zyncAPP coin statement"); }
            let mut t = z.tip_hash;
            t.reverse();
            let what = match z.spent_by { None => "UNSPENT".to_string(), Some((h, tx, v)) => format!("SPENT at height {h} by tx {tx} input {v}") };
            println!("  rests on core image {} with anchor_id {} and lineage {}", id_hex(&z.core_id), hex(&z.anchor_id), hex(&z.lineage));
            let c = Core { core_id: &z.core_id, anchor_id: &z.anchor_id, lineage: &z.lineage, profile: z.profile, height: z.height,
                           tip_hash: &z.tip_hash, cum_work: &z.cum_work, max_time: z.max_time };
            let mut txid = z.fields.txid;
            txid.reverse();
            println!("  coin: {}:{} value {} sat, script {}, created at height {}{}", hex(&txid), z.fields.vout, z.fields.value,
                     hex(&z.fields.script_pubkey), z.fields.height, if z.fields.coinbase { " (coinbase)" } else { "" });
            verdict(core_policy(&c, &a), format!("ZYNCAPP_COIN_OK: coin {}:{} (leaf {}) at log position {} is {what} at height {} (tip {})",
                                                 hex(&txid), z.fields.vout, hex(&z.coin), z.pos, z.height, hex(&t)));
        }
        _ => die("usage: zyncapp id | prove-coin <core.rcpt> <fact.json> <out.rcpt> [--signet] | verify-coin <zync.rcpt> | inspect-coin <zync.rcpt>"),
    }
}

/// The core ChainState with Serialize, to rebuild a journal from prep's state JSON (execute-only runs).
#[derive(Serialize, Deserialize, Clone)]
struct ChainStateJ {
    kind: u32, #[serde(default)] version: u32, tip_hash: [u8; 32], utxo_roots: Vec<Option<[u8; 32]>>, utxo_leaves: u64,
    #[serde(default)] utxo_value: u64, cum_work: [u8; 32], height: u32, prev_nbits: u32, prev_time: u32, epoch_start: u32,
    recent_times: Vec<u32>, anchor_id: [u8; 32], self_id: [u32; 8], #[serde(alias = "ss_set")] live_set: u32,
    #[serde(alias = "ss_acc")] live_hash: [u32; 16], #[serde(alias = "ss_hints")] trie_root: [u8; 32], smt_root: [u8; 32],
    #[serde(default)] max_time: u32, #[serde(default)] profile: u32, #[serde(default)] block_roots: Vec<Option<[u8; 32]>>,
    #[serde(default)] block_leaves: u64, #[serde(default)] lineage: [u8; 32],
}

/// The hints zyncAPP's statement (the guest's HintsStatement, field for field).
#[derive(Serialize, Deserialize, Debug)]
struct HintsStatement {
    kind: u32, format: u32, height: u32, block_hash: [u8; 32], hints_sha256: [u8; 32], hints_len: u64, n_counted: u64,
    n_unspent: u64, unspent_value: u64, undo_from: u32, undo_sha256: [u8; 32], core_id: [u32; 8], lineage: [u8; 32],
    anchor_id: [u8; 32], core_height: u32, tip_hash: [u8; 32], cum_work: [u8; 32], max_time: u32, profile: u32,
}

fn flag<'a>(a: &'a [String], f: &str) -> Option<&'a String> {
    a.iter().position(|x| x == f).and_then(|i| a.get(i + 1))
}

fn sha256(b: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(b).into()
}

fn disp(b: &[u8; 32]) -> String {
    b.iter().rev().map(|x| format!("{x:02x}")).collect()
}

fn hints_prove(a: &[String]) {
    let (dir, out) = (&a[1], &a[4]);
    let stop: u32 = a[2].parse().unwrap_or_else(|_| die("H?"));
    let format: u32 = flag(a, "--format").map(|x| x.parse().unwrap_or_else(|_| die("--format 1|2"))).unwrap_or(libzync::hints::FORMAT_BIP);
    let prof: u32 = flag(a, "--prof").map(|x| x.parse().unwrap_or_else(|_| die("--prof N"))).unwrap_or(0);
    let signet = a.iter().any(|x| x == "--signet");
    let execute = a.iter().any(|x| x == "--execute");
    // --elf <zyncapp-hints.bin>: the published ELF (reproduce/LIBZYNC_IDS) instead of the embedded one
    let elf: Vec<u8> = match flag(a, "--elf") {
        Some(p) => std::fs::read(p).unwrap_or_else(|e| die(&format!("--elf: {e}"))),
        None => ZYNCAPP_HINTS_ELF.to_vec(),
    };
    if elf.is_empty() { die("no hints guest: built with RISC0_SKIP_BUILD, so give --elf <zyncapp-hints.bin>"); }
    let elf_id = hex(risc0_zkvm::compute_image_id(&elf).unwrap_or_else(|e| die(&format!("image id: {e}"))).as_bytes());
    if elf_id != zapp_hints::HINTS_IMAGE_ID {
        println!("HINTS_ID_NOT_PUBLISHED this guest is {elf_id}, the published hints id is {}: its receipts will not verify under the published id", zapp_hints::HINTS_IMAGE_ID);
    }
    let (cs, journal, receipt): (ChainStateJ, Vec<u8>, Option<Receipt>) = if a[3].ends_with(".json") {
        if !execute { die("a state JSON is not a proof: give the core receipt, or --execute"); }
        let cs: ChainStateJ = serde_json::from_slice(&std::fs::read(&a[3]).unwrap_or_else(|e| die(&e.to_string())))
            .unwrap_or_else(|e| die(&format!("{}: {e}", a[3])));
        let j: Vec<u8> = risc0_zkvm::serde::to_vec(&cs).unwrap().iter().flat_map(|w| w.to_le_bytes()).collect();
        (cs, j, None)
    } else {
        let r: Receipt = bincode::deserialize(&std::fs::read(&a[3]).unwrap_or_else(|e| die(&e.to_string())))
            .unwrap_or_else(|e| die(&format!("core receipt: {e}")));
        if matches!(r.inner, risc0_zkvm::InnerReceipt::Fake(_)) && !execute { die("a FakeReceipt core receipt is not a proof"); }
        let cs: ChainStateJ = r.journal.decode().unwrap_or_else(|e| die(&format!("core journal: {e}")));
        (cs, r.journal.bytes.clone(), Some(r))
    };
    if stop == 0 || stop > cs.height { die(&format!("H must be in 1..={} (the core height)", cs.height)); }
    let t0 = std::time::Instant::now();
    let rp = hints_host::replay(cs.height, signet, |h| hints_host::read_block(dir, h));
    if rp.block_roots != libzync::hints::trimmed(&cs.block_roots) || rp.block_leaves != cs.block_leaves {
        die("the native replay's block forest is not the core state's: other blocks, or a rule differs");
    }
    // HINTS_TAMPER=<case>: a negative test (execute only): the witness or the core journal changed so the guest must refuse
    let tc = std::env::var("HINTS_TAMPER").ok();
    if tc.is_some() && !execute { die("HINTS_TAMPER is an execute-only negative test"); }
    let jcase = matches!(tc.as_deref(), Some("lineage" | "pre_undo" | "core_id" | "format" | "assumevalid"));
    let w = hints_host::witness(&rp, stop, format, signet, if jcase { None } else { tc.as_deref() }, |h| hints_host::read_block(dir, h)).unwrap_or_else(|e| die(&e));
    if let Some(e) = &w.native_error { println!("HINTS_NATIVE_REFUSED {e}"); }
    let (mut cs, mut journal, mut format) = (cs, journal, format);
    if jcase {
        if receipt.is_some() { die("a journal tamper case needs a state JSON (the claim is rebuilt from it)"); }
        match tc.as_deref().unwrap() {
            "lineage" => cs.lineage[0] ^= 1,
            "pre_undo" => cs.self_id = [2631239816, 2674494539, 4198157524, 4069463800, 1375117754, 2714639643, 453263298, 4095025963],
            "assumevalid" => cs.profile |= 2,
            "format" => format = 3,
            _ => {}
        }
        journal = risc0_zkvm::serde::to_vec(&cs).unwrap().iter().flat_map(|w| w.to_le_bytes()).collect();
        println!("HINTS_TAMPER {}: the core journal / statement input changed", tc.as_deref().unwrap());
    }
    let core_id_in: [u32; 8] = if tc.as_deref() == Some("core_id") { let mut c = cs.self_id; c[0] ^= 1; c } else { cs.self_id };
    std::fs::create_dir_all(out).unwrap();
    let net = if signet { "signet" } else { "mainnet" };
    if let Some(dv) = &w.derived {
        if sha256(&w.file) != dv.hints_sha256 || w.file.len() as u64 != dv.hints_len { die("internal: file bytes != derived hash"); }
        let (hp, up) = (format!("{out}/{net}-{stop}.hints"), format!("{out}/{net}-{stop}.undo"));
        if tc.is_none() {
            std::fs::write(&hp, &w.file).unwrap();
            std::fs::write(&up, &w.undo_file).unwrap();
        }
        println!("HINTS_NATIVE H={stop} T={} format={format} file={} ({} bytes, sha256 {}) counted={} unspent={} value={} undo_sha256={} block {} ({:.1} s)",
                 cs.height, hp, w.file.len(), hex(&dv.hints_sha256), dv.n_counted, dv.n_unspent, dv.unspent_value,
                 hex(&dv.undo_sha256), disp(&dv.block_hash), t0.elapsed().as_secs_f64());
    }
    let mut b = ExecutorEnv::builder();
    match &receipt {
        Some(r) => { b.add_assumption(r.clone()); }
        None => { b.add_assumption(risc0_zkvm::ReceiptClaim::ok(risc0_zkvm::sha::Digest::from(cs.self_id), journal.clone())); }
    }
    b.write(&core_id_in).unwrap().write(&journal).unwrap().write(&signet).unwrap().write(&format).unwrap().write(&stop).unwrap()
        .write(&hints_host::genesis(signet).to_vec()).unwrap().write(&prof).unwrap()
        .write_slice(&w.frames).write_slice(&[cs.height - stop]).write_slice(&w.tail);
    let env = b.build().unwrap();
    let t1 = std::time::Instant::now();
    let check = |z: &HintsStatement| {
        let dv = w.derived.as_ref().unwrap();
        if z.hints_sha256 != dv.hints_sha256 || z.height != stop || z.block_hash != dv.block_hash { die("the guest's statement is not the native derivation's"); }
    };
    if execute {
        let res = risc0_zkvm::default_executor().execute(env, &elf);
        if let Some(c) = &tc {
            match res {
                Err(e) => { println!("HINTS_GUEST_REFUSED {c}: {}", e.to_string().lines().next().unwrap_or("")); std::process::exit(3) }
                Ok(_) => { println!("HINTS_GUEST_ACCEPTED {c} (a negative test that passed: a BUG)"); std::process::exit(1) }
            }
        }
        let info = res.unwrap_or_else(|e| die(&format!("execute: {e}")));
        let z: HintsStatement = info.journal.decode().unwrap_or_else(|e| die(&format!("journal: {e}")));
        check(&z);
        let outs = w.derived.as_ref().unwrap().n_counted.max(1);
        println!("HINTS_EXEC H={stop} user_cycles={} padded_cycles={} segments={} executed in {:.1} s; {:.0} user cycles/block, {:.0} user cycles/counted output, input {} words{}",
                 info.cycles(), info.segments.iter().map(|s| 1u64 << s.po2).sum::<u64>(), info.segments.len(), t1.elapsed().as_secs_f64(),
                 info.cycles() as f64 / stop as f64, info.cycles() as f64 / outs as f64, w.frames.len() + w.tail.len(),
                 if receipt.is_none() { " [core receipt: unresolved assumption, journal from the state JSON]" } else { "" });
        println!("HINTS_JOURNAL height={} block={} format={} hints_sha256={} hints_len={} counted={} unspent={} value={} undo_sha256={} core={} core_height={}",
                 z.height, disp(&z.block_hash), z.format, hex(&z.hints_sha256), z.hints_len, z.n_counted, z.n_unspent, z.unspent_value,
                 hex(&z.undo_sha256), id_hex(&z.core_id), z.core_height);
    } else {
        let r = default_prover().prove_with_opts(env, &elf, &risc0_zkvm::ProverOpts::succinct())
            .unwrap_or_else(|e| die(&format!("prove: {e}"))).receipt;
        let z: HintsStatement = r.journal.decode().unwrap_or_else(|e| die(&format!("journal: {e}")));
        check(&z);
        let rp = format!("{out}/{net}-{stop}.receipt");
        std::fs::write(&rp, bincode::serialize(&r).unwrap()).unwrap();
        println!("HINTS_PROVED H={stop} -> {rp} in {:.1} s ({} bytes)", t1.elapsed().as_secs_f64(), std::fs::metadata(&rp).unwrap().len());
        println!("HINTS_JOURNAL height={} block={} format={} hints_sha256={} hints_len={} counted={} unspent={} value={} undo_sha256={} core={} core_height={}",
                 z.height, disp(&z.block_hash), z.format, hex(&z.hints_sha256), z.hints_len, z.n_counted, z.n_unspent, z.unspent_value,
                 hex(&z.undo_sha256), id_hex(&z.core_id), z.core_height);
    }
    println!("HINTS_ID {elf_id}");
}

/// The verifier: the same `zapp_hints::verify_hints` as `zapp-hints`, the WASM and the C ABI,
/// against the published hints image id. Exit 0 OK, 2 DEMO, 1 FAIL.
fn hints_verify(a: &[String]) {
    let fail = |m: String| -> ! { println!("ZYNCAPP_FAIL: {m}"); std::process::exit(1) };
    if std::env::var("RISC0_DEV_MODE").map(|v| !v.is_empty() && v != "0").unwrap_or(false) { fail("RISC0_DEV_MODE is set: unset it to verify".into()); }
    let rd = |p: &str| std::fs::read(p).unwrap_or_else(|e| fail(format!("{p}: {e}")));
    let (r, f) = (rd(&a[1]), rd(&a[2]));
    let u = flag(a, "--undo").map(|p| rd(p));
    let pe = |k: &str| flag(a, k).map(|e| swiftsync_verify::parse_expect(e).unwrap_or_else(|| fail(format!("{k}: want <height>:<64 hex digits>"))));
    let (e1, e2) = (pe("--expect"), pe("--expect-tip"));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    // the PUBLISHED hints id (reproduce/LIBZYNC_IDS), the same every tool trusts, not this build's (review H2)
    let rep = zapp_hints::verify_hints(&r, &f, u.as_deref(), zapp_hints::HINTS_IMAGE_ID, flag(a, "--core-image").map(|x| x.as_str()), now,
                                                    e1.as_ref().map(|(h, x)| (*h, x.as_str())), e2.as_ref().map(|(h, x)| (*h, x.as_str())));
    for (ok, what) in &rep.checks { println!("  [{}] {what}", if *ok { " ok " } else { "FAIL" }); }
    let z = match (&rep.verdict, &rep.statement) {
        (swiftsync_verify::Verdict::Fail, _) | (_, None) => fail(rep.checks.iter().find(|c| !c.0).map(|c| c.1.clone()).unwrap_or_default()),
        (_, Some(z)) => z,
    };
    let line = format!("ZYNCAPP_HINTS_OK: {} is the hints file (format {}) for block {} {}: {} outputs indexed, {} unspent ({} sat); undo hashes 1..{} sha256 {}",
                       a[2], z.format, z.height, disp(&z.block_hash), z.n_counted, z.n_unspent, z.unspent_value, z.height, hex(&z.undo_sha256));
    verdict(Ok(rep.verdict == swiftsync_verify::Verdict::Ok), line);
}

/// The utreexo zyncAPP's statement (the guest's UtreexoStatement, field for field).
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct UtreexoStatement {
    kind: u32, self_id: [u32; 8], network: u32, height: u32, to: u32, runs: u32, state_out: [u8; 32], done: u32,
    block_hash: [u8; 32], leaves: u64, roots: Vec<[u8; 32]>, n_live: u64, core_id: [u32; 8], lineage: [u8; 32],
    anchor_id: [u8; 32], core_height: u32, tip_hash: [u8; 32], cum_work: [u8; 32], max_time: u32, profile: u32,
}

/// Roots as Floresta's chainparams lists them (rustreexo's Display: the bytes in order; an Empty root as "empty").
fn roots_str(r: &[[u8; 32]]) -> String {
    r.iter().map(|x| if *x == [0u8; 32] { "empty".to_string() } else { hex(x) }).collect::<Vec<_>>().join(",")
}

fn utreexo_prove(a: &[String]) {
    use libzync::utreexo_roots::{self as ur, Deriver};
    let (dir, out) = (&a[1], &a[4]);
    let stop: u32 = a[2].parse().unwrap_or_else(|_| die("H?"));
    let prof: u32 = flag(a, "--prof").map(|x| x.parse().unwrap_or_else(|_| die("--prof N"))).unwrap_or(0);
    let signet = a.iter().any(|x| x == "--signet");
    let execute = a.iter().any(|x| x == "--execute");
    let split: u32 = flag(a, "--split").map(|x| x.parse().unwrap_or_else(|_| die("--split N"))).unwrap_or(stop).max(1);
    let elf: Vec<u8> = match flag(a, "--elf") {
        Some(p) => std::fs::read(p).unwrap_or_else(|e| die(&format!("--elf: {e}"))),
        None => ZYNCAPP_UTREEXO_ELF.to_vec(),
    };
    if elf.is_empty() { die("no utreexo guest: built with RISC0_SKIP_BUILD, so give --elf"); }
    let self_id_d = risc0_zkvm::compute_image_id(&elf).unwrap_or_else(|e| die(&format!("image id: {e}")));
    let self_id: [u32; 8] = self_id_d.as_words().try_into().unwrap();
    let (cs, journal, receipt): (ChainStateJ, Vec<u8>, Option<Receipt>) = if a[3].ends_with(".json") {
        if !execute { die("a state JSON is not a proof: give the core receipt, or --execute"); }
        let cs: ChainStateJ = serde_json::from_slice(&std::fs::read(&a[3]).unwrap_or_else(|e| die(&e.to_string())))
            .unwrap_or_else(|e| die(&format!("{}: {e}", a[3])));
        let j: Vec<u8> = risc0_zkvm::serde::to_vec(&cs).unwrap().iter().flat_map(|w| w.to_le_bytes()).collect();
        (cs, j, None)
    } else {
        let r: Receipt = bincode::deserialize(&std::fs::read(&a[3]).unwrap_or_else(|e| die(&e.to_string())))
            .unwrap_or_else(|e| die(&format!("core receipt: {e}")));
        if matches!(r.inner, risc0_zkvm::InnerReceipt::Fake(_)) && !execute { die("a FakeReceipt core receipt is not a proof"); }
        let cs: ChainStateJ = r.journal.decode().unwrap_or_else(|e| die(&format!("core journal: {e}")));
        (cs, r.journal.bytes.clone(), Some(r))
    };
    if stop == 0 || stop > cs.height { die(&format!("H must be in 1..={} (the core height)", cs.height)); }
    let t0 = std::time::Instant::now();
    let rp = utreexo_host::replay(cs.height, signet, |h| utreexo_host::read_block(dir, h));
    if rp.block_roots != ur::trimmed(&cs.block_roots) || rp.block_leaves != cs.block_leaves {
        die(&format!("the native replay's block forest is not the core state's (T = {}): other blocks, or a rule differs. \
                      If T >= 147,396 (mainnet's first BIP30-tracked coin), the likely cause is a core state written by a prep \
                      from before the tracked-flag fix (fix/bip30-tracked-flag, PR #736: its state_<h>.flags marker is missing): \
                      its coin log and block forest are not the guest's from 147,396 on. Use a state prepped after the fix", cs.height));
    }
    let tc = std::env::var("UTREEXO_TAMPER").ok();
    if tc.is_some() && !execute { die("UTREEXO_TAMPER is an execute-only negative test"); }
    let jcase = matches!(tc.as_deref(), Some("lineage" | "pre_undo" | "core_id" | "assumevalid" | "state_bytes" | "prev_id"));
    let mut blocks = Vec::new();
    let mut fr = Vec::with_capacity(stop as usize);
    for h in 1..=stop {
        let b = utreexo_host::read_block(dir, h);
        fr.push(utreexo_host::frame(&rp, &b, stop));
        if tc.is_some() { blocks.push(b); }
    }
    let mut tail: Vec<libzync::coinlog::H> = rp.blocks[stop as usize..].iter().map(|f| f.leaf).collect();
    if let (Some(c), false) = (&tc, jcase) {
        println!("UTREEXO_TAMPER {c}: {}", utreexo_host::tamper(c, &mut fr, &blocks, &mut tail).unwrap_or_else(|e| die(&e)));
    }
    // the native derivation, in the same runs as the guest (each run's state through encode/decode, as the guest gets it)
    let ranges: Vec<(u32, u32)> = (0..stop.div_ceil(split)).map(|i| (i * split + 1, ((i + 1) * split).min(stop))).collect();
    let mut states: Vec<Vec<u8>> = Vec::new(); // the state after each run but the last
    let mut d = Deriver::new(stop, &utreexo_host::genesis(signet), !signet).unwrap_or_else(|e| die(e));
    let mut native_err = None;
    'runs: for (i, &(lo, hi)) in ranges.iter().enumerate() {
        if i > 0 {
            let e = states.last().unwrap();
            d = Deriver::decode(e).unwrap_or_else(|e| die(e));
        }
        for h in lo..=hi {
            let f = &fr[h as usize - 1];
            if let Err(e) = d.block(&f.stripped, &f.tracked, &f.status, &f.state, &f.undo) {
                native_err = Some(format!("native derivation, block {h}: {e}"));
                break 'runs;
            }
        }
        if hi < stop { states.push(d.encode()); }
    }
    let derived = if native_err.is_none() {
        for l in &tail { if let Err(e) = d.tail_leaf(l) { native_err = Some(format!("native tail: {e}")); break; } }
        if native_err.is_none() { Some(d.finish().unwrap_or_else(|e| die(e))) } else { None }
    } else { None };
    if let Some(e) = &native_err {
        if tc.is_none() { die(e); }
        println!("UTREEXO_NATIVE_REFUSED {e}");
    }
    if let Some(dv) = &derived {
        if dv.block_roots != ur::trimmed(&cs.block_roots) || dv.block_leaves != cs.block_leaves {
            if tc.is_none() { die("the native derivation's block forest is not the core state's"); }
        }
        let roots = dv.utreexo.roots();
        println!("UTREEXO_NATIVE H={stop} T={} block={} leaves={} live={} roots={} ({:.1} s)", cs.height, disp(&dv.block_hash),
                 dv.utreexo.leaves, dv.n_live, roots_str(&roots), t0.elapsed().as_secs_f64());
        if let Some(rf) = flag(a, "--ref") {
            let log = std::fs::read_to_string(rf).unwrap_or_else(|e| die(&format!("--ref: {e}")));
            let key = format!("UTREEXO_REF height={stop} ");
            let line = log.lines().find(|l| l.starts_with(&key)).unwrap_or_else(|| die(&format!("--ref: no line for height {stop}")));
            let get = |k: &str| line.split_whitespace().find_map(|w| w.strip_prefix(k)).unwrap_or("").to_string();
            let (rl, rr, rb) = (get("leaves="), get("roots="), get("block="));
            if tc.is_none() {
                if rl != dv.utreexo.leaves.to_string() || rr != roots_str(&roots) || rb != disp(&dv.block_hash) {
                    die(&format!("NOT Floresta's: ref leaves={rl} roots={rr} block={rb}"));
                }
                println!("UTREEXO_REF_MATCH H={stop}: leaves and {} roots byte-identical to Floresta's update_acc", roots.len());
            }
        }
    }
    // journal tamper cases
    let (mut cs, mut journal) = (cs, journal);
    if let Some(c) = tc.as_deref().filter(|_| jcase) {
        match c {
            "lineage" => cs.lineage[0] ^= 1,
            "pre_undo" => cs.self_id = [2631239816, 2674494539, 4198157524, 4069463800, 1375117754, 2714639643, 453263298, 4095025963],
            "assumevalid" => cs.profile |= 2,
            _ => {}
        }
        if receipt.is_some() { die("a journal tamper case needs a state JSON"); }
        journal = risc0_zkvm::serde::to_vec(&cs).unwrap().iter().flat_map(|w| w.to_le_bytes()).collect();
        println!("UTREEXO_TAMPER {c}: the core journal / run chain input changed");
    }
    let core_id_in: [u32; 8] = if tc.as_deref() == Some("core_id") { let mut c = cs.self_id; c[0] ^= 1; c } else { cs.self_id };
    std::fs::create_dir_all(out).unwrap();
    let words = utreexo_host::frame_words(&fr);
    // word offset of each block's frame
    let mut off = vec![0usize];
    for f in &fr { off.push(off.last().unwrap() + 3 + f.stripped.len().div_ceil(4) + f.tracked.len() + f.status.len() + 16); }
    let tail_w: Vec<u32> = tail.iter().flat_map(libzync::coinlog::w_of).collect();
    let net = if signet { "signet" } else { "mainnet" };
    let mut prev: Option<(Vec<u8>, Option<Receipt>)> = None; // previous run's journal bytes and receipt
    let mut total = (0u64, 0u64, 0usize);
    for (i, &(lo, hi)) in ranges.iter().enumerate() {
        let last = hi == stop;
        let mut b = ExecutorEnv::builder();
        if last {
            match &receipt {
                Some(r) => { b.add_assumption(r.clone()); }
                None => { b.add_assumption(risc0_zkvm::ReceiptClaim::ok(risc0_zkvm::sha::Digest::from(cs.self_id), journal.clone())); }
            }
        }
        let sid = if tc.as_deref() == Some("prev_id") && i > 0 { let mut x = self_id; x[0] ^= 1; x } else { self_id };
        b.write(&sid).unwrap().write(&core_id_in).unwrap().write(&if last { journal.clone() } else { Vec::new() }).unwrap()
            .write(&signet).unwrap().write(&stop).unwrap().write(&utreexo_host::genesis(signet).to_vec()).unwrap()
            .write(&prof).unwrap().write(&hi).unwrap();
        match &prev {
            None => { b.write(&0u32).unwrap(); }
            Some((pj, pr)) => {
                let mut ps = states[i - 1].clone();
                if tc.as_deref() == Some("state_bytes") { let n = ps.len(); ps[n - 1] ^= 1; }
                match pr {
                    Some(r) => { b.add_assumption(r.clone()); }
                    None => { b.add_assumption(risc0_zkvm::ReceiptClaim::ok(risc0_zkvm::sha::Digest::from(self_id), pj.clone())); }
                }
                b.write(&1u32).unwrap().write(pj).unwrap().write(&ps).unwrap();
            }
        }
        b.write_slice(&words[off[lo as usize - 1]..off[hi as usize]]);
        if last { b.write_slice(&[cs.height - stop]).write_slice(&tail_w); }
        let env = b.build().unwrap();
        let t1 = std::time::Instant::now();
        let (jb, rc) = if execute {
            let res = risc0_zkvm::default_executor().execute(env, &elf);
            if let Some(c) = &tc {
                match res {
                    Err(e) => { println!("UTREEXO_GUEST_REFUSED {c}: run {} ({lo}..{hi}): {}", i + 1, e.to_string().lines().next().unwrap_or("")); std::process::exit(3) }
                    Ok(info) => {
                        if !last { prev = Some((info.journal.bytes.clone(), None)); continue; }
                        println!("UTREEXO_GUEST_ACCEPTED {c} (a negative test that passed: a BUG)"); std::process::exit(1)
                    }
                }
            }
            let info = res.unwrap_or_else(|e| die(&format!("execute run {} ({lo}..{hi}): {e}", i + 1)));
            let padded: u64 = info.segments.iter().map(|s| 1u64 << s.po2).sum();
            total = (total.0 + info.cycles(), total.1 + padded, total.2 + info.segments.len());
            println!("UTREEXO_EXEC run={} blocks={lo}..{hi} user_cycles={} padded_cycles={} segments={} executed in {:.1} s",
                     i + 1, info.cycles(), padded, info.segments.len(), t1.elapsed().as_secs_f64());
            (info.journal.bytes.clone(), None)
        } else {
            let r = default_prover().prove_with_opts(env, &elf, &risc0_zkvm::ProverOpts::succinct())
                .unwrap_or_else(|e| die(&format!("prove run {}: {e}", i + 1))).receipt;
            let p = format!("{out}/{net}-{stop}-run{}.receipt", i + 1);
            std::fs::write(&p, bincode::serialize(&r).unwrap()).unwrap();
            println!("UTREEXO_PROVED run={} blocks={lo}..{hi} -> {p} in {:.1} s ({} bytes)", i + 1, t1.elapsed().as_secs_f64(), std::fs::metadata(&p).unwrap().len());
            (r.journal.bytes.clone(), Some(r))
        };
        let z: UtreexoStatement = risc0_zkvm::serde::from_slice(&jb.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect::<Vec<u32>>())
            .unwrap_or_else(|e| die(&format!("journal: {e}")));
        if !last {
            let want: [u8; 32] = { use sha2::Digest; sha2::Sha256::digest(&states[i]).into() };
            if z.state_out != want || z.done != 0 || z.to != hi { die(&format!("run {}: the guest's end state is not the native one", i + 1)); }
            prev = Some((jb, rc));
            continue;
        }
        let dv = derived.as_ref().unwrap();
        if z.done != 1 || z.roots != dv.utreexo.roots() || z.leaves != dv.utreexo.leaves || z.block_hash != dv.block_hash || z.runs as usize != ranges.len() {
            die("the guest's statement is not the native derivation's");
        }
        if execute && ranges.len() > 1 {
            println!("UTREEXO_EXEC_TOTAL runs={} user_cycles={} padded_cycles={} segments={}", ranges.len(), total.0, total.1, total.2);
        }
        println!("UTREEXO_JOURNAL height={} block={} leaves={} live={} runs={} roots={} core={} core_height={}{}", z.height, disp(&z.block_hash),
                 z.leaves, z.n_live, z.runs, roots_str(&z.roots), id_hex(&z.core_id), z.core_height,
                 if receipt.is_none() { " [core receipt: unresolved assumption, journal from the state JSON]" } else { "" });
        if !execute { std::fs::copy(format!("{out}/{net}-{stop}-run{}.receipt", i + 1), format!("{out}/{net}-{stop}.receipt")).unwrap(); }
        else { std::fs::write(format!("{out}/{net}-{stop}.journal"), &jb).unwrap(); } // a tip step's execute-only start (utreexo-tip-prove)
    }
    println!("UTREEXO_ID {}", hex(self_id_d.as_bytes()));
}

/// The utreexo tip statement (the tip guest's TipStatement, field for field).
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct TipStatement {
    kind: u32, self_id: [u32; 8], base_id: [u32; 8], network: u32, base_height: u32, base_core_id: [u32; 8], steps: u32,
    from: u32, height: u32, block_hash: [u8; 32], leaves: u64, roots: Vec<[u8; 32]>, core_id: [u32; 8], lineage: [u8; 32],
    anchor_id: [u8; 32], core_height: u32, tip_hash: [u8; 32], cum_work: [u8; 32], max_time: u32, profile: u32,
}

fn jwords(b: &[u8]) -> Vec<u32> { b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect() }

/// The core state a step rests on: a receipt (proving) or prep's state JSON (execute only, an unresolved assumption).
fn load_core(path: &str, execute: bool) -> (ChainStateJ, Vec<u8>, Option<Receipt>) {
    if path.ends_with(".json") {
        if !execute { die("a state JSON is not a proof: give the core receipt, or --execute"); }
        let cs: ChainStateJ = serde_json::from_slice(&std::fs::read(path).unwrap_or_else(|e| die(&e.to_string())))
            .unwrap_or_else(|e| die(&format!("{path}: {e}")));
        let j: Vec<u8> = risc0_zkvm::serde::to_vec(&cs).unwrap().iter().flat_map(|w| w.to_le_bytes()).collect();
        (cs, j, None)
    } else {
        let r: Receipt = bincode::deserialize(&std::fs::read(path).unwrap_or_else(|e| die(&e.to_string())))
            .unwrap_or_else(|e| die(&format!("core receipt: {e}")));
        if matches!(r.inner, risc0_zkvm::InnerReceipt::Fake(_)) && !execute { die("a FakeReceipt core receipt is not a proof"); }
        let cs: ChainStateJ = r.journal.decode().unwrap_or_else(|e| die(&format!("core journal: {e}")));
        (cs, r.journal.bytes.clone(), Some(r))
    }
}

fn utreexo_tip_prove(a: &[String]) {
    use utreexo_tip_host::{Native, TipFrame};
    use utreexo_tip_step as step;
    let (dir, prev_path, core_path, out) = (&a[1], &a[2], &a[4], &a[5]);
    let stop: u32 = a[3].parse().unwrap_or_else(|_| die("H?"));
    let signet = a.iter().any(|x| x == "--signet");
    let execute = a.iter().any(|x| x == "--execute");
    let prof: u32 = flag(a, "--prof").map(|x| x.parse().unwrap_or_else(|_| die("--prof N"))).unwrap_or(0);
    let elf: Vec<u8> = match flag(a, "--elf") {
        Some(p) => std::fs::read(p).unwrap_or_else(|e| die(&format!("--elf: {e}"))),
        None => ZYNCAPP_UTREEXO_TIP_ELF.to_vec(),
    };
    if elf.is_empty() { die("no utreexo-tip guest: built with RISC0_SKIP_BUILD, so give --elf"); }
    let self_id_d = risc0_zkvm::compute_image_id(&elf).unwrap_or_else(|e| die(&format!("image id: {e}")));
    let self_id: [u32; 8] = self_id_d.as_words().try_into().unwrap();
    let base_id: [u32; 8] = match flag(a, "--base-id") {
        Some(x) => swiftsync_verify::words(&risc0_zkvm::sha::Digest::from_bytes(h32(x))),
        None => ZYNCAPP_UTREEXO_ID,
    };
    let tc = std::env::var("UTREEXO_TIP_TAMPER").ok();
    if tc.is_some() && !execute { die("UTREEXO_TIP_TAMPER is an execute-only negative test"); }

    // the previous statement: a base (kind 'ZU' 1) or a step ('ZU' 2), from a receipt or (execute only) a journal file
    let (pj, prev_receipt): (Vec<u8>, Option<Receipt>) = if prev_path.ends_with(".journal") {
        if !execute { die("a .journal is not a proof: give the previous receipt, or --execute"); }
        (std::fs::read(prev_path).unwrap_or_else(|e| die(&format!("{prev_path}: {e}"))), None)
    } else {
        let r: Receipt = bincode::deserialize(&std::fs::read(prev_path).unwrap_or_else(|e| die(&format!("{prev_path}: {e}"))))
            .unwrap_or_else(|e| die(&format!("previous receipt: {e}")));
        (r.journal.bytes.clone(), Some(r))
    };
    let pw = jwords(&pj);
    let kind = *pw.first().unwrap_or_else(|| die("the previous journal is empty"));
    let (prev_kind, prev_id, p_height, p_hash, p_leaves, p_roots) = if kind == libzync::utreexo_roots::KIND_UTREEXO {
        let z: UtreexoStatement = risc0_zkvm::serde::from_slice(&pw).unwrap_or_else(|e| die(&format!("base journal: {e}")));
        if z.done != 1 { die("the previous receipt is not a base's final run"); }
        (1u32, base_id, z.height, z.block_hash, z.leaves, z.roots)
    } else if kind == step::KIND_UTREEXO_TIP {
        let z: TipStatement = risc0_zkvm::serde::from_slice(&pw).unwrap_or_else(|e| die(&format!("step journal: {e}")));
        if z.base_id != base_id { die(&format!("the previous step is from base image {}, not {}", id_hex(&z.base_id), id_hex(&base_id))); }
        (2u32, self_id, z.height, z.block_hash, z.leaves, z.roots)
    } else {
        die(&format!("the previous journal's kind 0x{kind:08x} is neither a utreexo base nor a step"))
    };
    if let Some(r) = &prev_receipt {
        if !matches!(r.inner, risc0_zkvm::InnerReceipt::Fake(_)) {
            r.verify(risc0_zkvm::sha::Digest::from(prev_id)).unwrap_or_else(|e| die(&format!("the previous receipt does not verify under {}: {e}", id_hex(&prev_id))));
        }
    }
    let (cs, journal, core_receipt) = load_core(core_path, execute);
    if stop <= p_height || stop > cs.height { die(&format!("H must be in {}..={} (above the previous step, at most the core height)", p_height + 1, cs.height)); }

    // the native replay to the previous height (or the cache of one), then the step's blocks
    let t0 = std::time::Instant::now();
    let cache = flag(a, "--cache");
    let mut nat = match cache.map(|c| Native::load(c)) {
        Some(Ok(n)) if n.height == p_height && n.last == p_hash => { println!("UTREEXO_TIP_CACHE hit at {p_height}"); n }
        Some(Ok(n)) => { println!("UTREEXO_TIP_CACHE at {} {}, not the previous step's {p_height}: replaying from genesis", n.height, utreexo_tip_host::disp(&n.last)); Native::genesis(signet) }
        Some(Err(e)) => { println!("UTREEXO_TIP_CACHE unusable ({e}): replaying from genesis"); Native::genesis(signet) }
        None => Native::genesis(signet),
    };
    while nat.height < p_height {
        let b = utreexo_host::read_block(dir, nat.height + 1);
        nat.block(&b.stripped).unwrap_or_else(|e| die(&format!("native replay: {e}")));
    }
    if nat.last != p_hash { die(&format!("block {p_height} of the block files is {}, the previous statement's is {}", utreexo_tip_host::disp(&nat.last), disp(&p_hash))); }
    if nat.stump.leaves != p_leaves || nat.roots() != p_roots {
        die(&format!("the native accumulator after {p_height} (leaves {}, roots {}) is not the previous statement's (leaves {p_leaves}, roots {})",
                     nat.stump.leaves, roots_str(&nat.roots()), roots_str(&p_roots)));
    }
    nat.counts = Default::default();
    let mut fr: Vec<TipFrame> = Vec::new();
    for h in p_height + 1..=stop {
        let b = utreexo_host::read_block(dir, h);
        fr.push(nat.block(&b.stripped).unwrap_or_else(|e| die(&format!("native step: {e}"))));
    }
    // the link H+1..T: from a headers file ("<height> <80-byte header hex>" per line, as the follower writes it with
    // getblockheader; only the headers are needed), else from the block files
    let mut link: Vec<[u8; 80]> = match flag(a, "--headers") {
        Some(f) => {
            let txt = std::fs::read_to_string(f).unwrap_or_else(|e| die(&format!("--headers: {e}")));
            let rows: Vec<(u32, [u8; 80])> = txt.lines().filter(|l| !l.trim().is_empty()).map(|l| {
                let (h, x) = l.trim().split_once(' ').unwrap_or_else(|| die("--headers: want '<height> <hex>' lines"));
                let x = x.trim();
                if x.len() != 160 || !x.bytes().all(|c| c.is_ascii_hexdigit()) { die(&format!("--headers: line for {h}: want 160 hex digits (an 80-byte header)")); }
                let b: [u8; 80] = utreexo_host::unhex(x).try_into().unwrap_or_else(|_| die("--headers: a header is not 80 bytes"));
                (h.parse().unwrap_or_else(|_| die("--headers: height?")), b)
            }).collect();
            if rows.iter().map(|r| r.0).ne(stop + 1..=cs.height) { die(&format!("--headers: want heights {}..={} in order", stop + 1, cs.height)); }
            rows.into_iter().map(|r| r.1).collect()
        }
        None => (stop + 1..=cs.height).map(|h| utreexo_host::read_block(dir, h).header).collect(),
    };
    let mut tip = nat.last;
    for l in &link { tip = step::link_header(l, &tip).unwrap_or_else(|e| die(e)); }
    if tip != cs.tip_hash { die(&format!("the block files' block {} is {}, the core tip is {}", cs.height, utreexo_tip_host::disp(&tip), disp(&cs.tip_hash))); }
    let c = &nat.counts;
    println!("UTREEXO_TIP_NATIVE from={p_height} H={stop} T={} block={} leaves={} roots={} blocks={} adds={} dels={} proof_hashes={} forest_agrees={} ({:.1} s)",
             cs.height, utreexo_tip_host::disp(&nat.last), nat.stump.leaves, roots_str(&nat.roots()), c.blocks, c.adds, c.dels, c.proof_hashes,
             nat.forest_agrees(), t0.elapsed().as_secs_f64());
    if let Some(rf) = flag(a, "--ref") {
        let log = std::fs::read_to_string(rf).unwrap_or_else(|e| die(&format!("--ref: {e}")));
        let key = format!("UTREEXO_REF height={stop} ");
        let line = log.lines().find(|l| l.starts_with(&key)).unwrap_or_else(|| die(&format!("--ref: no line for height {stop}")));
        let get = |k: &str| line.split_whitespace().find_map(|w| w.strip_prefix(k)).unwrap_or("").to_string();
        if get("leaves=") != nat.stump.leaves.to_string() || get("roots=") != roots_str(&nat.roots()) || get("block=") != utreexo_tip_host::disp(&nat.last) {
            die(&format!("NOT Floresta's: ref {line}"));
        }
        println!("UTREEXO_REF_MATCH H={stop}: leaves and roots byte-identical to Floresta's update_acc");
    }
    if let Some(c) = &tc {
        println!("UTREEXO_TIP_TAMPER {c}: {}", utreexo_tip_host::tamper(c, &mut fr, &mut link).unwrap_or_else(|e| die(&e)));
    }
    // a journal-level tamper: the previous statement's roots (its receipt then does not match: no assumption resolves it)
    let mut pj_in = pj.clone();
    if tc.as_deref() == Some("prev_roots") {
        let n = pj_in.len();
        pj_in[n - 200] ^= 1;
    }
    let prev_kind_in = if tc.as_deref() == Some("prev_kind") { 3 - prev_kind } else { prev_kind };

    let words = utreexo_tip_host::frame_words(&fr);
    let link_w: Vec<u32> = link.iter().flat_map(|l| l.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap()))).collect();
    let mut b = ExecutorEnv::builder();
    match &core_receipt {
        Some(r) => { b.add_assumption(r.clone()); }
        None => { b.add_assumption(risc0_zkvm::ReceiptClaim::ok(risc0_zkvm::sha::Digest::from(cs.self_id), journal.clone())); }
    }
    match &prev_receipt {
        Some(r) => { b.add_assumption(r.clone()); }
        None => { b.add_assumption(risc0_zkvm::ReceiptClaim::ok(risc0_zkvm::sha::Digest::from(prev_id), pj.clone())); }
    }
    let core_id_in: [u32; 8] = if tc.as_deref() == Some("core_id") { let mut c = cs.self_id; c[0] ^= 1; c } else { cs.self_id };
    b.write(&self_id).unwrap().write(&base_id).unwrap().write(&core_id_in).unwrap().write(&journal).unwrap()
        .write(&signet).unwrap().write(&prof).unwrap().write(&stop).unwrap().write(&prev_kind_in).unwrap().write(&pj_in).unwrap();
    b.write_slice(&words);
    b.write_slice(&[link.len() as u32]).write_slice(&link_w);
    let env = b.build().unwrap();
    let t1 = std::time::Instant::now();
    let jb = if execute {
        let res = risc0_zkvm::default_executor().execute(env, &elf);
        if let Some(c) = &tc {
            match res {
                Err(e) => { println!("UTREEXO_TIP_GUEST_REFUSED {c}: {}", e.to_string().lines().next().unwrap_or("")); std::process::exit(3) }
                Ok(_) => { println!("UTREEXO_TIP_GUEST_ACCEPTED {c} (a negative test that passed: a BUG)"); std::process::exit(1) }
            }
        }
        let info = res.unwrap_or_else(|e| die(&format!("execute: {e}")));
        let padded: u64 = info.segments.iter().map(|s| 1u64 << s.po2).sum();
        println!("UTREEXO_TIP_EXEC blocks={}..{stop} link={} user_cycles={} padded_cycles={} segments={} per_block={} executed in {:.1} s",
                 p_height + 1, link.len(), info.cycles(), padded, info.segments.len(), info.cycles() / (stop - p_height) as u64, t1.elapsed().as_secs_f64());
        std::fs::write(format!("{out}.journal"), &info.journal.bytes).unwrap();
        info.journal.bytes.clone()
    } else {
        let r = default_prover().prove_with_opts(env, &elf, &risc0_zkvm::ProverOpts::succinct()).unwrap_or_else(|e| die(&format!("prove: {e}"))).receipt;
        std::fs::write(out, bincode::serialize(&r).unwrap()).unwrap();
        println!("UTREEXO_TIP_PROVED blocks={}..{stop} -> {out} in {:.1} s ({} bytes)", p_height + 1, t1.elapsed().as_secs_f64(), std::fs::metadata(out).unwrap().len());
        r.journal.bytes.clone()
    };
    let z: TipStatement = risc0_zkvm::serde::from_slice(&jwords(&jb)).unwrap_or_else(|e| die(&format!("journal: {e}")));
    if z.roots != nat.roots() || z.leaves != nat.stump.leaves || z.block_hash != nat.last || z.height != stop || z.from != p_height {
        die("the guest's statement is not the native replay's");
    }
    // --cache-out: where the state at H is written (default: over --cache). A follower writes it aside and renames it
    // over its cache only once the step is accepted, so a failed run never leaves a cache for the wrong height
    if let Some(c) = flag(a, "--cache-out").or(cache) { nat.save(c).unwrap_or_else(|e| die(&format!("--cache-out: {e}"))); }
    println!("UTREEXO_TIP_JOURNAL height={} block={} leaves={} steps={} base_height={} from={} roots={} core={} core_height={}{}", z.height, disp(&z.block_hash),
             z.leaves, z.steps, z.base_height, z.from, roots_str(&z.roots), id_hex(&z.core_id), z.core_height,
             if core_receipt.is_none() || prev_receipt.is_none() { " [unresolved assumptions: execute only]" } else { "" });
    println!("UTREEXO_TIP_ID {}", hex(self_id_d.as_bytes()));
}
