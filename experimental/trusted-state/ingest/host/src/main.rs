//! ts-ingest <anchor prefix> <snapshot> <side prefix>: EXPERIMENTAL (docs/libzync/TRUSTED_STATE.md §6). Executes the
//! ingest guest in the risc0 executor (no proof) on a snapshot and the inputs `ts-anchor build --ingest` wrote, checks
//! every committed value against that anchor's `.json`, and prints the cycles, total and per phase.
use risc0_zkvm::{default_executor, ExecutorEnv};
use ts_ingest_methods::{TS_INGEST_ELF as GUEST_ELF, TS_INGEST_ID as GUEST_ID, TS_CHUNK_ELF, TS_CHUNK_ID, TS_COMBINE_ELF, TS_COMBINE_ID, TS_BIP30_ELF, TS_BIP30_ID, TS_JOIN_ELF, TS_JOIN_ID, CONTROLS};

fn sections(b: &[u8]) -> Vec<Vec<u8>> {
    let mut v = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let n = u32::from_le_bytes(b[i..i + 4].try_into().unwrap()) as usize;
        v.push(b[i + 4..i + 4 + n].to_vec());
        i += 4 + n;
    }
    v
}
fn words(b: &[u8]) -> Vec<u32> { b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect() }
fn pad(mut b: Vec<u8>) -> Vec<u8> { while b.len() % 4 != 0 { b.push(0); } b }

/// `ts-ingest-host chunk <chunk_i.in> <receipt out>` / `ts-ingest-host combine <final.in> <expected.journal|-> <chunk receipts...>`:
/// the chunked ingest (TRUSTED_STATE.md §12). Both run the guest through the prover in DEV MODE (RISC0_DEV_MODE=1: executed,
/// a fake receipt carrying the claim, nothing proved), so the combine can resolve the chunks' receipts as assumptions.
fn section_writer(b: &mut risc0_zkvm::ExecutorEnvBuilder, s: &[u8]) {
    b.write(&(s.len() as u32)).unwrap();
    if !s.is_empty() { b.write_slice(&pad(s.to_vec())); }
}

fn prove_dev(env: risc0_zkvm::ExecutorEnv, elf: &[u8]) -> risc0_zkvm::ProveInfo {
    assert!(std::env::var("RISC0_DEV_MODE").as_deref() == Ok("1"), "set RISC0_DEV_MODE=1: the chunked ingest is measured in dev mode");
    risc0_zkvm::default_prover().prove(env, elf).expect("guest")
}

fn chunk_cmd(inp: &str, out: &str) {
    let s = sections(&std::fs::read(inp).unwrap());
    assert_eq!(s.len(), 8, "chunk input: 8 sections");
    let mut log: Vec<u8> = Vec::new();
    let t0 = std::time::Instant::now();
    let info = {
        let mut b = ExecutorEnv::builder();
        b.stdout(&mut log);
        for x in &s { section_writer(&mut b, x); }
        prove_dev(b.build().unwrap(), TS_CHUNK_ELF)
    };
    let tsc = String::from_utf8_lossy(&log).lines().find(|l| l.contains("TSC ")).unwrap_or("").to_string();
    std::fs::write(out, bincode::serialize(&info.receipt).unwrap()).unwrap();
    println!("CHUNK {inp}: user {} total(padded) {} segments {} ({:.1} s)", info.stats.user_cycles, info.stats.total_cycles, info.stats.segments, t0.elapsed().as_secs_f64());
    println!("  {}", tsc.trim());
}

/// The cut mask of a controls build (TS_CUT, default 0); a real build takes none and refuses TS_CUT.
fn write_cut(b: &mut risc0_zkvm::ExecutorEnvBuilder) {
    let cut: u32 = std::env::var("TS_CUT").ok().map(|v| v.parse().expect("TS_CUT: a number")).unwrap_or(0);
    if CONTROLS { b.write(&cut).unwrap(); } else { assert!(cut == 0, "TS_CUT needs the controls build (TS_CONTROLS=1)"); }
}

/// `ts-ingest-host bip30 <piece_i.in> <receipt out>`: one piece of the split BIP30 receipt (TRUSTED_STATE.md §13.1),
/// dev mode. Input sections: [d, p_lo, p_hi] (3 u32), e_in, r_in, entries (36 B each), REPEATABLE (32 B each).
fn bip30_cmd(inp: &str, out: &str) {
    let s = sections(&std::fs::read(inp).unwrap());
    assert_eq!(s.len(), 5, "piece input: 5 sections");
    let dlh = words(&s[0]);
    let mut log: Vec<u8> = Vec::new();
    let t0 = std::time::Instant::now();
    let r = {
        let mut b = ExecutorEnv::builder();
        b.stdout(&mut log);
        write_cut(&mut b);
        for x in &dlh { b.write(x).unwrap(); }
        for x in &s[1..] { section_writer(&mut b, x); }
        let env = b.build().unwrap();
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prove_dev(env, TS_BIP30_ELF)))
    };
    let text = String::from_utf8_lossy(&log).to_string();
    let info = match r { Ok(i) => i, Err(_) => { println!("BIP30_REFUSED {inp}"); std::process::exit(1); } };
    std::fs::write(out, bincode::serialize(&info.receipt).unwrap()).unwrap();
    println!("BIP30 {inp} (image {}): user {} total(padded) {} segments {} ({:.1} s)", hex_id(&TS_BIP30_ID), info.stats.user_cycles,
             info.stats.total_cycles, info.stats.segments, t0.elapsed().as_secs_f64());
    println!("  {}", text.lines().find(|l| l.contains("TSB ")).unwrap_or("").trim());
}

fn combine_cmd(fin: &str, expected: &str, args: &[String]) {
    let s = sections(&std::fs::read(fin).unwrap());
    assert_eq!(s.len(), 4, "final input: 4 sections");
    // <chunk receipts...> [--bip30 <piece receipts...>]: with --bip30 the tree comes from the pieces (§13.1)
    let cut_at = args.iter().position(|a| a == "--bip30");
    let (rcpts, prcpts) = match cut_at { Some(i) => (&args[..i], &args[i + 1..]), None => (args, &args[..0]) };
    let receipts: Vec<risc0_zkvm::Receipt> = rcpts.iter().map(|p| bincode::deserialize(&std::fs::read(p).unwrap()).unwrap()).collect();
    let preceipts: Vec<risc0_zkvm::Receipt> = prcpts.iter().map(|p| bincode::deserialize(&std::fs::read(p).unwrap()).unwrap()).collect();
    let mut log: Vec<u8> = Vec::new();
    let t0 = std::time::Instant::now();
    let r = {
        let mut b = ExecutorEnv::builder();
        b.stdout(&mut log);
        for r in receipts.iter().chain(preceipts.iter()) { b.add_assumption(r.clone()); }
        write_cut(&mut b);
        b.write(&TS_CHUNK_ID).unwrap();
        b.write(&(receipts.len() as u32)).unwrap();
        for r in &receipts { let w = words(&r.journal.bytes); assert_eq!(w.len(), 26, "a chunk journal is 26 words"); b.write_slice(&w); }
        section_writer(&mut b, &s[0]);
        section_writer(&mut b, &s[1]);
        if cut_at.is_none() {
            b.write(&0u32).unwrap();
            section_writer(&mut b, &s[2]);
            section_writer(&mut b, &s[3]);
        } else {
            b.write(&1u32).unwrap();
            b.write(&TS_BIP30_ID).unwrap();
            b.write(&(preceipts.len() as u32)).unwrap();
            for r in &preceipts { let w = words(&r.journal.bytes); b.write(&(w.len() as u32)).unwrap(); b.write_slice(&w); }
        }
        let env = b.build().unwrap();
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prove_dev(env, TS_COMBINE_ELF)))
    };
    let text = String::from_utf8_lossy(&log).to_string();
    let info = match r {
        Ok(i) => i,
        Err(_) => {
            println!("COMBINE_REFUSED: {}", text.lines().filter(|l| !l.contains("TSX")).last().unwrap_or("(see the panic above)"));
            std::process::exit(1);
        }
    };
    let tsx = text.lines().find(|l| l.contains("TSX ")).unwrap_or("").to_string();
    if !preceipts.is_empty() { println!("  BIP30 from {} pieces (bip30 image {})", preceipts.len(), hex_id(&TS_BIP30_ID)); }
    println!("COMBINE image {} of {} chunks (chunk image {}): user {} total(padded) {} segments {} ({:.1} s)", hex_id(&TS_COMBINE_ID), receipts.len(),
             hex_id(&TS_CHUNK_ID), info.stats.user_cycles, info.stats.total_cycles, info.stats.segments, t0.elapsed().as_secs_f64());
    println!("  {}", tsx.trim());
    let j = &info.receipt.journal.bytes;
    std::fs::write(format!("{fin}.journal"), j).unwrap();
    // the combine's (dev-mode) receipt: the ingest receipt the join guest verifies (§13.3)
    std::fs::write(format!("{fin}{}.rcpt", if cut_at.is_some() { ".split" } else { "" }), bincode::serialize(&info.receipt).unwrap()).unwrap();
    if expected != "-" {
        let e = std::fs::read(expected).unwrap();
        // the combine's journal is the one-shot ingest's layout, then 9 words (chunk image id, N), then with --bip30
        // 9 more (bip30 image id, pieces)
        let tail = if cut_at.is_some() { 72 } else { 36 };
        if j.len() == e.len() + tail && j[..e.len()] == e[..] { println!("COMBINE_MATCH: the combined journal = {expected}, byte for byte (+ chunk image, N{})", if tail == 72 { ", bip30 image, P" } else { "" }); }
        else { println!("COMBINE_MISMATCH against {expected}"); std::process::exit(1); }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(|x| x.as_str()) {
        Some("chunk") => return chunk_cmd(&a[2], &a[3]),
        Some("combine") => return combine_cmd(&a[2], &a[3], &a[4..]),
        Some("bip30") => return bip30_cmd(&a[2], &a[3]),
        Some("join") => return join_cmd(&a[2..]),
        _ => {}
    }
    let (anchor, snap_p, side) = (&a[1], &a[2], &a[3]);
    let hdr = std::fs::read(format!("{side}.headers")).unwrap();
    let snap = std::fs::read(snap_p).unwrap();
    let s = sections(&std::fs::read(format!("{anchor}.ingest")).unwrap());
    assert_eq!(s.len(), 7, "ingest file: 7 sections");
    let h = (hdr.len() / 80 - 1) as u32;
    let (ecmh, ys, trk, wit, perm, rep, pw) = (&s[0], &s[1], &s[2], words(&s[3]), words(&s[4]), &s[5], words(&s[6]));
    let mut log: Vec<u8> = Vec::new();
    let t0 = std::time::Instant::now();
    let session = {
        let mut b = ExecutorEnv::builder();
        b.stdout(&mut log);
        if let Ok(p) = std::env::var("TSI_SEG_PO2") { b.segment_limit_po2(p.parse().unwrap()); }
        b.write(&h).unwrap();
        b.write_slice(&pad(hdr.clone()));
        b.write(&(snap.len() as u32)).unwrap();
        b.write_slice(&pad(snap.clone()));
        b.write(&(ecmh.len() as u32)).unwrap();
        b.write_slice(&pad(ecmh.clone()));
        b.write(&((ys.len() / 32) as u32)).unwrap();
        b.write_slice(ys);
        b.write(&((trk.len() / 36) as u32)).unwrap();
        b.write_slice(trk);
        b.write(&(wit.len() as u32)).unwrap();
        b.write_slice(&wit);
        b.write_slice(&perm);
        b.write(&((rep.len() / 32) as u32)).unwrap();
        b.write_slice(rep);
        // TSI_SMT=bulk: the BIP30 root by tsi::smt_bulk (no proofs); default: one proven roots.rs apply per entry
        let mode: u32 = match std::env::var("TSI_SMT").as_deref() { Ok("bulk") => 1, Ok("raw") => 2, Ok("sys") => 3, _ => 0 };
        let bulk = mode != 0;
        b.write(&mode).unwrap();
        let pw: &[u32] = if bulk { &[] } else { &pw };
        b.write(&(pw.len() as u32)).unwrap();
        b.write_slice(pw);
        let env = b.build().unwrap();
        default_executor().execute(env, GUEST_ELF).expect("ingest guest")
    };
    let secs = t0.elapsed().as_secs_f64();
    let text = String::from_utf8_lossy(&log).to_string();
    let tsi = text.lines().find(|l| l.contains("TSI ")).unwrap_or("").to_string();
    let j: Vec<u32> = words(&session.journal.bytes);
    // check against the anchor ts-anchor built natively
    let cs: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(format!("{anchor}.json")).unwrap()).unwrap();
    let b32 = |w: &[u32]| -> Vec<u64> { w.iter().flat_map(|x| x.to_le_bytes()).map(|x| x as u64).collect() };
    let arr = |k: &str| -> Vec<u64> { cs[k].as_array().unwrap().iter().map(|x| x.as_u64().unwrap()).collect() };
    let roots_json = |k: &str| -> Vec<Option<Vec<u64>>> { cs[k].as_array().unwrap().iter().map(|r| r.as_array().map(|v| v.iter().map(|x| x.as_u64().unwrap()).collect())).collect() };
    let mut i = 0usize;
    let mut take = |n: usize| { let s = j[i..i + n].to_vec(); i += n; s };
    let mut bad: Vec<&str> = Vec::new();
    if take(1)[0] as u64 != cs["height"].as_u64().unwrap() { bad.push("height"); }
    if b32(&take(8)) != arr("tip_hash") { bad.push("tip_hash"); }
    let h3 = take(8);
    let _magic = take(1);
    let nv = take(5);
    let coins = nv[0] as u64 | (nv[1] as u64) << 32;
    if (nv[2] as u64 | (nv[3] as u64) << 32) != cs["utxo_value"].as_u64().unwrap() { bad.push("utxo_value"); }
    if coins != cs["utxo_leaves"].as_u64().unwrap() { bad.push("coins"); }
    if nv[4] as u64 != cs["live_set"].as_u64().unwrap() { bad.push("live_set"); }
    if take(16).iter().map(|x| *x as u64).collect::<Vec<_>>() != arr("live_hash") { bad.push("live_hash"); }
    if b32(&take(8)) != arr("trie_root") { bad.push("trie_root"); }
    if b32(&take(8)) != arr("smt_root") { bad.push("smt_root"); }
    let n_smt = take(1)[0];
    for k in ["utxo_roots", "block_roots"] {
        let n = take(1)[0] as usize;
        let mut got: Vec<Option<Vec<u64>>> = Vec::new();
        for _ in 0..n { if take(1)[0] == 1 { got.push(Some(b32(&take(8)))); } else { got.push(None); } }
        if got != roots_json(k) { bad.push(k); }
    }
    let mut h3b: Vec<u8> = h3.iter().flat_map(|x| x.to_le_bytes()).collect();
    h3b.reverse();
    let h3hex: String = h3b.iter().map(|x| format!("{x:02x}")).collect();
    println!("INGEST image {} H={h} coins={coins} smt_entries={n_smt} hash_serialized_3={h3hex}", hex_id(&GUEST_ID));
    let user = session.cycles();
    let total: u64 = session.segments.iter().map(|s| 1u64 << s.po2).sum();
    println!("  cycles: user {user} total(padded) {total} segments {} (executor, {:.1} s wall)", session.segments.len(), secs);
    println!("  {tsi}");
    println!("  per coin: {:.0} user cycles (all phases / coins)", user as f64 / coins.max(1) as f64);
    // the journal and the claim's image, for ts-anchor join-receipt (a prototype of the join receipt's checks)
    std::fs::write(format!("{anchor}.ingest.journal"), &session.journal.bytes).unwrap();
    std::fs::write(format!("{anchor}.ingest.image"), hex_id(&GUEST_ID)).unwrap();
    if bad.is_empty() { println!("INGEST_MATCH: every committed value = ts-anchor's native anchor"); }
    else { println!("INGEST_MISMATCH {bad:?}"); std::process::exit(1); }
}

fn unhex_id(s: &str) -> [u32; 8] {
    let b: Vec<u8> = (0..64).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex")).collect();
    std::array::from_fn(|k| u32::from_le_bytes(b[4 * k..4 * k + 4].try_into().unwrap()))
}

/// `ts-ingest-host join <dir> <combine receipt>` (TRUSTED_STATE.md §13.3), dev mode: the join guest on the inputs
/// `ts-anchor join-inputs` wrote to <dir> (main.rcpt, a REAL receipt of main@H; main_image; genesis; tip.journal, the
/// anchored tip's journal). The ingest receipt is the combine's (dev mode). The anchored tip enters as a dev-mode claim
/// (tip_image, tip.journal): no anchored receipt was proved. Environment: TS_TIP_IMAGE (default the main image),
/// TS_CHUNK_IMAGE / TS_BIP30_IMAGE (default this build's; TS_BIP30_IMAGE=0 for an inline BIP30 tree), TS_CUT.
fn join_cmd(a: &[String]) {
    let (dir, ing_p) = (&a[0], &a[1]);
    let rd = |f: &str| std::fs::read(format!("{dir}/{f}")).unwrap_or_else(|e| panic!("{dir}/{f}: {e}"));
    let main_r: risc0_zkvm::Receipt = bincode::deserialize(&rd("main.rcpt")).unwrap();
    let main_image = unhex_id(String::from_utf8(rd("main_image")).unwrap().trim());
    let genesis: Vec<u8> = { let g = String::from_utf8(rd("genesis")).unwrap(); (0..64).step_by(2).map(|i| u8::from_str_radix(&g.trim()[i..i + 2], 16).unwrap()).collect() };
    let genesis: [u8; 32] = genesis.try_into().unwrap();
    let jt = rd("tip.journal");
    let env_id = |k: &str, d: [u32; 8]| match std::env::var(k) { Ok(v) if v == "0" => [0u32; 8], Ok(v) => unhex_id(&v), Err(_) => d };
    let tip_image = env_id("TS_TIP_IMAGE", main_image);
    let chunk_image = env_id("TS_CHUNK_IMAGE", TS_CHUNK_ID);
    let ing_r: risc0_zkvm::Receipt = bincode::deserialize(&std::fs::read(ing_p).unwrap()).unwrap();
    let bip30_default = if words(&ing_r.journal.bytes).len() > 0 && std::env::var("TS_BIP30_IMAGE").is_err() {
        // the combine's tail: 9 words, or 18 with a split tree
        let w = words(&ing_r.journal.bytes);
        if w.len() >= 18 && w[w.len() - 9..w.len() - 1] == TS_BIP30_ID[..] { TS_BIP30_ID } else { [0; 8] }
    } else { [0; 8] };
    let bip30_image = env_id("TS_BIP30_IMAGE", bip30_default);
    let ingest_image = env_id("TS_INGEST_IMAGE", TS_COMBINE_ID);
    // the anchored tip: a dev-mode claim (what a dev-mode run of the core image on the anchor yields)
    let tip_claim = risc0_zkvm::ReceiptClaim::ok(risc0_zkvm::sha::Digest::from(tip_image), jt.clone());
    let tip_r = risc0_zkvm::Receipt::new(risc0_zkvm::InnerReceipt::Fake(risc0_zkvm::FakeReceipt::new(tip_claim)), jt.clone());
    let mut log: Vec<u8> = Vec::new();
    let t0 = std::time::Instant::now();
    let r = {
        let mut b = ExecutorEnv::builder();
        b.stdout(&mut log);
        b.add_assumption(main_r.clone());
        b.add_assumption(ing_r.clone());
        b.add_assumption(tip_r);
        write_cut(&mut b);
        for id in [main_image, tip_image, ingest_image, chunk_image, bip30_image] { b.write(&id).unwrap(); }
        b.write(&genesis).unwrap();
        for j in [&main_r.journal.bytes, &ing_r.journal.bytes, &jt] { let w = words(j); b.write(&(w.len() as u32)).unwrap(); b.write_slice(&w); }
        let env = b.build().unwrap();
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prove_dev(env, TS_JOIN_ELF)))
    };
    let text = String::from_utf8_lossy(&log).to_string();
    let info = match r { Ok(i) => i, Err(_) => { println!("JOIN_REFUSED"); std::process::exit(1); } };
    let j = words(&info.receipt.journal.bytes);
    let hx = |w: &[u32]| -> String { w.iter().flat_map(|x| x.to_le_bytes()).map(|x| format!("{x:02x}")).collect() };
    println!("JOINED (join image {}): H={} anchor link {} T={}; user {} total(padded) {} segments {} ({:.1} s)", hex_id(&TS_JOIN_ID), j[48],
             &hx(&j[57..65])[..16], j[65], info.stats.user_cycles, info.stats.total_cycles, info.stats.segments, t0.elapsed().as_secs_f64());
    println!("  {}", text.lines().find(|l| l.contains("TSJ ")).unwrap_or("").trim());
    std::fs::write(format!("{dir}/join.journal"), &info.receipt.journal.bytes).unwrap();
}

fn hex_id(id: &[u32; 8]) -> String { id.iter().flat_map(|x| x.to_le_bytes()).map(|x| format!("{x:02x}")).collect() }
