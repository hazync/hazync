//! `swiftsync-verify [--network mainnet|signet] [--image <hex>] [--anchor state.json] [--bitcoin-cli "<command>"] <receipt> <file.hints>`
//!
//! Checks a libzync proof that a SwiftSync hints file is correct (docs/libzync/SWIFTSYNC_PROOFS.md §5) and
//! prints every check. Exit 0 `SWIFTSYNC_VERIFY_OK`, 2 `SWIFTSYNC_VERIFY_DEMO` (checked against a supplied
//! anchor: never a proof of the file), 1 `SWIFTSYNC_VERIFY_FAIL` (anything else, including bad input).
//!
//! Step 7 (the proven tip is on YOUR most-work chain) needs your node: with `--bitcoin-cli "bitcoin-cli -signet"`
//! (any command line that accepts RPC names as arguments) it asks the node for its block at the proof's height
//! and that block's chainwork, and fails unless both equal the proof's. Without it, the tip and work are printed
//! for you to compare, and the verdict line says the step was not run.
//!
//! `--image <hex>` replaces the built-in image id for the network: the id you rebuilt yourself (§5 step 9).

use std::process::{exit, Command};
use swiftsync_verify::{hex, verify, ChainState, Image, Inputs, Verdict};

fn usage() -> ! {
    eprintln!("usage: swiftsync-verify [--network mainnet|signet] [--image <hex>] [--anchor state.json] [--bitcoin-cli \"<command>\"] <receipt> <file.hints>");
    exit(1);
}

fn fail(msg: &str) -> ! {
    println!("SWIFTSYNC_VERIFY_FAIL: {msg}");
    exit(1);
}

fn main() {
    // Any panic is a FAIL with exit 1: the documented outcomes are 0, 2 and 1 only.
    std::panic::set_hook(Box::new(|i| {
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), "SWIFTSYNC_VERIFY_FAIL: {i}");
        exit(1);
    }));
    let mut args = std::env::args().skip(1);
    let (mut network, mut anchor, mut cli, mut image_hex, mut files) = ("mainnet".to_string(), None, None, None, Vec::new());
    while let Some(a) = args.next() {
        match a.as_str() {
            "--network" => network = args.next().unwrap_or_else(|| usage()),
            "--anchor" => anchor = Some(args.next().unwrap_or_else(|| usage())),
            "--bitcoin-cli" => cli = Some(args.next().unwrap_or_else(|| usage())),
            "--image" => image_hex = Some(args.next().unwrap_or_else(|| usage())),
            "-h" | "--help" => usage(),
            _ if a.starts_with("--") => usage(),
            _ => files.push(a),
        }
    }
    let [receipt, hints] = files.as_slice() else { usage() };
    let mut image = Image::by_network(&network).unwrap_or_else(|| fail(&format!("unknown network {network}")));
    if let Some(h) = &image_hex {
        image = image.with_method_id(&h.to_lowercase()).unwrap_or_else(|| fail(&format!("--image {h}: not 64 hex digits")));
        println!("  image id given on the command line: {}", image.method_id_hex);
    }
    let read = |p: &str| std::fs::read(p).unwrap_or_else(|e| fail(&format!("{p}: {e}")));
    let anchor: Option<ChainState> = anchor.map(|p| {
        serde_json::from_slice(&read(&p)).unwrap_or_else(|e| fail(&format!("{p}: not a ChainState: {e}")))
    });
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("clock").as_secs();
    let (receipt_bytes, hints_bytes) = (read(receipt), read(hints));
    let mut rep = verify(&Inputs { receipt: &receipt_bytes, hints: &hints_bytes, image, anchor, now });

    // Step 7, against the user's own node.
    let mut tip_checked = false;
    if let (Some(cmd), Some(cs)) = (&cli, &rep.state) {
        match node_tip(cmd, cs.height) {
            Err(e) => rep.checks.push((false, format!("your node's chain ({e})"))),
            Ok((hash, work)) => {
                let tip = display(&cs.tip_hash);
                let pw = work_hex(&cs.cum_work);
                rep.checks.push((hash == tip, format!("your node's block {} is the proven tip {tip} (node: {hash})", cs.height)));
                rep.checks.push((work == pw, format!("your node's chainwork at it equals the proven cum_work {pw} (node: {work})")));
                tip_checked = true;
            }
        }
        if rep.checks.iter().any(|(ok, _)| !ok) {
            rep.verdict = Verdict::Fail;
        }
    }

    for (ok, what) in &rep.checks {
        println!("  [{}] {what}", if *ok { "ok" } else { "FAIL" });
    }
    if let Some(cs) = &rep.state {
        println!("  chain: tip {} at height {}, cum_work 0x{}", display(&cs.tip_hash), cs.height, work_hex(&cs.cum_work));
        println!("  UTXO set at H: {} coins (Utreexo roots, UTREEXO_FORMAT.md)", cs.utxo_leaves);
    }
    let step7 = if tip_checked { "" } else { "; step 7 NOT run: check the tip and work above against your own node" };
    let av = rep.state.as_ref().map_or(false, |c| c.profile & swiftsync_verify::PROFILE_ASSUMEVALID != 0);
    match rep.verdict {
        Verdict::Ok => println!("SWIFTSYNC_VERIFY_OK: {hints} is proven correct for the chain above at height {}{}{step7}",
            rep.state.as_ref().map_or(0, |c| c.height), if av { " (ASSUMEVALID: scripts and signatures NOT checked)" } else { "" }),
        Verdict::Demo => println!("SWIFTSYNC_VERIFY_DEMO: consistent with {hints} FROM THE GIVEN ANCHOR ONLY -- not a proof of the file{step7}"),
        Verdict::Fail => println!("SWIFTSYNC_VERIFY_FAIL: {} check(s) failed", rep.checks.iter().filter(|(ok, _)| !ok).count()),
    }
    exit(rep.verdict.exit_code());
}

/// Ask the node for (block hash at `height`, that block's chainwork), both as the node prints them.
fn node_tip(cmd: &str, height: u32) -> Result<(String, String), String> {
    let run = |extra: &[&str]| -> Result<String, String> {
        let mut parts = cmd.split_whitespace();
        let prog = parts.next().ok_or("empty --bitcoin-cli")?;
        let out = Command::new(prog).args(parts).args(extra).output().map_err(|e| format!("{prog}: {e}"))?;
        if !out.status.success() {
            return Err(format!("{cmd} {}: {}", extra.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let hash = run(&["getblockhash", &height.to_string()])?;
    let header: serde_json::Value = serde_json::from_str(&run(&["getblockheader", &hash])?).map_err(|e| e.to_string())?;
    let work = header["chainwork"].as_str().ok_or("getblockheader has no chainwork")?.to_string();
    Ok((hash, work))
}

/// A hash as `getblockhash` prints it (reversed internal order).
fn display(h: &[u8; 32]) -> String {
    let mut r = *h;
    r.reverse();
    hex(&r)
}

/// cum_work (32 bytes LE) as `getblockheader` prints chainwork: 64 hex digits, big-endian.
fn work_hex(w: &[u8; 32]) -> String {
    display(w)
}
