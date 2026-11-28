//! `zyncapp`: the zyncAPP guests' input writers for the OpenVM engine (engine/zapps, engine/ENGINE.md). Each `--stream`
//! command writes the guest's input stream (RISC Zero's serde word format, hazync-journal) and stops; `hzengine prove`
//! proves it, with the core proof (and a previous zAPP proof) as its children. RISC Zero's prover left with CUTOVER C8.
//!
//!   zyncapp prove-zeroconf <core tip .journal | state .json> <speculative chunk .journal> <record index> <paths.json> <out>
//!       --stream <file> [--signet | --regtest]
//!       paths.json: [[position, [sibling hex, ...]], ...], one per input of the transaction, from zync-path coin
//!   zyncapp headers-mmr-path <headers file from 0> <height> [--tip <height>]   (the MMR path for zapp-headers --path)
//!   zyncapp hints-prove <block json dir> <H> <core .journal | state_<T>.json> <out dir> --stream <file> [--format 1|2]
//!       [--signet] [--prof N]
//!       the SwiftSync hints file for H (docs/libzync/HINTS_ZAPP.md), its undo-hash file, and the guest's stream.
//!   zyncapp utreexo-prove <block json dir> <H> <core .journal | state_<T>.json> <out dir> --stream <file> [--signet]
//!       [--prof N] [--ref <utreexo-ref log>]
//!       Floresta's Utreexo accumulator after H (docs/libzync/UTREEXO_ZAPP.md), one run. --ref: the roots must equal those
//!       utreexo-ref (Floresta's own code) printed for H. The guest's own id is the stream's first 8 words (zero here:
//!       `hzengine prove --self-id-at 0` writes the engine image's id).
//!   zyncapp utreexo-tip-prove <block json dir> <previous .journal> <H> <core .journal | state_<T>.json> <out>
//!       --stream <file> --base-id <the engine utreexo image id> [--signet] [--prof N] [--cache <file>] [--ref <log>]
//!       [--headers <file>]   (the link H+1..T as "<height> <header hex>" lines; default: from the block files)
//!       [--cache-out <file>] (where the replay's state at H is saved; default: over --cache)
//!       [--source forest|native] (the witness source, default forest: the production source, UTREEXO_ZAPP.md §10.5.1)
//!       [--cache-required] (a --cache that is unusable, or not at or below the previous height, exits 4 instead of a
//!       replay from genesis)
//!       One STEP of the tip mode (UTREEXO_ZAPP.md §10): from the roots the previous statement proves (a base of
//!       utreexo-prove, or an earlier step) to the roots after H, linked by headers H+1..T to the core tip. --cache: the
//!       witness source's state at the previous height (forest: HZUFOR01; native: HZUTIP01), read if it matches.
//!       UTREEXO_TIP_SOURCE_LIE=node|store (--source forest): the source lies and skips its own checks (a negative test:
//!       the guest must refuse), as for UTREEXO_TIP_TAMPER.
//!   zyncapp core-info <core .journal | state_<T>.json>     the core statement's height, tip hash and image (unchecked)
//!   zyncapp utreexo-count <block_<h>.json ...>   per block: Utreexo adds, leaf SHA-512 blocks, inputs (cost model)
//!
//! *_TAMPER=<case> (HINTS_, UTREEXO_, UTREEXO_TIP_): negative tests, the stream changed so the guest must refuse it.
use serde::{Deserialize, Serialize};
mod hints_host;
extern crate alloc;
mod utreexo_host;
mod utreexo_tip_host;
/// The base derivation (libzync), under the name the shared tip step (#[path] below) uses for it.
mod utreexo_roots {
    pub use libzync::utreexo_roots::*;
}
/// The compact-size reader, under the name the shared tip step uses for it.
mod facts {
    pub use libzync::facts::*;
}
#[path = "../../methods/utreexo-tip/src/step.rs"]
mod utreexo_tip_step;
/// The name the witness source's files (also #[path]-included by zapps/utreexo/tip-step) use for step.rs.
use utreexo_tip_step as step;
/// The tip mode's production witness source (UTREEXO_ZAPP.md §10.5.1): the compact forest and the source.
mod utreexo_forest;
mod utreexo_witness;

#[derive(Deserialize)]
#[allow(dead_code)] // the journal's layout: every field must be read to decode it, not every one is used here
struct ChainState {
    kind: u32, version: u32, tip_hash: [u8; 32], utxo_roots: Vec<Option<[u8; 32]>>, utxo_leaves: u64, utxo_value: u64,
    cum_work: [u8; 32], height: u32, prev_nbits: u32, prev_time: u32, epoch_start: u32, recent_times: Vec<u32>,
    anchor_id: [u8; 32], self_id: [u32; 8], #[serde(alias = "ss_set")] live_set: u32,
    #[serde(alias = "ss_acc")] live_hash: [u32; 16], #[serde(alias = "ss_hints")] trie_root: [u8; 32], smt_root: [u8; 32],
    max_time: u32, profile: u32, block_roots: Vec<Option<[u8; 32]>>, block_leaves: u64, lineage: [u8; 32],
}


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
        Some("hints-prove") if a.len() >= 5 => hints_prove(&a),
        Some("utreexo-prove") if a.len() >= 5 => utreexo_prove(&a),
        Some("utreexo-tip-prove") if a.len() >= 6 => utreexo_tip_prove(&a),
        Some("core-info") if a.len() == 2 => {
            // the core statement's height and tip (for a follower choosing H), from a receipt or a state JSON; not a check
            let (cs, _) = load_core(&a[1]);
            println!("CORE height={} tip={} image={} utxo_value={}{}", cs.height, disp(&cs.tip_hash), id_hex(&cs.self_id), cs.utxo_value,
                     if a[1].ends_with(".json") { " (a state JSON)" } else { "" });
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
        Some("prove-zeroconf") if a.len() >= 6 => {
            // <core tip> and <chunk> are the JOURNALS of engine proofs (the proofs go to `hzengine prove --child`, tip first,
            // then the chunk: the guest's env::verify order); the guest's input stream is written and nothing is proved here
            let stream = flag(&a, "--stream").unwrap_or_else(|| die("prove-zeroconf writes the engine's stream: give --stream <file>"));
            let jread = |p: &str| std::fs::read(p).unwrap_or_else(|e| die(&format!("{p}: {e}")));
            // the tip as a journal file, or as a state JSON whose journal is rebuilt (load_core, as the other zAPP hosts take one)
            let (tip_j, chunk_j) = (if a[1].ends_with(".json") { load_core(&a[1]).1 } else { jread(&a[1]) }, jread(&a[2]));
            let cs: ChainState = hazync_journal::serde::from_slice(&jwords(&tip_j)).unwrap_or_else(|e| die(&format!("tip journal: {e}")));
            let idx: u32 = a[3].parse().unwrap_or_else(|_| die("record index?"));
            let raw: Vec<(u64, Vec<String>)> = serde_json::from_slice(&std::fs::read(&a[4]).unwrap_or_else(|e| die(&e.to_string())))
                .unwrap_or_else(|e| die(&format!("paths.json: {e}")));
            let paths: Vec<(u64, Vec<[u8; 32]>)> = raw.into_iter().map(|(p, v)| (p, v.iter().map(|x| h32(x)).collect())).collect();
            // the network code the guest reads (0 mainnet, 1 signet, 2 regtest): one word, as `signet: bool` was
            let net: u32 = if a.iter().any(|x| x == "--regtest") { 2 } else if a.iter().any(|x| x == "--signet") { 1 } else { 0 };
            // the stream: write(&T) is write_slice(serde::to_vec(T))
            let mut ws: Vec<u32> = Vec::new();
            ws.extend(sw(&cs.self_id)); ws.extend(sw(&tip_j)); ws.extend(sw(&chunk_j)); ws.extend(sw(&net));
            ws.extend(sw(&idx)); ws.extend(sw(&paths));
            let bytes: Vec<u8> = ws.iter().flat_map(|x| x.to_le_bytes()).collect();
            std::fs::write(stream, &bytes).unwrap_or_else(|e| die(&format!("--stream: {e}")));
            println!("ZEROCONF_STREAM record {idx} -> {stream} ({} bytes); core={} core_height={} children: tip, then chunk",
                     bytes.len(), id_hex(&cs.self_id), cs.height);
        }
        _ => die("usage: zyncapp prove-zeroconf | hints-prove | utreexo-prove | utreexo-tip-prove (each --stream <file>) | headers-mmr-path | core-info | utreexo-count (see the source's header)"),
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

/// The utreexo zyncAPP's statement (the guest's UtreexoStatement, field for field).
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct UtreexoStatement {
    kind: u32, self_id: [u32; 8], network: u32, height: u32, to: u32, runs: u32, state_out: [u8; 32], done: u32,
    block_hash: [u8; 32], leaves: u64, roots: Vec<[u8; 32]>, n_live: u64, core_id: [u32; 8], lineage: [u8; 32],
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
    // --stream <file> (the OpenVM engine, FEASIBILITY Q20): write the guest's input stream (RISC Zero's serde words) and
    // stop. The core proof goes to `hzengine prove --child`.
    let stream = flag(a, "--stream").unwrap_or_else(|| die("hints-prove writes the engine's stream: give --stream <file>"));
    let (cs, journal) = load_core(&a[3]);
    if stop == 0 || stop > cs.height { die(&format!("H must be in 1..={} (the core height)", cs.height)); }
    let t0 = std::time::Instant::now();
    let rp = hints_host::replay(cs.height, signet, |h| hints_host::read_block(dir, h));
    if rp.block_roots != libzync::hints::trimmed(&cs.block_roots) || rp.block_leaves != cs.block_leaves {
        die("the native replay's block forest is not the core state's: other blocks, or a rule differs");
    }
    // HINTS_TAMPER=<case>: a negative test (execute only): the witness or the core journal changed so the guest must refuse
    let tc = std::env::var("HINTS_TAMPER").ok();
    let jcase = matches!(tc.as_deref(), Some("lineage" | "pre_undo" | "core_id" | "format" | "assumevalid"));
    let w = hints_host::witness(&rp, stop, format, signet, if jcase { None } else { tc.as_deref() }, |h| hints_host::read_block(dir, h)).unwrap_or_else(|e| die(&e));
    if let Some(e) = &w.native_error { println!("HINTS_NATIVE_REFUSED {e}"); }
    let (mut cs, mut journal, mut format) = (cs, journal, format);
    if jcase {
        match tc.as_deref().unwrap() {
            "lineage" => cs.lineage[0] ^= 1,
            "pre_undo" => cs.self_id = [2631239816, 2674494539, 4198157524, 4069463800, 1375117754, 2714639643, 453263298, 4095025963],
            "assumevalid" => cs.profile |= 2,
            "format" => format = 3,
            _ => {}
        }
        journal = hazync_journal::serde::to_vec(&cs).unwrap().iter().flat_map(|w| w.to_le_bytes()).collect();
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
    // the stream: write(&T) is write_slice(serde::to_vec(T))
    let mut ws: Vec<u32> = Vec::new();
    ws.extend(sw(&core_id_in)); ws.extend(sw(&journal)); ws.extend(sw(&signet)); ws.extend(sw(&format)); ws.extend(sw(&stop));
    ws.extend(sw(&hints_host::genesis(signet).to_vec())); ws.extend(sw(&prof));
    ws.extend_from_slice(&w.frames); ws.push(cs.height - stop); ws.extend_from_slice(&w.tail);
    let bytes: Vec<u8> = ws.iter().flat_map(|x| x.to_le_bytes()).collect();
    std::fs::write(stream, &bytes).unwrap_or_else(|e| die(&format!("--stream: {e}")));
    let dv = w.derived.as_ref().unwrap_or_else(|| die("no native derivation to check the engine's statement against"));
    println!("HINTS_STREAM H={stop} -> {stream} ({} bytes); expect hints_sha256={} block={} core={} core_height={}",
             bytes.len(), hex(&dv.hints_sha256), disp(&dv.block_hash), id_hex(&core_id_in), cs.height);
}

fn roots_str(r: &[[u8; 32]]) -> String {
    r.iter().map(|x| if *x == [0u8; 32] { "empty".to_string() } else { hex(x) }).collect::<Vec<_>>().join(",")
}

fn utreexo_prove(a: &[String]) {
    use libzync::utreexo_roots::{self as ur, Deriver};
    let (dir, out) = (&a[1], &a[4]);
    let stop: u32 = a[2].parse().unwrap_or_else(|_| die("H?"));
    let prof: u32 = flag(a, "--prof").map(|x| x.parse().unwrap_or_else(|_| die("--prof N"))).unwrap_or(0);
    let signet = a.iter().any(|x| x == "--signet");
    // --stream <file> (the OpenVM engine): write the guest's input stream and stop. One run (a chained run needs the
    // previous run's ENGINE proof). The guest's own id is the stream's first 8 words: zero here, the engine writes its
    // image id there (`hzengine prove --self-id-at 0`).
    let stream = flag(a, "--stream").unwrap_or_else(|| die("utreexo-prove writes the engine's stream: give --stream <file>"));
    if flag(a, "--split").is_some() { die("--stream writes one run: no --split"); }
    let split = stop.max(1);
    let self_id: [u32; 8] = [0; 8];
    let (cs, journal) = load_core(&a[3]);
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
        journal = hazync_journal::serde::to_vec(&cs).unwrap().iter().flat_map(|w| w.to_le_bytes()).collect();
        println!("UTREEXO_TAMPER {c}: the core journal / run chain input changed");
    }
    let core_id_in: [u32; 8] = if tc.as_deref() == Some("core_id") { let mut c = cs.self_id; c[0] ^= 1; c } else { cs.self_id };
    std::fs::create_dir_all(out).unwrap();
    let words = utreexo_host::frame_words(&fr);
    // word offset of each block's frame
    let mut off = vec![0usize];
    for f in &fr { off.push(off.last().unwrap() + 3 + f.stripped.len().div_ceil(4) + f.tracked.len() + f.status.len() + 16); }
    let tail_w: Vec<u32> = tail.iter().flat_map(libzync::coinlog::w_of).collect();
    let _ = (&states, ranges.len());
    // the one run's stream (write(&T) is write_slice(serde::to_vec(T))); prev_id: the next run's id check (no next run here)
    let sid = self_id;
    let hi = stop;
    let mut ws: Vec<u32> = Vec::new();
    ws.extend(sw(&sid)); ws.extend(sw(&core_id_in)); ws.extend(sw(&journal));
    ws.extend(sw(&signet)); ws.extend(sw(&stop)); ws.extend(sw(&utreexo_host::genesis(signet).to_vec()));
    ws.extend(sw(&prof)); ws.extend(sw(&hi));
    ws.extend(sw(&0u32));
    ws.extend_from_slice(&words[off[0]..off[stop as usize]]);
    ws.push(cs.height - stop); ws.extend_from_slice(&tail_w);
    let bytes: Vec<u8> = ws.iter().flat_map(|x| x.to_le_bytes()).collect();
    std::fs::write(stream, &bytes).unwrap_or_else(|e| die(&format!("--stream: {e}")));
    let dv = derived.as_ref().unwrap_or_else(|| die("no native derivation to check the engine's statement against"));
    println!("UTREEXO_STREAM H={stop} -> {stream} ({} bytes, self id at byte 0); expect leaves={} roots={} block={} core={} core_height={}",
             bytes.len(), dv.utreexo.leaves, roots_str(&dv.utreexo.roots()), disp(&dv.block_hash), id_hex(&core_id_in), cs.height);
}

/// The utreexo tip statement (the tip guest's TipStatement, field for field).
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct TipStatement {
    kind: u32, self_id: [u32; 8], base_id: [u32; 8], network: u32, base_height: u32, base_core_id: [u32; 8], steps: u32,
    from: u32, height: u32, block_hash: [u8; 32], leaves: u64, roots: Vec<[u8; 32]>, core_id: [u32; 8], lineage: [u8; 32],
    anchor_id: [u8; 32], core_height: u32, tip_hash: [u8; 32], cum_work: [u8; 32], max_time: u32, profile: u32,
}

/// A value as the guest's env::read sees it: RISC Zero's serde words.
fn sw<T: Serialize + ?Sized>(v: &T) -> Vec<u32> { hazync_journal::serde::to_vec(v).unwrap() }

fn jwords(b: &[u8]) -> Vec<u32> { b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect() }

/// The core state a stream rests on: an engine proof's journal (`.journal`, the proof itself goes to `hzengine prove
/// --child`) or prep's state JSON (its journal rebuilt: an input for execute-only engine runs).
fn load_core(path: &str) -> (ChainStateJ, Vec<u8>) {
    if path.ends_with(".json") {
        let cs: ChainStateJ = serde_json::from_slice(&std::fs::read(path).unwrap_or_else(|e| die(&e.to_string())))
            .unwrap_or_else(|e| die(&format!("{path}: {e}")));
        let j: Vec<u8> = hazync_journal::serde::to_vec(&cs).unwrap().iter().flat_map(|w| w.to_le_bytes()).collect();
        (cs, j)
    } else {
        let j = std::fs::read(path).unwrap_or_else(|e| die(&format!("{path}: {e}")));
        if j.len() % 4 != 0 { die(&format!("{path}: not an engine journal (not whole words)")); }
        let cs: ChainStateJ = hazync_journal::serde::from_slice(&jwords(&j)).unwrap_or_else(|e| die(&format!("{path}: not a core journal: {e}")));
        (cs, j)
    }
}

fn utreexo_tip_prove(a: &[String]) {
    use utreexo_tip_host::{Source, TipFrame};
    let (dir, prev_path, core_path) = (&a[1], &a[2], &a[4]);
    let stop: u32 = a[3].parse().unwrap_or_else(|_| die("H?"));
    let signet = a.iter().any(|x| x == "--signet");
    // --stream <file> (the OpenVM engine): write the guest's input stream and stop. Its own id is the stream's first 8
    // words, zero here (`hzengine prove --self-id-at 0` writes the engine image's id); --base-id is the ENGINE utreexo
    // image's id, and the previous statement a .journal of an engine proof (the proof itself goes to --child).
    let stream = flag(a, "--stream").unwrap_or_else(|| die("utreexo-tip-prove writes the engine's stream: give --stream <file>"));
    let prof: u32 = flag(a, "--prof").map(|x| x.parse().unwrap_or_else(|_| die("--prof N"))).unwrap_or(0);
    let self_id: [u32; 8] = [0; 8];
    let base_id: [u32; 8] = swiftsync_verify::words(&h32(flag(a, "--base-id").unwrap_or_else(|| die("--stream needs --base-id <the engine utreexo image id>"))));
    let tc = std::env::var("UTREEXO_TIP_TAMPER").ok();
    let forest = match flag(a, "--source").map(|x| x.as_str()) {
        None | Some("forest") => true,
        Some("native") => false,
        Some(x) => die(&format!("--source {x}: forest or native")),
    };
    let lie = match std::env::var("UTREEXO_TIP_SOURCE_LIE").ok().as_deref() {
        None => None,
        Some("node") => Some(utreexo_witness::Lie::Node),
        Some("store") => Some(utreexo_witness::Lie::Store),
        Some(x) => die(&format!("UTREEXO_TIP_SOURCE_LIE={x}: node or store")),
    };
    if lie.is_some() && (!forest || tc.is_some()) { die("UTREEXO_TIP_SOURCE_LIE is a negative test of --source forest, alone"); }

    // the previous statement: a base (kind 'ZU' 1) or a step ('ZU' 2), the journal of its engine proof
    if !prev_path.ends_with(".journal") { die("the previous statement is an engine proof's .journal (the proof goes to --child)"); }
    let pj: Vec<u8> = std::fs::read(prev_path).unwrap_or_else(|e| die(&format!("{prev_path}: {e}")));
    let pw = jwords(&pj);
    let kind = *pw.first().unwrap_or_else(|| die("the previous journal is empty"));
    let (prev_kind, _prev_id, p_height, p_hash, p_leaves, p_roots) = if kind == libzync::utreexo_roots::KIND_UTREEXO {
        let z: UtreexoStatement = hazync_journal::serde::from_slice(&pw).unwrap_or_else(|e| die(&format!("base journal: {e}")));
        if z.done != 1 { die("the previous receipt is not a base's final run"); }
        (1u32, base_id, z.height, z.block_hash, z.leaves, z.roots)
    } else if kind == step::KIND_UTREEXO_TIP {
        let z: TipStatement = hazync_journal::serde::from_slice(&pw).unwrap_or_else(|e| die(&format!("step journal: {e}")));
        if z.base_id != base_id { die(&format!("the previous step is from base image {}, not {}", id_hex(&z.base_id), id_hex(&base_id))); }
        (2u32, self_id, z.height, z.block_hash, z.leaves, z.roots)
    } else {
        die(&format!("the previous journal's kind 0x{kind:08x} is neither a utreexo base nor a step"))
    };
    let (cs, journal) = load_core(core_path);
    if stop <= p_height || stop > cs.height { die(&format!("H must be in {}..={} (above the previous step, at most the core height)", p_height + 1, cs.height)); }

    // the native replay to the previous height (or the cache of one), then the step's blocks
    let t0 = std::time::Instant::now();
    let cache = flag(a, "--cache");
    let cache_required = a.iter().any(|x| x == "--cache-required");
    if cache_required && cache.is_none() { die("--cache-required needs --cache"); }
    let mut nat = match cache.map(|c| Source::load(forest, c)) {
        // the cache of exactly P, or of a height BELOW P from which the source replays forward over the block files: each
        // replayed block must follow the cache's block by hash (parse_block), and the result must be the previous
        // statement's block, leaf count and roots (checked below), so a cache of another branch cannot get through
        Some(Ok(n)) if n.height() == p_height && n.last() == p_hash => { println!("UTREEXO_TIP_CACHE hit at {p_height} ({}, {:.1} s)", n.name(), t0.elapsed().as_secs_f64()); n }
        Some(Ok(n)) if n.height() < p_height => { println!("UTREEXO_TIP_CACHE below at {} {} ({}): replaying {}..{p_height}", n.height(), utreexo_tip_host::disp(&n.last()), n.name(), n.height() + 1); n }
        // --cache-required (the follower, review round 2 #2): a cache that cannot be used is EXIT 4, never a silent replay
        // from genesis (whose bodies a pruned follower does not have); the follower moves it aside and tries an older one
        Some(Ok(n)) if cache_required => { println!("UTREEXO_TIP_CACHE_REFUSED at {} {}: not at or below the previous step's {p_height} (or another block at it)", n.height(), utreexo_tip_host::disp(&n.last())); std::process::exit(4) }
        Some(Err(e)) if cache_required => { println!("UTREEXO_TIP_CACHE_REFUSED unusable ({e})"); std::process::exit(4) }
        Some(Ok(n)) => { println!("UTREEXO_TIP_CACHE at {} {}, not at or below the previous step's {p_height}: replaying from genesis", n.height(), utreexo_tip_host::disp(&n.last())); Source::genesis(forest, signet) }
        Some(Err(e)) => { println!("UTREEXO_TIP_CACHE unusable ({e}): replaying from genesis"); Source::genesis(forest, signet) }
        None => Source::genesis(forest, signet),
    };
    let replay_from = nat.height();
    while nat.height() < p_height {
        let b = utreexo_host::read_block(dir, nat.height() + 1);
        nat.block(&b).unwrap_or_else(|e| die(&format!("native replay: {e}")));
    }
    if nat.height() > replay_from { println!("UTREEXO_TIP_REPLAY {}..{p_height} ({}) in {:.1} s", replay_from + 1, nat.name(), t0.elapsed().as_secs_f64()); }
    if !nat.forest_agrees() { die("the source's forest and its Stump disagree at the previous height: not proving"); }
    if nat.last() != p_hash { die(&format!("block {p_height} of the block files is {}, the previous statement's is {}", utreexo_tip_host::disp(&nat.last()), disp(&p_hash))); }
    if nat.leaves() != p_leaves || nat.roots() != p_roots {
        die(&format!("the native accumulator after {p_height} (leaves {}, roots {}) is not the previous statement's (leaves {p_leaves}, roots {})",
                     nat.leaves(), roots_str(&nat.roots()), roots_str(&p_roots)));
    }
    if let (Some(l), Source::Forest(f)) = (lie, &mut nat) {
        f.set_lie(l);
        println!("UTREEXO_TIP_SOURCE_LIE {l:?}: the source lies from block {} on and skips its own checks", p_height + 1);
    }
    *nat.counts() = Default::default();
    let mut fr: Vec<TipFrame> = Vec::new();
    let tw = std::time::Instant::now();
    let mut read_s = 0f64;
    for h in p_height + 1..=stop {
        let tr = std::time::Instant::now();
        let b = utreexo_host::read_block(dir, h);
        read_s += tr.elapsed().as_secs_f64();
        fr.push(nat.block(&b).unwrap_or_else(|e| die(&format!("native step: {e}"))));
    }
    {
        // the witness of this step: its size, the time the source took (block files read excluded), the forest's size
        let (sb, qb): (usize, usize) = (fr.iter().map(|f| f.spent.len()).sum(), fr.iter().map(|f| f.proof.len()).sum());
        let digest = sha256(&utreexo_tip_host::frame_words(&fr).iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>());
        let c = nat.counts().clone();
        let fs = nat.forest_stats().map(|(n, l, b)| format!(" forest_nodes={n} forest_leaves={l} forest_heap_bytes={b}")).unwrap_or_default();
        println!("UTREEXO_TIP_WITNESS source={} blocks={} dels={} spent_bytes={sb} proof_bytes={qb} proof_hashes={} frames_sha256={} source_seconds={:.3} read_seconds={read_s:.3}{fs}",
                 nat.name(), stop - p_height, c.dels, c.proof_hashes, hex(&digest), tw.elapsed().as_secs_f64() - read_s);
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
    let mut tip = nat.last();
    for l in &link { tip = step::link_header(l, &tip).unwrap_or_else(|e| die(e)); }
    if tip != cs.tip_hash { die(&format!("the block files' block {} is {}, the core tip is {}", cs.height, utreexo_tip_host::disp(&tip), disp(&cs.tip_hash))); }
    let c = nat.counts().clone();
    println!("UTREEXO_TIP_NATIVE from={p_height} H={stop} T={} block={} leaves={} roots={} blocks={} adds={} dels={} proof_hashes={} forest_agrees={} ({:.1} s)",
             cs.height, utreexo_tip_host::disp(&nat.last()), nat.leaves(), roots_str(&nat.roots()), c.blocks, c.adds, c.dels, c.proof_hashes,
             nat.forest_agrees(), t0.elapsed().as_secs_f64());
    // the forest the next step starts from must be the accumulator the guest will commit (review F2): a forest that
    // drifted from the Stump would give proofs that fail later, or a cache that is wrong
    if lie.is_none() && !nat.forest_agrees() { die("the source's forest and its Stump disagree after the step: not proving, nothing saved"); }
    if let Some(rf) = flag(a, "--ref") {
        let log = std::fs::read_to_string(rf).unwrap_or_else(|e| die(&format!("--ref: {e}")));
        let key = format!("UTREEXO_REF height={stop} ");
        let line = log.lines().find(|l| l.starts_with(&key)).unwrap_or_else(|| die(&format!("--ref: no line for height {stop}")));
        let get = |k: &str| line.split_whitespace().find_map(|w| w.strip_prefix(k)).unwrap_or("").to_string();
        if get("leaves=") != nat.leaves().to_string() || get("roots=") != roots_str(&nat.roots()) || get("block=") != utreexo_tip_host::disp(&nat.last()) {
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

    // F3: the source's state at H is saved BEFORE proving, to <cache-out>.pending (renamed into place only after the
    // guest's statement is checked), and the source is dropped, so the forest (~21 GB at the tip, ESTIMATE) is not held
    // through the proof. What the statement check needs is kept: the block, the leaf count and the roots
    let (n_roots, n_leaves, n_last, n_name) = (nat.roots(), nat.leaves(), nat.last(), nat.name());
    let cache_target = flag(a, "--cache-out").or(cache).cloned();
    let pending = cache_target.as_ref().map(|c| format!("{c}.pending"));
    if let Some(pp) = &pending {
        if lie.is_none() {
            let ts = std::time::Instant::now();
            let n = nat.save(pp).unwrap_or_else(|e| die(&format!("--cache-out: {e}")));
            println!("UTREEXO_TIP_CACHE_SAVED {n_name} at {stop}: {n} bytes in {:.1} s (pending until the statement is checked)", ts.elapsed().as_secs_f64());
        }
    }
    drop(nat);
    let words = utreexo_tip_host::frame_words(&fr);
    let link_w: Vec<u32> = link.iter().flat_map(|l| l.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap()))).collect();
    let core_id_in: [u32; 8] = if tc.as_deref() == Some("core_id") { let mut c = cs.self_id; c[0] ^= 1; c } else { cs.self_id };
    // the stream: write(&T) is write_slice(serde::to_vec(T))
    let mut ws: Vec<u32> = Vec::new();
    ws.extend(sw(&self_id)); ws.extend(sw(&base_id)); ws.extend(sw(&core_id_in)); ws.extend(sw(&journal));
    ws.extend(sw(&signet)); ws.extend(sw(&prof)); ws.extend(sw(&stop)); ws.extend(sw(&prev_kind_in)); ws.extend(sw(&pj_in));
    ws.extend_from_slice(&words);
    ws.push(link.len() as u32); ws.extend_from_slice(&link_w);
    let bytes: Vec<u8> = ws.iter().flat_map(|x| x.to_le_bytes()).collect();
    std::fs::write(stream, &bytes).unwrap_or_else(|e| die(&format!("--stream: {e}")));
    println!("UTREEXO_TIP_STREAM blocks={}..{stop} -> {stream} ({} bytes, self id at byte 0); expect leaves={n_leaves} roots={} block={} base={} core={}",
             p_height + 1, bytes.len(), roots_str(&n_roots), utreexo_tip_host::disp(&n_last), id_hex(&base_id), id_hex(&core_id_in));
}
