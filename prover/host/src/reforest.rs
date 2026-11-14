//! `libzync-reforest`: carry a prep run made before the undo-data commitment (docs/libzync/UNDO.md) over to it, without
//! re-running prep. Only the block forest changed: every leaf from block 1 on now commits its block's undo hash, so
//! `block_roots` differs in every state; the coin log, trie, BIP30 tree and every other ChainState field do not.
//!
//!   libzync-reforest <out dir> <S> --bitcoin-cli "<cmd>" [--all] [--jobs N] [--bundles <leaf bundle dir>]
//!
//! 1. Undo hashes for blocks 1..=S from the node (`getblock <hash> 3`: each input's prevout height, coinbase flag,
//!    value and script; libzync::undo), into `<out dir>/reforest/undo.tsv` (`height block-hash undo-hash inputs`, height
//!    order, appended as it goes: a rerun resumes after its last line).
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
//! and it holds the out dir's LOCK, so it refuses to run while prep does. Run it at the switch, with prep stopped at a
//! save point S (state_S.coinlog present).

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

/// Satoshis from the node's BTC amount. A JSON number with at most 8 decimals below 21e6 BTC: as an f64 its error is
/// under 0.25 sat, so rounding is exact; checked anyway.
fn sats(v: &serde_json::Value) -> u64 {
    let f = v.as_f64().expect("getblock: prevout value is not a number");
    let s = (f * 1e8).round();
    assert!(s >= 0.0 && (f * 1e8 - s).abs() < 1e-3 && s <= 2.1e15, "getblock: prevout value {f} is not a satoshi amount");
    s as u64
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
            let ch = p["height"].as_u64().ok_or(format!("getblock {h}: prevout height"))? as u32;
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

/// undo.tsv, contiguous from 1: (display block hash, undo hash) per height. A torn last line is dropped.
fn read_cache(path: &str) -> Vec<(String, [u8; 32])> {
    let Ok(t) = std::fs::read_to_string(path) else { return Vec::new() };
    let mut v = Vec::new();
    for l in t.lines() {
        let f: Vec<&str> = l.split('\t').collect();
        let ok = f.len() == 4 && f[0].parse::<usize>().ok() == Some(v.len() + 1) && hex32(f[1]).is_some() && f[3].parse::<u64>().is_ok();
        match (ok, hex32(f.get(2).copied().unwrap_or(""))) {
            (true, Some(u)) => v.push((f[1].to_string(), u)),
            _ => { println!("  reforest: {path}: stopping at line {} (torn or malformed); fetched again", v.len() + 1); break; }
        }
    }
    v
}

pub(crate) fn reforest_cmd(out: &str, s_height: u32, cli: &str, all: bool, jobs: usize, bundles: Option<&str>) {
    for v in ["HAZYNC_COINLOG_DIR", "HAZYNC_TRIE_DIR"] {
        assert!(std::env::var_os(v).is_none(), "libzync-reforest touches no store: unset {v}");
    }
    assert!(s_height >= 1, "libzync-reforest: S must be at least 1");
    crate::lz::lock_run_dirs(out);   // the out dir only: a running prep holds it
    if std::env::var("HAZYNC_REFOREST_ANY_HEIGHT").as_deref() != Ok("1") {
        assert!(std::path::Path::new(&format!("{out}/state_{s_height}.coinlog")).exists(),
                "{out}/state_{s_height}.coinlog is missing: S must be a save point of the prep run (HAZYNC_REFOREST_ANY_HEIGHT=1 overrides)");
    }
    let rd = format!("{out}/reforest");
    std::fs::create_dir_all(&rd).unwrap_or_else(|e| panic!("{rd}: {e}"));
    let cache_path = format!("{rd}/undo.tsv");

    // 1. Undo hashes from the node, in height order, `jobs` heights in flight.
    let mut cache = read_cache(&cache_path);
    // the good prefix only: a torn or malformed tail is cut off and fetched again (a longer cache than S is kept)
    let good_len: usize = std::fs::read_to_string(&cache_path).map(|t| t.lines().take(cache.len()).map(|l| l.len() + 1).sum()).unwrap_or(0);
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
            writeln!(file.borrow_mut(), "{h}\t{hash}\t{}\t{n}", crate::hex(u)).unwrap();
            cache.push((hash.to_string(), *u));
        }, &mut || file.borrow().sync_data().unwrap());
    }
    assert_eq!(cache.len(), s_height as usize);

    // 2. The forest walk.
    let g = crate::genesis_anchor();
    let mut old = hazync_utreexo::Stump::new();
    old.roots = g.block_roots.clone();
    old.num_leaves = g.block_leaves;
    let mut new = hazync_utreexo::Stump::new();
    new.roots = g.block_roots.clone();
    new.num_leaves = g.block_leaves;
    let (mut rewritten, mut kept, mut b_rewritten) = (0u64, 0u64, 0u64);
    let t0 = std::time::Instant::now();
    for h in 1..=s_height {
        if h % 10_000 == 0 {
            let d = t0.elapsed().as_secs_f64();
            println!("REFOREST_WALK h={h} blocks_per_s={:.0} rewritten={rewritten}", h as f64 / d.max(1e-9));
        }
        let p = format!("{out}/state_{h}.json");
        let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"));
        let mut st: ChainState = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{p}: {e}"));
        let (hash, undo) = &cache[h as usize - 1];
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
        if !(all || h == s_height) { continue; }
        let json = serde_json::to_string(&st).unwrap();
        if json == text { kept += 1; continue; }
        if h == s_height {
            crate::lz::write_atomic(&p, json.as_bytes());
        } else {
            let tmp = format!("{p}.reforest.tmp");
            std::fs::write(&tmp, &json).unwrap_or_else(|e| panic!("{tmp}: {e}"));
            std::fs::rename(&tmp, &p).unwrap_or_else(|e| panic!("{p}: {e}"));
        }
        rewritten += 1;
    }
    crate::lz::cl_sync_dir(std::path::Path::new(out));
    if let Some(bd) = bundles {
        crate::lz::cl_sync_dir(std::path::Path::new(bd));
        // bundles above S + 1 were cut from states this run did not rewrite: the continuing prep cuts them again
        let above: Vec<u32> = std::fs::read_dir(bd).map(|it| it.flatten().filter_map(|e| e.file_name().to_string_lossy()
            .strip_prefix("leaf_").and_then(|x| x.strip_suffix(".bin")).and_then(|x| x.parse().ok())).filter(|h| *h > s_height + 1).collect())
            .unwrap_or_default();
        println!("  reforest: {b_rewritten} leaf bundle(s) rewritten in {bd}{}", if above.is_empty() { String::new() } else {
            format!("; {} bundle(s) above {} are still the old format: remove them (the continuing prep cuts them again)", above.len(), s_height + 1) });
    }
    println!("LZ_REFOREST_DONE 1..={s_height}: {rewritten} state(s) rewritten, {kept} already in the new format{}; \
              continue with HAZYNC_PREP_CONTINUE=1 from state_{s_height}.json",
             if all { "" } else { " (only state_S; --all rewrites every state)" });
}
