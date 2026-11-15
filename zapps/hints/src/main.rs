//! `zapp-hints`: the SwiftSync hints zyncAPP's CLI (docs/libzync/HINTS_ZAPP.md), a zAPP module of its own
//! (docs/libzync/ZYNCAPPS.md §9) on the core verifier's library. Until 15.11.2026 it was `swiftsync-verify hints`;
//! the flags, verdict lines and exit codes are unchanged.

use std::process::exit;
use swiftsync_verify::Verdict;

fn usage() -> ! {
    println!("SWIFTSYNC_VERIFY_FAIL: bad arguments (see usage on stderr)");
    eprintln!("usage: zapp-hints [--image <hex>] [--core-image <hex>] [--expect <H>:<hash>] [--expect-tip <T>:<hash>] [--undo <file>] [--now <unix seconds>] <receipt> <hints file>");
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

/// `zapp-hints [--image <hex>] [--core-image <hex>] [--expect <H>:<hash>] [--expect-tip <T>:<hash>]
/// [--undo <file>] [--now <unix seconds>] <receipt> <hints file>`: the SwiftSync hints zyncAPP (docs/libzync/HINTS_ZAPP.md).
/// `--expect` is block H from YOUR node's header chain. Exit 0 OK, 2 DEMO (below minimum work or the checkpoint, or the
/// block at H / the core tip not compared), 1 FAIL.
fn run(a: Vec<String>) -> ! {
    let (mut image, mut core, mut expect, mut expect_tip, mut undo, mut now, mut files) =
        (zapp_hints::HINTS_IMAGE_ID.to_string(), None, None, None, None, None, Vec::new());
    let mut it = a.into_iter();
    while let Some(x) = it.next() {
        match x.as_str() {
            "--image" => image = it.next().unwrap_or_else(|| usage()),
            "--core-image" => core = Some(it.next().unwrap_or_else(|| usage())),
            "--expect" => expect = Some(it.next().unwrap_or_else(|| usage())),
            "--expect-tip" => expect_tip = Some(it.next().unwrap_or_else(|| usage())),
            "--undo" => undo = Some(it.next().unwrap_or_else(|| usage())),
            "--now" => now = Some(it.next().unwrap_or_else(|| usage()).parse::<u64>().unwrap_or_else(|_| fail("--now: not a number"))),
            f if f.starts_with("--") => fail(&format!("unknown flag {}", esc(f))),
            f => files.push(f.to_string()),
        }
    }
    if files.len() != 2 { usage(); }
    let clock = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let now = match now { Some(t) if t <= clock => t, Some(_) => fail("--now must not be later than this clock"), None => clock };
    let rd = |f: &str| std::fs::read(f).unwrap_or_else(|e| fail(&format!("{}: {e}", esc(f))));
    let (r, f) = (rd(&files[0]), rd(&files[1]));
    let u = undo.as_deref().map(rd);
    let pe = |e: &Option<String>| e.as_deref().map(|e| {
        let (h, x) = e.split_once(':').unwrap_or_else(|| fail("--expect <height>:<hash>"));
        (h.parse::<u32>().unwrap_or_else(|_| fail("--expect: bad height")), x.to_string())
    });
    let (e1, e2) = (pe(&expect), pe(&expect_tip));
    let rep = zapp_hints::verify_hints(&r, &f, u.as_deref(), &image, core.as_deref(), now,
                                                    e1.as_ref().map(|(h, x)| (*h, x.as_str())), e2.as_ref().map(|(h, x)| (*h, x.as_str())));
    for (ok, what) in &rep.checks { println!("  [{}] {what}", if *ok { " ok " } else { "FAIL" }); }
    match (&rep.verdict, &rep.statement) {
        (Verdict::Fail, _) | (_, None) => { println!("SWIFTSYNC_VERIFY_FAIL: {} check(s) failed", rep.checks.iter().filter(|c| !c.0).count()); exit(1) }
        (v, Some(z)) => {
            let h: String = z.block_hash.iter().rev().map(|b| format!("{b:02x}")).collect();
            let what = format!("{} is the hints file (format {}) for block {} {h}: {} outputs indexed, {} unspent ({} sat)",
                               esc(&files[1]), z.format, z.height, z.n_counted, z.n_unspent, z.unspent_value);
            if matches!(v, Verdict::Ok) { println!("SWIFTSYNC_VERIFY_OK: {what}"); exit(0) }
            let why = if !rep.identity_checked { "block H not compared with your header chain (--expect)" } else { "the core chain is below Core's minimum work or the checkpoint, or its tip was not compared (--expect-tip)" };
            println!("SWIFTSYNC_VERIFY_DEMO: {what} -- but {why}"); exit(2)
        }
    }
}
