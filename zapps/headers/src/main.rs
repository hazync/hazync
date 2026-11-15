//! `zapp-headers`: the header-chain zyncAPP's CLI (ROADMAP K1, N3), a zAPP module of its own (docs/libzync/ZYNCAPPS.md
//! §9) on the core verifier's library. Until 15.11.2026 it was `swiftsync-verify headers`; the flags, verdict lines and
//! exit codes are unchanged.

use std::process::exit;
use swiftsync_verify::Verdict;
use zapp_common::{Identity, IDENTITY_USAGE};

fn usage() -> ! {
    println!("SWIFTSYNC_VERIFY_FAIL: bad arguments (see usage on stderr)");
    eprintln!("usage: zapp-headers [--image <hex>] {IDENTITY_USAGE} [--now <s>] [--include <height>:<hash> --path <file> [--tx <txid>:<tx.json>]] <receipt>...");
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

fn main() {
    // Any panic is a FAIL with exit 1: the documented outcomes are 0, 2 and 1 only.
    std::panic::set_hook(Box::new(|i| {
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), "SWIFTSYNC_VERIFY_FAIL: {}", format!("{i}").escape_debug());
        exit(1);
    }));
    run(std::env::args().skip(1).collect());
}

/// `zapp-headers [--image <hex>] [--expect <height>:<hash>] [--now <unix seconds>]
/// [--include <height>:<hash> --path <file> [--tx <txid>:<tx.json>]] <receipt>...`: a
/// proof of the mainnet header chain from genesis (the header-chain zyncAPP), as one receipt or a chain of runs, the
/// first from genesis and each next one starting where the one before ended. Exit 0 OK (chainwork at least Core's
/// minimum), 2 DEMO (below it), 1 FAIL.
/// `--include`: also check that block <hash> (as explorers print it) is at <height> on the proven chain, by its MMR
/// path (`--path`: the file `zyncapp headers-mmr-path` prints, one 64-hex hash per line, lowest first; empty for a
/// one-block peak). `--tx <txid>:<tx.json>`: and that transaction <txid> (as explorers print it: the one YOU ask about)
/// is in that block, by the JSON the path service's /tx answers: the block's header, the raw transaction and its
/// merkle branch, and the raw coinbase and its branch, which pin the tree's depth (headers.rs, verify_tx_in_header,
/// explains the 64-byte attack this closes). The raw transaction must hash to <txid>; the answer's own "txid" is not
/// read, so an answer about another transaction is refused. What was shown (block height and hash, and the txid) is
/// printed on the verdict line.
fn run(a: Vec<String>) -> ! {
    let (mut image, mut id, mut now, mut files) = (zapp_headers::HEADERS_IMAGE_ID.to_string(), Identity::default(), None, Vec::new());
    let (mut include, mut path, mut tx) = (None, None, None);
    let mut it = a.into_iter();
    while let Some(x) = it.next() {
        match x.as_str() {
            "--image" => image = it.next().unwrap_or_else(|| usage()),
            "--now" => now = Some(it.next().unwrap_or_else(|| usage()).parse::<u64>().unwrap_or_else(|_| fail("--now: not a number"))),
            "--include" => include = Some(it.next().unwrap_or_else(|| usage())),
            "--path" => path = Some(it.next().unwrap_or_else(|| usage())),
            "--tx" => tx = Some(it.next().unwrap_or_else(|| usage())),
            f if id.take(f, &mut || it.next()).unwrap_or_else(|e| fail(&e)) => {}
            f if f.starts_with("--") => fail(&format!("unknown flag {}", esc(f))),
            f => files.push(f.to_string()),
        }
    }
    if files.is_empty() { usage(); }
    let read_lines = |f: &str| -> Vec<String> {
        std::fs::read_to_string(f).unwrap_or_else(|e| fail(&format!("{}: {e}", esc(f))))
            .lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
    };
    let inc = match (&include, &path) {
        (None, None) if tx.is_none() => None,
        (Some(i), Some(p)) => {
            let (h, x) = i.split_once(':').unwrap_or_else(|| fail("--include <height>:<hash>"));
            let h = h.parse::<i64>().unwrap_or_else(|_| fail("--include: bad height"));
            let pl = read_lines(p);
            let pr: Vec<&str> = pl.iter().map(|s| s.as_str()).collect();
            let t = tx.as_ref().map(|a: &String| {
                let (txid, f) = a.split_once(':').unwrap_or_else(|| fail("--tx <txid>:<tx.json> (the txid you ask about, then the answer's file)"));
                let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(f).unwrap_or_else(|e| fail(&format!("{}: {e}", esc(f)))))
                    .unwrap_or_else(|e| fail(&format!("--tx {}: not JSON ({e})", esc(f))));
                zapp_headers::TxInclusion::from_json(txid, &j).unwrap_or_else(|e| fail(&format!("--tx: {e}")))
            });
            Some(zapp_headers::Inclusion::parse(h, x, &pr, t).unwrap_or_else(|e| fail(&format!("--include: {e}"))))
        }
        _ => fail("--include <height>:<hash> needs --path <file> (and --tx needs --include)"),
    };
    let clock = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let now = match now { Some(t) if t <= clock => t, Some(_) => fail("--now must not be later than this clock"), None => clock };
    let bytes: Vec<Vec<u8>> = files.iter().map(|f| std::fs::read(f).unwrap_or_else(|e| fail(&format!("{}: {e}", esc(f))))).collect();
    let refs: Vec<&[u8]> = bytes.iter().map(|b| b.as_slice()).collect();
    // chain identity (zapp-common): --mempool / --bitcoin-cli are asked for the block at the proven header tip, which the
    // receipts give, and their answer is checked as --expect is
    let (mut expect, mut extra) = (id.expect.clone(), Vec::new());
    if id.needs_lookup() {
        let pre = zapp_headers::verify_headers(&refs, &image, now, None, None);
        if let Some(st) = pre.state.as_ref().filter(|_| pre.verdict != Verdict::Fail) {
            let (x, c) = zapp_common::expect_at(&id, "mainnet", st.height as u32);
            expect = x;
            extra = c;
        }
    }
    let exp = expect.as_deref().map(|e| {
        let (h, x) = e.split_once(':').unwrap_or_else(|| fail("--expect <height>:<hash>"));
        (h.parse::<i64>().unwrap_or_else(|_| fail("--expect: bad height")), x)
    });
    let mut rep = zapp_headers::verify_headers(&refs, &image, now, exp, inc.as_ref());
    if extra.iter().any(|c| !c.0) { rep.verdict = Verdict::Fail; }
    rep.checks.splice(0..0, extra);
    for (ok, what) in &rep.checks { println!("  [{}] {what}", if *ok { " ok " } else { "FAIL" }); }
    let ident = if expect.is_some() { "" } else { "; chain identity NOT checked (give --expect <height>:<hash>, --mempool or --bitcoin-cli: a source you trust)" };
    match (&rep.verdict, &rep.state) {
        (Verdict::Fail, _) | (_, None) => { println!("SWIFTSYNC_VERIFY_FAIL: {} check(s) failed", rep.checks.iter().filter(|c| !c.0).count()); exit(1) }
        (v, Some(s)) => {
            let shown = rep.included.as_ref().map(|i| format!("; INCLUDED: block {} {} is at that height on this chain{}", i.height, i.hash_hex(),
                i.txid_hex().map(|t| format!(", with transaction {t} in it")).unwrap_or_default())).unwrap_or_default();
            let what = format!("header chain from genesis to {} {} (chainwork {}, MMR root {}){shown}{ident}", s.height, s.hash_hex(), s.work_hex(), s.mmr_root_hex());
            if matches!(v, Verdict::Ok) { println!("SWIFTSYNC_VERIFY_OK: {what}"); exit(0) }
            println!("SWIFTSYNC_VERIFY_DEMO: a valid header chain below Core's minimum chain work: {what}"); exit(2)
        }
    }
}
