//! Reorg undo records (docs/libzync/REORG.md §6 option B; phase P1: WRITTEN, not yet read by anything).
//!
//! libzync-prep advances its coin log, trie and BIP30 tree in place, so a save point is a marker, not a copy, and the
//! stores cannot go back below their height. To undo blocks after a reorg (P3), prep keeps, for each of the last J
//! blocks (`HAZYNC_PREP_UNDO_KEEP`, default 2,016), what undoes that block in each store: `<out>/reorg/u_<h>.bin`.
//!
//! **When it is written.** After the block is applied, not before: the BIP30 tree's prior values (`smt_advance_ex`'s
//! second return) and the trie's `replaced` flag exist only once the block has been applied. That is safe because a
//! record is only ever needed for a block at or below the newest save point, or for a block applied in the same
//! process: a crash undoes everything above the last save point (`pending.bin` for the coin log, the last flush for
//! the trie), and the restart deletes the records above its anchor (`prune_at_start`) and writes them again as it
//! re-applies those blocks. Records are made durable at each save point before its state file (`sync_since`), so
//! every record at or below a complete save point survives a power loss. `pending.bin` stays the crash journal,
//! unchanged.
//!
//! **Layout** (all integers little-endian): `LZUNDO01` | height u32 | block hash 32 | prev hash 32 (both in the
//! state's `tip_hash` byte order) | coin log body length u64 | the body (exactly `pending.bin`'s record body: leaf count
//! before the block, the spends' positions and unspent leaves, the appends; `cl_undo_to` reads it) | trie: replaced u8,
//! mark count u32, each mark's key (64), insert count u32, each insert's key (64) | BIP30 tree: count u32, each touched
//! key (32), present u8, prior value u32 | SHA-256 of everything before it.
//!
//! The trie needs only the KEYS: its shape is a function of its key set, so removing an insert and unmarking a mark
//! by key is exact (`TrieStore::undo`), except over a BIP30 re-creation, which `replaced` records (impossible after
//! BIP34 until height 1,983,702; P3 refuses such a block).

use sha2::{Digest, Sha256};

pub(crate) const UNDO_MAGIC: &[u8; 8] = b"LZUNDO01";
/// J, the blocks whose records are kept (REORG.md decision 2, 16.11.2026).
pub(crate) const UNDO_KEEP_DEFAULT: u32 = 2016;
/// The exit code of a stop for a reorg (REORG.md §11): the units' RestartPreventExitStatus, so the unit fails and
/// its OnFailure alert fires instead of a restart loop. 3 is the free-space stop.
pub(crate) const EXIT_REORG: i32 = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UndoRecord {
    pub(crate) height: u32,
    pub(crate) hash: [u8; 32],
    pub(crate) prev: [u8; 32],
    /// the coin log's undo body (lz::cl_undo_body), as pending.bin carries it
    pub(crate) cl: Vec<u8>,
    /// the block re-created a coin over its own spent record (TrieStore::undo cannot restore that)
    pub(crate) trie_replaced: bool,
    pub(crate) trie_marks: Vec<libzync::ttree::Key>,
    pub(crate) trie_inserts: Vec<libzync::ttree::Key>,
    /// every BIP30 key the block touched and its value before the block (None: absent)
    pub(crate) smt_prior: Vec<([u8; 32], Option<u32>)>,
}

/// The record prep_block leaves for the block it just applied (taken by lz_prep_cmd).
pub(crate) static LAST: std::sync::Mutex<Option<UndoRecord>> = std::sync::Mutex::new(None);

impl UndoRecord {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(160 + self.cl.len() + 64 * (self.trie_marks.len() + self.trie_inserts.len()) + 37 * self.smt_prior.len());
        b.extend_from_slice(UNDO_MAGIC);
        b.extend_from_slice(&self.height.to_le_bytes());
        b.extend_from_slice(&self.hash);
        b.extend_from_slice(&self.prev);
        b.extend_from_slice(&(self.cl.len() as u64).to_le_bytes());
        b.extend_from_slice(&self.cl);
        b.push(self.trie_replaced as u8);
        for keys in [&self.trie_marks, &self.trie_inserts] {
            b.extend_from_slice(&(keys.len() as u32).to_le_bytes());
            for k in keys { b.extend_from_slice(k); }
        }
        b.extend_from_slice(&(self.smt_prior.len() as u32).to_le_bytes());
        for (k, v) in &self.smt_prior {
            b.extend_from_slice(k);
            b.push(v.is_some() as u8);
            b.extend_from_slice(&v.unwrap_or(0).to_le_bytes());
        }
        let sum = Sha256::digest(&b);
        b.extend_from_slice(&sum);
        b
    }

    /// The record in `b`, or why not: a wrong magic, a checksum that fails (a torn or damaged file), or lengths that
    /// do not add up to exactly the file.
    pub(crate) fn decode(b: &[u8]) -> Result<UndoRecord, String> {
        if b.len() < 8 + 32 || &b[..8] != UNDO_MAGIC { return Err("not an undo record (magic)".into()); }
        let (body, sum) = b.split_at(b.len() - 32);
        if Sha256::digest(body)[..] != sum[..] { return Err("checksum mismatch (torn or damaged)".into()); }
        let mut o = 8usize;
        let mut take = |n: usize| -> Result<&[u8], String> {
            let s = body.get(o..o.checked_add(n).ok_or("length overflow")?).ok_or_else(|| format!("truncated at byte {o}"))?;
            o += n;
            Ok(s)
        };
        let u32_ = |s: &[u8]| u32::from_le_bytes(s.try_into().unwrap());
        let height = u32_(take(4)?);
        let hash: [u8; 32] = take(32)?.try_into().unwrap();
        let prev: [u8; 32] = take(32)?.try_into().unwrap();
        let n = u64::from_le_bytes(take(8)?.try_into().unwrap());
        let cl = take(usize::try_from(n).map_err(|_| "coin log body length")?)?.to_vec();
        let trie_replaced = match take(1)?[0] { 0 => false, 1 => true, x => return Err(format!("replaced flag {x}")) };
        let mut keys = || -> Result<Vec<libzync::ttree::Key>, String> {
            let n = u32_(take(4)?) as usize;
            (0..n).map(|_| Ok(take(64)?.try_into().unwrap())).collect()
        };
        let trie_marks = keys()?;
        let trie_inserts = keys()?;
        let n = u32_(take(4)?) as usize;
        let mut smt_prior = Vec::with_capacity(n.min(1 << 16));
        for _ in 0..n {
            let k: [u8; 32] = take(32)?.try_into().unwrap();
            let present = match take(1)?[0] { 0 => false, 1 => true, x => return Err(format!("present flag {x}")) };
            let v = u32_(take(4)?);
            smt_prior.push((k, present.then_some(v)));
        }
        if o != body.len() { return Err(format!("{} trailing bytes", body.len() - o)); }
        // the coin log body must parse as pending.bin's does (k spends of 40 bytes, m appends of 32)
        let rd = |at: usize| cl.get(at..at + 8).map(|x| u64::from_le_bytes(x.try_into().unwrap()));
        let ok = (|| {
            let k = usize::try_from(rd(8)?).ok()?;
            let at_m = 16usize.checked_add(40usize.checked_mul(k)?)?;
            let m = usize::try_from(rd(at_m)?).ok()?;
            (at_m.checked_add(8)?.checked_add(32usize.checked_mul(m)?)? == cl.len()).then_some(())
        })();
        ok.ok_or("the coin log body does not parse")?;
        Ok(UndoRecord { height, hash, prev, cl, trie_replaced, trie_marks, trie_inserts, smt_prior })
    }
}

/// `<out>/reorg`.
pub(crate) fn dir(out: &str) -> std::path::PathBuf { std::path::Path::new(out).join("reorg") }

fn path(d: &std::path::Path, h: u32) -> std::path::PathBuf { d.join(format!("u_{h}.bin")) }

/// J: HAZYNC_PREP_UNDO_KEEP, else 2,016 for a run with on-disk stores (HAZYNC_COINLOG_DIR) and 0 (no records) for an
/// in-memory one, whose save points are whole snapshots.
pub(crate) fn keep() -> u32 {
    match std::env::var("HAZYNC_PREP_UNDO_KEEP") {
        Ok(v) => v.trim().parse().unwrap_or_else(|_| panic!("HAZYNC_PREP_UNDO_KEEP: not a number: {v}")),
        Err(_) => if crate::lz::cl_dir().is_some() { UNDO_KEEP_DEFAULT } else { 0 },
    }
}

/// Write block `r.height`'s record (tmp, then renamed; made durable at the next save point by `sync_since`). Nothing
/// is pruned here: the window is cut only at save points (`prune_to`), so a crash never leaves fewer than J records
/// below the save point a restart resumes from (review of P1, M3: pruning u_<h-J> per block left J - (c - A) after a
/// crash at c and a restart from A).
pub(crate) fn write(d: &std::path::Path, r: &UndoRecord) {
    let p = path(d, r.height);
    let tmp = d.join(format!("u_{}.bin.tmp", r.height));
    std::fs::write(&tmp, r.encode()).unwrap_or_else(|e| panic!("{}: {e}", tmp.display()));
    std::fs::rename(&tmp, &p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
}

/// Every record in (`from`, `to`] synced, then the directory: called at a save point before its state file is
/// written, so a complete save point never has a torn record below it. Returns the highest height in the range whose
/// record is MISSING (review L6: the window then starts above it; nothing is silently assumed), or None.
pub(crate) fn sync_since(d: &std::path::Path, from: u32, to: u32) -> Option<u32> {
    let mut missing = None;
    for h in from.saturating_add(1)..=to {
        // opened for writing (no truncation): Windows' FlushFileBuffers refuses a read-only handle (as prep's state files)
        match std::fs::OpenOptions::new().write(true).open(path(d, h)) {
            Ok(f) => f.sync_data().unwrap_or_else(|e| panic!("{}: {e}", path(d, h).display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => missing = Some(h),
            Err(e) => panic!("{}: {e}", path(d, h).display()),
        }
    }
    crate::lz::cl_sync_dir(d);
    missing
}

/// At a save point: remove the records in (`from`, `to`], the ones that fell out of the window (to = save - J).
pub(crate) fn prune_to(d: &std::path::Path, from: u32, to: u32) {
    for h in from.saturating_add(1)..=to {
        match std::fs::remove_file(path(d, h)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("{}: {e}", path(d, h).display()),
        }
    }
}

/// The lowest height of the unbroken run of records ending at `anchor` (at most `keep` of them): the records reach a
/// fork F when F + 1 >= this. `anchor + 1` when there is no record at the anchor (a run that wrote none yet, e.g. one
/// started by a host from before P1: its records begin with its next block).
pub(crate) fn undo_from(d: &std::path::Path, anchor: u32, keep: u32) -> u32 {
    let mut k = anchor;
    while k >= 1 && anchor - k < keep && path(d, k).exists() { k -= 1; }
    k + 1
}

/// prep's real position, `<out>/PREP_TIP.json` (review of P2, M2): the height and hash of the newest block its STORES
/// hold, J, and the lowest height whose undo record exists (`undo_from`; records reach heights undo_from..height).
/// The highest state_<h>.json is not that: a crashed run leaves state files above the save point it restarts from.
/// Written at the start (the anchor), after every block's state file, and durably at every save point; always by an
/// atomic rename, so a reader sees one whole version. Only the fetcher reads it (fetch_block_rpc.py).
pub(crate) fn write_tip(out: &str, height: u32, hash: &[u8; 32], keep: u32, undo_from: u32, durable: bool) {
    let mut h = *hash;
    h.reverse();
    let body = format!("{{\"height\": {height}, \"hash\": \"{}\", \"undo_keep\": {keep}, \"undo_from\": {undo_from}}}\n", crate::hex(&h));
    let p = format!("{out}/PREP_TIP.json");
    if durable { crate::lz::write_atomic(&p, body.as_bytes()); return; }
    let tmp = format!("{p}.tmp");
    std::fs::write(&tmp, body.as_bytes()).unwrap_or_else(|e| panic!("{tmp}: {e}"));
    std::fs::rename(&tmp, &p).unwrap_or_else(|e| panic!("{p}: {e}"));
}

/// At the start of a run from the state at `anchor`: keep exactly the records of (anchor - keep, anchor]. Records above
/// the anchor describe blocks a crash undid (they are written again as prep re-applies them); records below the window
/// are left over from a larger J or an older run. A `.tmp` is a write that never finished. Returns how many it removed.
pub(crate) fn prune_at_start(d: &std::path::Path, anchor: u32, keep: u32) -> usize {
    let Ok(it) = std::fs::read_dir(d) else { return 0 };
    let mut n = 0;
    for e in it.flatten() {
        let f = e.file_name();
        let f = f.to_string_lossy();
        let h = f.strip_prefix("u_").and_then(|x| x.strip_suffix(".bin")).and_then(|x| x.parse::<u32>().ok());
        let stale = match h {
            Some(h) => h > anchor || anchor.checked_sub(keep).is_some_and(|lo| h <= lo),
            None => f.starts_with("u_") && f.ends_with(".bin.tmp"),
        };
        if stale {
            std::fs::remove_file(e.path()).unwrap_or_else(|x| panic!("{}: {x}", e.path().display()));
            n += 1;
        }
    }
    if n > 0 { crate::lz::cl_sync_dir(d); }
    n
}

/// The block files' reorg markers (written by fetch_block_rpc.py, REORG.md §4): while one is in the block directory,
/// prep applies nothing (P2: every reorg below prep's tip is a stop for a person; P3 rewinds within W by itself).
pub(crate) fn marker(blocks_dir: &str) -> Option<std::path::PathBuf> {
    ["REORG", "REORG_TOO_DEEP"].iter().map(|m| std::path::Path::new(blocks_dir).join(m)).find(|p| p.exists())
}

/// Stop for a reorg: say why and exit with EXIT_REORG. Nothing is half-written at the points this is called from: a
/// restart from the newest save point undoes the blocks since it, as after any crash.
pub(crate) fn stop(why: &str) -> ! {
    println!("PREP_STOPPED_REORG {why}");
    println!("  a person decides: docs/libzync/REORG.md §11 (remove the marker in the block directory once the stores are on the active chain)");
    std::process::exit(EXIT_REORG);
}

/// The exit code of a stop FOR a rewind (P3, HAZYNC_REORG_AUTO=1, a fork within W): not in the units'
/// RestartPreventExitStatus, so systemd starts prep again, and libzync-prep-run.sh runs the rewind before it does.
pub(crate) const EXIT_REWIND: i32 = 5;

/// Automatic rewinds (P3) on: HAZYNC_REORG_AUTO=1. Off by default: REORG.md §13 turns them on with the coordinator's
/// side (P4), so until then every fork below prep's tip stays a stop for a person, as in P2.
pub(crate) fn auto() -> bool { std::env::var("HAZYNC_REORG_AUTO").as_deref() == Ok("1") }

/// The fork of a `REORG` marker the fetcher classed `auto` (depth within W, the records reaching it), else None.
pub(crate) fn auto_fork(blocks_dir: &str) -> Option<u32> {
    let m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(std::path::Path::new(blocks_dir).join("REORG")).ok()?).ok()?;
    (m["class"].as_str() == Some("auto")).then(|| m["fork"].as_u64().map(|f| f as u32)).flatten()
}

/// A marker while prep runs: with automatic rewinds on and a fork the fetcher classed `auto`, exit EXIT_REWIND (the
/// stores are journaled; libzync-prep-run.sh rewinds them to the fork before prep starts again); else stop (exit 4).
pub(crate) fn stop_or_rewind(blocks_dir: &str, why: &str) -> ! {
    if auto() {
        if let Some(f) = auto_fork(blocks_dir) {
            println!("PREP_STOPPED_FOR_REWIND {why}: the fork at {f} is within W; the restart rewinds the stores to it (REORG.md §6)");
            std::process::exit(EXIT_REWIND);
        }
    }
    stop(why)
}

/// A block file's hash (the state's tip_hash order), from its header fields, as prep_block computes it.
pub(crate) fn hash_of(j: &serde_json::Value) -> Option<[u8; 32]> {
    let merkle: [u8; 32] = crate::rev(crate::hx(j["merkle"].as_str()?)).try_into().ok()?;
    let h = crate::build_header_v(j["version"].as_i64().unwrap_or(1) as i32, j["prev"].as_str()?, &merkle,
                                  j["time"].as_u64()? as u32, j["bits"].as_u64()? as u32, j["nonce"].as_u64()? as u32);
    Some(crate::header_hash(&h))
}

fn state_tip(out: &str, h: u32) -> Option<[u8; 32]> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(format!("{out}/state_{h}.json")).ok()?).ok()?;
    let t: Vec<u8> = v["tip_hash"].as_array()?.iter().map(|x| x.as_u64().map(|b| b as u8)).collect::<Option<_>>()?;
    t.try_into().ok()
}

fn disp(b: &[u8; 32]) -> String { let mut x = *b; x.reverse(); crate::hex(&x) }

/// Before block h is applied: is there a state_<h>.json of an earlier run for another block? Then the states h.. on disk
/// are another chain's (the node reorged while this run replays from its save point): write the fetcher's marker for
/// a fork at h-1 (unless one is there) and stop as for any marker (exit 5 with automatic rewinds and a depth within W,
/// else 4). A state that is there but does not read is a stop too (its states are written by rename: damage, not a
/// write in progress). Static block files (not fed by the fetcher, `consume` false): a stop with no marker written into
/// a directory that is not the fetcher's. Returns when there is no such state.
pub(crate) fn stale_state_fork(out: &str, blocks_dir: &str, h: u32, prev_tip: &[u8; 32], j: &serde_json::Value, consume: bool) {
    // test only (RISC0_DEV_MODE): the check off, for rewind_test.sh's positive control
    // Refused in board mode (leaf bundles written), and loud when it fires (review round 5, LOW 1).
    if crate::verify::dev_mode_env() && std::env::var("HAZYNC_TEST_SKIP_STALE_CHECK").as_deref() == Ok("1") {
        if std::env::var("HAZYNC_PREP_LEAF_BUNDLES").is_ok_and(|v| !v.is_empty()) {
            eprintln!("HAZYNC_TEST_SKIP_STALE_CHECK refused: this prep writes leaf bundles (a board run); the check stays on");
        } else {
            eprintln!("WARNING: HAZYNC_TEST_SKIP_STALE_CHECK=1 under RISC0_DEV_MODE: the replay-window reorg check is OFF (testing only)");
            return;
        }
    }
    let p = format!("{out}/state_{h}.json");
    // Missing, or zero-length: the states between save points are written by rename without an fsync (synced only at
    // the next save point), so a power loss can leave them empty. That is crash damage, not another chain: the replay
    // overwrites it, as it did before this check (review round 5, MED). The board fails closed on it meanwhile: an
    // empty state reads as none, so verify-any has no end state and the commit-time hash check refuses.
    match std::fs::metadata(&p) {
        Ok(m) if m.len() > 0 => {}
        _ => return,
    }
    let Some(old) = state_tip(out, h) else {
        // a non-empty file that does not parse: not a torn write (renamed whole), so damage worth a person's look
        println!("PREP_STOPPED_REORG before block {h}: {p}, left by an earlier run above the save point, is not empty and does not read: \
                  its chain cannot be compared");
        println!("  a person: look at it; if the node's chain at {h} is the one prep is replaying (fetch_block_rpc.py fork-point <out>), \
                  move {p} aside and start prep again (the replay writes it anew); if not, rewind (docs/libzync/REORG.md §11 step 3)");
        std::process::exit(EXIT_REORG);
    };
    let Some(new) = hash_of(j) else { return };
    if old == new { return; }
    let fork = h - 1;
    let mut orphaned = Vec::new();
    let mut k = h;
    while let Some(x) = state_tip(out, k) { orphaned.push(serde_json::json!([k, disp(&x)])); k += 1; }
    let top = k - 1;
    let w: u32 = std::env::var("HAZYNC_REORG_AUTO_DEPTH").ok().and_then(|v| v.parse().ok()).unwrap_or(288);
    let depth = top - fork;
    let why = format!("before block {h}: state_{h}.json of an earlier run is block {}, the block file {}: the states \
                       {h}..={top} on disk are another chain's (a reorg inside the replay window)", disp(&old), disp(&new));
    if !consume { stop(&format!("{why}; static block files: nothing is written into {blocks_dir}")); }
    if marker(blocks_dir).is_none() {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let m = serde_json::json!({"fork": fork, "fork_hash": disp(prev_tip), "prep_tip": top, "depth": depth,
            "class": if depth <= w { "auto" } else { "manual" }, "W": w, "orphaned": orphaned, "new": [],
            "source": "prep: stale state files above its save point", "at": now});
        crate::lz::write_atomic(&format!("{blocks_dir}/REORG"), m.to_string().as_bytes());
    }
    if auto() && auto_fork(blocks_dir).is_some() { stop_or_rewind(blocks_dir, &why) }
    // prep's OWN marker (round 4, L7): the recovery is the rewind, not "remove the marker"
    println!("PREP_STOPPED_REORG {why}");
    println!("  prep wrote {blocks_dir}/REORG (source: prep): the state files {h}..={top} are another chain's. A person rewinds: \
              libzync-prep-rewind <out> <blocks> [--allow-deep] (docs/libzync/REORG.md §11 step 3), then starts the units");
    std::process::exit(EXIT_REORG);
}

/// A block file's `prev` (display hex) as the state's tip_hash byte order.
pub(crate) fn prev_of(j: &serde_json::Value) -> Option<[u8; 32]> {
    let x = j["prev"].as_str()?;
    if x.len() != 64 { return None; }
    let mut b = [0u8; 32];
    for i in 0..32 { b[31 - i] = u8::from_str_radix(x.get(2 * i..2 * i + 2)?, 16).ok()?; }
    Some(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("lz-reorg-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn rec(h: u32) -> UndoRecord {
        // a coin log body as pending.bin's: 10 leaves before, one spend at 3, two appends
        let mut cl = Vec::new();
        cl.extend_from_slice(&10u64.to_le_bytes());
        cl.extend_from_slice(&1u64.to_le_bytes());
        cl.extend_from_slice(&3u64.to_le_bytes());
        cl.extend_from_slice(&[7; 32]);
        cl.extend_from_slice(&2u64.to_le_bytes());
        cl.extend_from_slice(&[1; 32]);
        cl.extend_from_slice(&[2; 32]);
        UndoRecord { height: h, hash: [h as u8; 32], prev: [h.wrapping_sub(1) as u8; 32], cl, trie_replaced: false,
                     trie_marks: vec![[9; 64]], trie_inserts: vec![[1; 64], [2; 64]],
                     smt_prior: vec![([5; 32], None), ([6; 32], Some(3))] }
    }

    #[test]
    fn records_round_trip_and_refuse_damage() {
        let r = rec(1234);
        let b = r.encode();
        // 8 + 4 + 32 + 32 + 8 + (24 + 40 + 64) + 1 + 4 + 64 + 4 + 128 + 4 + 2 * 37 + 32
        assert_eq!(b.len(), 8 + 4 + 64 + 8 + 128 + 1 + 4 + 64 + 4 + 128 + 4 + 74 + 32);
        assert_eq!(UndoRecord::decode(&b), Ok(r.clone()));
        let empty = UndoRecord { cl: { let mut c = 5u64.to_le_bytes().to_vec(); c.extend([0; 16]); c }, trie_marks: vec![],
                                 trie_inserts: vec![], smt_prior: vec![], trie_replaced: true, ..rec(1) };
        assert_eq!(UndoRecord::decode(&empty.encode()), Ok(empty));
        // every single-byte change is refused, and so is every truncation
        for i in 0..b.len() {
            let mut x = b.clone();
            x[i] ^= 0x40;
            assert!(UndoRecord::decode(&x).is_err(), "byte {i} flipped");
        }
        for n in 0..b.len() { assert!(UndoRecord::decode(&b[..n]).is_err(), "cut to {n}"); }
        // a record whose checksum is right but whose coin log body does not parse is refused too
        let mut bad = r.clone();
        bad.cl.pop();
        assert!(UndoRecord::decode(&bad.encode()).unwrap_err().contains("coin log body"));
    }

    #[test]
    fn the_window_is_cut_at_save_points_and_never_below_j() {
        let d = tmpdir("window");
        let keep = 5;
        let have = |d: &std::path::Path| -> Vec<u32> {
            let mut v: Vec<u32> = std::fs::read_dir(d).unwrap().flatten()
                .filter_map(|e| e.file_name().to_string_lossy().strip_prefix("u_")?.strip_suffix(".bin")?.parse().ok()).collect();
            v.sort();
            v
        };
        // save points every 4 blocks: at 4, 8, 12 the window is cut to (save - J, ...]; between them it grows
        let mut pruned = 0;
        for h in 1..=14 {
            write(&d, &rec(h));
            if h % 4 == 0 {
                assert_eq!(sync_since(&d, h - 4, h), None);
                let to = h.saturating_sub(keep);
                prune_to(&d, pruned, to);
                pruned = to;
            }
        }
        assert_eq!(have(&d), (8..=14).collect::<Vec<_>>(), "(12 - J, 14]: cut at the save point 12 only");
        assert_eq!(UndoRecord::decode(&std::fs::read(d.join("u_10.bin")).unwrap()).unwrap(), rec(10));
        // a crash at 14 and a restart from the save point 12: 13 and 14 (blocks the crash undid) go, and the window
        // below 12 is still a whole J (M3: per-block pruning left only J - 2); a stray from a larger J goes too, and
        // an unfinished write
        write(&d, &rec(2));
        std::fs::write(d.join("u_15.bin.tmp"), b"x").unwrap();
        std::fs::write(d.join("REWIND"), b"not ours").unwrap();
        assert_eq!(prune_at_start(&d, 12, keep), 4);
        assert_eq!(have(&d), vec![8, 9, 10, 11, 12], "(12 - J, 12] after the restart: J records");
        assert_eq!(undo_from(&d, 12, keep), 8);
        // a hole in the window: sync_since names it, and undo_from starts above it
        std::fs::remove_file(d.join("u_10.bin")).unwrap();
        assert_eq!(sync_since(&d, 8, 12), Some(10));
        assert_eq!(undo_from(&d, 12, keep), 11);
        assert_eq!(undo_from(&d, 20, keep), 21, "no record at the anchor: none reach below the next block");
        // J = 0 (records off): every record goes
        assert_eq!(prune_at_start(&d, 12, 0), 4);
        assert!(std::fs::read_dir(&d).unwrap().flatten().all(|e| e.file_name() == "REWIND"));
        // a missing directory is nothing to prune
        assert_eq!(prune_at_start(&d.join("none"), 10, keep), 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_tip_file_is_prep_position_in_display_order() {
        let d = tmpdir("tip");
        let out = d.to_str().unwrap();
        let mut hash = [0u8; 32];
        hash[0] = 0xab;
        write_tip(out, 105, &hash, 2016, 90, false);
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(d.join("PREP_TIP.json")).unwrap()).unwrap();
        assert_eq!((v["height"].as_u64(), v["undo_keep"].as_u64(), v["undo_from"].as_u64()), (Some(105), Some(2016), Some(90)));
        assert_eq!(v["hash"].as_str().unwrap(), format!("{}ab", "00".repeat(31)), "display order, as getblockhash");
        write_tip(out, 106, &hash, 2016, 90, true);
        assert!(std::fs::read_to_string(d.join("PREP_TIP.json")).unwrap().contains("\"height\": 106"));
        assert!(!d.join("PREP_TIP.json.tmp").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A record's trie KEYS undo its block (in the session that applied it, the undo P3 builds on): random blocks,
    /// undone newest first back to two earlier blocks, give those blocks' roots again.
    #[test]
    fn the_trie_keys_of_a_record_undo_its_block() {
        use libzync::trie_store::TrieStore;
        let mut st = TrieStore::new(true);
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut rnd = || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x };
        let (mut unspent, mut roots, mut recs): (Vec<libzync::ttree::Key>, Vec<_>, Vec<UndoRecord>) = (vec![], vec![st.root()], vec![]);
        for blk in 1..=60u32 {
            let mut marks = Vec::new();
            for _ in 0..(rnd() % 4) {
                if unspent.is_empty() { break; }
                let k = unspent.swap_remove((rnd() % unspent.len() as u64) as usize);
                marks.push((k, [blk, 1, 0, 0, 0, 0, 0, 0]));
            }
            let mut ins = Vec::new();
            for _ in 0..(1 + rnd() % 6) {
                let mut k = [0u8; 64];
                for b in k.iter_mut() { *b = rnd() as u8; }
                k[0] &= 0x0f; // shared address prefixes
                if rnd() % 3 == 0 { k[1..4].fill(0); }
                let spent = rnd() % 5 == 0;
                if !spent { unspent.push(k); }
                ins.push((k, spent.then_some([blk, 2, 0, 0, 0, 0, 0, 0]), rnd() >> 30));
            }
            st.apply(&marks, &ins).unwrap();
            let r = UndoRecord { trie_replaced: st.last_apply_replaced() == Some(true),
                                 trie_marks: marks.iter().map(|m| m.0).collect(), trie_inserts: ins.iter().map(|i| i.0).collect(), ..rec(blk) };
            recs.push(UndoRecord::decode(&r.encode()).unwrap());
            roots.push(st.root());
        }
        for (from, to) in [(60usize, 35usize), (35, 0)] {
            for r in recs[to..from].iter().rev() {
                assert!(!r.trie_replaced);
                // keys only: the spender and value play no part in an undo
                let m: Vec<(libzync::ttree::Key, libzync::coinlog::W)> = r.trie_marks.iter().map(|k| (*k, [0; 8])).collect();
                let i: Vec<(libzync::ttree::Key, Option<libzync::coinlog::W>, u64)> = r.trie_inserts.iter().map(|k| (*k, None, 0)).collect();
                st.undo(&m, &i).unwrap_or_else(|e| panic!("block {}: {e:?}", r.height));
            }
            assert_eq!(st.root(), roots[to], "undone back to block {to}");
        }
    }

    /// A record's BIP30 priors undo its block's changes to the BIP30 tree (smt_advance_ex's second return, kept by
    /// prep since P1): inserts of new coinbases and tracked transactions, and tracked spends, two from one coinbase.
    #[test]
    fn the_bip30_priors_of_a_record_undo_its_block() {
        let mut smt = crate::Smt::new();
        let t = |i: u8| [i; 32];
        // (coinbase, its outputs, coinbase spends with their tracked bits, tracked transactions created)
        let blocks: Vec<([u8; 32], u32, Vec<([u8; 32], bool)>, Vec<([u8; 32], u32)>)> = vec![
            (t(1), 2, vec![], vec![(t(10), 3)]),
            (t(2), 1, vec![(t(10), true), (t(10), true), (t(1), false)], vec![]),
            (t(3), 1, vec![(t(10), true)], vec![(t(11), 1)]),
            (t(4), 2, vec![(t(11), true)], vec![]),
        ];
        let (mut roots, mut recs) = (vec![smt.root()], Vec::new());
        for (h, (cb, n, spends, tracked)) in blocks.iter().enumerate() {
            let sp: Vec<[u8; 32]> = spends.iter().map(|s| s.0).collect();
            let tr: Vec<bool> = spends.iter().map(|s| s.1).collect();
            let (_, prior) = crate::smt_advance_ex(&mut smt, *cb, *n, &sp, false, &tr, tracked);
            let r = UndoRecord { smt_prior: prior, ..rec(h as u32 + 1) };
            recs.push(UndoRecord::decode(&r.encode()).unwrap());
            roots.push(smt.root());
        }
        assert_ne!(roots[1], roots[4]);
        for r in recs.iter().rev() {
            for (k, v) in &r.smt_prior {
                match v { Some(v) => smt.insert(*k, *v), None => smt.remove(k) }
            }
            assert_eq!(smt.root(), roots[r.height as usize - 1], "undone back to block {}", r.height - 1);
        }
        assert_eq!(smt.len(), 0);
    }

    #[test]
    fn prev_of_reads_display_hex_into_state_order() {
        let j = serde_json::json!({"prev": "000000002a22cfee1f2c846adbd12b3e183d4f97683f85dad08a79780a84bd55"});
        let p = prev_of(&j).unwrap();
        assert_eq!(p[0], 0x55);
        assert_eq!(p[31], 0x00);
        assert!(prev_of(&serde_json::json!({"prev": "zz"})).is_none());
        assert!(prev_of(&serde_json::json!({})).is_none());
    }
}
