//! `zapp-statement`: checks a balance statement (docs/libzync/STATEMENT_ZAPP.md) against the core proof it names.
//!
//!   zapp-statement [--network mainnet|signet] [--image <hex>] [--expect <height>:<hash>] [--mempool | --mempool-url <base>]
//!                  [--bitcoin-cli "<command>"] [--purpose <text> --nonce <text>] [--now <unix seconds>] <receipt> <statement.json>
//!
//! The core receipt is verified by `swiftsync_verify::verify`, the same checks as `swiftsync-verify` (real proof, image,
//! profile, anchor, block forest, time-too-new). Then the statement: its network, core image, height and block are the
//! proof's; each balance fact holds against the proof's trie root; the total is their sum; each script's BIP-322
//! signature verifies over the statement's message; purpose and nonce are the ones you give.
//!
//! Exit 0 `ZAPP_STATEMENT_OK`, 2 `ZAPP_STATEMENT_DEMO` (everything holds, but the chain's work is below Core's minimum,
//! the tip was not compared with a source you trust, or no --purpose / --nonce was given), 1 `ZAPP_STATEMENT_FAIL`.

use balance_statement::{check_statement, Challenge, Statement};
use std::process::{exit, Command};
use swiftsync_verify::{identity_matches, parse_expect, verify, Image, Inputs, Verdict};

fn fail(m: &str) -> ! {
    println!("ZAPP_STATEMENT_FAIL: {m}");
    exit(1)
}

fn main() {
    std::panic::set_hook(Box::new(|i| {
        println!("ZAPP_STATEMENT_FAIL: {}", format!("{i}").escape_debug());
        exit(1);
    }));
    let mut it = std::env::args().skip(1);
    let (mut network, mut image_hex, mut expect, mut mempool, mut cli) = ("mainnet".to_string(), None, None, None::<String>, None);
    let (mut purpose, mut nonce, mut now_arg, mut files) = (None, None, None, Vec::new());
    let need = |it: &mut dyn Iterator<Item = String>, f: &str| it.next().unwrap_or_else(|| fail(&format!("{f} needs a value")));
    while let Some(a) = it.next() {
        match a.as_str() {
            "--network" => network = need(&mut it, &a),
            "--image" => image_hex = Some(need(&mut it, &a)),
            "--expect" => expect = Some(need(&mut it, &a)),
            "--mempool" => mempool = Some(String::new()),
            "--mempool-url" => mempool = Some(need(&mut it, &a)),
            "--bitcoin-cli" => cli = Some(need(&mut it, &a)),
            "--purpose" => purpose = Some(need(&mut it, &a)),
            "--nonce" => nonce = Some(need(&mut it, &a)),
            "--now" => now_arg = Some(need(&mut it, &a)),
            f if f.starts_with("--") => fail(&format!("unknown flag {f:?} (see the source header)")),
            _ => files.push(a),
        }
    }
    let [receipt, stf] = files.as_slice() else { fail("give <receipt> <statement.json>") };
    let mut image = Image::by_network(&network).unwrap_or_else(|| fail(&format!("unknown network {network:?}")));
    if let Some(h) = &image_hex {
        image = match Image::by_id(&network, h) {
            Some(i) => i,
            None => image.with_method_id(&h.to_lowercase()).unwrap_or_else(|| fail("--image: not 64 hex digits")),
        };
    }
    let clock = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("clock").as_secs();
    let now = match now_arg.map(|v| v.parse::<u64>()) {
        None => clock,
        Some(Ok(t)) if t <= clock => t,
        _ => fail("--now: unix seconds, not later than this clock"),
    };
    let rb = std::fs::read(receipt).unwrap_or_else(|e| fail(&format!("{receipt:?}: {e}")));
    let st: Statement = std::fs::read(stf).map_err(|e| e.to_string())
        .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()))
        .unwrap_or_else(|e| fail(&format!("{stf:?}: not a balance statement ({e})")));
    let mut rep = verify(&Inputs { receipt: &rb, image, anchor: None, now });
    let mut tip_checked = false;
    let mut challenged = false;
    if let Some(cs) = rep.state.clone().filter(|_| rep.verdict != Verdict::Fail) {
        let mut tip = cs.tip_hash;
        tip.reverse();
        let tip: String = tip.iter().map(|b| format!("{b:02x}")).collect();
        if let Some(e) = &expect {
            match parse_expect(e) {
                Some((h, x)) => rep.checks.push((identity_matches(h, &x, cs.height, &cs.tip_hash), format!("--expect: block {h} is {x}; the proof's tip is block {} {tip}", cs.height))),
                None => rep.checks.push((false, format!("--expect {e:?}: want <height>:<64 hex digits>"))),
            }
            tip_checked = true;
        }
        if let Some(base) = &mempool {
            let base = if !base.is_empty() { base.trim_end_matches('/').to_string() }
                       else if image.network == "signet" { "https://mempool.space/signet/api".into() } else { "https://mempool.space/api".into() };
            let url = format!("{base}/block-height/{}", cs.height);
            let got = Command::new("curl").args(["-sf", "--max-time", "30", &url]).output().ok().filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_lowercase());
            rep.checks.push((got.as_deref().map_or(false, |g| identity_matches(cs.height, g, cs.height, &cs.tip_hash)),
                             format!("{url} says block {} is {got:?}; the proof's tip is {tip}", cs.height)));
            tip_checked = true;
        }
        if let Some(cmd) = &cli {
            let mut p = cmd.split_whitespace();
            let got = p.next().and_then(|prog| Command::new(prog).args(p).args(["getblockhash", &cs.height.to_string()]).output().ok())
                .filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_lowercase());
            rep.checks.push((got.as_deref() == Some(tip.as_str()), format!("your node's block {} is {got:?}; the proof's tip is {tip}", cs.height)));
            tip_checked = true;
        }
        let ch = Challenge { purpose: purpose.as_deref(), nonce: nonce.as_deref() };
        match check_statement(&cs, rep.undo_from, &st, &ch) {
            Ok(c) => { challenged = c.challenged; rep.checks.push((true, c.line)); }
            Err(e) => rep.checks.push((false, format!("statement: {e}"))),
        }
    } else {
        rep.checks.push((false, "statement NOT checked: the receipt did not verify".into()));
    }
    let failed = rep.checks.iter().filter(|c| !c.0).count();
    for (ok, what) in &rep.checks {
        println!("  [{}] {what}", if *ok { "ok" } else { "FAIL" });
    }
    if failed > 0 || rep.verdict == Verdict::Fail {
        println!("ZAPP_STATEMENT_FAIL: {failed} check(s) failed");
        exit(1);
    }
    let mut why = Vec::new();
    if rep.verdict == Verdict::Demo { why.push("the chain is below Core's minimum chainwork or the pinned checkpoint"); }
    if !tip_checked { why.push("the tip was not compared with a source you trust (--expect, --mempool, --bitcoin-cli)"); }
    if !challenged { why.push("the statement was not compared with your challenge (--purpose and --nonce): it may be an old one shown again"); }
    if why.is_empty() {
        println!("ZAPP_STATEMENT_OK: the statement holds at block {}", st.height);
        exit(0);
    }
    println!("ZAPP_STATEMENT_DEMO: the statement holds, but {}", why.join("; "));
    exit(2);
}
