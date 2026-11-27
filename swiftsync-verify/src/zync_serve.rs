//! `zync-serve <history.bin> <addr:port>`: the path service (ROADMAP C2, ZYNCAPPS.md §8). Serves the same CoinFact JSON
//! as `zync-path`, over plain HTTP GET, for the tip the history ends at. It holds no secrets and proves nothing by
//! itself: every answer is checked by the client against a verified tip receipt (`swiftsync-verify --coin`), so the
//! server needs no trust. ⚠ No payment hook yet, one request at a time, whole history in memory (C1 for mainnet).
//!
//!   GET /tip                      {"tip": h}
//!   GET /coin/<at>/<position>     CoinFact (at = tip for the state now, or an earlier height)
//!   GET /find/<coin leaf hex>     {"position": p}
//! With a trie snapshot (`zync-serve <history.bin> <addr:port> <state_<tip>.trie>`), counting-trie facts at the tip
//! (TrieFact, checked with `swiftsync-verify --trie`):
//!   GET /trie/coin/<scriptPubKey hex>/<txid>/<vout>   unspent / spent by / absent. A coin the trie HOLDS verifies only with
//!                                                     its creating transaction attached as `origin` (`zync-path tx`):
//!                                                     this server has no node, so the client adds it (TrieFact::origin)
//!   GET /trie/balance/<scriptPubKey hex>              unspent count and value
//!   GET /trie/history/<scriptPubKey hex>              every coin ever paid to the script
//!
//! TIP MODE (docs/libzync/ZAPP_FOLLOWER.md §10 Z2-Z4; built with `--features tip --profile zync-serve`):
//! `zync-serve --tip --at-file <at.json> <addr:port>`. No history: counting-trie facts only, read from a prep run's disk
//! trie at the roots it logged (`roots.log`), at one or more pinned heights. `at.json` (written by the zAPP follower) is
//! a list of {height, hash (the block, internal-order hex as verify-any prints out_bhash), trie_root (the proof's state's
//! trie_root, hex), trie_dir}; each is opened read-only and checked against its trie_root at start (any that does not
//! open: exit 1, and the follower puts the previous set back).
//!   GET /tip                                  {"tip": <newest height>, "at": [every height served]}
//!   GET /trie/...?at=<h>                      the fact at h (default: the newest); a height not served: 404
//! A walk the store moved under (prep restarted or rewound, or the root released) is answered 503 (ask again: the
//! follower repins); a history past ZYNC_WALK_CAP records (100,000) is refused with 400.
use libzync::history::{block_path, decode_all, replay, Rec};
use std::io::{BufRead, BufReader, Write};
// ⚠ Single-threaded, and /coin replays the history per request: a production service caches the tip replay (C1).
use swiftsync_verify::{hex, hex32, CoinFact};

fn fact(recs: &[Rec], tip: u32, at: u32, pos: u64) -> Result<String, String> {
    let r = replay(recs, at, tip).map_err(|e| e.to_string())?;
    if pos >= r.log.len() {
        return Err(format!("position {pos}: the log after {at} has {} coins", r.log.len()));
    }
    let f = CoinFact {
        at, pos,
        coin: hex(&r.coins[pos as usize]),
        spent_by: r.spenders.get(&pos).copied(),
        path: r.log.path(pos).iter().map(|x| hex(x)).collect(),
        log_leaves: r.log.len(),
        log_value: r.value,
        log_roots: r.log.roots().iter().map(|x| x.map(|h| hex(&h))).collect(),
        block: (at < tip).then(|| (hex(&recs[at as usize].hash), block_path(&r.block_leaves, at as u64).0.iter().map(|x| hex(x)).collect())),
        block_undo: if at < tip { recs[at as usize].undo.map(|u| hex(&u)) } else { None },
        preimage: None };
    Ok(serde_json::to_string(&f).unwrap())
}

fn answer(recs: &[Rec], tip: u32, coins: &[[u8; 32]], trie: Option<&libzync::trie_store::TrieStore>, path: &str) -> (u16, String) {
    let p: Vec<&str> = path.trim_matches('/').split('/').collect();
    let err = |c: u16, m: String| (c, serde_json::json!({ "error": m }).to_string());
    match p.as_slice() {
        ["tip"] => (200, serde_json::json!({ "tip": tip }).to_string()),
        ["coin", at, pos] => match (at.parse::<u32>(), pos.parse::<u64>()) {
            (Ok(at), Ok(pos)) => fact(recs, tip, at, pos).map(|b| (200, b)).unwrap_or_else(|e| err(404, e)),
            _ => err(400, "want /coin/<at>/<position>".into()),
        },
        ["find", leaf] => match hex32(&leaf.to_lowercase()) {
            Some(l) => match coins.iter().position(|c| *c == l) {
                Some(i) => (200, serde_json::json!({ "position": i }).to_string()),
                None => err(404, "no such coin leaf in the log".into()),
            },
            None => err(400, "want 64 hex digits".into()),
        },
        ["trie", kind, spk, rest @ ..] => {
            let Some(t) = trie else { return err(404, "no trie snapshot loaded".into()) };
            let Some(spk) = swiftsync_verify::unhex(spk) else { return err(400, "scriptPubKey: not hex".into()) };
            let (txid, vout) = match (*kind, rest) {
                ("coin", [txid, vout]) => match vout.parse() { Ok(v) => (Some(*txid), Some(v)), Err(_) => return err(400, "vout?".into()) },
                ("balance" | "history", []) => (None, None),
                _ => return err(400, "want /trie/coin/<spk>/<txid>/<vout>, /trie/balance/<spk> or /trie/history/<spk>".into()),
            };
            match swiftsync_verify::make_trie_fact(t, kind, &spk, txid, vout) {
                Ok(f) => (200, serde_json::to_string(&f).unwrap()),
                Err(e) => err(400, e),
            }
        }
        _ => err(404, "unknown path".into()),
    }
}

/// Tip mode: the heights served, each a read-only reader of a prep run's disk trie at its logged root.
#[cfg(feature = "tip")]
mod tip {
    use libzync::trie_store::{Reader, WalkErr};

    pub struct At { pub height: u32, pub reader: Reader }

    fn h32(x: &serde_json::Value, k: &str) -> Result<[u8; 32], String> {
        x[k].as_str().and_then(|s| swiftsync_verify::hex32(&s.to_lowercase())).ok_or_else(|| format!("at.json: {k} is not 64 hex digits"))
    }

    /// Open every height of `at.json` (cap: the records one walk may read).
    pub fn load(path: &str, cap: u64) -> Result<Vec<At>, String> {
        let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?)
            .map_err(|e| format!("{path}: {e}"))?;
        let list = j.as_array().filter(|l| !l.is_empty()).ok_or_else(|| format!("{path}: a non-empty list of heights"))?;
        let mut out = Vec::new();
        for x in list {
            let h = x["height"].as_u64().filter(|h| *h <= u32::MAX as u64).ok_or("at.json: height")? as u32;
            let dir = x["trie_dir"].as_str().ok_or("at.json: trie_dir")?;
            let r = Reader::open(std::path::Path::new(dir), h as u64, &h32(x, "hash")?, Some(&h32(x, "trie_root")?), cap)
                .map_err(|e| format!("height {h} in {dir}: {e}"))?;
            out.push(At { height: h, reader: r });
        }
        out.sort_by_key(|a| a.height);
        out.dedup_by_key(|a| a.height);
        Ok(out)
    }

    /// (status, body) for one request path (with its query, if any).
    pub fn answer(ats: &[At], full: &str) -> (u16, String) {
        let err = |c: u16, m: String| (c, serde_json::json!({ "error": m }).to_string());
        let (path, query) = full.split_once('?').unwrap_or((full, ""));
        let mut at = None;
        for kv in query.split('&').filter(|x| !x.is_empty()) {
            match kv.split_once('=') {
                Some(("at", v)) => match v.parse::<u32>() { Ok(h) => at = Some(h), Err(_) => return err(400, "at: a height".into()) },
                _ => return err(400, format!("unknown parameter {kv:?} (only at=<height>)")),
            }
        }
        let heights: Vec<u32> = ats.iter().map(|a| a.height).collect();
        let newest = *heights.last().expect("at least one height");
        let p: Vec<&str> = path.trim_matches('/').split('/').collect();
        if p == ["tip"] { return (200, serde_json::json!({ "tip": newest, "at": heights }).to_string()); }
        let h = at.unwrap_or(newest);
        let Some(a) = ats.iter().find(|a| a.height == h) else {
            return err(404, format!("facts are served at {heights:?}, not at {h}"));
        };
        match p.as_slice() {
            ["trie", kind, spk, rest @ ..] => {
                let Some(spk) = swiftsync_verify::unhex(spk) else { return err(400, "scriptPubKey: not hex".into()) };
                let (txid, vout) = match (*kind, rest) {
                    ("coin", [txid, vout]) => match vout.parse() { Ok(v) => (Some(*txid), Some(v)), Err(_) => return err(400, "vout?".into()) },
                    ("balance" | "history", []) => (None, None),
                    _ => return err(400, "want /trie/coin/<spk>/<txid>/<vout>, /trie/balance/<spk> or /trie/history/<spk>".into()),
                };
                match a.reader.walk(|s| swiftsync_verify::make_trie_fact(s, kind, &spk, txid, vout)) {
                    Ok(Ok(f)) => (200, serde_json::to_string(&f).unwrap()),
                    Ok(Err(e)) => err(400, e),
                    Err(WalkErr::TooLarge(n)) => err(400, format!("this fact needs more than {n} trie records (ZYNC_WALK_CAP): too large to serve")),
                    Err(WalkErr::Moved(m)) => err(503, format!("the trie moved under this request ({m}): ask again")),
                    Err(WalkErr::Damaged(m)) => err(503, format!("the trie could not be read here ({m}): ask again")),
                }
            }
            ["coin", ..] | ["find", ..] => err(404, "tip mode serves counting-trie facts only (/trie/*): no coin log here".into()),
            _ => err(404, "unknown path".into()),
        }
    }
}

fn serve(addr: &str, mut handle: impl FnMut(&str) -> (u16, String)) {
    let l = std::net::TcpListener::bind(addr).unwrap_or_else(|e| panic!("{addr}: {e}"));
    for s in l.incoming().flatten() {
        // One request line, at most 8 KiB, within 10 s: a slow or endless client cannot stall or exhaust the service.
        let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(10)));
        let _ = s.set_write_timeout(Some(std::time::Duration::from_secs(10)));
        let mut rd = BufReader::new(std::io::Read::take(&s, 8192));
        let mut line = String::new();
        if rd.read_line(&mut line).is_err() {
            continue;
        }
        let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
        let (code, body) = if line.starts_with("GET ") { handle(&path) } else { (405, "{}".into()) };
        let mut w = &s;
        let _ = write!(w, "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    }
}

fn main() {
    let mut a: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = swiftsync_verify::take_node_format_flag(&mut a) {
        eprintln!("{e}");
        std::process::exit(1);
    }
    let a = a;
    if a.first().map(String::as_str) == Some("--tip") {
        #[cfg(not(feature = "tip"))]
        {
            eprintln!("zync-serve --tip: this build has no tip mode (cargo build --profile zync-serve --features tip --bin zync-serve)");
            std::process::exit(2);
        }
        #[cfg(feature = "tip")]
        {
            if cfg!(panic = "abort") {
                eprintln!("zync-serve --tip: built with panic = \"abort\"; build it with --profile zync-serve (a walk the store moved under must be answered, not abort the service)");
                std::process::exit(2);
            }
            let usage = "usage: zync-serve --tip --at-file <at.json> <addr:port>";
            let (Some(i), Some(addr)) = (a.iter().position(|x| x == "--at-file"), a.last()) else { eprintln!("{usage}"); std::process::exit(1) };
            let file = a.get(i + 1).unwrap_or_else(|| { eprintln!("{usage}"); std::process::exit(1) });
            let cap: u64 = std::env::var("ZYNC_WALK_CAP").ok().and_then(|v| v.parse().ok()).unwrap_or(100_000);
            // a walk refused by a panic inside the reader is answered, and logged once per request, not with a backtrace
            std::panic::set_hook(Box::new(|i| eprintln!("zync-serve: a walk was refused: {i}")));
            let ats = tip::load(file, cap).unwrap_or_else(|e| { eprintln!("zync-serve --tip: {e}"); std::process::exit(1) });
            eprintln!("zync-serve --tip: facts at {:?} (newest {}), listening on {addr}", ats.iter().map(|x| x.height).collect::<Vec<_>>(),
                      ats.last().unwrap().height);
            serve(addr, |p| tip::answer(&ats, p));
            return;
        }
    }
    if a.len() != 2 && a.len() != 3 {
        eprintln!("usage: zync-serve [--node-format 1|2] <history.bin> <addr:port> [state_<tip>.trie]");
        std::process::exit(1);
    }
    let recs = decode_all(&std::fs::read(&a[0]).unwrap_or_else(|e| panic!("{}: {e}", a[0]))).unwrap_or_else(|e| panic!("{e}"));
    let tip = recs.len().checked_sub(1).expect("empty history") as u32;
    let coins = replay(&recs, tip, tip).unwrap().coins;
    let trie = a.get(2).map(|p| {
        let t = libzync::trie_store::TrieStore::from_bytes(&std::fs::read(p).unwrap_or_else(|e| panic!("{p}: {e}")))
            .unwrap_or_else(|e| panic!("{p}: not a trie snapshot ({e:?})"));
        assert_eq!(t.height, tip as u64, "{p}: the trie snapshot is at {}, the history's tip is {tip}", t.height);
        t
    });
    eprintln!("zync-serve: tip {tip}, {} coins, listening on {}", coins.len(), a[1]);
    // `?at=<h>` (ZAPP_FOLLOWER Z4, for tip mode): this mode serves its tip alone; any other height is not served here
    serve(&a[1], |full| match full.split_once('?') {
        None => answer(&recs, tip, &coins, trie.as_ref(), full),
        Some((path, q)) => match q.strip_prefix("at=").map(|h| h.parse::<u32>()) {
            Some(Ok(h)) if h == tip => answer(&recs, tip, &coins, trie.as_ref(), path),
            Some(Ok(h)) => (404, serde_json::json!({ "error": format!("facts are served at {tip}, not at {h}") }).to_string()),
            _ => (400, serde_json::json!({ "error": "only at=<height>" }).to_string()),
        },
    });
}

#[cfg(all(test, feature = "tip"))]
mod tip_tests {
    //! Z2/Z4: tip mode answers, at each pinned height, exactly what an in-memory store of that height answers (the
    //! fact JSON, byte for byte); `?at=` picks the height, the newest by default; a height not served is 404; a walk the
    //! store moved under is 503; /coin and /find are gone. Disk tries from `TrieStore::write_disk_fixture` (two heights,
    //! two directories: the follower's two sources across the handover height X, ZAPP_FOLLOWER §6).
    use super::tip;
    use libzync::trie_store::TrieStore;

    fn spk(i: u8) -> Vec<u8> { vec![0x00, 0x14, i, i, i, i, i, i, i, i, i, i, i, i, i, i, i, i, i, i, i, i] }
    fn store(n: u8) -> TrieStore {
        // blocks 1..n: block b pays scripts b % 3 one coin each, and spends block b-2's first coin
        let mut st = TrieStore::new(true);
        for b in 1..=n {
            let txid = [b; 32];
            let ins: Vec<_> = (0..3u32).map(|v| (libzync::trie_fact::coin_key(&spk((b + v as u8) % 3), &txid, v), None, 1000 * b as u64 + v as u64)).collect();
            let marks: Vec<_> = if b > 2 { vec![(libzync::trie_fact::coin_key(&spk((b - 2) % 3), &[b - 2; 32], 0), [b as u32, 1, 0, 0, 0, 0, 0, 0])] } else { vec![] };
            st.apply(&marks, &ins).unwrap();
        }
        (st.height, st.hash) = (n as u64, [n; 32]);
        st
    }
    fn hx(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }

    #[test]
    fn tip_mode_serves_each_pinned_height_as_its_own_store() {
        let t = std::env::temp_dir().join(format!("zync-serve-tip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let (s10, s12) = (store(10), store(12));
        s10.write_disk_fixture(&t.join("board"), 1).unwrap();
        s12.write_disk_fixture(&t.join("tip"), 4).unwrap();
        let root = |s: &TrieStore| hx(&libzync::coinlog::h_of(&s.root()));
        let at = serde_json::json!([
            {"height": 12, "hash": hx(&[12; 32]), "trie_root": root(&s12), "trie_dir": t.join("tip")},
            {"height": 10, "hash": hx(&[10; 32]), "trie_root": root(&s10), "trie_dir": t.join("board")},
        ]);
        std::fs::write(t.join("at.json"), at.to_string()).unwrap();
        let ats = tip::load(t.join("at.json").to_str().unwrap(), 100_000).unwrap();
        assert_eq!(tip::answer(&ats, "/tip"), (200, r#"{"at":[10,12],"tip":12}"#.into()));
        for (path, kind, txid, vout) in [("balance/{}", "balance", None, None), ("history/{}", "history", None, None),
                                          ("coin/{}/0505050505050505050505050505050505050505050505050505050505050505/1", "coin", Some("0505050505050505050505050505050505050505050505050505050505050505"), Some(1u32))] {
            for i in 0..3u8 {
                let p = format!("/trie/{}", path.replace("{}", &hx(&spk(i))));
                for (h, st) in [(10u32, &s10), (12, &s12)] {
                    let want = serde_json::to_string(&swiftsync_verify::make_trie_fact(st, kind, &spk(i), txid, vout).unwrap()).unwrap();
                    assert_eq!(tip::answer(&ats, &format!("{p}?at={h}")), (200, want.clone()), "{p} at {h}");
                    if h == 12 { assert_eq!(tip::answer(&ats, &p), (200, want), "{p}: the newest by default"); }
                }
                // the two heights differ (the control for "each is its own store")
                assert_ne!(tip::answer(&ats, &format!("{p}?at=10")), tip::answer(&ats, &format!("{p}?at=12")), "{p}");
            }
        }
        assert_eq!(tip::answer(&ats, &format!("/trie/balance/{}?at=11", hx(&spk(1)))).0, 404, "a height not served");
        assert_eq!(tip::answer(&ats, &format!("/trie/balance/{}?at=x", hx(&spk(1)))).0, 400);
        assert_eq!(tip::answer(&ats, &format!("/trie/balance/{}?from=1", hx(&spk(1)))).0, 400);
        assert_eq!(tip::answer(&ats, "/coin/10/0").0, 404, "no coin-log facts in tip mode");
        // the history cap: a walk past it is refused (400), not served
        let small = tip::load(t.join("at.json").to_str().unwrap(), 6).unwrap();
        let (c, b) = tip::answer(&small, &format!("/trie/history/{}", hx(&spk(1))));
        assert!(c == 400 && b.contains("too large"), "{c} {b}");
        // the store moves under the reader (its writer reopened: GEN; the root of 12 is no longer logged): 503
        std::fs::write(t.join("tip").join("GEN"), 5u32.to_le_bytes()).unwrap();
        std::fs::write(t.join("tip").join("roots.log"), b"").unwrap();
        assert_eq!(tip::answer(&ats, &format!("/trie/balance/{}", hx(&spk(1)))).0, 503);
        assert_eq!(tip::answer(&ats, &format!("/trie/balance/{}?at=10", hx(&spk(1)))).0, 200, "the other height still served");
        // a trie_root that is not the store's: refused at start
        let bad = serde_json::json!([{"height": 10, "hash": hx(&[10; 32]), "trie_root": hx(&[7; 32]), "trie_dir": t.join("board")}]);
        std::fs::write(t.join("bad.json"), bad.to_string()).unwrap();
        assert!(tip::load(t.join("bad.json").to_str().unwrap(), 100_000).is_err());
        let _ = std::fs::remove_dir_all(&t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn answers() {
        let h = |i: u8| [i; 32];
        let recs = vec![
            Rec { height: 0, hash: h(9), value: 0, flips: vec![], appends: vec![], undo: None },
            Rec { height: 1, hash: h(8), value: 10, flips: vec![], appends: vec![h(1), h(2)], undo: None },
            Rec { height: 2, hash: h(7), value: 5, flips: vec![(0, 1, 0)], appends: vec![], undo: None },
        ];
        let coins = replay(&recs, 2, 2).unwrap().coins;
        assert_eq!(answer(&recs, 2, &coins, None, "/tip").1, "{\"tip\":2}");
        let (c, b) = answer(&recs, 2, &coins, None, "/coin/2/0");
        assert_eq!(c, 200);
        let f: CoinFact = serde_json::from_str(&b).unwrap();
        assert_eq!(f.spent_by, Some((2, 1, 0)));
        assert_eq!(answer(&recs, 2, &coins, None, &format!("/find/{}", hex(&h(2)))).1, "{\"position\":1}");
        assert_eq!(answer(&recs, 2, &coins, None, "/coin/2/9").0, 404);
        assert_eq!(answer(&recs, 2, &coins, None, "/coin/1/0").0, 200);
    }
}
