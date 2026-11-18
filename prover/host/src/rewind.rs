//! REORG.md §5-§6, phase P3: `libzync-prep-rewind <out> <block dir> [--allow-deep]`, the prep side of a reorg. Undoes
//! prep's stores from its newest complete save point A back to the fork F with the undo records (reorg.rs), writes a
//! complete save point at F, moves everything above F aside, and clears the fetcher's marker. libzync-prep-run.sh runs
//! it before prep starts whenever `out/REWIND` is there, or `blocks/REORG` with HAZYNC_REORG_AUTO=1.
//!
//! The target comes from `out/REWIND` (the intent, written first, durably) or else the fetcher's `blocks/REORG`: a fork
//! the fetcher classed `auto` (depth within W), or any fork in J with `--allow-deep` (the runbook's one command,
//! REORG.md §11). REORG_TOO_DEEP, missing or damaged records, a record that does not chain, or a block that re-created
//! a coin over its spent record: refused, exit 4, nothing changed.
//!
//! **Crash safety.** Every step is idempotent, so a rewind that dies part-way is run again from `out/REWIND`:
//!   - the coin log is undone in place with cl_undo_to to F's leaf count (lz::cl_rewind), which holds however much of
//!     any block or of an earlier attempt reached the disk;
//!   - the disk trie is untouched until the save point at F flushes it: it is at A (undo again) or at F (done);
//!   - the BIP30 tree is rebuilt from `state_A.smt` each time;
//!   - files above F are moved only after the save point at F is complete, and A stops being a complete save point as
//!     soon as one of its files has moved, so a rerun then finds F as the newest and goes straight to the moves.
//! The coordinator's side (REORG.md §5 steps 4-6, P4): once everything above F has moved, a RECORD `out/reorg/records/
//! <E>.json` is written (once; its bytes are what the ack names), and the rewind waits for the coordinator's ACK
//! `<E>.ack` before prep's position moves to F and the intent goes, so prep publishes nothing of the new branch while
//! the board still holds rows of the orphaned one (record_and_ack).

use crate::reorg::{self, UndoRecord};
use crate::ChainState;

fn read_state(out: &str, h: u32) -> Option<ChainState> {
    serde_json::from_str(&std::fs::read_to_string(format!("{out}/state_{h}.json")).ok()?).ok()
}

/// The disk trie's flushed height, read from its meta (bytes 17..25, as libzync-prep-run.sh reads it) WITHOUT opening the
/// arena: an open bumps `trie/GEN` (Z1), and a refused rewind must change nothing. 0 for a trie never flushed.
fn trie_height() -> u64 {
    let d = crate::trie::trie_dir().expect("a rewind needs the on-disk trie (HAZYNC_TRIE_DIR)");
    match std::fs::read(d.join("meta")) {
        Ok(b) if b.len() >= 25 && &b[..8] == b"HZTRIED1" => u64::from_le_bytes(b[17..25].try_into().unwrap()),
        Ok(_) => panic!("{}/meta: not a trie arena meta", d.display()),
        Err(_) => 0,
    }
}

/// The newest COMPLETE save point, by libzync-prep-run.sh's rule: state_<h>.json (non-empty), .smt and .coinlog, and
/// the trie's marker or the disk trie at h (a crash between the trie's flush and its marker).
pub(crate) fn newest_complete_save(out: &str, trie_h: u64) -> u32 {
    let mut hs: Vec<u32> = std::fs::read_dir(out).unwrap_or_else(|e| panic!("{out}: {e}")).flatten()
        .filter_map(|e| e.file_name().to_string_lossy().strip_prefix("state_")?.strip_suffix(".smt")?.parse().ok()).collect();
    hs.sort_unstable_by(|a, b| b.cmp(a));
    hs.into_iter().find(|h| {
        let p = |x: &str| std::path::PathBuf::from(format!("{out}/state_{h}.{x}"));
        p("json").metadata().is_ok_and(|m| m.len() > 0) && p("coinlog").exists() && (p("trie").exists() || trie_h == *h as u64)
    }).unwrap_or(0)
}

fn refuse(why: &str) -> ! {
    println!("PREP_REWIND_REFUSED {why}");
    println!("  nothing was changed; a person decides (docs/libzync/REORG.md §11)");
    std::process::exit(reorg::EXIT_REORG);
}

/// A block file's hash (the state's tip_hash order), as prep_block computes it.
fn block_file_hash(p: &std::path::Path) -> Option<[u8; 32]> {
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
    let merkle: [u8; 32] = crate::rev(crate::hx(j["merkle"].as_str()?)).try_into().ok()?;
    let h = crate::build_header_v(j["version"].as_i64().unwrap_or(1) as i32, j["prev"].as_str()?, &merkle,
                                  j["time"].as_u64()? as u32, j["bits"].as_u64()? as u32, j["nonce"].as_u64()? as u32);
    Some(crate::header_hash(&h))
}

fn move_into(from: &std::path::Path, dir: &std::path::Path) {
    if !from.exists() { return; }
    std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let to = dir.join(from.file_name().unwrap());
    std::fs::rename(from, &to).unwrap_or_else(|e| panic!("{} -> {}: {e}", from.display(), to.display()));
}

/// Once the intent is written, any failure is a stop for a person (exit 4: the unit fails and alerts), never a panic's
/// 101 that systemd restarts every 30 s in silence (review 16.11, M1). The rewind is idempotent, so the person can
/// run it again once the cause is fixed.
fn exit_4_on_panic() {
    let dflt = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        dflt(info);
        println!("PREP_REWIND_FAILED after its intent was written: exit 4; out/REWIND is kept, a rerun continues it (docs/libzync/REORG.md §11)");
        std::process::exit(reorg::EXIT_REORG);
    }));
}

fn hook(name: &str) {
    if std::env::var("HAZYNC_CRASH_AT").as_deref() == Ok(name) { println!("HAZYNC_CRASH_AT={name}: dying here"); std::process::exit(9); }
}

pub(crate) fn rewind_cmd(out: &str, blocks: &str, allow_deep: bool, no_leaves: bool) {
    // any panic, from the first line on, is exit 4 (review 16.11, L1): a rerun that panics before its intent is read
    // must not become systemd's silent 101 loop either
    exit_4_on_panic();
    assert!(methods::COINLOG && methods::TRIE, "libzync-prep-rewind needs the product image (coin log + trie)");
    // the leaf bundles above F are the orphaned branch's and must move: a rewind that does not know where they are
    // would leave them for the coordinator to serve (review 16.11). Say where, or that there are none.
    let leaves = std::env::var("HAZYNC_PREP_LEAF_BUNDLES").ok().filter(|v| !v.is_empty());
    if leaves.is_none() && !no_leaves { refuse("HAZYNC_PREP_LEAF_BUNDLES is not set: where are the leaf bundles? (--no-leaf-bundles if this run writes none)"); }
    for (k, v) in [("HAZYNC_COINLOG_DIR", crate::lz::cl_dir().is_some()), ("HAZYNC_TRIE_DIR", crate::trie::trie_dir().is_some())] {
        if !v { refuse(&format!("{k} is not set: the rewind works on prep's on-disk stores")); }
    }
    crate::lz::lock_run_dirs(out);
    let w: u32 = std::env::var("HAZYNC_REORG_AUTO_DEPTH").ok().and_then(|v| v.parse().ok()).unwrap_or(288);
    let rw = format!("{out}/REWIND");
    // 1. the target: the intent if a rewind already started, else the fetcher's marker (the intent is written once
    //    everything is checked, so a refusal leaves nothing behind)
    let prior = std::fs::read_to_string(&rw).ok().and_then(|x| serde_json::from_str::<serde_json::Value>(&x).ok());
    let (fork, epoch, marker) = match &prior {
        Some(v) => (v["fork"].as_u64().expect("REWIND: fork") as u32, v["epoch"].as_u64().expect("REWIND: epoch") as u32, None),
        None => {
            if std::path::Path::new(blocks).join("REORG_TOO_DEEP").exists() {
                refuse("the fetcher's marker is REORG_TOO_DEEP: the undo records cannot reach the fork");
            }
            let m: serde_json::Value = std::fs::read_to_string(std::path::Path::new(blocks).join("REORG")).ok()
                .and_then(|x| serde_json::from_str(&x).ok()).unwrap_or_else(|| refuse(&format!("nothing to rewind to: no {rw} (no rewind in progress) and no readable {blocks}/REORG (none asked for, or a rewind already finished and removed it)")));
            let fork = m["fork"].as_u64().unwrap_or_else(|| refuse("REORG has no fork")) as u32;
            // a rewind that finished all but removing the marker (a crash after the intent went; review of P4, L5):
            // prep is AT the fork's block already. Nothing to undo and no new epoch to open: the marker goes.
            let tip = std::fs::read_to_string(format!("{out}/PREP_TIP.json")).ok().and_then(|x| serde_json::from_str::<serde_json::Value>(&x).ok());
            if let (Some(t), Some(fh)) = (&tip, m["fork_hash"].as_str()) {
                // ...and nothing above it is left (round 3: prep writes its tip AT the fork too, when it stops before
                // block F+1 because its old state files above are another chain's; those still have to move)
                if t["height"].as_u64() == Some(fork as u64) && t["hash"].as_str() == Some(fh)
                    && !std::path::Path::new(&format!("{out}/state_{}.json", fork + 1)).exists() {
                    let _ = std::fs::remove_file(std::path::Path::new(blocks).join("REORG"));
                    crate::lz::cl_sync_dir(std::path::Path::new(blocks));
                    println!("PREP_REWOUND to {fork}: prep is already at the fork's block (a rewind that stopped before removing the marker); the marker removed");
                    return;
                }
            }
            let depth = m["depth"].as_u64().unwrap_or(u64::MAX);
            if m["class"].as_str() != Some("auto") && !allow_deep {
                refuse(&format!("the fork at {fork} is {depth} deep, beyond the automatic window (class {}): --allow-deep", m["class"]));
            }
            if depth > w as u64 && !allow_deep { refuse(&format!("depth {depth} > W = {w}: --allow-deep")); }
            // the next epoch is above every one used before: the orphaned directories AND the board's records (review of
            // P4, H1: with out/orphaned/ cleared by hand, the epoch came round to a number whose record and ack were
            // still there, so an old ack "acked" a new rewind the coordinator never saw)
            let max_in = |d: String, suf: &str| std::fs::read_dir(d).map(|it| it.flatten()
                .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(suf)?.parse::<u32>().ok()).max().unwrap_or(0)).unwrap_or(0);
            let epoch = max_in(format!("{out}/orphaned"), "").max(max_in(format!("{out}/reorg/records"), ".json")) + 1;
            (fork, epoch, Some(m))
        }
    };
    let st_f = read_state(out, fork).unwrap_or_else(|| refuse(&format!("state_{fork}.json (the fork's state) does not read")));
    let trie_h = trie_height();
    let a = newest_complete_save(out, trie_h);
    println!("  rewind: fork {fork}, epoch {epoch}; the stores' newest complete save point {a}, the disk trie at {trie_h}");
    let undo_keep = reorg::keep();
    let udir = reorg::dir(out);
    // the records F+1..A, checked before anything changes: present, intact, chaining from F's block to A's
    let undo_needed = trie_h != fork as u64 && a > fork;
    let mut recs: Vec<UndoRecord> = Vec::new();
    let st_a = if undo_needed { Some(read_state(out, a).unwrap_or_else(|| refuse(&format!("state_{a}.json does not read")))) } else { None };
    if let Some(st_a) = &st_a {
        if trie_h != a as u64 { refuse(&format!("the disk trie is at {trie_h}, neither the save point {a} nor the fork {fork}")); }
        for h in fork + 1..=a {
            let p = udir.join(format!("u_{h}.bin"));
            let r = std::fs::read(&p).map_err(|e| e.to_string()).and_then(|b| UndoRecord::decode(&b))
                .unwrap_or_else(|e| refuse(&format!("{}: {e}: the records do not reach the fork", p.display())));
            let want_prev = recs.last().map_or(st_f.tip_hash, |x| x.hash);
            if r.height != h || r.prev != want_prev { refuse(&format!("{}: does not chain from block {} (records of another branch?)", p.display(), h - 1)); }
            if r.trie_replaced { refuse(&format!("block {h} re-created a coin over its spent record (BIP30): the trie cannot undo it")); }
            recs.push(r);
        }
        if recs.last().map(|r| r.hash) != Some(st_a.tip_hash) { refuse(&format!("the record of {a} is not state_{a}.json's block")); }
    }
    // The orphaned blocks, (height, hash), recorded in the intent (review 16.11, M3): a rerun after a crash during the
    // moves no longer has their state files to find them by. From the state files above F (prep's tip may be above
    // its save point) and the records.
    let tip_h = std::fs::read_to_string(format!("{out}/PREP_TIP.json")).ok()
        .and_then(|x| serde_json::from_str::<serde_json::Value>(&x).ok()).and_then(|v| v["height"].as_u64()).unwrap_or(0) as u32;
    let orphaned: Vec<(u32, String)> = match &prior {
        Some(v) => v["orphaned"].as_array().map(|x| x.iter().filter_map(|e| Some((e[0].as_u64()? as u32, e[1].as_str()?.to_string()))).collect()).unwrap_or_default(),
        None => {
            let mut o: std::collections::BTreeMap<u32, [u8; 32]> = recs.iter().map(|r| (r.height, r.hash)).collect();
            let mut h = fork + 1;
            loop {
                match read_state(out, h) { Some(s) => { o.insert(h, s.tip_hash); } None if h > a.max(tip_h) => break, None => {} }
                h += 1;
            }
            o.into_iter().map(|(h, x)| (h, crate::hex(&x))).collect()
        }
    };
    // the coin log's journal must be readable, every record with its height, before anything changes (review 16.11:
    // a damaged one used to panic only after the logged roots were cut)
    if let Err(e) = crate::lz::cl_journal_check() { refuse(&format!("the coin log's journal: {e}")); }
    if let Some(m) = &marker {
        // the epoch's directory is made with the intent, so the next rewind's epoch differs even if nothing moves (L5)
        std::fs::create_dir_all(format!("{out}/orphaned/{epoch}")).unwrap_or_else(|e| panic!("{out}/orphaned/{epoch}: {e}"));
        let intent = serde_json::json!({"fork": fork, "epoch": epoch, "marker": m,
                                        "orphaned": orphaned.iter().map(|(h, x)| serde_json::json!([h, x])).collect::<Vec<_>>()});
        crate::lz::write_atomic(&rw, intent.to_string().as_bytes());
    }
    // 2. the stores, from A back to F. The trie and the BIP30 tree are undone IN MEMORY and checked against F's state
    //    first (review 16.11, M1); only then is the coin log, the one store changed in place, touched.
    if trie_h == fork as u64 {
        // the trie is at F: a save point at F (an earlier attempt's, or prep's own when it stopped right after one).
        // Blocks prep applied after it are in the coin log's journal only: undone (idempotent), then the journal goes.
        if crate::lz::cl_dir().is_some_and(|d| d.join("pending.bin").exists()) { crate::lz::cl_rewind(fork, &[], &st_f); }
        crate::lz::cl_journal_commit();
        // a crash between the trie's flush at F and its marker: the marker now (review 16.11, L3)
        if !std::path::Path::new(&format!("{out}/state_{fork}.trie")).exists() {
            let d = crate::trie::trie_dir().unwrap();
            *crate::trie::TS.lock().unwrap() = Some(libzync::trie_store::TrieStore::open_dir(&d, true).unwrap_or_else(|e| panic!("{}: {e}", d.display())));
            crate::trie::trie_save(&format!("{out}/state_{fork}.trie"));
        }
        println!("  rewind: the stores are already at {fork}");
    } else if let Some(st_a) = &st_a {
        // the trie (copy-on-write: the disk is unchanged until the save point's flush). Opening the arena bumps its
        // generation (ZAPP_FOLLOWER.md §4): a fact reader re-resolves its root before any slot freed here is reused.
        let d = crate::trie::trie_dir().unwrap();
        let mut ts = libzync::trie_store::TrieStore::open_dir(&d, true).unwrap_or_else(|e| panic!("{}: {e}", d.display()));
        assert!(ts.hash == st_a.tip_hash, "rewind: the disk trie at {a} is not state_{a}.json's block");
        for r in recs.iter().rev() {
            ts.undo_recorded(&r.trie_marks, &r.trie_inserts, r.trie_replaced)
                .unwrap_or_else(|e| panic!("rewind: the trie refused the undo of block {}: {e:?}", r.height));
        }
        assert_eq!(libzync::coinlog::h_of(&ts.root()), st_f.trie_root, "rewind: the trie undone to {fork} is not the fork state's");
        // the BIP30 tree, from A's snapshot
        let b = std::fs::read(format!("{out}/state_{a}.smt")).unwrap_or_else(|e| panic!("{out}/state_{a}.smt: {e}"));
        let mut smt = crate::prep::smt_from_bytes(&b).unwrap_or_else(|e| panic!("{out}/state_{a}.smt: {e}"));
        assert_eq!(smt.root(), st_a.smt_root, "rewind: state_{a}.smt is not state_{a}.json's BIP30 tree");
        for r in recs.iter().rev() {
            for (k, v) in &r.smt_prior { match v { Some(v) => smt.insert(*k, *v), None => smt.remove(k) } }
        }
        assert_eq!(smt.root(), st_f.smt_root, "rewind: the BIP30 tree undone to {fork} is not the fork state's");
        // the logged roots of the orphaned blocks: archived with the epoch's files, then cut (review 16.11: archive first)
        let rl = d.join("roots.log");
        let above: Vec<u8> = std::fs::read(&rl).map(|b| b.chunks_exact(libzync::trie_store::ROOTS_REC)
            .filter(|e| u32::from_le_bytes(e[..4].try_into().unwrap()) >= fork).flatten().copied().collect()).unwrap_or_default();
        // (a rerun keeps the first archive: by then the file may hold the new F entry too; re-review 16.11, L-c)
        if !above.is_empty() && !std::path::Path::new(&format!("{out}/orphaned/{epoch}/roots.log")).exists() {
            let od = format!("{out}/orphaned/{epoch}");
            std::fs::create_dir_all(&od).unwrap_or_else(|e| panic!("{od}: {e}"));
            crate::lz::write_atomic(&format!("{od}/roots.log"), &above);
        }
        // F's entry goes too and is logged again below, in the new generation (a rerun then leaves one entry for F)
        ts.cut_roots_above((fork as u64).saturating_sub(1));
        (ts.height, ts.hash) = (fork as u64, st_f.tip_hash);
        ts.log_root(); // F's root as it is now (a new generation, new slots), for a reader (review 16.11, M4)
        *crate::trie::TS.lock().unwrap() = Some(ts);
        // the coin log, in place (idempotent), last
        let bodies: Vec<Vec<u8>> = recs.iter().rev().map(|r| r.cl.clone()).collect();
        crate::lz::cl_rewind(fork, &bodies, &st_f);
        // the save point at F (state_F.json is the one prep wrote; its bytes stay)
        crate::prep::save_stores(out, fork, &smt);
        println!("  rewind: {} block(s) undone, a complete save point at {fork}", recs.len());
        hook("rewind-after-save");
    } else {
        // the stores' save point is at or below the fork: nothing to undo (prep resumes from it on the shared chain)
        println!("  rewind: the newest save point {a} is at or below the fork: the stores need no undo");
    }
    // 3. everything above F belongs to the orphaned branch: states and their save files, leaf bundles (moved within their
    //    own filesystem: leaves/ and out/ may be on different ones, review 16.11 M1), records, and the block files whose
    //    block is an orphaned one (the fetcher fetches the new branch's)
    let odir = std::path::PathBuf::from(format!("{out}/orphaned/{epoch}"));
    let ohash: std::collections::HashMap<u32, String> = orphaned.iter().cloned().collect();
    // every leaf bundle above F is the orphaned branch's (prep cut it from a state above F), however far above prep's
    // last state file a crash left it (it is written before the state): found by listing, as the records are
    let heights_in = |dir: &str, pre: &str, suf: &str| -> Vec<u32> {
        std::fs::read_dir(dir).map(|it| it.flatten().filter_map(|e| e.file_name().to_string_lossy().strip_prefix(pre)?
            .strip_suffix(suf)?.parse::<u32>().ok()).filter(|h| *h > fork).collect()).unwrap_or_default()
    };
    let bundles: Vec<u32> = leaves.as_deref().map(|l| heights_in(l, "leaf_", ".bin")).unwrap_or_default();
    let records_above = heights_in(&udir.to_string_lossy(), "u_", ".bin");
    let top = orphaned.iter().map(|o| o.0).chain(bundles.iter().copied()).chain(records_above.iter().copied())
        .max().unwrap_or(fork).max(a).max(tip_h);
    let mut h = fork + 1;
    let mut moved = 0;
    loop {
        if !std::path::Path::new(&format!("{out}/state_{h}.json")).exists() && h > top { break; }
        for x in ["forest", "flags", "smt", "coinlog", "trie", "json"] {
            move_into(std::path::Path::new(&format!("{out}/state_{h}.{x}")), &odir);
        }
        if let Some(l) = &leaves {
            if bundles.contains(&h) { move_into(std::path::Path::new(&format!("{l}/leaf_{h}.bin")), &std::path::PathBuf::from(format!("{l}/orphaned/{epoch}"))); }
        }
        moved += 1;
        if moved == 2 { hook("rewind-mid-moves"); }
        h += 1;
    }
    let moved_to = h - 1;
    if let Ok(it) = std::fs::read_dir(&udir) {
        for e in it.flatten() {
            let k = e.file_name().to_string_lossy().strip_prefix("u_").and_then(|x| x.strip_suffix(".bin")).and_then(|x| x.parse::<u32>().ok());
            if k.is_some_and(|k| k > fork) { let _ = std::fs::remove_file(e.path()); }
        }
    }
    let mut gone = 0;
    if let Ok(it) = std::fs::read_dir(blocks) {
        for e in it.flatten() {
            let k = e.file_name().to_string_lossy().strip_prefix("block_").and_then(|x| x.strip_suffix(".json")).and_then(|x| x.parse::<u32>().ok());
            if let Some(k) = k.filter(|k| *k > fork) {
                if block_file_hash(&e.path()).is_some_and(|x| ohash.get(&k) == Some(&crate::hex(&x))) { let _ = std::fs::remove_file(e.path()); gone += 1; }
            }
        }
    }
    for d in [std::path::PathBuf::from(out), odir.clone(), udir.clone(), std::path::PathBuf::from(blocks)] { crate::lz::cl_sync_dir(&d); }
    // 3b. the board's side (P4): the record, then the coordinator's ack, before anything of the new branch can appear
    let new = marker.as_ref().or(prior.as_ref().map(|v| &v["marker"])).map(|m| m["new"].clone()).unwrap_or(serde_json::Value::Null);
    record_and_ack(out, epoch, fork, &st_f.tip_hash, &orphaned, &new);
    // 4. prep's position is the fork; the intent and the fetcher's marker go, last
    reorg::write_tip(out, fork, &st_f.tip_hash, undo_keep, if undo_keep > 0 { reorg::undo_from(&udir, fork, undo_keep) } else { fork + 1 }, true);
    let _ = std::fs::remove_file(&rw);
    crate::lz::cl_sync_dir(std::path::Path::new(out));
    hook("rewind-before-marker");
    let _ = std::fs::remove_file(std::path::Path::new(blocks).join("REORG"));
    crate::lz::cl_sync_dir(std::path::Path::new(blocks));
    println!("PREP_REWOUND to {fork} (epoch {epoch}): states {}..={moved_to} moved to {}, {gone} orphaned block file(s) removed",
             fork + 1, odir.display());
}

/// A hash in internal-order hex as Bitcoin displays it (byte-reversed); the coordinator shows these.
fn display(x: &str) -> String {
    let mut b = crate::hx(x);
    b.reverse();
    crate::hex(&b)
}

/// REORG.md §5 steps 4-6 (P4). `out/reorg/records/<E>.json`: the fork, its block, the orphaned blocks and the node's new
/// ones (display hex), written ONCE (a rerun keeps the bytes, so an ack already given still names them), durably. Then
/// the coordinator's `<E>.ack` is waited for: in `HAZYNC_REORG_ACK_DIR` (default the records' directory; on the box the
/// coordinator cannot write prep's out/), and it must carry the record's sha256. `HAZYNC_REORG_ACK=0`: no coordinator
/// follows this run (a prep nobody serves, the tests of prep alone), nothing is waited for. No ack within
/// `HAZYNC_REORG_ACK_TIMEOUT_S` (600, REORG_ACK_TIMEOUT): exit 4, so the unit fails and alerts; the intent stays, and a
/// rerun waits again.
fn record_and_ack(out: &str, epoch: u32, fork: u32, fork_hash: &[u8; 32], orphaned: &[(u32, String)], new: &serde_json::Value) {
    use sha2::{Digest, Sha256};
    let rd = std::path::PathBuf::from(format!("{out}/reorg/records"));
    std::fs::create_dir_all(&rd).unwrap_or_else(|e| panic!("{}: {e}", rd.display()));
    let rp = rd.join(format!("{epoch}.json"));
    let fork_hash = display(&crate::hex(fork_hash));
    let orph: Vec<serde_json::Value> = orphaned.iter().map(|(h, x)| serde_json::json!([h, display(x)])).collect();
    let bytes = match std::fs::read(&rp) {
        Ok(b) if !b.is_empty() => {
            // a record already there must be THIS rewind's (a rerun keeps its bytes); another one's is a stop for a
            // person, never an ack inherited from it (review of P4, H1)
            let v: serde_json::Value = serde_json::from_slice(&b).unwrap_or(serde_json::Value::Null);
            if v["fork"].as_u64() != Some(fork as u64) || v["fork_hash"].as_str() != Some(fork_hash.as_str())
                || v["orphaned"].as_array() != Some(&orph) {
                println!("PREP_REWIND_FAILED {} is another rewind's record (fork {}, not {fork}): exit 4; out/REWIND is kept \
                          (docs/libzync/REORG.md §17)", rp.display(), v["fork"]);
                std::process::exit(reorg::EXIT_REORG);
            }
            b
        }
        _ => {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            let rec = serde_json::json!({
                "epoch": epoch, "fork": fork, "fork_hash": fork_hash,
                "depth": orphaned.iter().map(|o| o.0).max().map_or(0, |t| t.saturating_sub(fork)),
                "orphaned": orph,
                "new": if new.is_array() { new.clone() } else { serde_json::json!([]) },
                "at": now, "source": "libzync-prep-rewind"});
            let b = rec.to_string().into_bytes();
            crate::lz::write_atomic(&rp.to_string_lossy(), &b);
            crate::lz::cl_sync_dir(&rd);
            b
        }
    };
    hook("rewind-after-record");
    if std::env::var("HAZYNC_REORG_ACK").as_deref() == Ok("0") {
        println!("  rewind: record {} written; HAZYNC_REORG_ACK=0: no coordinator to wait for", rp.display());
        return;
    }
    let sha = crate::hex(&Sha256::digest(&bytes));
    let ad = std::env::var("HAZYNC_REORG_ACK_DIR").ok().filter(|v| !v.is_empty()).map(std::path::PathBuf::from).unwrap_or(rd);
    let ap = ad.join(format!("{epoch}.ack"));
    let limit: u64 = std::env::var("HAZYNC_REORG_ACK_TIMEOUT_S").ok().and_then(|v| v.parse().ok()).unwrap_or(600);
    println!("  rewind: record {} written; waiting for the coordinator's ack {} (at most {limit} s)", rp.display(), ap.display());
    let t0 = std::time::Instant::now();
    loop {
        let acked = std::fs::read_to_string(&ap).ok().and_then(|x| serde_json::from_str::<serde_json::Value>(&x).ok())
            .is_some_and(|v| v["record_sha256"].as_str() == Some(sha.as_str()));
        if acked { println!("  rewind: the coordinator acked epoch {epoch}"); return; }
        if t0.elapsed().as_secs() >= limit {
            println!("PREP_REWIND_NO_ACK the coordinator did not ack {} within {limit} s (HAZYNC_REORG_ACK_TIMEOUT_S): exit 4; \
                      out/REWIND is kept, a rerun waits again (docs/libzync/REORG.md §5 step 6)", rp.display());
            std::process::exit(reorg::EXIT_REORG);
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

/// `libzync-store-digest <out> <h>`: the CONTENT of prep's on-disk stores at the save point h, for comparing two runs
/// that reached the same state by different histories (a rewind and a fresh run): the trie's canonical snapshot, the
/// coin log's leaves, the position index's live entries (its file differs: slots and tombstones depend on history),
/// and the save point's BIP30 snapshot. Read-only: it opens the arena as its WRITER only if nothing else holds it
/// (`ARENA.LOCK`), and then just bumps `trie/GEN` (Z1); beside a running prep it is a reader.
pub(crate) fn digest_cmd(out: &str, h: u32, root_at: Option<u32>) {
    use sha2::{Digest, Sha256};
    let hexs = |d: &[u8]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let td = crate::trie::trie_dir().expect("HAZYNC_TRIE_DIR");
    let ts = libzync::trie_store::TrieStore::open_dir(&td, true).unwrap_or_else(|e| panic!("{}: {e}", td.display()));
    assert_eq!(ts.height, h as u64, "the disk trie is at {}, not {h}", ts.height);
    println!("TRIE {}", hexs(&Sha256::digest(ts.to_bytes())));
    // --root-at H: the NEWEST roots.log entry for H, walked whole and loaded again (every hash recomputed): its root hash,
    // or BROKEN if a slot of it was reused (ZAPP_FOLLOWER Z1: a pinned root must survive)
    if let Some(at) = root_at {
        let b = std::fs::read(td.join("roots.log")).unwrap_or_default();
        let e = b.chunks_exact(libzync::trie_store::ROOTS_REC).filter(|e| u32::from_le_bytes(e[..4].try_into().unwrap()) == at).last()
            .unwrap_or_else(|| panic!("no roots.log entry for {at}"));
        let r = u64::from_le_bytes(e[36..44].try_into().unwrap());
        let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut v = libzync::trie_store::TrieStore::open_dir(&td, true).unwrap();
            v.set_root_for_reading(r, at as u64);
            libzync::trie_store::TrieStore::from_bytes(&v.to_bytes()).ok().map(|t| libzync::coinlog::h_of(&t.root()))
        })).ok().flatten();
        println!("ROOT_AT {at} {}", got.map_or("BROKEN".into(), |x| hexs(&x)));
    }
    let cd = crate::lz::cl_dir().expect("HAZYNC_COINLOG_DIR");
    let store = libzync::coinlog_forest::NodeStore::with_nodes(libzync::host::AnyNodes::Disk(
        libzync::host::DiskNodes::open(&cd).unwrap_or_else(|e| panic!("{}: {e}", cd.display()))));
    let mut s = Sha256::new();
    for p in 0..store.len() { s.update(store.leaf(p)); }
    println!("COINLOG {} {}", store.len(), hexs(&s.finalize()));
    let ix = libzync::host::DiskIndex::open(&cd.join("index.bin")).unwrap_or_else(|e| panic!("{}/index.bin: {e}", cd.display()));
    let e = ix.entries();
    let mut s = Sha256::new();
    for (l, p) in &e { s.update(l); s.update(p.to_le_bytes()); }
    println!("INDEX {} {}", e.len(), hexs(&s.finalize()));
    let b = std::fs::read(format!("{out}/state_{h}.smt")).unwrap_or_else(|e| panic!("{out}/state_{h}.smt: {e}"));
    println!("SMT {}", hexs(&Sha256::digest(&b)));
}
