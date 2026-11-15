//! `zapp-liveness`: the coin-liveness zyncAPP's CLI (docs/libzync/LIVENESS_ZAPP.md), a zAPP module of its own
//! (docs/libzync/ZYNCAPPS.md §9) on the core verifier's library. Until 15.11.2026 the check was
//! `swiftsync-verify --liveness`, the producer `zync-path live` and the server route `zync-serve /live`.
//!
//! ```text
//!   zapp-liveness [--network mainnet|signet] [--image <hex>] [--coinlog | --product] [--anchor state.json]
//!                 [--now <unix seconds>] [--expect <height>:<hash>] (--liveness <proof> <question>)... <receipt|bundle.sspf>
//!       check each liveness proof against the verified core proof and against YOUR question
//!       (`<txid>:<vout>@<h|tip>`, optionally `=unspent` / `=spent[:<h>[:<tx>:<vin>]]`, `,value=<sat>`, `,script=<hex>`;
//!       without a claim the answer is stated). `--expect` is the network's block at the proof's height from a source
//!       YOU trust; without it the verdict is at most DEMO. Exit 0 OK, 2 DEMO, 1 FAIL.
//!   zapp-liveness prove <history.bin> <tip> <at|tip> <txid>:<vout> (--blocks <dir> | --bitcoin-cli "<cmd>") [--height <h>]
//!       a liveness proof, hex: the outpoint unspent after <at>, or spent and by whom
//!   zapp-liveness serve <history.bin> <addr:port> (--blocks <dir> | --bitcoin-cli "<cmd>")
//!       GET /tip, GET /live/<at|tip>/<txid>/<vout>: {"proof": hex, "answer": "unspent" | "spent:<h>:<tx>:<vin>"}
//!       (the answer is the server's word until the client checks the proof)
//! ```
//! ⚠ `prove` and `serve` replay the whole history in memory, as zync-path / zync-serve do.

use libzync::history::{decode_all, Rec};
use std::io::{BufRead, BufReader, Write};
use std::process::exit;
use swiftsync_verify::{hex, ChainState, Verdict};
use zapp_liveness::{coin_source, LivenessAsk, LivenessFact};

fn usage() -> ! {
    println!("SWIFTSYNC_VERIFY_FAIL: bad arguments (see usage on stderr)");
    eprintln!("usage: zapp-liveness [--network mainnet|signet] [--image <hex>] [--coinlog | --product] [--anchor state.json] [--now <unix seconds>] [--expect <height>:<hash>] (--liveness <proof> <txid>:<vout>@<h|tip>[=unspent|=spent[:<h>[:<tx>:<vin>]]][,value=<sat>][,script=<hex>])... <receipt|bundle.sspf>");
    eprintln!("       zapp-liveness prove <history.bin> <tip> <at|tip> <txid>:<vout> (--blocks <dir> | --bitcoin-cli \"<cmd>\") [--height <h>]");
    eprintln!("       zapp-liveness serve <history.bin> <addr:port> (--blocks <dir> | --bitcoin-cli \"<cmd>\")");
    exit(1);
}

fn fail(msg: &str) -> ! {
    println!("SWIFTSYNC_VERIFY_FAIL: {msg}");
    exit(1);
}

/// A path as printed: escaped, so a file name cannot carry a newline or look like a verdict line of its own.
fn esc(p: &str) -> String {
    format!("{p:?}")
}

fn die(m: &str) -> ! {
    eprintln!("zapp-liveness: {m}");
    exit(1);
}

fn history(p: &str) -> Vec<Rec> {
    decode_all(&std::fs::read(p).unwrap_or_else(|e| die(&format!("{p}: {e}")))).unwrap_or_else(|e| die(&format!("{p}: {e}")))
}

/// Positional arguments: everything that is not a block-source flag or its value.
fn positional(a: &[String]) -> Vec<String> {
    let (mut out, mut it) = (Vec::new(), a.iter());
    while let Some(x) = it.next() {
        if x == "--blocks" || x == "--bitcoin-cli" || x == "--height" { it.next(); } else { out.push(x.clone()); }
    }
    out
}

fn main() {
    // Any panic is a FAIL with exit 1: the documented outcomes are 0, 2 and 1 only.
    std::panic::set_hook(Box::new(|i| {
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), "SWIFTSYNC_VERIFY_FAIL: {}", format!("{i}").escape_debug());
        exit(1);
    }));
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a.first().map(|x| x.as_str()) {
        Some("prove") => prove_main(&a[1..]),
        Some("serve") => serve_main(&a[1..]),
        _ => check_main(a),
    }
}

fn check_main(a: Vec<String>) -> ! {
    let (mut network, mut image_hex, mut anchor, mut now, mut expect) = ("mainnet".to_string(), String::new(), None, None, None);
    let (mut coinlog, mut product, mut liveness, mut files) = (false, false, Vec::<(String, String)>::new(), Vec::new());
    let mut it = a.into_iter();
    while let Some(x) = it.next() {
        match x.as_str() {
            "--network" => network = it.next().unwrap_or_else(|| usage()),
            "--image" => image_hex = it.next().unwrap_or_else(|| usage()),
            "--anchor" => anchor = Some(it.next().unwrap_or_else(|| usage())),
            "--now" => now = Some(it.next().unwrap_or_else(|| usage()).parse::<u64>().unwrap_or_else(|_| fail("--now: not a number"))),
            "--expect" => expect = Some(it.next().unwrap_or_else(|| usage())),
            "--coinlog" => coinlog = true,
            "--product" => { coinlog = true; product = true }
            "--liveness" => {
                let f = it.next().unwrap_or_else(|| usage());
                liveness.push((f, it.next().unwrap_or_else(|| usage())));
            }
            "-h" | "--help" => usage(),
            f if f.starts_with("--") => fail(&format!("unknown flag {}", esc(f))),
            f => files.push(f.to_string()),
        }
    }
    if files.len() != 1 || liveness.is_empty() { usage(); }
    let clock = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let now = match now { Some(t) if t <= clock => t, Some(_) => fail("--now must not be later than this clock"), None => clock };
    let image = zapp_liveness::image_for(&network, &image_hex, coinlog, product).unwrap_or_else(|e| fail(&e));
    if !image_hex.is_empty() { println!("  image id given on the command line: {}", image.method_id_hex); }
    let rd = |f: &str| std::fs::read(f).unwrap_or_else(|e| fail(&format!("{}: {e}", esc(f))));
    let anchor: Option<ChainState> = anchor.map(|p| serde_json::from_slice(&rd(&p)).unwrap_or_else(|e| fail(&format!("{}: not a ChainState: {e}", esc(&p)))));
    // The proof file's bytes (raw or hex) as the C ABI and WASM take them: lower-case hex.
    let facts: Vec<LivenessFact> = liveness.iter().map(|(p, q)| {
        let b = zapp_liveness::liveness_bytes(&rd(p)).unwrap_or_else(|e| fail(&format!("--liveness {}: {e}", esc(p))));
        LivenessFact { proof: hex(&b), ask: q.clone() }
    }).collect();
    let o = zapp_liveness::verify_liveness(&rd(&files[0]), image, anchor, now, &facts, expect.as_deref(), false);
    for (ok, what) in &o.report.checks { println!("  [{}] {what}", if *ok { " ok " } else { "FAIL" }); }
    for ((p, _), r) in liveness.iter().zip(o.facts.iter().chain(std::iter::repeat(&Err(String::new())))) {
        match r {
            Ok(w) => println!("  [ ok ] --liveness {}: {w}", esc(p)),
            Err(e) => println!("  [FAIL] --liveness {}: {e}", esc(p)),
        }
    }
    for r in o.facts.iter().skip(liveness.len()) {
        if let Err(e) = r { println!("  [FAIL] {e}"); }
    }
    match o.report.verdict {
        Verdict::Ok => { println!("SWIFTSYNC_VERIFY_OK: {} liveness proof(s) hold on the proven chain, its tip the block --expect names", facts.len()); exit(0) }
        Verdict::Demo => { println!("SWIFTSYNC_VERIFY_DEMO: the liveness proof(s) hold, but the chain is not shown to be the network's (no matching --expect, below minimum work, or from an anchor)"); exit(2) }
        Verdict::Fail => { println!("SWIFTSYNC_VERIFY_FAIL: {} check(s) failed", o.report.checks.iter().filter(|c| !c.0).count() + o.facts.iter().filter(|r| r.is_err()).count()); exit(1) }
    }
}

fn prove_main(all: &[String]) -> ! {
    let a = positional(all);
    if a.len() != 4 { usage(); }
    let num = |i: usize| -> u32 { a[i].parse().unwrap_or_else(|_| die(&format!("{}: not a number", a[i]))) };
    let tip = num(1);
    let at = if a[2] == "tip" { tip } else { num(2) };
    let src = coin_source::Source::from_args(all).unwrap_or_else(|| die("prove needs --blocks <dir> or --bitcoin-cli \"<cmd>\""));
    let ask = LivenessAsk::parse(&format!("{}@{at}", a[3])).unwrap_or_else(|e| die(&e));
    let height = all.iter().position(|x| x == "--height").and_then(|i| all.get(i + 1)).map(|h| h.parse().unwrap_or_else(|_| die("--height?")));
    let recs = history(&a[0]);
    match zapp_liveness::prove(&recs, tip, at, &src, &ask.txid, ask.vout, height) {
        Ok(p) => { println!("{}", hex(&p)); exit(0) }
        Err(e) => die(&e),
    }
}

/// GET /live/<at|tip>/<txid>/<vout>: the outpoint's liveness proof after `at`.
fn live(recs: &[Rec], tip: u32, src: &coin_source::Source, at: &str, txid: &str, vout: &str) -> Result<String, (u16, String)> {
    let at = if at == "tip" { tip } else { at.parse().map_err(|_| (400, "at: a height or tip".to_string()))? };
    if at > tip {
        return Err((404, format!("height {at} is above this server's tip {tip}")));
    }
    let ask = LivenessAsk::parse(&format!("{txid}:{vout}@{at}")).map_err(|e| (400, e))?;
    let p = zapp_liveness::prove(recs, tip, at, src, &ask.txid, ask.vout, None).map_err(|e| (404, e))?;
    let st = libzync::liveness::decode(&p).map_err(|e| (500, e.text().to_string()))?.state;
    Ok(serde_json::json!({ "proof": hex(&p), "answer": zapp_liveness::liveness_token(&st), "at": at, "tip": tip }).to_string())
}

fn answer(recs: &[Rec], tip: u32, src: Option<&coin_source::Source>, path: &str) -> (u16, String) {
    let p: Vec<&str> = path.trim_matches('/').split('/').collect();
    let err = |c: u16, m: String| (c, serde_json::json!({ "error": m }).to_string());
    match p.as_slice() {
        ["tip"] => (200, serde_json::json!({ "tip": tip }).to_string()),
        ["live", at, txid, vout] => match src {
            None => err(404, "no block source loaded (--blocks or --bitcoin-cli)".into()),
            Some(s) => live(recs, tip, s, at, txid, vout).map(|b| (200, b)).unwrap_or_else(|(c, e)| err(c, e)),
        },
        _ => err(404, "routes: /tip, /live/<at|tip>/<txid>/<vout>".into()),
    }
}

fn serve_main(all: &[String]) -> ! {
    let a = positional(all);
    if a.len() != 2 { usage(); }
    let src = coin_source::Source::from_args(all).unwrap_or_else(|| die("serve needs --blocks <dir> or --bitcoin-cli \"<cmd>\""));
    let recs = history(&a[0]);
    let tip = recs.len().checked_sub(1).unwrap_or_else(|| die("empty history")) as u32;
    let l = std::net::TcpListener::bind(&a[1]).unwrap_or_else(|e| die(&format!("{}: {e}", a[1])));
    eprintln!("zapp-liveness serve: tip {tip}, listening on {}", a[1]);
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
        let (code, body) = if line.starts_with("GET ") { answer(&recs, tip, Some(&src), &path) } else { (405, "{}".into()) };
        let mut w = &s;
        let _ = write!(w, "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    }
    exit(0)
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// /live against the real mainnet 1..200 history and the repo's block files (moved from zync-serve, 15.11.2026)
    #[test]
    fn live_route() {
        let dir = env!("CARGO_MANIFEST_DIR");
        let recs = decode_all(&std::fs::read(format!("{dir}/vectors/mainnet_1_200_history.bin")).unwrap()).unwrap();
        let src = coin_source::Source::Dir(format!("{dir}/../../prover/testdata/libzync/mainnet").into());
        let t9 = "0437cd7f8525ceed2324359c2d0ba26006d92d856a9c20fa0241106ee5a597c9";
        assert_eq!(answer(&recs, 200, Some(&src), "/tip").1, "{\"tip\":200}");
        let (c, b) = answer(&recs, 200, Some(&src), &format!("/live/tip/{t9}/0"));
        assert_eq!(c, 200, "{b}");
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["answer"], "spent:170:1:0");
        let want = std::fs::read_to_string(format!("{dir}/vectors/b9_0_at200.hex")).unwrap();
        assert_eq!(v["proof"].as_str().unwrap(), want.trim());
        let (_, b) = answer(&recs, 200, Some(&src), &format!("/live/169/{t9}/0"));
        assert_eq!(serde_json::from_str::<serde_json::Value>(&b).unwrap()["answer"], "unspent");
        assert_eq!(answer(&recs, 200, Some(&src), &format!("/live/201/{t9}/0")).0, 404);
        assert_eq!(answer(&recs, 200, Some(&src), &format!("/live/tip/{t9}/1")).0, 404, "no output 1");
        assert_eq!(answer(&recs, 200, Some(&src), &format!("/live/tip/{}/0", "42".repeat(32))).0, 404);
        assert_eq!(answer(&recs, 200, None, &format!("/live/tip/{t9}/0")).0, 404, "no block source");
        assert_eq!(answer(&recs, 200, Some(&src), &format!("/live/x/{t9}/0")).0, 400);
    }
}
