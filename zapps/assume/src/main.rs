//! `zapp-assume`: the assumevalid and assumeutxo zyncAPPs' CLI (docs/libzync/ASSUME_ZAPPS.md), a zAPP module of its own
//! (docs/libzync/ZYNCAPPS.md §9) on the core verifier's library. Until 15.11.2026 these were the core CLI's
//! `swiftsync-verify assumevalid` / `assumeutxo` (and its `--assumevalid` / `--snapshot` flags); the checks, verdict lines
//! and exit codes are unchanged.
//!
//!   zapp-assume assumevalid [--block <hash>] [--headers <file>] [flags] <receipt>
//!   zapp-assume assumeutxo --headers <file> [--tracked <file>] [flags] <receipt> <snapshot>
//!
//! flags: [--network mainnet|signet] [--image <hex>] [--now <unix seconds>] and the chain-identity flags every zAPP CLI
//! takes (zapp-common): [--expect <height>:<hash>] [--mempool | --mempool-url <base>] [--bitcoin-cli "<command>"].
//! The core receipt is verified by the core verifier (`swiftsync_verify::verify`, a receipt or an .sspf bundle) from
//! genesis: no anchor, a node's shortcut must rest on a chain from genesis. OK (exit 0) needs Core's minimum chainwork
//! and the tip compared with a source you trust; DEMO (2) otherwise; FAIL (1) on any failed check.

use std::process::exit;
use swiftsync_verify::{ChainState, Verdict};
use zapp_common::{check_core_tip, image_for, verify_core, Identity, IDENTITY_USAGE};

fn usage() -> ! {
    println!("SWIFTSYNC_VERIFY_FAIL: bad arguments (see usage on stderr)");
    eprintln!("usage: zapp-assume assumevalid [--block <hash>] [--headers <file>] [--network <name>] [--image <hex>] [--now <s>] {IDENTITY_USAGE} <receipt>");
    eprintln!("       zapp-assume assumeutxo --headers <file> [--tracked <file>] [--network <name>] [--image <hex>] [--now <s>] {IDENTITY_USAGE} <receipt> <snapshot>");
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
    let a: Vec<String> = std::env::args().skip(1).collect();
    let sub = a.first().cloned().unwrap_or_default();
    if sub != "assumevalid" && sub != "assumeutxo" { usage(); }
    let (mut network, mut image_hex, mut now, mut id) = ("mainnet".to_string(), String::new(), None, Identity::default());
    let (mut block, mut headers, mut tracked, mut pos) = (None, None, None, Vec::new());
    let mut it = a.into_iter().skip(1);
    while let Some(x) = it.next() {
        match x.as_str() {
            "--network" => network = it.next().unwrap_or_else(|| usage()),
            "--image" => image_hex = it.next().unwrap_or_else(|| usage()),
            "--now" => now = Some(it.next().unwrap_or_else(|| usage()).parse::<u64>().unwrap_or_else(|_| fail("--now: not a number"))),
            "--block" => block = Some(it.next().unwrap_or_else(|| usage())),
            "--headers" => headers = Some(it.next().unwrap_or_else(|| usage())),
            "--tracked" => tracked = Some(it.next().unwrap_or_else(|| usage())),
            "-h" | "--help" => usage(),
            f if id.take(f, &mut || it.next()).unwrap_or_else(|e| fail(&e)) => {}
            f if f.starts_with("--") => fail(&format!("{sub}: unknown flag {}", esc(f))),
            f => pos.push(f.to_string()),
        }
    }
    let (receipt, av, snap) = if sub == "assumevalid" {
        let [r] = pos.as_slice() else { fail("usage: zapp-assume assumevalid [--block <hash>] [--headers <file>] [flags] <receipt>") };
        (r.clone(), Some(block.unwrap_or_else(|| "tip".into())), None)
    } else {
        if block.is_some() { fail("assumeutxo: --block is for assumevalid") }
        let [r, s] = pos.as_slice() else { fail("usage: zapp-assume assumeutxo --headers <file> [--tracked <file>] [flags] <receipt> <snapshot>") };
        (r.clone(), None, Some(s.clone()))
    };
    let clock = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let now = match now { Some(t) if t <= clock => t, Some(_) => fail("--now must not be later than this clock"), None => clock };
    let image = image_for(&network, &image_hex, false, false).unwrap_or_else(|e| fail(&e));
    if !image_hex.is_empty() { println!("  image id given on the command line: {}", image.method_id_hex); }
    let rb = std::fs::read(&receipt).unwrap_or_else(|e| fail(&format!("{}: {e}", esc(&receipt))));
    let mut rep = verify_core(&rb, image, None, now, None).report;
    let mut tip_checked = false;
    let mut lines = Vec::new();
    if let Some(cs) = rep.state.clone() {
        let (idc, ok) = check_core_tip(&id, image.network, &cs);
        rep.checks.extend(idc);
        tip_checked = ok;
        if rep.verdict == Verdict::Fail {
            rep.checks.push((false, "assumevalid / assumeutxo: NOT checked: the receipt did not verify".into()));
        } else {
            assume_checks(&cs, &mut rep.checks, &mut lines, av.as_deref(), snap.as_deref(), headers.as_deref(), tracked.as_deref());
        }
    }
    for (ok, what) in &rep.checks { println!("  [{}] {what}", if *ok { " ok " } else { "FAIL" }); }
    for l in &lines { println!("  {l}"); }
    let failed = rep.checks.iter().filter(|(ok, _)| !ok).count();
    let h = rep.state.as_ref().map(|c| c.height).unwrap_or(0);
    let ident = if id.is_empty() { "; chain identity NOT checked: compare the tip with a source you trust (--mempool, --expect, --bitcoin-cli)" } else { "" };
    match rep.verdict {
        _ if failed > 0 || rep.state.is_none() => { println!("SWIFTSYNC_VERIFY_FAIL: {} check(s) failed", failed.max(1)); exit(1) }
        Verdict::Fail => { println!("SWIFTSYNC_VERIFY_FAIL: {} check(s) failed", failed.max(1)); exit(1) }
        Verdict::Ok if tip_checked => { println!("SWIFTSYNC_VERIFY_OK: the chain above and its coin log are proven at height {h}{}", assume_what(&lines)); exit(0) }
        Verdict::Ok => { println!("SWIFTSYNC_VERIFY_DEMO: the chain and the facts{} check, but facts need the tip compared with the network (--mempool, --expect or --bitcoin-cli)", assume_what(&lines)); exit(2) }
        Verdict::Demo if rep.low_work => { println!("SWIFTSYNC_VERIFY_DEMO: the chain and its coin log{}: a valid chain from genesis, but below Core's minimum chainwork or the pinned checkpoint for {}: not shown to be the network's chain{ident}", assume_what(&lines), image.network); exit(2) }
        Verdict::Demo => { println!("SWIFTSYNC_VERIFY_DEMO: the chain and its coin log{}: not shown to be the network's chain{ident}", assume_what(&lines)); exit(2) }
    }
}

/// " ; <summary>" of the assume checks that passed, for the verdict line.
fn assume_what(lines: &[String]) -> String {
    let mut s = String::new();
    if lines.iter().any(|l| l.starts_with("assumevalid:")) { s += "; -assumevalid block proven"; }
    if lines.iter().any(|l| l.starts_with("assumeutxo:")) { s += "; the snapshot is the proven UTXO set"; }
    s
}

/// The assumevalid / assumeutxo checks against a VERIFIED state (`assume::run_assume`, as the C ABI and WASM).
fn assume_checks(cs: &ChainState, checks: &mut Vec<(bool, String)>, lines: &mut Vec<String>, av: Option<&str>, snap: Option<&str>,
                 headers: Option<&str>, tracked: Option<&str>) {
    use zapp_assume as assume;
    let read = |p: &str| std::fs::read(p).map_err(|e| format!("{}: {e}", esc(p)));
    let headers = match headers.map(read) {
        None => None,
        Some(Ok(b)) => Some(b),
        Some(Err(e)) => { checks.push((false, format!("--headers {e}"))); return }
    };
    let tracked = match tracked.map(|p| std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", esc(p)))) {
        None => None,
        Some(Ok(t)) => Some(t),
        Some(Err(e)) => { checks.push((false, format!("--tracked {e}"))); return }
    };
    let assumevalid = match av {
        None => None,
        Some("tip") => Some(None),
        Some(x) => match assume::parse_display_hash(x) {
            Some(h) => Some(Some(h)),
            None => { checks.push((false, format!("assumevalid: --block {x:?}: not 64 hex digits"))); return }
        },
    };
    let snapshot = match snap.map(|p| std::fs::File::open(p).map_err(|e| format!("{}: {e}", esc(p)))) {
        None => None,
        Some(Ok(f)) => Some(std::io::BufReader::with_capacity(1 << 20, f)),
        Some(Err(e)) => { checks.push((false, format!("assumeutxo: {e}"))); return }
    };
    let t0 = std::time::Instant::now();
    let o = assume::run_assume(cs, assume::AssumeRequest { assumevalid, headers: headers.as_deref(), snapshot, tracked: tracked.as_deref() });
    let secs = t0.elapsed().as_secs_f64();
    for (ok, what) in o.checks {
        checks.push((ok, if what.starts_with("assumeutxo: snapshot of") { format!("{what} [{}; {secs:.2} s]", esc(snap.unwrap_or(""))) } else { what }));
    }
    if let Some(r) = o.valid {
        lines.push(format!("assumevalid: -assumevalid={} (height {}; scripts proven valid for blocks 0..{})", assume::display(&r.block), r.height, r.proven_to));
    }
    if let Some(r) = o.utxo {
        lines.push(format!("assumeutxo: {} is the proven UTXO set at height {}; Core's hash_serialized_3 of it: {}", esc(snap.unwrap_or("")), cs.height, r.hash_serialized_3));
        let (x, y) = assume::live_hash_hex(&r.live_hash);
        lines.push(if r.live_set == 0 { "live_hash identity".into() } else { format!("live_hash x {x} y {y}") });
    }
}
