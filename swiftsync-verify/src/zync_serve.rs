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
//! With a block source (`--blocks <dir>` or `--bitcoin-cli "<cmd>"`), liveness proofs (docs/libzync/LIVENESS_ZAPP.md,
//! checked with `swiftsync-verify --liveness`):
//!   GET /live/<at|tip>/<txid>/<vout>                  {"proof": hex, "answer": "unspent" | "spent:<h>:<tx>:<vin>"}
//!                                                     the answer is the server's word until the client checks the proof
use libzync::history::{block_path, decode_all, replay, Rec};
use std::io::{BufRead, BufReader, Write};
// ⚠ Single-threaded, and /coin replays the history per request: a production service caches the tip replay (C1).
use swiftsync_verify::{hex, hex32, CoinFact};

mod coin_source;

/// GET /live/<at|tip>/<txid>/<vout>: the outpoint's liveness proof after `at`.
fn live(recs: &[Rec], tip: u32, src: &coin_source::Source, at: &str, txid: &str, vout: &str) -> Result<String, (u16, String)> {
    let at = if at == "tip" { tip } else { at.parse().map_err(|_| (400, "at: a height or tip".to_string()))? };
    if at > tip {
        return Err((404, format!("height {at} is above this server's tip {tip}")));
    }
    let ask = swiftsync_verify::LivenessAsk::parse(&format!("{txid}:{vout}@{at}")).map_err(|e| (400, e))?;
    let coins = coin_source::candidates(src, &ask.txid, ask.vout, None).map_err(|e| (404, e))?;
    let mut last = String::new();
    for c in &coins {
        match swiftsync_verify::make_liveness(recs, tip, at, c) {
            Ok(p) => {
                let st = libzync::liveness::decode(&p).map_err(|e| (500, e.text().to_string()))?.state;
                return Ok(serde_json::json!({ "proof": hex(&p), "answer": swiftsync_verify::liveness_token(&st), "at": at, "tip": tip }).to_string());
            }
            Err(e) => last = e,
        }
    }
    Err((404, last))
}

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

fn answer(recs: &[Rec], tip: u32, coins: &[[u8; 32]], trie: Option<&libzync::trie_store::TrieStore>, src: Option<&coin_source::Source>,
          path: &str) -> (u16, String) {
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
        ["live", at, txid, vout] => match src {
            None => err(404, "no block source loaded (--blocks or --bitcoin-cli)".into()),
            Some(s) => live(recs, tip, s, at, txid, vout).map(|b| (200, b)).unwrap_or_else(|(c, e)| err(c, e)),
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

fn main() {
    let all: Vec<String> = std::env::args().skip(1).collect();
    let src = coin_source::Source::from_args(&all);
    // positional arguments: everything that is not a flag or a flag's value
    let mut a = Vec::new();
    let mut it = all.iter();
    while let Some(x) = it.next() {
        if x == "--blocks" || x == "--bitcoin-cli" { it.next(); } else { a.push(x.clone()); }
    }
    if a.len() != 2 && a.len() != 3 {
        eprintln!("usage: zync-serve <history.bin> <addr:port> [state_<tip>.trie] [--blocks <dir> | --bitcoin-cli \"<cmd>\"]");
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
    let l = std::net::TcpListener::bind(&a[1]).unwrap_or_else(|e| panic!("{}: {e}", a[1]));
    eprintln!("zync-serve: tip {tip}, {} coins, listening on {}", coins.len(), a[1]);
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
        let (code, body) = if line.starts_with("GET ") { answer(&recs, tip, &coins, trie.as_ref(), src.as_ref(), &path) } else { (405, "{}".into()) };
        let mut w = &s;
        let _ = write!(w, "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
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
        assert_eq!(answer(&recs, 2, &coins, None, None, "/tip").1, "{\"tip\":2}");
        let (c, b) = answer(&recs, 2, &coins, None, None, "/coin/2/0");
        assert_eq!(c, 200);
        let f: CoinFact = serde_json::from_str(&b).unwrap();
        assert_eq!(f.spent_by, Some((2, 1, 0)));
        assert_eq!(answer(&recs, 2, &coins, None, None, &format!("/find/{}", hex(&h(2)))).1, "{\"position\":1}");
        assert_eq!(answer(&recs, 2, &coins, None, None, "/coin/2/9").0, 404);
        assert_eq!(answer(&recs, 2, &coins, None, None, "/coin/1/0").0, 200);
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// /live against the real mainnet 1..200 history and the repo's block files
    #[test]
    fn live_route() {
        let dir = env!("CARGO_MANIFEST_DIR");
        let recs = decode_all(&std::fs::read(format!("{dir}/vectors/liveness/mainnet_1_200_history.bin")).unwrap()).unwrap();
        let coins = replay(&recs, 200, 200).unwrap().coins;
        let src = coin_source::Source::Dir(format!("{dir}/../prover/testdata/libzync/mainnet").into());
        let t9 = "0437cd7f8525ceed2324359c2d0ba26006d92d856a9c20fa0241106ee5a597c9";
        let (c, b) = answer(&recs, 200, &coins, None, Some(&src), &format!("/live/tip/{t9}/0"));
        assert_eq!(c, 200, "{b}");
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["answer"], "spent:170:1:0");
        let want = std::fs::read_to_string(format!("{dir}/vectors/liveness/b9_0_at200.hex")).unwrap();
        assert_eq!(v["proof"].as_str().unwrap(), want.trim());
        let (_, b) = answer(&recs, 200, &coins, None, Some(&src), &format!("/live/169/{t9}/0"));
        assert_eq!(serde_json::from_str::<serde_json::Value>(&b).unwrap()["answer"], "unspent");
        assert_eq!(answer(&recs, 200, &coins, None, Some(&src), &format!("/live/201/{t9}/0")).0, 404);
        assert_eq!(answer(&recs, 200, &coins, None, Some(&src), &format!("/live/tip/{t9}/1")).0, 404, "no output 1");
        assert_eq!(answer(&recs, 200, &coins, None, Some(&src), &format!("/live/tip/{}/0", "42".repeat(32))).0, 404);
        assert_eq!(answer(&recs, 200, &coins, None, None, &format!("/live/tip/{t9}/0")).0, 404, "no block source");
        assert_eq!(answer(&recs, 200, &coins, None, Some(&src), &format!("/live/x/{t9}/0")).0, 400);
    }
}
