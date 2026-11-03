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
//!
//! `--handover <image hex>:<first height>` (repeat, oldest first): the lineage an image pins, for a reader who rebuilt
//! one that pins any (docs/libzync/HANDOVER.md).
//!
//! Chain identity (is the proven chain THE network's chain?). The proof shows a valid chain from genesis; which chain
//! it is, is the tip's hash, and a verifier compares it with a source it trusts:
//! - `--expect <height>:<hash>`: the proof's tip must be that block (display order, as `getblockhash` prints it);
//! - `--mempool`: ask mempool.space (one GET of /api/block-height/<height>; signet: /signet/api) for the block at the
//!   proof's height and require it to be the tip; `--mempool-url <base>` for another esplora-compatible API;
//! - `--bitcoin-cli` (step 7 above), your own node.
//! A failed identity check is FAIL. With none of them the verdict says the identity was NOT checked and prints the hash.
//!
//! `--coinlog`: the image is a coin-log build (docs/libzync/COINLOG.md, profile bit 4). It has no hints file: pass `-`.
//!
//! A proof whose chainwork is below Core's `nMinimumChainWork` for the network is at best DEMO (exit 2): a valid chain
//! from genesis, but a low-work fork costs little to mine, so it is no evidence of the network's chain.
//!
//! `<receipt>` may also be a proof bundle (`.sspf`, docs/libzync/PROOF_BUNDLE.md); its header is then checked
//! against the verified result. `swiftsync-verify [flags] bundle <receipt> <file.hints> <out.sspf>` writes one,
//! and refuses a receipt that does not verify.

use std::process::{exit, Command};
use swiftsync_verify::{check_bundle, hex, verify, Bundle, ChainState, Image, Inputs, Verdict, BUNDLE_MAGIC};

fn usage() -> ! {
    println!("SWIFTSYNC_VERIFY_FAIL: bad arguments (see usage on stderr)");
    eprintln!("usage: swiftsync-verify [--network mainnet|signet] [--image <hex>] [--coinlog] [--anchor state.json] [--bitcoin-cli \"<command>\"] [--now <unix seconds, earlier than the clock>] [--coin <fact.json>]... [--journey <journey.json>]... <receipt|bundle.sspf> <file.hints | - with --coinlog>");
    eprintln!("       swiftsync-verify [--network ...] [--image <hex>] bundle <receipt> <file.hints> <out.sspf>   (OK, or DEMO for low work only; never from an anchor)");
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
    let mut args = std::env::args().skip(1);
    let (mut network, mut anchor, mut cli, mut image_hex, mut files) = ("mainnet".to_string(), None, None, None, Vec::new());
    let mut now_arg: Option<String> = None;
    let mut handovers: Vec<swiftsync_verify::Handover> = Vec::new();
    let mut coinlog = false;
    let (mut expect, mut mempool): (Option<String>, Option<String>) = (None, None);
    let mut coin_facts: Vec<String> = Vec::new();
    let mut journeys: Vec<String> = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--network" => network = args.next().unwrap_or_else(|| usage()),
            "--anchor" => anchor = Some(args.next().unwrap_or_else(|| usage())),
            "--bitcoin-cli" => cli = Some(args.next().unwrap_or_else(|| usage())),
            "--image" => image_hex = Some(args.next().unwrap_or_else(|| usage())),
            "--now" => now_arg = Some(args.next().unwrap_or_else(|| usage())),
            "--coinlog" => coinlog = true,
            "--expect" => expect = Some(args.next().unwrap_or_else(|| usage())),
            "--mempool" => mempool = Some(String::new()),
            "--mempool-url" => mempool = Some(args.next().unwrap_or_else(|| usage())),
            "--coin" => coin_facts.push(args.next().unwrap_or_else(|| usage())),
            "--journey" => journeys.push(args.next().unwrap_or_else(|| usage())),
            "--handover" => {
                let v = args.next().unwrap_or_else(|| usage());
                let bad = || -> ! { fail(&format!("--handover {v:?}: want <64 hex digits>:<height>")) };
                let (id, h) = v.split_once(':').unwrap_or_else(|| bad());
                let id = swiftsync_verify::hex32(&id.to_lowercase()).unwrap_or_else(|| bad());
                handovers.push(swiftsync_verify::Handover {
                    image: swiftsync_verify::words(&risc0_zkvm::sha::Digest::from_bytes(id)),
                    first_height: h.parse().unwrap_or_else(|_| bad()),
                });
            }
            "-h" | "--help" => usage(),
            _ if a.starts_with("--") => usage(),
            _ => files.push(a),
        }
    }
    let make_bundle = files.first().map(|f| f == "bundle").unwrap_or(false);
    if make_bundle {
        files.remove(0);
    }
    let mut image = Image::by_network(&network).unwrap_or_else(|| fail(&format!("unknown network {network:?}")));
    if let Some(h) = &image_hex {
        image = image.with_method_id(&h.to_lowercase()).unwrap_or_else(|| fail(&format!("--image {h:?}: not 64 hex digits")));
        println!("  image id given on the command line: {}", image.method_id_hex);
    }
    if !handovers.is_empty() {
        image = image.with_lineage(handovers);
    }
    if coinlog {
        image = image.with_coinlog();
    }
    // `-` is "no hints file", and only a coin-log image has none.
    let read = |p: &str| -> Vec<u8> {
        if p == "-" {
            if !coinlog { fail("'-' (no hints file) is only for a coin-log image (--coinlog)") }
            return Vec::new();
        }
        std::fs::read(p).unwrap_or_else(|e| fail(&format!("{}: {e}", esc(p))))
    };
    let anchor_given = anchor.is_some();
    let anchor: Option<ChainState> = anchor.map(|p| {
        serde_json::from_slice(&read(&p)).unwrap_or_else(|e| fail(&format!("{}: not a ChainState: {e}", esc(&p))))
    });
    let mut now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("clock").as_secs();
    // For testing time-too-new. Only an earlier clock is accepted: it can make the check fail, never make it pass.
    if let Some(v) = now_arg {
        let t: u64 = v.parse().unwrap_or_else(|_| fail("--now takes unix seconds"));
        if t > now {
            fail("--now may only set the clock back");
        }
        now = t;
    }
    if make_bundle {
        let [receipt, hints, out] = files.as_slice() else { usage() };
        let b = Bundle::make(&read(receipt), &read(hints), image, anchor, now).unwrap_or_else(|e| fail(&e));
        std::fs::write(out, b.encode()).unwrap_or_else(|e| fail(&format!("{}: {e}", esc(out))));
        println!("SWIFTSYNC_BUNDLE_WRITTEN: {} (height {}, image {})", esc(out), b.stop_height, image.method_id_hex);
        exit(0);
    }
    let [receipt, hints] = files.as_slice() else { usage() };
    // A coin-log proof has no hints file: say so with '-', so an empty or wrong file is never taken for "none".
    if coinlog && hints != "-" { fail("a coin-log image takes no hints file: pass '-'"); }
    let (receipt_bytes, hints_bytes) = (read(receipt), read(hints));
    let mut rep = if receipt_bytes.starts_with(BUNDLE_MAGIC) {
        let b = Bundle::decode(&receipt_bytes).unwrap_or_else(|e| fail(&format!("{}: {e}", esc(receipt))));
        println!("  {} is a proof bundle; its header is checked against the verified result", esc(receipt));
        check_bundle(&b, &hints_bytes, image, anchor, now)
    } else {
        verify(&Inputs { receipt: &receipt_bytes, hints: &hints_bytes, image, anchor, now })
    };

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

    // Chain identity: the proven tip against a block the user trusts.
    if let Some(cs) = &rep.state {
        let tip = display(&cs.tip_hash);
        if let Some(e) = &expect {
            match swiftsync_verify::parse_expect(e) {
                Some((h, x)) => {
                    rep.checks.push((swiftsync_verify::identity_matches(h, &x, cs.height, &cs.tip_hash), format!("--expect: block {h} is {x}; the proof's tip is block {} {tip}{}",
                        cs.height, if h != cs.height { " (only the tip height can be checked from the receipt alone)" } else { "" })));
                    tip_checked = true;
                }
                None => rep.checks.push((false, format!("--expect {e:?}: want <height>:<64 hex digits>"))),
            }
        }
        if let Some(base) = &mempool {
            let base = if base.is_empty() {
                if image.network == "signet" { "https://mempool.space/signet/api".to_string() } else { "https://mempool.space/api".to_string() }
            } else { base.trim_end_matches('/').to_string() };
            let url = format!("{base}/block-height/{}", cs.height);
            let out = Command::new("curl").args(["-sf", "--max-time", "30", &url]).output();
            match out {
                Ok(o) if o.status.success() => {
                    let got = String::from_utf8_lossy(&o.stdout).trim().to_lowercase();
                    rep.checks.push((swiftsync_verify::identity_matches(cs.height, &got, cs.height, &cs.tip_hash), format!("{url} says block {} is {got:?}; the proof's tip is {tip}", cs.height)));
                    tip_checked = true;
                }
                Ok(o) => rep.checks.push((false, format!("{url}: request failed ({}): chain identity not checked", o.status))),
                Err(e) => rep.checks.push((false, format!("curl {url}: {e}"))),
            }
        }
        // zyncAPP coin facts (ZYNCAPPS.md §5) against this verified state.
        for p in &coin_facts {
            let f: Result<swiftsync_verify::CoinFact, String> = std::fs::read(p).map_err(|e| e.to_string())
                .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()));
            match f.and_then(|f| swiftsync_verify::check_coin_fact(cs, &f)) {
                Ok(line) => rep.checks.push((true, format!("--coin {}: {line}", esc(p)))),
                Err(e) => rep.checks.push((false, format!("--coin {}: {e}", esc(p)))),
            }
        }
        for p in &journeys {
            let f: Result<swiftsync_verify::CoinJourney, String> = std::fs::read(p).map_err(|e| e.to_string())
                .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()));
            match f.and_then(|f| swiftsync_verify::check_coin_journey(cs, &f)) {
                Ok(line) => rep.checks.push((true, format!("--journey {}: {line}", esc(p)))),
                Err(e) => rep.checks.push((false, format!("--journey {}: {e}", esc(p)))),
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
        if coinlog {
            println!("  coin log: {} coins ever appended, {} sat live (COINLOG.md)", cs.utxo_leaves, cs.utxo_value);
        } else {
            println!("  UTXO set at H: {} coins (Utreexo roots, UTREEXO_FORMAT.md)", cs.utxo_leaves);
        }
    }
    let step7 = if tip_checked { String::new() } else {
        rep.state.as_ref().map_or(String::new(), |c| format!("; chain identity NOT checked: compare block {} = {} with a source you trust (--mempool, --expect, --bitcoin-cli)",
            c.height, display(&c.tip_hash)))
    };
    let av = rep.state.as_ref().map_or(false, |c| c.profile & swiftsync_verify::PROFILE_ASSUMEVALID != 0);
    let h = rep.state.as_ref().map_or(0, |c| c.height);
    let avs = if av { " (ASSUMEVALID: scripts and signatures NOT checked)" } else { "" };
    let what = if coinlog { "the chain and its coin log".to_string() } else { esc(hints) };
    match rep.verdict {
        Verdict::Ok if coinlog => println!("SWIFTSYNC_VERIFY_OK: the chain above and its coin log are proven at height {h}{avs}{step7}"),
        Verdict::Ok => println!("SWIFTSYNC_VERIFY_OK: {} is proven correct for the chain above at height {h}{avs}{step7}", esc(hints)),
        Verdict::Demo if rep.low_work && anchor_given => println!("SWIFTSYNC_VERIFY_DEMO: {what}: FROM THE GIVEN ANCHOR ONLY, and below Core's minimum chainwork{step7}"),
        Verdict::Demo if rep.low_work => println!("SWIFTSYNC_VERIFY_DEMO: {what}: a valid chain from genesis, but below Core's minimum chainwork for {}: not shown to be the network's chain{step7}", image.network),
        Verdict::Demo => println!("SWIFTSYNC_VERIFY_DEMO: consistent with {what} FROM THE GIVEN ANCHOR ONLY -- not a proof{step7}"),
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
