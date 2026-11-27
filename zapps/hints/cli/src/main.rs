//! `zapp-hints`: the SwiftSync hints zyncAPP's CLI (docs/libzync/HINTS_ZAPP.md), a zAPP module of its own
//! (docs/libzync/ZYNCAPPS.md §9) on the core verifier's library. Until 15.11.2026 it was `swiftsync-verify hints`;
//! the flags, verdict lines and exit codes are unchanged.

use std::process::exit;
use swiftsync_verify::{Verdict, PROFILE_SIGNET};
use zapp_common::{Identity, IDENTITY_USAGE};

fn usage() -> ! {
    println!("SWIFTSYNC_VERIFY_FAIL: bad arguments (see usage on stderr)");
    eprintln!("usage: zapp-hints --image <hex> --vk <key file> --vk-sha256 <hex> --core-image <hex> {IDENTITY_USAGE} [--expect-tip <T>:<hash>] [--undo <file>] [--now <unix seconds>] <proof.hze> <hints file>");
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

/// `zapp-hints --image <hex> --vk <key file> --vk-sha256 <hex> --core-image <hex> [--expect <H>:<hash>] [--expect-tip <T>:<hash>]
/// [--undo <file>] [--now <unix seconds>] <proof.hze> <hints file>`: the SwiftSync hints zyncAPP (docs/libzync/HINTS_ZAPP.md).
/// `--expect` is block H from YOUR node's header chain; `--mempool` / `--bitcoin-cli` (zapp-common, every zAPP CLI's
/// chain-identity flags) are asked for block H, and for the core tip when it is above H and no `--expect-tip` is given. Exit 0 OK, 2 DEMO (below minimum work or the checkpoint, or the
/// block at H / the core tip not compared), 1 FAIL. The proof is an ENGINE (OpenVM) proof file; `--vk` is the key file
/// of the hints image that proved it, from your own build of that image: its SHA-256 is the pin (engine/VERIFIER.md).
/// No engine image is published yet, so `--image`, `--vk` and `--core-image` are all required.
fn run(a: Vec<String>) -> ! {
    let (mut image, mut core, mut id, mut expect_tip, mut undo, mut now, mut files) =
        (String::new(), None, Identity::default(), None, None, None, Vec::new());
    let mut vk_path: Option<String> = None;
    let mut vk_pin: Option<String> = None;
    let mut it = a.into_iter();
    while let Some(x) = it.next() {
        match x.as_str() {
            "--image" => image = it.next().unwrap_or_else(|| usage()),
            "--vk" => vk_path = Some(it.next().unwrap_or_else(|| usage())),
            "--vk-sha256" => vk_pin = Some(it.next().unwrap_or_else(|| usage())),
            "--core-image" => core = Some(it.next().unwrap_or_else(|| usage())),
            "--expect-tip" => expect_tip = Some(it.next().unwrap_or_else(|| usage())),
            "--undo" => undo = Some(it.next().unwrap_or_else(|| usage())),
            "--now" => now = Some(it.next().unwrap_or_else(|| usage()).parse::<u64>().unwrap_or_else(|_| fail("--now: not a number"))),
            f if id.take(f, &mut || it.next()).unwrap_or_else(|e| fail(&e)) => {}
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
    // the key file, pinned by the CALLER (--vk-sha256; review R1-6 #1): never pinned to itself
    if image.is_empty() || vk_path.is_none() || core.is_none() {
        fail("no engine hints image is published yet: give --image, --vk and --core-image (the images you rebuilt)");
    }
    let core = core.unwrap();
    let vk = swiftsync_verify::read_pinned_key(vk_path.as_deref().unwrap(), vk_pin.as_deref(), "--vk").unwrap_or_else(|e| fail(&esc(&e)));
    let key = swiftsync_verify::EngineKey { bytes: &vk, pin: swiftsync_verify::hex32(vk_pin.as_deref().unwrap_or_default().trim()).unwrap_or_default() };
    let pe = |e: &Option<String>| e.as_deref().map(|e| {
        let (h, x) = e.split_once(':').unwrap_or_else(|| fail("--expect <height>:<hash>"));
        (h.parse::<u32>().unwrap_or_else(|_| fail("--expect: bad height")), x.to_string())
    });
    // chain identity (zapp-common): --mempool / --bitcoin-cli are asked for block H (and for the core tip when it is
    // above H and no --expect-tip is given), which the receipt names, and their answers are checked as --expect is
    let (mut expect, mut extra) = (id.expect.clone(), Vec::new());
    if id.needs_lookup() {
        let pre = zapp_hints::verify_hints(&r, &f, u.as_deref(), &image, &key, &core, now, None, None);
        if let Some(z) = pre.statement.as_ref().filter(|_| pre.verdict != Verdict::Fail) {
            let net = if z.profile & PROFILE_SIGNET != 0 { "signet" } else { "mainnet" };
            // the proof's chainwork is the core tip's: it is block H's only when H is the tip
            let (x, c) = zapp_common::expect_at(&id, net, z.height, (z.height == z.core_height).then_some(&z.cum_work));
            expect = x;
            extra.extend(c);
            if z.core_height > z.height && expect_tip.is_none() {
                let (x, c) = zapp_common::expect_at(&Identity { expect: None, ..id.clone() }, net, z.core_height, Some(&z.cum_work));
                expect_tip = x;
                extra.extend(c);
            }
        }
    }
    let (e1, e2) = (pe(&expect), pe(&expect_tip));
    let mut rep = zapp_hints::verify_hints(&r, &f, u.as_deref(), &image, &key, &core, now,
                                                    e1.as_ref().map(|(h, x)| (*h, x.as_str())), e2.as_ref().map(|(h, x)| (*h, x.as_str())));
    if extra.iter().any(|c| !c.0) { rep.verdict = Verdict::Fail; }
    rep.checks.splice(0..0, extra);
    for (ok, what) in &rep.checks { println!("  [{}] {what}", if *ok { " ok " } else { "FAIL" }); }
    match (&rep.verdict, &rep.statement) {
        (Verdict::Fail, _) | (_, None) => { println!("SWIFTSYNC_VERIFY_FAIL: {} check(s) failed", rep.checks.iter().filter(|c| !c.0).count()); exit(1) }
        (v, Some(z)) => {
            let h: String = z.block_hash.iter().rev().map(|b| format!("{b:02x}")).collect();
            let what = format!("{} is the hints file (format {}) for block {} {h}: {} outputs indexed, {} unspent ({} sat)",
                               esc(&files[1]), z.format, z.height, z.n_counted, z.n_unspent, z.unspent_value);
            if matches!(v, Verdict::Ok) { println!("SWIFTSYNC_VERIFY_OK: {what}"); exit(0) }
            let why = if !rep.identity_checked { "block H not compared with your header chain (--expect, --mempool or --bitcoin-cli)" } else { "the core chain is below Core's minimum work or the checkpoint, or its tip was not compared (--expect-tip)" };
            println!("SWIFTSYNC_VERIFY_DEMO: {what} -- but {why}"); exit(2)
        }
    }
}
