//! `swiftsync-verify [--network mainnet|signet] [--image <hex>] [--anchor state.json] [--bitcoin-cli "<command>"] <receipt>`
//!
//! Checks a libzync chain proof (the chain from genesis, its coin log, and with the product image its counting trie
//! and live-coin hash) and the facts given with it, and prints every check. Exit 0 `SWIFTSYNC_VERIFY_OK`, 2
//! `SWIFTSYNC_VERIFY_DEMO` (a valid chain, but from a supplied anchor or below the minimum work: not shown to be the
//! network's), 1 `SWIFTSYNC_VERIFY_FAIL` (anything else, including bad input). The SwiftSync hints file (design B) is
//! retired: no proof takes one.
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
//! `--coinlog`, `--product`: kept for old command lines. The published images already are the product build (coin log,
//! counting trie, live-coin hash; profile bits 4-6), and the summary follows the proven profile, so neither is needed.
//! `--trie <fact.json>` (repeat): a counting-trie fact at the tip (coin unspent / spent by / absent, a script's balance,
//! its complete history), as `zync-path trie` writes it; checked against the verified state's trie root.
//!
//! A proof whose chainwork is below Core's `nMinimumChainWork` for the network is at best DEMO (exit 2): a valid chain
//! from genesis, but a low-work fork costs little to mine, so it is no evidence of the network's chain.
//!
//! `<receipt>` may also be a proof bundle (`.sspf`, docs/libzync/PROOF_BUNDLE.md); its header is then checked
//! against the verified result. `swiftsync-verify [flags] bundle <receipt> <out.sspf>` writes one,
//! and refuses a receipt that does not verify.

use std::process::{exit, Command};
use swiftsync_verify::{check_bundle, hex, verify, Bundle, ChainState, Image, Inputs, Verdict, BUNDLE_MAGIC};

fn usage() -> ! {
    println!("SWIFTSYNC_VERIFY_FAIL: bad arguments (see usage on stderr)");
    eprintln!("usage: swiftsync-verify [--network mainnet|signet] [--image <hex>] [--coinlog] [--anchor state.json] [--bitcoin-cli \"<command>\"] [--now <unix seconds, earlier than the clock>] [--product] [--coin <fact.json>]... [--journey <journey.json>]... [--trie <fact.json>]... <receipt|bundle.sspf>");
    eprintln!("       swiftsync-verify [--network ...] [--image <hex>] bundle <receipt> <out.sspf>   (OK, or DEMO for low work only; never from an anchor)");
    eprintln!("       swiftsync-verify headers [--image <hex>] [--expect <height>:<hash>] [--now <s>] [--include <height>:<hash> --path <file> [--tx <tx.json>]] <receipt>...");
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
    if std::env::args().nth(1).as_deref() == Some("headers") {
        headers_main(std::env::args().skip(2).collect());
    }
    let mut args = std::env::args().skip(1);
    let (mut network, mut anchor, mut cli, mut image_hex, mut files) = ("mainnet".to_string(), None, None, None, Vec::new());
    let mut now_arg: Option<String> = None;
    let mut handovers: Vec<swiftsync_verify::Handover> = Vec::new();
    let mut coinlog = false;
    let mut product = false;
    let mut trie_facts: Vec<String> = Vec::new();
    let (mut expect, mut mempool): (Option<String>, Option<String>) = (None, None);
    let mut coin_facts: Vec<String> = Vec::new();
    let mut journeys: Vec<String> = Vec::new();
    let mut n_facts = 0usize;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--network" => network = args.next().unwrap_or_else(|| usage()),
            "--anchor" => anchor = Some(args.next().unwrap_or_else(|| usage())),
            "--bitcoin-cli" => cli = Some(args.next().unwrap_or_else(|| usage())),
            "--image" => image_hex = Some(args.next().unwrap_or_else(|| usage())),
            "--now" => now_arg = Some(args.next().unwrap_or_else(|| usage())),
            "--coinlog" => coinlog = true,
            "--product" => { coinlog = true; product = true }
            "--trie" => trie_facts.push(args.next().unwrap_or_else(|| usage())),
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
    if product {
        image = image.with_product();
    } else if coinlog {
        image = image.with_coinlog();
    }
    let read = |p: &str| -> Vec<u8> { std::fs::read(p).unwrap_or_else(|e| fail(&format!("{}: {e}", esc(p)))) };
    // The retired hints-file argument: say so rather than print the usage.
    if files.iter().any(|f| f == "-" || f.ends_with(".hints")) {
        fail("no proof takes a SwiftSync hints file any more (design B is retired, E2): give the receipt alone");
    }
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
        let [receipt, out] = files.as_slice() else { usage() };
        let b = Bundle::make(&read(receipt), image, anchor, now).unwrap_or_else(|e| fail(&e));
        std::fs::write(out, b.encode()).unwrap_or_else(|e| fail(&format!("{}: {e}", esc(out))));
        println!("SWIFTSYNC_BUNDLE_WRITTEN: {} (height {}, image {})", esc(out), b.stop_height, image.method_id_hex);
        exit(0);
    }
    let [receipt] = files.as_slice() else { usage() };
    let receipt_bytes = read(receipt);
    let mut rep = if receipt_bytes.starts_with(BUNDLE_MAGIC) {
        let b = Bundle::decode(&receipt_bytes).unwrap_or_else(|e| fail(&format!("{}: {e}", esc(receipt))));
        println!("  {} is a proof bundle; its header is checked against the verified result", esc(receipt));
        check_bundle(&b, image, anchor, now)
    } else {
        verify(&Inputs { receipt: &receipt_bytes, image, anchor, now })
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
        // zyncAPP coin facts (ZYNCAPPS.md §8) against this verified state.
        // Facts are checked only against a state whose receipt verified: on a failed receipt the journal is the
        // prover's word, so a fact "holding" against it means nothing (review 11-03).
        let receipt_failed = rep.verdict == Verdict::Fail;
        n_facts = coin_facts.len() + journeys.len() + trie_facts.len();
        if receipt_failed {
            for p in coin_facts.iter().chain(journeys.iter()).chain(trie_facts.iter()) {
                rep.checks.push((false, format!("{}: NOT checked: the receipt did not verify", esc(p))));
            }
            coin_facts.clear();
            journeys.clear();
            trie_facts.clear();
        }
        for p in &trie_facts {
            let f: Result<swiftsync_verify::TrieFact, String> = std::fs::read(p).map_err(|e| e.to_string())
                .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()));
            match f.and_then(|f| swiftsync_verify::check_trie_fact(cs, &f)) {
                Ok(line) => rep.checks.push((true, format!("--trie {}: {line}", esc(p)))),
                Err(e) => rep.checks.push((false, format!("--trie {}: {e}", esc(p)))),
            }
        }
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
    // Only a journal that passed: under FAIL its fields are the prover's word (review round 3).
    if let Some(cs) = rep.state.as_ref().filter(|_| rep.verdict != Verdict::Fail) {
        println!("  chain: tip {} at height {}, cum_work 0x{}", display(&cs.tip_hash), cs.height, work_hex(&cs.cum_work));
        // Described by the PROVEN profile (it passed: cs.profile == the image's), not by the flags: the published images
        // already are the product build, so without --product a v2 journal was summarised the v1 way ("UTXO set at H:
        // N coins (Utreexo roots)"), its verdict right and its description wrong (stranger test 11-12, ROADMAP O8).
        if cs.profile & swiftsync_verify::PROFILE_COINLOG != 0 {
            println!("  coin log: {} coins ever appended, {} sat live (COINLOG.md)", cs.utxo_leaves, cs.utxo_value);
            if cs.profile & swiftsync_verify::PROFILE_TRIE != 0 {
                println!("  counting trie root: {} (absence, history, balances: --trie)", hex(&cs.trie_root));
                // live_set is a flag, not a count: 0 = the hash is the identity (no live coins), 1 = live_hash is the point
                println!("  live-coin hash: {} (instant IBD: a downloaded UTXO set must hash to it)",
                         if cs.live_set == 0 { "the empty set".to_string() } else { format!("x {:08x}..", cs.live_hash[7]) });
            }
        } else {
            println!("  UTXO set at H: {} coins (Utreexo roots, UTREEXO_FORMAT.md)", cs.utxo_leaves);
        }
    }
    // Facts about a chain whose tip nobody compared with the network: a valid chain with enough work can still be a
    // stale branch, and a transaction "in block h" of it may have been reorged out. So facts need step 7 for OK
    // (adversarial review, 11-03).
    let facts_unanchored = rep.verdict == Verdict::Ok && !tip_checked && n_facts > 0;
    if facts_unanchored {
        rep.verdict = Verdict::Demo;
    }
    let step7 = if tip_checked { String::new() } else {
        rep.state.as_ref().map_or(String::new(), |c| format!("; chain identity NOT checked: compare block {} = {} with a source you trust (--mempool, --expect, --bitcoin-cli)",
            c.height, display(&c.tip_hash)))
    };
    let av = rep.state.as_ref().map_or(false, |c| c.profile & swiftsync_verify::PROFILE_ASSUMEVALID != 0);
    let h = rep.state.as_ref().map_or(0, |c| c.height);
    let avs = if av { " (ASSUMEVALID: scripts and signatures NOT checked)" } else { "" };
    let what = "the chain and its coin log";
    match rep.verdict {
        Verdict::Demo if facts_unanchored => println!("SWIFTSYNC_VERIFY_DEMO: the chain and the facts check, but facts need the tip compared with the network (--mempool, --expect or --bitcoin-cli){step7}"),
        Verdict::Ok => println!("SWIFTSYNC_VERIFY_OK: the chain above and its coin log are proven at height {h}{avs}{step7}"),
        Verdict::Demo if rep.low_work && anchor_given => println!("SWIFTSYNC_VERIFY_DEMO: {what}: FROM THE GIVEN ANCHOR ONLY, and below Core's minimum chainwork or the pinned checkpoint{step7}"),
        Verdict::Demo if rep.low_work => println!("SWIFTSYNC_VERIFY_DEMO: {what}: a valid chain from genesis, but below Core's minimum chainwork or the pinned checkpoint for {}: not shown to be the network's chain{step7}", image.network),
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

/// `swiftsync-verify headers [--image <hex>] [--expect <height>:<hash>] [--now <unix seconds>]
/// [--include <height>:<hash> --path <file> [--tx <tx.json>]] <receipt>...`: a
/// proof of the mainnet header chain from genesis (the header-chain zyncAPP), as one receipt or a chain of runs, the
/// first from genesis and each next one starting where the one before ended. Exit 0 OK (chainwork at least Core's
/// minimum), 2 DEMO (below it), 1 FAIL.
/// `--include`: also check that block <hash> (as explorers print it) is at <height> on the proven chain, by its MMR
/// path (`--path`: the file `zyncapp headers-mmr-path` prints, one 64-hex hash per line, lowest first; empty for a
/// one-block peak). `--tx <tx.json>`: and that a transaction is in that block, by the JSON the path service's /tx
/// answers: the block's header, the raw transaction and its merkle branch, and the raw coinbase and its branch, which
/// pin the tree's depth (headers.rs, verify_tx_in_header, explains the 64-byte attack this closes).
fn headers_main(a: Vec<String>) -> ! {
    let (mut image, mut expect, mut now, mut files) = (swiftsync_verify::headers::HEADERS_IMAGE_ID.to_string(), None, None, Vec::new());
    let (mut include, mut path, mut tx) = (None, None, None);
    let mut it = a.into_iter();
    while let Some(x) = it.next() {
        match x.as_str() {
            "--image" => image = it.next().unwrap_or_else(|| usage()),
            "--expect" => expect = Some(it.next().unwrap_or_else(|| usage())),
            "--now" => now = Some(it.next().unwrap_or_else(|| usage()).parse::<u64>().unwrap_or_else(|_| fail("--now: not a number"))),
            "--include" => include = Some(it.next().unwrap_or_else(|| usage())),
            "--path" => path = Some(it.next().unwrap_or_else(|| usage())),
            "--tx" => tx = Some(it.next().unwrap_or_else(|| usage())),
            f if f.starts_with("--") => fail(&format!("headers: unknown flag {}", esc(f))),
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
            let t = tx.as_ref().map(|f| {
                let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(f).unwrap_or_else(|e| fail(&format!("{}: {e}", esc(f)))))
                    .unwrap_or_else(|e| fail(&format!("--tx {}: not JSON ({e})", esc(f))));
                swiftsync_verify::headers::TxInclusion::from_json(&j).unwrap_or_else(|e| fail(&format!("--tx: {e}")))
            });
            Some(swiftsync_verify::headers::Inclusion::parse(h, x, &pr, t).unwrap_or_else(|e| fail(&format!("--include: {e}"))))
        }
        _ => fail("--include <height>:<hash> needs --path <file> (and --tx needs --include)"),
    };
    let clock = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let now = match now { Some(t) if t <= clock => t, Some(_) => fail("--now must not be later than this clock"), None => clock };
    let bytes: Vec<Vec<u8>> = files.iter().map(|f| std::fs::read(f).unwrap_or_else(|e| fail(&format!("{}: {e}", esc(f))))).collect();
    let exp = expect.as_deref().map(|e| {
        let (h, x) = e.split_once(':').unwrap_or_else(|| fail("--expect <height>:<hash>"));
        (h.parse::<i64>().unwrap_or_else(|_| fail("--expect: bad height")), x)
    });
    let refs: Vec<&[u8]> = bytes.iter().map(|b| b.as_slice()).collect();
    let rep = swiftsync_verify::headers::verify_headers(&refs, &image, now, exp, inc.as_ref());
    for (ok, what) in &rep.checks { println!("  [{}] {what}", if *ok { " ok " } else { "FAIL" }); }
    let ident = if expect.is_some() { "" } else { "; chain identity NOT checked (give --expect <height>:<hash> from a source you trust)" };
    match (&rep.verdict, &rep.state) {
        (Verdict::Fail, _) | (_, None) => { println!("SWIFTSYNC_VERIFY_FAIL: {} check(s) failed", rep.checks.iter().filter(|c| !c.0).count()); exit(1) }
        (v, Some(s)) => {
            let what = format!("header chain from genesis to {} {} (chainwork {}, MMR root {}){ident}", s.height, s.hash_hex(), s.work_hex(), s.mmr_root_hex());
            if matches!(v, Verdict::Ok) { println!("SWIFTSYNC_VERIFY_OK: {what}"); exit(0) }
            println!("SWIFTSYNC_VERIFY_DEMO: a valid header chain below Core's minimum chain work: {what}"); exit(2)
        }
    }
}
