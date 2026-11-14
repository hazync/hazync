//! `libzync-reforest`: carry a prep run made before the undo-data commitment (docs/libzync/UNDO.md) over to it, without
//! re-running prep. Only the block forest changed: every leaf from block 1 on now commits its block's undo hash, so
//! `block_roots` differs in every state; the coin log, trie, BIP30 tree and every other ChainState field do not.
//!
//!   libzync-reforest <out dir> <S> --bitcoin-cli "<cmd>" [--all] [--jobs N]
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

pub(crate) fn reforest_cmd(out: &str, s_height: u32, cli: &str, all: bool, jobs: usize) {
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
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&cache_path).unwrap_or_else(|e| panic!("{cache_path}: {e}"));
    cache.truncate(s_height as usize);
    let start = cache.len() as u32 + 1;
    if start <= s_height {
        println!("  reforest: undo hashes {start}..={s_height} from the node ({jobs} at a time)");
        let t0 = std::time::Instant::now();
        let jobs = jobs.max(1);
        let mut next = start;
        let mut tick = std::time::Instant::now();
        while next <= s_height {
            let hi = s_height.min(next + 16 * jobs as u32 - 1);
            let heights: Vec<u32> = (next..=hi).collect();
            let queue = std::sync::Mutex::new(heights.into_iter());
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
                writeln!(file, "{h}\t{hash}\t{}\t{n}", crate::hex(&u)).unwrap();
                cache.push((hash, u));
            }
            file.sync_data().unwrap();
            next = hi + 1;
            if tick.elapsed().as_secs() >= 30 || next > s_height {
                let d = t0.elapsed().as_secs_f64();
                println!("REFOREST_FETCH h={} blocks_per_s={:.1} elapsed_s={:.0}", next - 1, (next - start) as f64 / d.max(1e-9), d);
                tick = std::time::Instant::now();
            }
        }
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
    let (mut rewritten, mut kept) = (0u64, 0u64);
    let t0 = std::time::Instant::now();
    for h in 1..=s_height {
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
        if !(all || h == s_height) { continue; }
        st.block_roots = rn;
        st.block_leaves = new.num_leaves;
        crate::stamp(&mut st);
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
        if h % 10_000 == 0 {
            let d = t0.elapsed().as_secs_f64();
            println!("REFOREST_WALK h={h} blocks_per_s={:.0} rewritten={rewritten}", h as f64 / d.max(1e-9));
        }
    }
    crate::lz::cl_sync_dir(std::path::Path::new(out));
    println!("LZ_REFOREST_DONE 1..={s_height}: {rewritten} state(s) rewritten, {kept} already in the new format{}; \
              continue with HAZYNC_PREP_CONTINUE=1 from state_{s_height}.json",
             if all { "" } else { " (only state_S; --all rewrites every state)" });
}
