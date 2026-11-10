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
//!   zyncapp id
//!   zyncapp bench-trie <dump file>                                          (E1 split: the trie update in its own receipt)
//!
//! The core receipt is an ASSUMPTION of the zyncAPP receipt (risc0 composition): the zyncAPP receipt's verification
//! covers the core proof. The verifier still pins the committed core image id and anchor (policy).
use risc0_zkvm::{default_prover, ExecutorEnv, Receipt};
use serde::{Deserialize, Serialize};
use zyncapp_methods::{ZYNCAPP_COIN_ELF, ZYNCAPP_COIN_ID, ZYNCAPP_ZEROCONF_ELF, ZYNCAPP_ZEROCONF_ID, ZYNCAPP_HASHBENCH_ELF, ZYNCAPP_TRIEBENCH_ELF, ZYNCAPP_HEADERS_ELF, ZYNCAPP_HEADERS_ID};

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
            struct HState { height: i64, hash: [u8; 32], work: [u8; 32], bits: u32, time: u32, epoch_first_time: u32, recent_times: Vec<u32>, max_time: u32 }
            let (st, run): (HState, &[(i64, Vec<u8>)]) = if lines[0].0 == 0 {
                (HState { height: -1, hash: [0; 32], work: [0; 32], bits: 0, time: 0, epoch_first_time: 0, recent_times: vec![], max_time: 0 }, &lines[..])
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
                (HState { height: seed[10].0, hash: dsha(last), work, bits: le(&last[72..76]), time: le(&last[68..72]), epoch_first_time: le(&lines[b].1[68..72]),
                          recent_times: seed.iter().map(|(_, h)| le(&h[68..72])).collect(), max_time: seed.iter().map(|(_, h)| le(&h[68..72])).max().unwrap() }, &lines[start..])
            };
            let words: Vec<u32> = run.iter().flat_map(|(_, h)| h.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect::<Vec<_>>()).collect();
            let env = ExecutorEnv::builder().write(&st).unwrap().write(&(run.len() as u32)).unwrap().write_slice(&words).build().unwrap();
            let t = std::time::Instant::now();
            let info = risc0_zkvm::default_executor().execute(env, ZYNCAPP_HEADERS_ELF).unwrap_or_else(|e| die(&e.to_string()));
            #[derive(serde::Deserialize)]
            struct HOut { height: i64, hash: [u8; 32], work: [u8; 32], bits: u32, time: u32, epoch_first_time: u32, recent_times: Vec<u32>, max_time: u32 }
            let (_d, out): ([u8; 32], HOut) = info.journal.decode().unwrap_or_else(|e| die(&e.to_string()));
            let _ = (out.bits, out.time, out.epoch_first_time, &out.recent_times, out.max_time);
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
            // --execute: execute only (cycles, no proof). Verify with `swiftsync-verify headers <receipt>...`.
            // --elf <file>: prove with this guest ELF (the published one, reproduce/LIBZYNC_IDS) instead of the embedded
            // one, so a GPU box builds the host with RISC0_SKIP_BUILD=1 and needs no guest toolchain; its image id is printed.
            let elf: Vec<u8> = match a.iter().position(|x| x == "--elf").and_then(|i| a.get(i + 1)) {
                Some(p) => std::fs::read(p).unwrap_or_else(|e| die(&format!("--elf: {e}"))),
                None => ZYNCAPP_HEADERS_ELF.to_vec(),
            };
            if elf.is_empty() { die("no headers guest: built with RISC0_SKIP_BUILD, so give --elf <zyncapp-headers.bin>"); }
            let elf_id = risc0_zkvm::compute_image_id(&elf).unwrap_or_else(|e| die(&format!("image id: {e}")));
            use swiftsync_verify::headers::HeaderState;
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
            let env = ExecutorEnv::builder().write(&st).unwrap().write(&(run.len() as u32)).unwrap().write_slice(&words).build().unwrap();
            let t = std::time::Instant::now();
            if a.iter().any(|x| x == "--execute") {
                let info = risc0_zkvm::default_executor().execute(env, &elf).unwrap_or_else(|e| die(&e.to_string()));
                let (_d, out): ([u8; 32], HeaderState) = info.journal.decode().unwrap_or_else(|e| die(&e.to_string()));
                println!("HEADERS_EXEC {} headers ({}..{}) {} cycles, {} segments, {:.1} s; END {} {} chainwork {}", run.len(), st.height + 1, out.height,
                         info.cycles(), info.segments.len(), t.elapsed().as_secs_f64(), out.height, out.hash_hex(), out.work_hex());
            } else {
                let r = default_prover().prove_with_opts(env, &elf, &risc0_zkvm::ProverOpts::succinct())
                    .unwrap_or_else(|e| die(&format!("prove: {e}"))).receipt;
                let (_d, out): ([u8; 32], HeaderState) = r.journal.decode().unwrap_or_else(|e| die(&e.to_string()));
                std::fs::write(&a[2], bincode::serialize(&r).unwrap()).unwrap_or_else(|e| die(&e.to_string()));
                println!("HEADERS_PROVED {} headers ({}..{}) in {:.1} s -> {}; END {} {} chainwork {}", run.len(), st.height + 1, out.height,
                         t.elapsed().as_secs_f64(), a[2], out.height, out.hash_hex(), out.work_hex());
            }
            println!("HEADERS_ID {}", hex(elf_id.as_bytes()));
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
