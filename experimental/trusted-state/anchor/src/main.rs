//! ts-anchor: EXPERIMENTAL Trusted State proof of concept (docs/libzync/TRUSTED_STATE.md). Not a release tool.
//!
//!   ts-anchor build  --network <regtest|mainnet> --snapshot <utxo.dat> --side <prefix> (--expect <H>:<block>:<hash3> | --no-expect) --out <prefix>
//!   ts-anchor join   --anchor <A.json> --main <state.json | receipt.rcpt --image <hex>> [--tip <state.json>]
//!   ts-anchor compare <a.json> <b.json>
//!   ts-anchor facts  --state <tip.json> --trie <tip.trie> --scenarios <scenarios.json> --who <NAME> [--history <spk hex>]... [--undo-from <h>]
//!   ts-anchor synth  --coins <N> --height <H> --base <block display hash> --out <utxo.dat>
//!
//! `build` turns a Core `dumptxoutset` snapshot at H, with the three side inputs of `side_inputs.py`, into an anchor
//! the UNCHANGED core image takes as a base step (`is_base = 1`): `<out>.json` (a ChainState), `<out>.coinlog`,
//! `<out>.trie` and `<out>.smt` (the BIP30 tree's entries, for the host's `HAZYNC_TS_SMT_SEED`). It refuses unless the
//! snapshot's `hash_serialized_3` and base block are the expected (chainparams) values, and it prints the anchor's
//! link digest: the `anchor_id` every proof on the anchored lineage commits.

use std::collections::HashSet;
use std::time::Instant;

use hazync_coinbase_smt::bip30::REPEATABLE;
use hazync_coinbase_smt::Smt;
use libzync::coinlog::W;
use libzync::trie_store::TrieStore;
use libzync::ttree::Key;
use num_bigint::BigUint;
use sha2::{Digest, Sha256};
use swiftsync_verify::{pin, utxo_state, ChainState, CHAIN_STATE_VERSION, KIND_CHAIN};
use zapp_assume as za;

fn sha256(b: &[u8]) -> [u8; 32] { Sha256::digest(b).into() }
fn dsha(b: &[u8]) -> [u8; 32] { sha256(&sha256(b)) }
fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }
fn unhex(s: &str) -> Vec<u8> { (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex")).collect() }
fn display(h: &[u8; 32]) -> String { let mut r = *h; r.reverse(); hex(&r) }
fn from_display(s: &str) -> [u8; 32] { let mut b: [u8; 32] = unhex(s).try_into().expect("32-byte hash"); b.reverse(); b }
fn die(m: impl std::fmt::Display) -> ! { eprintln!("ts-anchor: {m}"); std::process::exit(1) }

/// The coin state every block leaf below the anchor height commits: unknown. A fixed digest that is no 0x04 coin-state
/// preimage's SHA-256 (domain-separated by its text), so a coin path can never open against a block below H.
const UNKNOWN_STATE_TAG: &[u8] = b"hazync trusted-state: the coin state after this block is not known (below the anchor)";

struct Net { name: &'static str, magic: [u8; 4], profile: u32, genesis_anchor: &'static str }
fn net(name: &str) -> Net {
    match name {
        // profiles as the published product images commit them (LIBZYNC 1 | COINLOG 16 | TRIE 32 | UTXOHASH 64, + regtest 8)
        // the pinned genesis anchors: the core verifier's own table (mainnet), the zAPP regtest harness's (regtest)
        "mainnet" => Net { name: "mainnet", magic: [0xf9, 0xbe, 0xb4, 0xd9], profile: 0x71,
                           genesis_anchor: swiftsync_verify::Image::by_network("mainnet").expect("mainnet image").genesis_anchor_hex },
        "regtest" => Net { name: "regtest", magic: [0xfa, 0xbf, 0xb5, 0xda], profile: 0x79,
                           genesis_anchor: "99509fc506d022f06dfd72bb146fc894718ce8047e69e10b201e1a78d4bccf63" },
        n => die(format!("network {n:?}: want regtest or mainnet")),
    }
}

/// Core's GetBlockProof: 2^256 / (target + 1); 0 for a negative, overflowing or zero target.
fn block_work(bits: u32) -> BigUint {
    let exp = bits >> 24;
    let mant = bits & 0x007f_ffff;
    let neg = bits & 0x0080_0000 != 0 && mant != 0;
    let over = mant != 0 && (exp > 34 || (mant > 0xff && exp > 33) || (mant > 0xffff && exp > 32));
    let target = if exp <= 3 { BigUint::from(mant >> (8 * (3 - exp))) } else { BigUint::from(mant) << (8 * (exp - 3)) };
    if neg || over || target == BigUint::from(0u32) { return BigUint::from(0u32); }
    (BigUint::from(1u32) << 256u32) / (target + 1u32)
}

/// The guest's link digest (main.rs `link_digest`): SHA-256d of the state's journal words with anchor_id zeroed.
fn link_digest(s: &ChainState) -> [u8; 32] {
    let mut z = s.clone();
    z.anchor_id = [0; 32];
    let w = risc0_zkvm::serde::to_vec(&z).expect("ChainState serialises");
    dsha(&w.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>())
}

/// A Utreexo-shaped forest of roots (UTREEXO_FORMAT.md §5: parent = SHA-256(0x01 ‖ L ‖ R), the existing root LEFT).
fn stump_add(roots: &mut Vec<Option<[u8; 32]>>, leaf: [u8; 32]) {
    let mut node = leaf;
    let mut h = 0;
    while h < roots.len() && roots[h].is_some() {
        let l = roots[h].take().unwrap();
        node = sha256(&[&[0x01u8][..], &l, &node].concat());
        h += 1;
    }
    if h == roots.len() { roots.push(None); }
    roots[h] = Some(node);
}
fn normalize(mut r: Vec<Option<[u8; 32]>>) -> Vec<Option<[u8; 32]>> { while r.last() == Some(&None) { r.pop(); } r }

fn arg(args: &[String], k: &str) -> Option<String> { args.iter().position(|a| a == k).map(|i| args.get(i + 1).cloned().unwrap_or_else(|| die(format!("{k}: missing value")))) }
fn args_all(args: &[String], k: &str) -> Vec<String> { args.iter().enumerate().filter(|(_, a)| *a == k).map(|(i, _)| args[i + 1].clone()).collect() }
fn need(args: &[String], k: &str) -> String { arg(args, k).unwrap_or_else(|| die(format!("{k} is required"))) }

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(|s| s.as_str()) {
        Some("build") => build(&a[2..]),
        Some("join") => join(&a[2..]),
        Some("compare") => { let (x, y) = (read_state(&a[2]), read_state(&a[3])); compare(&x, &y); }
        Some("facts") => facts(&a[2..]),
        Some("synth") => synth(&a[2..]),
        Some("split-history") => split_history(&a[2..]),
        Some("join-receipt") => join_receipt(&a[2..]),
        _ => die("usage: ts-anchor build|join|compare|facts|synth (see src/main.rs)"),
    }
}

fn read_state(p: &str) -> ChainState {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap_or_else(|e| die(format!("{p}: {e}")))).unwrap_or_else(|e| die(format!("{p}: {e}")))
}

// ------------------------------------------------------------------------------------------------------------------
// build

/// One txid group of the snapshot (Core's key order: one group per txid), for the BIP30 tree.
struct Group { txid: [u8; 32], n: u32, coinbase: bool, tracked: bool }

fn build(a: &[String]) {
    let t_all = Instant::now();
    let nw = net(&need(a, "--network"));
    let snap = need(a, "--snapshot");
    let side = need(a, "--side");
    let out = need(a, "--out");
    let expect = arg(a, "--expect");
    if expect.is_none() && !a.iter().any(|x| x == "--no-expect") { die("--expect <H>:<block>:<hash_serialized_3> (chainparams) or --no-expect (synthetic only)"); }
    let hdr = std::fs::read(format!("{side}.headers")).unwrap_or_else(|e| die(format!("{side}.headers: {e}")));
    let tracked = za::parse_tracked(&std::fs::read_to_string(format!("{side}.tracked")).unwrap_or_default()).unwrap_or_else(|e| die(e));
    let repeatable: Vec<[u8; 32]> = std::fs::read_to_string(format!("{side}.repeatable")).unwrap_or_default().lines()
        .map(|l| l.split('#').next().unwrap().trim()).filter(|l| !l.is_empty()).map(from_display).collect();
    if hdr.is_empty() || hdr.len() % 80 != 0 { die("headers: not a positive multiple of 80 bytes"); }
    let h_tip = (hdr.len() / 80 - 1) as u32;
    let tip_hash = dsha(&hdr[hdr.len() - 80..]);
    // the headers link by hash from genesis to the tip; bound to the snapshot's base block below
    let hc = za::HeaderChain::bind(&hdr, &tip_hash, h_tip).unwrap_or_else(|e| die(e));
    if hc.first_height != 0 { die("headers must start at genesis"); }
    let bits: Vec<u32> = hdr.chunks_exact(80).map(|h| u32::from_le_bytes(h[72..76].try_into().unwrap())).collect();
    if let Some(e) = &expect {
        let p: Vec<&str> = e.split(':').collect();
        if p.len() != 3 { die("--expect <H>:<block display hash>:<hash_serialized_3>"); }
        if p[0].parse::<u32>().ok() != Some(h_tip) { die(format!("--expect height {} but the headers end at {h_tip}", p[0])); }
        if from_display(p[1]) != tip_hash { die(format!("--expect block {} but the headers end at {}", p[1], display(&tip_hash))); }
    }

    // ---- one pass over the snapshot: coin log, ECMH, hash_serialized_3, value, trie inserts, BIP30 groups
    let t0 = Instant::now();
    let mut log = libzync::coinlog_forest::Store::with_nodes(Default::default());
    let mut ecmh = za::Ecmh::default();
    let mut h3 = za::HashSerialized3::default();
    let mut value = 0u64;
    let mut ins: Vec<(Key, Option<W>, u64)> = Vec::new();
    let mut groups: Vec<Group> = Vec::new();
    let mut mtp: Vec<Option<u32>> = vec![None; h_tip as usize + 1];
    let mut tracked_used = 0usize;
    let mut coins = 0u64;
    let ingest = arg(a, "--ingest");
    let (mut leaves, mut p2pk_y, mut trk_ord): (Vec<[u8; 32]>, Vec<u8>, Vec<u8>) = (Vec::new(), Vec::new(), Vec::new());
    let meta = za::read_snapshot_with(std::fs::File::open(&snap).map(std::io::BufReader::new).unwrap_or_else(|e| die(format!("{snap}: {e}"))),
        |m| {
            if m.network_magic != nw.magic { return Err(format!("network magic {} is not {}'s", hex(&m.network_magic), nw.name)); }
            if m.base_blockhash != tip_hash { return Err(format!("base block {} is not the headers' tip {}", display(&m.base_blockhash), display(&tip_hash))); }
            Ok(())
        },
        |c| {
            if c.height == 0 || c.height > h_tip { return Err(format!("coin at height {} outside 1..{h_tip}", c.height)); }
            let m = match mtp[c.height as usize] { Some(m) => m, None => { let m = hc.mtp_before(c.height)?; mtp[c.height as usize] = Some(m); m } };
            let t = tracked.contains(&(c.txid, c.vout));
            tracked_used += t as usize;
            let leaf = za::coin_leaf(c, t, m);
            if ingest.is_some() {
                leaves.push(leaf);
                // Core stores a valid uncompressed P2PK key as special type 4/5 (x only): the ingest guest needs y
                let s = &c.script_pubkey;
                if s.len() == 67 && s[0] == 0x41 && s[1] == 0x04 && s[66] == 0xac { p2pk_y.extend_from_slice(&s[34..66]); }
                if t { trk_ord.extend_from_slice(&c.txid); trk_ord.extend_from_slice(&c.vout.to_le_bytes()); }
            }
            log.push(leaf);
            ecmh.add_leaf(&leaf)?;
            h3.add(c);
            value = value.checked_add(c.value).ok_or("value overflows")?;
            ins.push((libzync::trie_fact::coin_key(&c.script_pubkey, &c.txid, c.vout), None, c.value));
            match groups.last_mut() {
                Some(g) if g.txid == c.txid => { g.n += 1; g.coinbase |= c.coinbase; g.tracked |= t; }
                _ => groups.push(Group { txid: c.txid, n: 1, coinbase: c.coinbase, tracked: t }),
            }
            coins += 1;
            Ok(())
        }).unwrap_or_else(|e| die(e));
    let hash3 = h3.finish();
    let t_pass = t0.elapsed().as_secs_f64();
    if tracked_used != tracked.len() { die(format!("{} of the {} tracked outpoints are not in the snapshot", tracked.len() - tracked_used, tracked.len())); }
    if let Some(e) = &expect {
        let want = e.rsplit(':').next().unwrap();
        if hash3 != want { die(format!("hash_serialized_3 {hash3} is not the expected {want}: NOT the assumed snapshot")); }
    }
    let (live_set, live_hash) = ecmh.committed();
    // the same snapshot through zapp-assume's own check path (the reviewed code), which must agree field for field
    let chk = za::hash_snapshot(std::io::BufReader::new(std::fs::File::open(&snap).unwrap()), &hc, &tracked, h_tip).unwrap_or_else(|e| die(e));
    assert!(chk.coins == coins && chk.value == value && chk.hash_serialized_3 == hash3 && (chk.live_set, chk.live_hash) == (live_set, live_hash),
            "the build pass and zapp-assume's hash_snapshot disagree");

    // ---- NEGATIVE CONTROL ONLY (TRUSTED_STATE.md §11.1): `--forge-extra <value>:<spk hex>:<height>` appends one coin
    // that is NOT in the snapshot to the coin log and the trie, leaving live_hash, utxo_value and hash_serialized_3 (the
    // fields a join compares) as the honest snapshot gives them. A natively built anchor cannot be told from this by
    // the join; only an ingest receipt (one pass committing the log, the trie AND live_hash) can.
    if let Some(f) = arg(a, "--forge-extra") {
        let p: Vec<&str> = f.split(':').collect();
        let (v, spk, h): (u64, Vec<u8>, u32) = (p[0].parse().unwrap(), unhex(p[1]), p[2].parse().unwrap());
        let c = za::SnapCoin { txid: sha256(b"hazync trusted-state FORGED coin"), vout: 0, height: h, coinbase: false, value: v, script_pubkey: spk };
        let m = hc.mtp_before(h).unwrap_or_else(|e| die(e));
        let leaf = za::coin_leaf(&c, false, m);
        log.push(leaf);
        ins.push((libzync::trie_fact::coin_key(&c.script_pubkey, &c.txid, 0), None, v));
        println!("FORGED extra coin {}:0 value={v} height={h} mtp={m} leaf={} at log position {} (NOT in the snapshot, NOT in live_hash)",
                 display(&c.txid), hex(&leaf), log.len() - 1);
    }

    // ---- the counting trie: every live coin inserted, nothing spent (no history before H)
    let t1 = Instant::now();
    let mut trie = TrieStore::new(true);
    let troot = trie.apply(&Vec::new(), &ins).unwrap_or_else(|e| die(format!("trie: {e:?} (two coins with one trie key?)")));
    trie.height = h_tip as u64;
    trie.hash = tip_hash;
    let t_trie = t1.elapsed().as_secs_f64();

    // ---- the BIP30 tree: every live coinbase and every tracked transaction with its live count; REPEATABLE slots
    let t2 = Instant::now();
    let mut smt = Smt::new();
    let mut smt_lines = String::new();
    let mut n_cb = 0usize;
    for g in groups.iter().filter(|g| g.coinbase || g.tracked) {
        smt.insert(g.txid, g.n);
        smt_lines += &format!("{} {}\n", hex(&g.txid), g.n);
        n_cb += g.coinbase as usize;
    }
    let live_txids: HashSet<[u8; 32]> = groups.iter().map(|g| g.txid).collect();
    for r in &repeatable {
        if live_txids.contains(r) { die(format!("repeatable txid {} has live coins in the snapshot", display(r))); }
        smt.insert(*r, REPEATABLE);
        smt_lines += &format!("{} {}\n", hex(r), REPEATABLE);
    }
    let smt_root = smt.root();
    let t_smt = t2.elapsed().as_secs_f64();

    // ---- the block forest: blocks 0..H-1 with their real hashes and an UNKNOWN coin state; H with the anchored one
    let roots = normalize(log.roots());
    let st_h = utxo_state(log.len(), value, &roots);
    let unknown = sha256(UNKNOWN_STATE_TAG);
    let mut bf: Vec<Option<[u8; 32]>> = Vec::new();
    for (i, h) in hc.hashes.iter().enumerate() {
        let st = if i as u32 == h_tip { st_h } else { unknown };
        stump_add(&mut bf, sha256(&pin::block_leaf_preimage(h, i as u32, &st)));
    }
    let mut work = BigUint::from(0u32);
    for b in &bits { work += block_work(*b); }
    let mut cw = work.to_bytes_le();
    if cw.len() > 32 { die("chain work overflows 256 bits"); }
    cw.resize(32, 0);
    let times = &hc.times;
    let hu = h_tip as usize;
    let cs = ChainState {
        kind: KIND_CHAIN, version: CHAIN_STATE_VERSION, tip_hash, utxo_roots: roots.clone(), utxo_leaves: log.len(), utxo_value: value,
        cum_work: cw.try_into().unwrap(), height: h_tip, prev_nbits: bits[hu], prev_time: times[hu],
        epoch_start: times[hu - hu % 2016], recent_times: times[hu.saturating_sub(10)..=hu].to_vec(),
        anchor_id: [0; 32], self_id: [0; 8], live_set, live_hash, trie_root: libzync::coinlog::h_of(&troot), smt_root,
        max_time: *times.iter().max().unwrap(), profile: nw.profile, block_roots: normalize(bf), block_leaves: h_tip as u64 + 1,
        lineage: [0; 32],
    };
    let anchor_id = link_digest(&cs);
    if let Some(p) = &ingest { write_ingest(p, &leaves, &p2pk_y, &trk_ord, &ins, &groups, &repeatable); }

    // ---- write the anchor
    std::fs::write(format!("{out}.json"), serde_json::to_string(&cs).unwrap()).unwrap();
    let mut b = Vec::with_capacity(8 + 32 * log.len() as usize);
    b.extend_from_slice(&log.len().to_le_bytes());
    for i in 0..log.len() { b.extend_from_slice(&log.leaf(i)); }
    std::fs::write(format!("{out}.coinlog"), &b).unwrap();
    std::fs::write(format!("{out}.trie"), trie.to_bytes()).unwrap();
    std::fs::write(format!("{out}.smt"), &smt_lines).unwrap();
    let (lx, _) = za::live_hash_hex(&live_hash);
    println!("ANCHOR {} H={h_tip} block={} hash_serialized_3={hash3}{}", nw.name, display(&tip_hash),
             if expect.is_some() { " (= the expected chainparams value)" } else { " (NOT checked: --no-expect)" });
    println!("  coins={coins} value={value} live_hash.x={lx} tracked={tracked_used} repeatable={} smt_entries={} (coinbase groups {n_cb}) groups={}",
             repeatable.len(), smt_lines.lines().count(), groups.len());
    println!("  coinlog leaves={} trie_root={} smt_root={} block_leaves={} cum_work=0x{}", log.len(), hex(&cs.trie_root), hex(&smt_root),
             cs.block_leaves, work.to_str_radix(16));
    println!("  native time: snapshot pass {t_pass:.3} s (coin log, ECMH 1 thread, hash3, trie keys), trie {t_trie:.3} s, BIP30 tree {t_smt:.3} s, total {:.3} s",
             t_all.elapsed().as_secs_f64());
    println!("  files: {out}.json .coinlog ({} B) .trie ({} B) .smt", b.len(), trie.to_bytes().len());
    println!("ANCHOR_ID {}", hex(&anchor_id));
    let _ = meta;
}

/// The inputs of the experimental ingest guest (experimental/trusted-state/ingest), every hint it needs, as sections of
/// (u32 LE byte length, bytes): ECMH hints, P2PK y's, tracked outpoints in snapshot order, the trie witness (words), the
/// trie update order (words), the repeatable txids, the BIP30 tree's insert proofs (words: bitmap 8, n, siblings 8n).
fn write_ingest(p: &str, leaves: &[[u8; 32]], p2pk_y: &[u8], trk: &[u8], ins: &[(Key, Option<W>, u64)], groups: &[Group], rep: &[[u8; 32]]) {
    let t0 = Instant::now();
    let ecmh: Vec<u8> = leaves.iter().flat_map(|l| libzync::host::ecmh_hint(l)).collect();
    let t_ecmh = t0.elapsed().as_secs_f64();
    let wit = TrieStore::new(true).witness(&Vec::new(), &ins.to_vec());
    let ks: Vec<(W, W)> = ins.iter().map(|x| libzync::ttree::split(&x.0)).collect();
    let mut perm: Vec<u32> = (0..ks.len() as u32).collect();
    perm.sort_by(|&x, &y| ks[x as usize].cmp(&ks[y as usize]));
    let mut smt = Smt::new();
    let mut pw: Vec<u32> = Vec::new();
    let entries: Vec<([u8; 32], u32)> = groups.iter().filter(|g| g.coinbase || g.tracked).map(|g| (g.txid, g.n))
        .chain(rep.iter().map(|r| (*r, REPEATABLE))).collect();
    for (k, v) in &entries {
        let pr = smt.prove(k);
        pw.extend(pr.bitmap.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())));
        pw.push(pr.siblings.len() as u32);
        for s in &pr.siblings { pw.extend(s.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap()))); }
        smt.insert(*k, *v);
    }
    let words = |w: &[u32]| -> Vec<u8> { w.iter().flat_map(|x| x.to_le_bytes()).collect() };
    let mut o = Vec::new();
    for sec in [ecmh, p2pk_y.to_vec(), trk.to_vec(), words(&wit), words(&perm), rep.concat(), words(&pw)] {
        o.extend_from_slice(&(sec.len() as u32).to_le_bytes());
        o.extend_from_slice(&sec);
    }
    std::fs::write(p, &o).unwrap();
    println!("  ingest inputs -> {p} ({} B; ECMH hints {t_ecmh:.3} s native)", o.len());
}

// ------------------------------------------------------------------------------------------------------------------
// join and compare

type Field = (&'static str, bool, String, String);

fn fields(a: &ChainState, b: &ChainState) -> Vec<Field> {
    let s = |x: &dyn std::fmt::Debug| { let t = format!("{x:?}"); if t.len() > 40 { format!("{}…", &t[..40]) } else { t } };
    let mut v: Vec<Field> = Vec::new();
    macro_rules! f { ($n:ident) => { v.push((stringify!($n), a.$n == b.$n, s(&a.$n), s(&b.$n))); } }
    f!(kind); f!(version); f!(tip_hash); f!(height); f!(utxo_value); f!(cum_work); f!(prev_nbits); f!(prev_time);
    f!(epoch_start); f!(recent_times); f!(max_time); f!(live_set); f!(live_hash); f!(profile); f!(block_leaves);
    f!(utxo_roots); f!(utxo_leaves); f!(trie_root); f!(smt_root); f!(block_roots); f!(anchor_id); f!(self_id); f!(lineage);
    v
}

/// The fields that must be equal for the anchored state to be the main lineage's at H, as far as every later block's
/// verdict depends on it (docs/libzync/TRUSTED_STATE.md §4). The rest are each lineage's own by construction.
const JOIN_FIELDS: &[&str] = &["kind", "version", "tip_hash", "height", "utxo_value", "cum_work", "prev_nbits", "prev_time",
    "epoch_start", "recent_times", "max_time", "live_set", "live_hash", "profile", "block_leaves"];

fn compare(a: &ChainState, b: &ChainState) {
    for (n, eq, x, y) in fields(a, b) {
        println!("  {:<13} {}  {}{}", n, if eq { "equal  " } else { "DIFFERS" }, x, if eq { String::new() } else { format!("  vs  {y}") });
    }
}

fn join(a: &[String]) {
    let anchor = read_state(&need(a, "--anchor"));
    let main_p = need(a, "--main");
    let main = if main_p.ends_with(".rcpt") {
        let img = need(a, "--image");
        let bytes = std::fs::read(&main_p).unwrap_or_else(|e| die(format!("{main_p}: {e}")));
        let r = swiftsync_verify::decode_receipt(&bytes).unwrap_or_else(|e| die(e));
        let d: [u8; 32] = unhex(&img).try_into().unwrap_or_else(|_| die("--image: 64 hex digits"));
        r.verify(risc0_zkvm::sha::Digest::from(d)).unwrap_or_else(|e| die(format!("{main_p}: the receipt does not verify against image {img}: {e}")));
        println!("main: {main_p}: receipt VERIFIED against image {img} (a real proof; risc0 3.1.0 verifier, dev mode disabled)");
        swiftsync_verify::decode_journal::<ChainState>(&r.journal).unwrap_or_else(|e| die(e))
    } else { read_state(&main_p) };
    let nw = if main.profile & 8 != 0 { net("regtest") } else { net("mainnet") };
    let mut ok = true;
    let ga = hex(&main.anchor_id) == nw.genesis_anchor;
    println!("main lineage: height {} anchor_id {} {}", main.height, hex(&main.anchor_id),
             if ga { "= the network's genesis anchor" } else { "!= the genesis anchor: not a from-genesis proof" });
    ok &= ga;
    println!("anchored state: height {} link digest (its lineage's anchor_id) {}", anchor.height, hex(&link_digest(&anchor)));
    for (n, eq, x, y) in fields(&anchor, &main) {
        let req = JOIN_FIELDS.contains(&n);
        if req { ok &= eq; }
        println!("  {:<13} {:<8} {}  {}{}", n, if req { "JOIN" } else { "lineage" }, if eq { "equal  " } else if req { "DIFFERS" } else { "differs" },
                 x, if eq { String::new() } else { format!("  vs  {y}") });
    }
    if let Some(t) = arg(a, "--tip") {
        let tip = read_state(&t);
        let l = tip.anchor_id == link_digest(&anchor);
        println!("anchored tip {t}: height {} anchor_id {} {}", tip.height, hex(&tip.anchor_id), if l { "= this anchor's link digest" } else { "!= this anchor's link digest" });
        ok &= l;
    }
    println!("{}", if ok { "JOIN_OK: the anchored state is the main lineage's state at H in every field a later block's verdict reads; the assumption is discharged" }
                   else { "JOIN_FAIL" });
    if !ok { std::process::exit(1); }
}

// ------------------------------------------------------------------------------------------------------------------
// zAPP facts (the balance statement, trie facts) on a state, checked by the zAPP libraries themselves

fn facts(a: &[String]) {
    use bitcoin::secp256k1::Secp256k1;
    let cs = read_state(&need(a, "--state"));
    let tp = need(a, "--trie");
    let trie = TrieStore::from_bytes(&std::fs::read(&tp).unwrap_or_else(|e| die(format!("{tp}: {e}")))).unwrap_or_else(|e| die(format!("{tp}: {e:?}")));
    let undo_from: u32 = arg(a, "--undo-from").map(|s| s.parse().unwrap()).unwrap_or(1);
    let sc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(need(a, "--scenarios")).unwrap()).unwrap();
    let who = need(a, "--who");
    let ks = sc["keys"][&who].as_object().unwrap_or_else(|| die(format!("no keys for {who}")));
    let mut keys = Vec::new();
    let mut scripts = Vec::new();
    let secp = Secp256k1::new();
    for (t, v) in ks {
        let sk = bitcoin::secp256k1::SecretKey::from_slice(&unhex(v["secret"].as_str().unwrap())).unwrap();
        let mut pk = bitcoin::PrivateKey::new(sk, bitcoin::Network::Regtest);
        pk.compressed = t != "p2pk_u";
        let _ = pk.public_key(&secp);
        keys.push(pk);
        scripts.push(unhex(v["scripts"][t]["spk"].as_str().unwrap()));
    }
    // the balance statement zAPP (zapps/statement), its own draft, sign and check, against this state
    let mut st = zapp_statement::draft(&cs, &trie, &scripts, "trusted-state-poc", "7e57").unwrap_or_else(|e| die(format!("draft: {e}")));
    zapp_statement::sign(&mut st, &keys).unwrap_or_else(|e| die(format!("sign: {e}")));
    let ch = zapp_statement::Challenge { purpose: Some("trusted-state-poc"), nonce: Some("7e57") };
    match zapp_statement::check_statement(&cs, undo_from, &st, &ch) {
        Ok(c) => println!("STATEMENT_OK {}", c.line),
        Err(e) => println!("STATEMENT_FAIL {e}"),
    }
    for e in &st.entries { println!("  balance {} {}", e.fact.script_pubkey, e.fact.expect.as_deref().unwrap_or("?")); }
    for s in args_all(a, "--history") {
        let spk = unhex(&s);
        for kind in ["balance", "history"] {
            let f = swiftsync_verify::make_trie_fact(&trie, kind, &spk, None, None).unwrap_or_else(|e| die(e));
            let r = swiftsync_verify::check_trie_fact(&cs, undo_from, &f);
            println!("  {kind:<8} {s}: claims {} -> {}", f.expect.as_deref().unwrap_or("?"), match r { Ok(l) => format!("OK {l}"), Err(e) => format!("FAIL {e}") });
        }
    }
}

// ------------------------------------------------------------------------------------------------------------------
// join-receipt (TRUSTED_STATE.md §11.4): the checks a join receipt's guest would make, natively. A PROTOTYPE of the
// logic, not a proof: in the guest each input below would be an env::verify'd receipt journal.

fn receipt_state(p: &str, img: &str) -> ChainState {
    let bytes = std::fs::read(p).unwrap_or_else(|e| die(format!("{p}: {e}")));
    let r = swiftsync_verify::decode_receipt(&bytes).unwrap_or_else(|e| die(e));
    let d: [u8; 32] = unhex(img).try_into().unwrap_or_else(|_| die("image: 64 hex digits"));
    r.verify(risc0_zkvm::sha::Digest::from(d)).unwrap_or_else(|e| die(format!("{p}: does not verify against {img}: {e}")));
    swiftsync_verify::decode_journal::<ChainState>(&r.journal).unwrap_or_else(|e| die(e))
}

fn join_receipt(a: &[String]) {
    // 1. the main lineage at H: a receipt of a published core image, genesis-anchored
    let main = receipt_state(&need(a, "--main"), &need(a, "--main-image"));
    let nw = if main.profile & 8 != 0 { net("regtest") } else { net("mainnet") };
    let mut fails: Vec<String> = Vec::new();
    if hex(&main.anchor_id) != nw.genesis_anchor { fails.push("main@H is not genesis-anchored".into()); }
    // 2. the ingest journal (in the guest: env::verify(INGEST_ID, journal)); its layout is the ingest guest's commit
    let jb = std::fs::read(need(a, "--ingest-journal")).unwrap();
    let j: Vec<u32> = jb.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    let mut i = 0usize;
    let mut take = |n: usize| { let s = j[i..i + n].to_vec(); i += n; s };
    let b32 = |w: Vec<u32>| -> [u8; 32] { w.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>().try_into().unwrap() };
    let h = take(1)[0];
    let tip = b32(take(8));
    let _hash3 = take(8);
    let _magic = take(1);
    let nv = take(5);
    let (coins, value, live_set) = (nv[0] as u64 | (nv[1] as u64) << 32, nv[2] as u64 | (nv[3] as u64) << 32, nv[4]);
    let live_hash: [u32; 16] = take(16).try_into().unwrap();
    let trie_root = b32(take(8));
    let smt_root = b32(take(8));
    let _n_smt = take(1);
    let roots = |t: &mut dyn FnMut(usize) -> Vec<u32>| -> Vec<Option<[u8; 32]>> {
        let n = t(1)[0] as usize;
        (0..n).map(|_| if t(1)[0] == 1 { Some(b32(t(8))) } else { None }).collect()
    };
    let utxo_roots = roots(&mut take);
    let block_roots = roots(&mut take);
    // 3. the anchor A, rebuilt: every coin-derived field from the ingest journal, every history-free header-derived
    //    field FROM main@H (so the join's time-context comparison is implied by the link digest matching)
    let anchor = ChainState {
        kind: KIND_CHAIN, version: CHAIN_STATE_VERSION, tip_hash: tip, utxo_roots, utxo_leaves: coins, utxo_value: value,
        cum_work: main.cum_work, height: h, prev_nbits: main.prev_nbits, prev_time: main.prev_time, epoch_start: main.epoch_start,
        recent_times: main.recent_times.clone(), anchor_id: [0; 32], self_id: [0; 8], live_set, live_hash, trie_root, smt_root,
        max_time: main.max_time, profile: main.profile, block_roots, block_leaves: h as u64 + 1, lineage: [0; 32],
    };
    let link = link_digest(&anchor);
    // 4. the coin-derived join fields: the ingest's snapshot coins ARE main's live coins at H
    if (h, tip, value, live_set, live_hash) != (main.height, main.tip_hash, main.utxo_value, main.live_set, main.live_hash) {
        fails.push("ingest (H, tip, value, live set) != main@H's".into());
    }
    // 5. the anchored lineage's tip (in the guest: env::verify(core image, J_T)) rests on exactly this anchor
    let tipst = read_state(&need(a, "--anchored-tip"));
    if tipst.anchor_id != link { fails.push(format!("anchored tip's anchor_id {} != the rebuilt anchor's link digest {}", hex(&tipst.anchor_id), hex(&link))); }
    if tipst.profile != main.profile { fails.push("profiles differ".into()); }
    let dg = |s: &ChainState| { let w = risc0_zkvm::serde::to_vec(s).unwrap(); sha256(&w.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>()) };
    println!("rebuilt anchor at H={h}: link digest {}", hex(&link));
    if fails.is_empty() {
        // the joined statement: what the receipt would commit, and what split answers check facts against
        println!("JOINED: blocks 0..{} valid from genesis (main image, anchor {}), blocks {}..{} valid on the anchor (core image, \
                  anchor_id {}), joined at {h} {}", main.height, &nw.genesis_anchor[..8], h + 1, tipst.height, &hex(&link)[..8], display(&tip));
        println!("  commits: J_H digest {}  J_T digest {}  T={} tip {}", hex(&dg(&main)), hex(&dg(&tipst)), tipst.height, display(&tipst.tip_hash));
    } else {
        for f in &fails { println!("  FAIL {f}"); }
        println!("JOIN_RECEIPT_REFUSED");
        std::process::exit(1);
    }
}

// ------------------------------------------------------------------------------------------------------------------
// split-history (TRUSTED_STATE.md §11.2): a script's complete history at T from TWO proofs joined at H

/// A trie snapshot and its state, the snapshot checked against the state's trie root.
fn load_trie(state: &str) -> (ChainState, TrieStore) {
    let cs = read_state(&format!("{state}.json"));
    let tp = format!("{state}.trie");
    let t = TrieStore::from_bytes(&std::fs::read(&tp).unwrap_or_else(|e| die(format!("{tp}: {e}")))).unwrap_or_else(|e| die(format!("{tp}: {e:?}")));
    if libzync::coinlog::h_of(&t.root()) != cs.trie_root { die(format!("{tp}: not the trie of {state}.json")); }
    (cs, t)
}

/// The history fact of `spk`, checked against the state's trie root as a verifier would (the proof from the store, the
/// answer read back out of the proof only): None = never paid.
fn hist(cs: &ChainState, t: &TrieStore, spk: &[u8]) -> Option<Vec<libzync::trie_fact::HistCoin>> {
    use libzync::trie_fact as tf;
    let sh = tf::script_hash(spk);
    let p = t.prove_history(&sh);
    tf::history(&p, &libzync::coinlog::w_of(&cs.trie_root), &sh).unwrap_or_else(|e| die(format!("history proof: {e:?}"))).map(|(_, v)| v)
}

fn split_history(a: &[String]) {
    use libzync::trie_fact::CoinState::*;
    let (mh, tmh) = load_trie(&need(a, "--main-h"));     // main lineage at H (from genesis)
    let (at, tat) = load_trie(&need(a, "--anchored-t"));  // anchored lineage at T
    let reference = arg(a, "--main-t").map(|p| load_trie(&p)); // main lineage at T, only to compare against
    if mh.height > at.height { die("--main-h must be at or below --anchored-t"); }
    let sc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(need(a, "--scenarios")).unwrap()).unwrap();
    // every scriptPubKey the harness names (each key's scripts, ANYONE's OP_1), wherever it sits in scenarios.json
    fn collect(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(m) => for (k, x) in m { if k == "spk" { if let Some(s) = x.as_str() { out.push(s.into()) } } else { collect(x, out) } },
            serde_json::Value::Array(a) => for x in a { collect(x, out) },
            _ => {}
        }
    }
    let mut spks: Vec<String> = Vec::new();
    collect(&sc["keys"], &mut spks);
    spks.sort(); spks.dedup();
    let (mut ok, mut bad) = (0, 0);
    for s in &spks {
        let spk = unhex(s);
        let hm = hist(&mh, &tmh, &spk).unwrap_or_default();
        let ha = hist(&at, &tat, &spk).unwrap_or_default();
        // the matching rule: every coin UNSPENT at H on the main lineage is in the anchored history (the snapshot's
        // entries) with the same value, and spent, if at all, above H; the rest of the anchored history was created after H
        let mut err = None;
        for c in hm.iter().filter(|c| matches!(c.state, Unspent { .. })) {
            let v = if let Unspent { value } = c.state { value } else { 0 };
            match ha.iter().find(|x| x.okey == c.okey).map(|x| x.state) {
                Some(Unspent { value }) if value == v => {}
                Some(Spent { value, by }) if value == v && by.height > mh.height => {}
                x => err = Some(format!("coin {} unspent at H on main is {x:?} on the anchored lineage", hex(&c.okey))),
            }
        }
        // merged: the main lineage's coins SPENT by H, then the anchored lineage's coins; a key in both (a coin re-created
        // over its own spent record, BIP30) keeps the later coin, as the main trie itself does (ZYNCAPPS.md §2)
        let mut merged: Vec<libzync::trie_fact::HistCoin> = hm.iter().filter(|c| matches!(c.state, Spent { .. }))
            .filter(|c| !ha.iter().any(|x| x.okey == c.okey)).cloned().chain(ha.iter().cloned()).collect();
        merged.sort_by(|x, y| x.okey.cmp(&y.okey));
        let spent = merged.iter().filter(|c| matches!(c.state, Spent { .. })).count();
        let verdict = match (&err, &reference) {
            (Some(e), _) => { bad += 1; format!("MATCH FAIL {e}") }
            (None, Some((rs, rt))) => {
                let mut r = hist(rs, rt, &spk).unwrap_or_default();
                r.sort_by(|x, y| x.okey.cmp(&y.okey));
                if r == merged { ok += 1; "= main lineage at T, coin for coin".into() } else { bad += 1; format!("!= main lineage at T ({} coins)", r.len()) }
            }
            (None, None) => { ok += 1; "matched".into() }
        };
        println!("  {s:<70} main@H {:>3} ({} spent) + anchored@T {:>3} -> {:>3} coins ever, {spent} spent: {verdict}",
                 hm.len(), hm.iter().filter(|c| matches!(c.state, Spent { .. })).count(), ha.len(), merged.len());
    }
    println!("{} scripts: {ok} merged histories {}, {bad} not", spks.len(), if reference.is_some() { "equal to the main lineage's at T" } else { "matched" });
    if bad > 0 { std::process::exit(1); }
}

// ------------------------------------------------------------------------------------------------------------------
// synth: a synthetic Core-format snapshot for the cost curve (NOT chain data: random txids, P2WPKH-sized scripts)

fn synth(a: &[String]) {
    let n: u64 = need(a, "--coins").parse().unwrap();
    let h: u32 = need(a, "--height").parse().unwrap();
    let base = from_display(&need(a, "--base"));
    let magic = net(&arg(a, "--network").unwrap_or("regtest".into())).magic;
    let out = need(a, "--out");
    // --coinbase-every K: every K-th txid group's coins are coinbase outputs (so BIP30 entries; for the SMT cost curve)
    let cb_every: u64 = arg(a, "--coinbase-every").map(|v| v.parse().unwrap()).unwrap_or(0);
    // 2 coins per txid; txids ascending (Core's key order); heights spread 1..=h; scripts P2WPKH, which Core's
    // ScriptCompression has no special type for: stored raw, as VARINT(size + 6) then the 22 bytes
    let mut txids: Vec<[u8; 32]> = (0..n.div_ceil(2)).map(|i| sha256(&[b"ts-synth".as_slice(), &i.to_le_bytes()].concat())).collect();
    txids.sort();
    let mut f = Vec::new();
    f.extend_from_slice(b"utxo\xff");
    f.extend_from_slice(&2u16.to_le_bytes());
    f.extend_from_slice(&magic);
    f.extend_from_slice(&base);
    f.extend_from_slice(&n.to_le_bytes());
    let varint = |v: u64, o: &mut Vec<u8>| o.extend_from_slice(&za::varint_bytes(v));
    let mut left = n;
    for (i, t) in txids.iter().enumerate() {
        let k = left.min(2);
        f.extend_from_slice(t);
        f.push(k as u8);
        for v in 0..k {
            f.push(v as u8);
            let height = 1 + (i as u32 * 7919) % h;
            varint((height as u64) << 1 | (cb_every > 0 && i as u64 % cb_every == 0) as u64, &mut f);
            varint(compress_amount(10_000 + i as u64), &mut f);
            let mut spk = vec![0x00, 0x14];
            spk.extend_from_slice(&t[..20]);
            varint(spk.len() as u64 + 6, &mut f);
            f.extend_from_slice(&spk);
        }
        left -= k;
    }
    std::fs::write(&out, &f).unwrap();
    println!("SYNTH {n} coins in {} txid groups, heights 1..{h}, {} bytes -> {out}", txids.len(), f.len());
}

/// Core's CompressAmount.
fn compress_amount(mut n: u64) -> u64 {
    if n == 0 { return 0; }
    let mut e = 0;
    while n % 10 == 0 && e < 9 { n /= 10; e += 1; }
    if e < 9 { let d = n % 10; n /= 10; 1 + (n * 9 + d - 1) * 10 + e } else { 1 + (n - 1) * 10 + 9 }
}
