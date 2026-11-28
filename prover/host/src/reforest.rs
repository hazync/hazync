//! `libzync-reforest`: carry a prep run made before the undo-data commitment (docs/libzync/UNDO.md) over to it, without
//! re-running prep. Only the block forest changed: every leaf from block 1 on now commits its block's undo hash, so
//! `block_roots` differs in every state; the coin log, trie, BIP30 tree and every other ChainState field do not.
//!
//!   libzync-reforest <out dir> <S> --bitcoin-cli "<cmd>" [--all] [--jobs N] [--bundles <leaf bundle dir>]
//!                    [--check-bundles <leaf bundle dir>]
//!
//! 1. Undo hashes for blocks 1..=S from the node (`getblock <hash> 3`: each input's prevout height, coinbase flag,
//!    value and script; libzync::undo), into `<out dir>/reforest/undo.tsv` (`height block-hash undo-hash inputs check`,
//!    height order, appended as it goes; `check` = the first 16 hex of SHA-256 of the line's other fields). A rerun
//!    resumes after its last line: a torn last line is fetched again, any other bad line stops the command (delete
//!    `reforest/` and run again), and a sample of the cached heights is fetched again and must agree.
//!    A prevout of height 0 or above its block (an undo record from before Core 0.15, say) is refused.
//! 1b. Cross-check: every leaf bundle at heights 1..=S in the `--bundles` / `--check-bundles` directories carries prep's
//!    own copy of its block's spent coins (block_json); their undo hash must be the node's (prep::undo_of_block_json).
//! 2. The forest walk h = 1..=S over `state_<h>.json`: each state must be block h of the node's chain (its tip hash is
//!    the one the undo hash was computed for) and its `block_roots` must be the pre-commitment forest of the states before
//!    it (or, on a rerun, the new one): anything else is not one contiguous prep run, and the command stops. The new
//!    forest is built with the leaf prep now uses (prep::block_leaf_host), and the state is stamped as prep stamps it.
//!    state_S.json is rewritten (the point `libzync-prep` continues from with HAZYNC_PREP_CONTINUE=1); with `--all` every
//!    state_<h>.json, so leaf bundles cut later from earlier states are right too. Each write is a temporary file renamed
//!    over the old one (a crash leaves either format, and a rerun accepts both); state_S.json is synced.
//!    With `--bundles`, every leaf_<h>.bin for h = 2..=S+1 gets the re-forested S(h-1) as its `prev` (its witnesses and
//!    block do not depend on the forest); bundles above S+1 are named, for removal (the continuing prep cuts them again).
//!    The runbook is docs/libzync/UNDO.md §8.
//!
//! It never opens the coin log, the trie or the BIP30 snapshots (HAZYNC_COINLOG_DIR / HAZYNC_TRIE_DIR must be unset),
//! and it holds the LOCK of the out dir and of every bundle directory it is given (prep holds them while it runs), so
//! it refuses to run while prep does. Run it at the switch, with prep stopped at a save point S (state_S.coinlog
//! present). Every rewritten file is written to a temporary file, synced and renamed. A `history.bin` in the out dir
//! has its records 1..=S given their undo hashes. At the end it writes `reforest/DONE_<S>` and `state_<S>.forest`, the
//! marker without which the new prep refuses to continue from state_S.json (prep::FOREST_MARK).

use crate::ChainState;
use sha2::{Digest, Sha256};
use std::io::Write as _;

fn node(cmd: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut parts = cmd.split_whitespace();
    let prog = parts.next().ok_or("empty --bitcoin-cli")?;
    let o = std::process::Command::new(prog).args(parts).args(args).output().map_err(|e| format!("{prog}: {e}"))?;
    if !o.status.success() { return Err(format!("{cmd} {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim())); }
    Ok(o.stdout)
}

/// Satoshis from the node's BTC amount (a JSON number with at most 8 decimals, at most 21e6). As an f64 it carries under
/// 0.19 sat of representation error (ulp(2^25) / 2 BTC) and the product under 0.13 sat more, so the rounded product is
/// exactly the amount. The check: those satoshis, back in BTC, are the node's value to within 2 ulp; a value with more
/// than 8 decimals is refused. (It was `|f * 1e8 - s| < 1e-3`, which refused real amounts: 175,815.27 BTC carries
/// 0.003 sat of representation error. Found by the 11-14 pre-computation, block ~123,000.)
fn sats_of(f: f64) -> Result<u64, String> {
    if !(0.0..=21e6).contains(&f) { return Err(format!("prevout value {f} is not a bitcoin amount")); }
    let s = (f * 1e8).round();
    if (s / 1e8 - f).abs() > 2.0 * f64::EPSILON * f.max(1e-8) { return Err(format!("prevout value {f} is not a whole number of satoshis")); }
    Ok(s as u64)
}

fn sats(v: &serde_json::Value) -> u64 {
    sats_of(v.as_f64().expect("getblock: prevout value is not a number")).unwrap_or_else(|e| panic!("getblock: {e}"))
}

#[cfg(test)]
mod sats_tests {
    use super::sats_of;
    #[test]
    fn amounts_convert_exactly_and_others_are_refused() {
        for (txt, want) in [("175815.27", 17_581_527_000_000u64), ("50", 5_000_000_000), ("0.00000001", 1), ("0", 0),
                            ("20999999.9769", 2_099_999_997_690_000), ("12345678.12345678", 1_234_567_812_345_678),
                            ("0.1", 10_000_000), ("21000000", 2_100_000_000_000_000), ("1234.56789012", 123_456_789_012)] {
            let f: f64 = txt.parse().unwrap();
            assert_eq!(sats_of(f), Ok(want), "{txt}");
            // the JSON path the node's output takes
            let v: serde_json::Value = serde_json::from_str(txt).unwrap();
            assert_eq!(sats_of(v.as_f64().unwrap()), Ok(want), "{txt} via serde_json");
        }
        for txt in ["0.000000001", "1.123456789", "-1", "21000000.00000001", "1e20"] {
            assert!(sats_of(txt.parse().unwrap()).is_err(), "{txt}");
        }
        // every amount of the form k / 100,000,000 BTC near the magnitudes the chain has, exactly
        for base in [1u64, 123_456_789, 17_581_527_000_000, 2_099_999_997_690_000] {
            for k in base.saturating_sub(500)..base + 500 {
                let txt = format!("{}.{:08}", k / 100_000_000, k % 100_000_000);
                let v: serde_json::Value = serde_json::from_str(&txt).unwrap();
                assert_eq!(sats_of(v.as_f64().unwrap()), Ok(k), "{txt}");
            }
        }
    }
}

/// Block `h` from the node: (block hash, display order; undo hash; inputs).
pub(crate) fn node_undo(cmd: &str, h: u32) -> Result<(String, [u8; 32], u64), String> {
    let hash = String::from_utf8_lossy(&node(cmd, &["getblockhash", &h.to_string()])?).trim().to_lowercase();
    let b: serde_json::Value = serde_json::from_slice(&node(cmd, &["getblock", &hash, "3"])?).map_err(|e| format!("getblock {h}: {e}"))?;
    if b["height"].as_u64() != Some(h as u64) { return Err(format!("getblock {h}: the node returned height {}", b["height"])); }
    let txs = b["tx"].as_array().ok_or(format!("getblock {h}: no tx list (verbosity 3 needs Core >= 25)"))?;
    let mut u = libzync::undo::UndoHasher::new();
    for t in txs.iter().skip(1) {   // the coinbase spends nothing
        for v in t["vin"].as_array().ok_or(format!("getblock {h}: a transaction without vin"))? {
            let p = &v["prevout"];
            if !p.is_object() { return Err(format!("getblock {h}: an input without prevout (a pruned node, or Core < 25)")); }
            let ch = p["height"].as_u64().ok_or(format!("getblock {h}: prevout height"))?;
            // A coin is created at or before the block that spends it, never at 0 (the genesis coinbase is unspendable).
            // Undo data written before Core 0.15 can lack the height, and then the node reports one that is not the coin's
            // (review 11-14, round 2): refused, never hashed.
            if ch == 0 || ch > h as u64 {
                return Err(format!("getblock {h}: a prevout of height {ch} (not 1..={h}): the node's undo data has no real height for it"));
            }
            let ch = ch as u32;
            let cb = p["generated"].as_bool().ok_or(format!("getblock {h}: prevout generated"))?;
            let spk = crate::hx(p["scriptPubKey"]["hex"].as_str().ok_or(format!("getblock {h}: prevout scriptPubKey"))?);
            u.coin(ch, cb, sats(&p["value"]), &spk);
        }
    }
    let n = u.inputs();
    Ok((hash, u.finish(), n))
}

/// Undo hashes of blocks lo..=hi from the node, `jobs` heights in flight, handed to `each` in height order; `flush`
/// after each batch. Prints `<tag> h=<h> blocks_per_s=<r>` every 30 s and at the end.
fn fetch_undos(cli: &str, lo: u32, hi: u32, jobs: usize, tag: &str, each: &mut dyn FnMut(u32, &str, &[u8; 32], u64),
               flush: &mut dyn FnMut()) {
    let t0 = std::time::Instant::now();
    let jobs = jobs.max(1);
    let mut next = lo;
    let mut tick = std::time::Instant::now();
    while next <= hi {
        let top = hi.min(next + 16 * jobs as u32 - 1);
        let queue = std::sync::Mutex::new(next..=top);
        let res: std::sync::Mutex<Vec<(u32, Result<(String, [u8; 32], u64), String>)>> = std::sync::Mutex::new(Vec::new());
        std::thread::scope(|sc| {
            for _ in 0..jobs {
                sc.spawn(|| loop {
                    let Some(h) = queue.lock().unwrap().next() else { break };
                    let r = node_undo(cli, h);
                    res.lock().unwrap().push((h, r));
                });
            }
        }); // every worker has finished here
        let mut got = res.into_inner().unwrap();
        got.sort_by_key(|x| x.0);
        for (h, r) in got {
            let (hash, u, n) = r.unwrap_or_else(|e| panic!("block {h}: {e}"));
            each(h, &hash, &u, n);
        }
        flush();
        next = top + 1;
        if tick.elapsed().as_secs() >= 30 || next > hi {
            let d = t0.elapsed().as_secs_f64();
            println!("{tag} h={} blocks_per_s={:.2} elapsed_s={:.1}", next - 1, (next - lo) as f64 / d.max(1e-9), d);
            tick = std::time::Instant::now();
        }
    }
}

/// `libzync-undo-hashes <lo> <hi> --bitcoin-cli "<cmd>" [--jobs N]`: each block's undo hash (docs/libzync/UNDO.md) from
/// a node, one `undo: block <h> inputs <n> hash <hex> blockhash <display hex>` line per block: what a proof's block
/// leaves commit, for a node operator checking them, and the rate libzync-reforest's first phase runs at.
pub(crate) fn undo_hashes_cmd(lo: u32, hi: u32, cli: &str, jobs: usize) {
    fetch_undos(cli, lo, hi, jobs, "UNDO_HASHES", &mut |h, hash, u, n| {
        println!("undo: block {h} inputs {n} hash {} blockhash {hash}", crate::hex(u));
    }, &mut || {});
}

fn hex32(s: &str) -> Option<[u8; 32]> {
    (s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())).then(|| crate::hx(s).try_into().ok()).flatten()
}

/// One undo.tsv line, with its check field.
fn cache_line(h: u32, hash: &str, u: &[u8; 32], n: u64) -> String {
    let body = format!("{h}\t{hash}\t{}\t{n}", crate::hex(u));
    let c = Sha256::digest(body.as_bytes());
    format!("{body}\t{}\n", crate::hex(&c[..8]))
}

/// undo.tsv, contiguous from 1: (display block hash, undo hash) per height, and the byte length of the good lines. A
/// last line without its newline (a torn append) is dropped; any other line that is not exactly what `cache_line`
/// writes is an error: the cache is not this tool's, or it was damaged, and nothing in it is used.
fn read_cache(path: &str) -> Result<(Vec<(String, [u8; 32])>, usize), String> {
    let Ok(t) = std::fs::read_to_string(path) else { return Ok((Vec::new(), 0)) };
    let mut v = Vec::new();
    let mut good = 0usize;
    for l in t.split_inclusive('\n') {
        if !l.ends_with('\n') { println!("  reforest: {path}: a torn last line is fetched again"); break; }
        let f: Vec<&str> = l.trim_end_matches('\n').split('\t').collect();
        let parsed = (f.len() == 5).then(|| (f[0].parse::<u32>().ok(), hex32(f[2]), f[3].parse::<u64>().ok())).and_then(|x| match x {
            (Some(h), Some(u), Some(n)) if h as usize == v.len() + 1 && hex32(f[1]).is_some() => Some((h, u, n)),
            _ => None,
        });
        match parsed {
            Some((h, u, n)) if cache_line(h, f[1], &u, n) == l => v.push((f[1].to_string(), u)),
            _ => return Err(format!("{path}: line {} is not a line this tool wrote (damaged, or another format): delete {} and run again",
                                    v.len() + 1, path.trim_end_matches("/undo.tsv"))),
        }
        good += l.len();
    }
    Ok((v, good))
}

/// DEV-MODE TEST HOOK (the acceptance test's control): HAZYNC_REFOREST_TEST_UNDO=<h>:<hex> replaces block h's undo hash
/// in the forest walk. Refused outside dev mode.
fn test_undo() -> Option<(u32, [u8; 32])> {
    let v = std::env::var("HAZYNC_REFOREST_TEST_UNDO").ok()?;
    assert!(crate::verify::dev_mode_env(), "HAZYNC_REFOREST_TEST_UNDO is a dev-mode test hook");
    let (h, x) = v.split_once(':').expect("HAZYNC_REFOREST_TEST_UNDO=<h>:<hex>");
    Some((h.parse().expect("HAZYNC_REFOREST_TEST_UNDO height"), hex32(x).expect("HAZYNC_REFOREST_TEST_UNDO hash")))
}

/// Every leaf_<h>.bin in `dir` with 1 <= h <= s: (h, path).
fn bundles_in(dir: &str, s: u32) -> Vec<(u32, String)> {
    let mut v: Vec<(u32, String)> = std::fs::read_dir(dir).map(|it| it.flatten().filter_map(|e| {
        let n = e.file_name().to_string_lossy().into_owned();
        let h: u32 = n.strip_prefix("leaf_")?.strip_suffix(".bin")?.parse().ok()?;
        (h >= 1 && h <= s).then(|| (h, format!("{dir}/{n}")))
    }).collect()).unwrap_or_default();
    v.sort();
    v
}

/// A file replaced so that a crash leaves the old one or the new one: a temporary file, synced, renamed over it. The
/// directory is synced by the caller (in batches, and at the end).
fn replace_file(path: &str, bytes: &[u8]) {
    let tmp = format!("{path}.reforest.tmp");
    std::fs::File::create(&tmp).and_then(|mut f| { f.write_all(bytes)?; f.sync_data() }).unwrap_or_else(|e| panic!("{tmp}: {e}"));
    std::fs::rename(&tmp, path).unwrap_or_else(|e| panic!("{path}: {e}"));
}

pub(crate) fn reforest_cmd(out: &str, s_height: u32, cli: &str, all: bool, jobs: usize, bundles: Option<&str>, check_bundles: &[String]) {
    for v in ["HAZYNC_COINLOG_DIR", "HAZYNC_TRIE_DIR", "HAZYNC_PREP_LEAF_BUNDLES"] {
        assert!(std::env::var_os(v).is_none(), "libzync-reforest touches no store: unset {v}");
    }
    assert!(s_height >= 1, "libzync-reforest: S must be at least 1");
    crate::lz::lock_run_dirs(out);   // the out dir: a running prep holds it
    // every bundle directory too: prep holds its HAZYNC_PREP_LEAF_BUNDLES directory while it runs (lock_run_dirs)
    for d in bundles.into_iter().chain(check_bundles.iter().map(|x| x.as_str())) { crate::lz::lock_dir(d); }
    if std::env::var("HAZYNC_REFOREST_ANY_HEIGHT").as_deref() != Ok("1") {
        assert!(std::path::Path::new(&format!("{out}/state_{s_height}.coinlog")).exists(),
                "{out}/state_{s_height}.coinlog is missing: S must be a save point of the prep run (HAZYNC_REFOREST_ANY_HEIGHT=1 overrides)");
    }
    let rd = format!("{out}/reforest");
    std::fs::create_dir_all(&rd).unwrap_or_else(|e| panic!("{rd}: {e}"));
    let cache_path = format!("{rd}/undo.tsv");

    // 1. Undo hashes from the node, in height order, `jobs` heights in flight.
    let (mut cache, good_len) = read_cache(&cache_path).unwrap_or_else(|e| panic!("{e}"));
    // Resuming: a sample of what the cache holds is fetched again and must agree (the node, the chain and this tool
    // are the ones that wrote it): the last height and up to 31 spread over the rest.
    if !cache.is_empty() {
        let n = cache.len() as u32;
        let mut sample: Vec<u32> = (0..31u32).map(|k| 1 + ((k as u64 * 2_654_435_761 + n as u64 * 40_503) % n as u64) as u32).collect();
        sample.push(n);
        sample.sort();
        sample.dedup();
        for h in &sample {
            let (hash, u, _) = node_undo(cli, *h).unwrap_or_else(|e| panic!("resume check, block {h}: {e}"));
            let (ch, cu) = &cache[*h as usize - 1];
            assert!(*ch == hash && *cu == u, "{cache_path}: block {h} is not what the node gives now: delete {rd} and run again");
        }
        println!("  reforest: resuming after {n} cached heights; {} of them fetched again, all agree", sample.len());
    }
    if std::fs::metadata(&cache_path).map(|m| m.len() as usize).unwrap_or(0) > good_len {
        let f = std::fs::OpenOptions::new().write(true).open(&cache_path).unwrap();
        f.set_len(good_len as u64).unwrap();
    }
    let file = std::cell::RefCell::new(std::fs::OpenOptions::new().create(true).append(true).open(&cache_path)
        .unwrap_or_else(|e| panic!("{cache_path}: {e}")));
    cache.truncate(s_height as usize);
    let start = cache.len() as u32 + 1;
    if start <= s_height {
        println!("  reforest: undo hashes {start}..={s_height} from the node ({jobs} at a time)");
        fetch_undos(cli, start, s_height, jobs, "REFOREST_FETCH", &mut |h, hash, u, n| {
            file.borrow_mut().write_all(cache_line(h, hash, u, n).as_bytes()).unwrap();
            cache.push((hash.to_string(), *u));
        }, &mut || file.borrow().sync_data().unwrap());
    }
    assert_eq!(cache.len(), s_height as usize);

    // 1b. Prep's own copy of the spent coins: every bundle at heights 1..=S must give the node's undo hash.
    let mut checked = 0usize;
    for d in bundles.into_iter().chain(check_bundles.iter().map(|x| x.as_str())) {
        for (h, bp) in bundles_in(d, s_height) {
            let b = crate::prep::leaf_read(&bp);
            let j: serde_json::Value = serde_json::from_slice(&b.block_json).unwrap_or_else(|e| panic!("{bp}: block_json: {e}"));
            assert_eq!(j["height"].as_u64(), Some(h as u64), "{bp}: its block is not block {h}");
            let (u, _) = crate::prep::undo_of_block_json(&j);
            assert!(u == cache[h as usize - 1].1, "{bp}: prep's own coins give undo hash {} for block {h}, the node {}: stopped, nothing written",
                    crate::hex(&u), crate::hex(&cache[h as usize - 1].1));
            checked += 1;
        }
    }
    println!("  reforest: {checked} leaf bundle(s) cross-checked: prep's coins give the node's undo hash{}",
             if checked == 0 { " (NONE given: --bundles / --check-bundles)" } else { "" });
    let test = test_undo();

    // 2. The forest walk.
    let g = crate::genesis_anchor();
    let mut old = hazync_utreexo::Stump::new();
    old.roots = g.block_roots.clone();
    old.num_leaves = g.block_leaves;
    let mut new = hazync_utreexo::Stump::new();
    new.roots = g.block_roots.clone();
    new.num_leaves = g.block_leaves;
    let (mut rewritten, mut kept, mut b_rewritten) = (0u64, 0u64, 0u64);
    let mut final_json = String::new();
    let t0 = std::time::Instant::now();
    for h in 1..=s_height {
        if h % 10_000 == 0 {
            let d = t0.elapsed().as_secs_f64();
            println!("REFOREST_WALK h={h} blocks_per_s={:.0} rewritten={rewritten}", h as f64 / d.max(1e-9));
        }
        let p = format!("{out}/state_{h}.json");
        let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"));
        let mut st: ChainState = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{p}: {e}"));
        let (hash, cached) = &cache[h as usize - 1];
        let undo_v: [u8; 32] = match test { Some((th, tu)) if th == h => tu, _ => *cached };
        let undo = &undo_v;
        assert_eq!(st.height, h, "{p}: height {}", st.height);
        assert_eq!(crate::hex(&crate::rev(st.tip_hash.to_vec())), *hash, "{p}: its tip is not the node's block {h}");
        let mut pre: Vec<u8> = Vec::new();
        crate::handover_pin::utxo_state_preimage(st.utxo_leaves, st.utxo_value, &crate::normalize_host(st.utxo_roots.clone()), &mut |b: &[u8]| pre.extend_from_slice(b));
        let state: [u8; 32] = Sha256::digest(&pre).into();
        old.add(Sha256::digest(crate::handover_pin::block_leaf_preimage(&st.tip_hash, h, &state)).into());
        new.add(crate::prep::block_leaf_host(&st.tip_hash, h, &state, undo).0);
        let (ro, rn) = (crate::normalize_host(old.roots.clone()), crate::normalize_host(new.roots.clone()));
        let have = crate::normalize_host(st.block_roots.clone());
        assert!(st.block_leaves == h as u64 + 1 && (have == ro || have == rn),
                "{p}: block_roots are neither the pre-commitment forest nor the new one over states 1..{h}: not one contiguous prep run from genesis");
        // the state as the old run wrote it, and as the new prep would: the new forest, stamped by this image
        let mut old_st = st.clone();
        crate::stamp(&mut old_st);
        st.block_roots = rn;
        st.block_leaves = new.num_leaves;
        crate::stamp(&mut st);
        // --bundles: leaf_<h+1>.bin carries S(h) as its `prev` (read stamped, as leaf_read always does). It must be the
        // old state h or, on a rerun, the new one; it becomes the new one. Its witnesses (coin log, trie, BIP30) and its
        // block are untouched: they do not depend on the block forest.
        if let Some(bd) = bundles {
            let bp = format!("{bd}/leaf_{}.bin", h + 1);
            if std::path::Path::new(&bp).exists() {
                let mut b = crate::prep::leaf_read(&bp);
                let have = crate::state_journal_bytes(&b.prev);
                if have == crate::state_journal_bytes(&old_st) {
                    b.prev = st.clone();
                    crate::prep::leaf_write(&bp, &b);
                    b_rewritten += 1;
                } else {
                    assert!(have == crate::state_journal_bytes(&st), "{bp}: its prev is neither state_{h} nor its re-forested form");
                }
            }
        }
        if h == s_height { final_json = serde_json::to_string(&st).unwrap(); }
        if !(all || h == s_height) { continue; }
        let json = serde_json::to_string(&st).unwrap();
        if json == text { kept += 1; continue; }
        replace_file(&p, json.as_bytes());
        rewritten += 1;
        if rewritten % 1000 == 0 { crate::lz::cl_sync_dir(std::path::Path::new(out)); }
    }
    crate::lz::cl_sync_dir(std::path::Path::new(out));
    // history.bin (a path service's record of the chain; the deployed prep writes none): records 1..=S get their undo
    // hashes, so zync-path / zync-serve rebuild the new leaves. Records past S are cut by the continuing prep.
    let hp = format!("{out}/history.bin");
    if let Ok(b) = std::fs::read(&hp) {
        let mut recs = libzync::history::decode_all(&b).unwrap_or_else(|e| panic!("{hp}: {e}"));
        for r in recs.iter_mut().filter(|r| r.height >= 1 && r.height <= s_height) {
            assert_eq!(crate::hex(&crate::rev(r.hash.to_vec())), cache[r.height as usize - 1].0, "{hp}: record {} is not the node's block", r.height);
            r.undo = crate::handover_pin::leaf_has_undo(&crate::own_image(), crate::pinned_lineage(), r.height).then_some(cache[r.height as usize - 1].1);
        }
        crate::lz::write_atomic(&hp, &recs.iter().flat_map(|r| r.encode()).collect::<Vec<u8>>());
        println!("  reforest: {hp}: undo hashes in records 1..={}", s_height.min(recs.len().saturating_sub(1) as u32));
    }
    if let Some(bd) = bundles {
        crate::lz::cl_sync_dir(std::path::Path::new(bd));
        // bundles above S + 1 were cut from states this run did not rewrite: the continuing prep cuts them again
        let above: Vec<u32> = std::fs::read_dir(bd).map(|it| it.flatten().filter_map(|e| e.file_name().to_string_lossy()
            .strip_prefix("leaf_").and_then(|x| x.strip_suffix(".bin")).and_then(|x| x.parse().ok())).filter(|h| *h > s_height + 1).collect())
            .unwrap_or_default();
        println!("  reforest: {b_rewritten} leaf bundle(s) rewritten in {bd}{}", if above.is_empty() { String::new() } else {
            format!("; {} bundle(s) above {} are still the old format: remove them (the continuing prep cuts them again)", above.len(), s_height + 1) });
    }
    // The completion marker and the state's forest marker, last: prep continues from state_S only when it is there.
    crate::prep::write_forest_mark(&format!("{out}/state_{s_height}.json"));
    let done = format!("{rd}/DONE_{s_height}");
    crate::lz::write_atomic(&done, format!("{s_height} {} {}\n", cache[s_height as usize - 1].0,
                                           crate::hex(&Sha256::digest(final_json.as_bytes()))).as_bytes());
    println!("LZ_REFOREST_DONE 1..={s_height}: {rewritten} state(s) rewritten, {kept} already in the new format{}; \
              continue with HAZYNC_PREP_CONTINUE=1 from state_{s_height}.json",
             if all { "" } else { " (only state_S; --all rewrites every state)" });
}
