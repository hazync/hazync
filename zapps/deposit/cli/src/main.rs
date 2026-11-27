//! `zapp-deposit`: the deposit-proof zyncAPP's CLI (docs/libzync/DEPOSIT_ZAPP.md), a zAPP module of its own
//! (docs/libzync/ZYNCAPPS.md §9). Until 15.11.2026 the check was `swiftsync-verify deposit` and the builder `zync-deposit`;
//! flags, verdict lines and exit codes are unchanged.
//!
//!   zapp-deposit --txid <txid> [--vout <n>] [--amount <sat>] [--script <hex>] [--op-return <hex>] [--min-depth <n>]
//!                [--min-validated-depth <n>] [--allow-pow-only-block] [--expect <height>:<hash>] [--mempool | --mempool-url <base>]
//!                [--bitcoin-cli "<cmd>"] [--max-tip-age <s>]
//!                [--no-require-identity] [--allow-demo]
//!                [--now <s>] [--network <name>] --image <hex> --vk <key file> --vk-sha256 <hex> [--headers-image <hex> --headers-vk <key file> --headers-vk-sha256 <hex>]
//!                <deposit.json>
//! `--image` / `--vk`: the engine core image id and its key; `--headers-image` / `--headers-vk`: the header-chain image and
//! its key, for a bundle with header runs. No engine image is published yet: every key is pinned by the caller (--vk-sha256,
//! --headers-vk-sha256), never to itself (review R1-6 #1).
//!       check a deposit proof against YOUR expectations. Exit 0 ACCEPTED (verdict OK: every check passed, the tip
//!       proof has Core's minimum chain work, and the tip was compared with --expect / --mempool / --bitcoin-cli),
//!       2 DEMO (valid, but below the minimum work or the core checkpoint, or the tip not compared: NOT a deposit to
//!       credit; the statement is printed only with --allow-demo), 1 FAIL. --no-require-identity: OK without a tip
//!       source (a caller that compares the tip by other means says so explicitly). --min-validated-depth: default 1 (the
//!       deposit's own block fully validated, its burial may be proof of work only); a block above the core tip takes
//!       --allow-pow-only-block AND --min-validated-depth 0.
//!   zapp-deposit build --core <core.hze> --image <hex> --vk <key file> --vk-sha256 <hex> --history <history.bin> [--headers <proof.hze>]...
//!                      [--headers-image <hex> --headers-vk <key file> --headers-vk-sha256 <hex>] [--headers-file <file>]
//!                --height <h> --txid <txid> (--block <raw block hex file> | --bitcoin-cli "<cmd>")
//!                [--tip-path <file>] [--network mainnet|signet] -o <deposit.json>
//!       build a bundle (was zync-deposit; holds no secrets, proves nothing: it runs the verifier on what it built)
//!   zapp-deposit mmr-path <headers file> <tip height> <height>     an MMR path, one 64-hex hash per line
//!
//! `build`: `--history`: the core proving run's history.bin (libzync::history), for the block's leaf in the core proof's
//! block forest (its coin state and undo hash). `--headers-file`: "height hex80" lines from genesis (as the header zAPP is
//! proved from), for the MMR paths of the core tip and of a block above it; `--tip-path`: the core tip's path, made
//! before with `mmr-path` (then the headers file is needed only for a block above the core tip). `--block`: the raw block
//! as `getblock <hash> 0` prints it; `--bitcoin-cli` asks a node for it instead.

use std::process::exit;
use swiftsync_verify::{hex, hex32, Verdict};
use zapp_deposit::{core_state, make_bundle, verify_deposit, DepositAsk};
use zapp_common::{Identity, IDENTITY_USAGE};
use zapp_headers::HeaderState;

fn usage() -> ! {
    println!("SWIFTSYNC_VERIFY_FAIL: bad arguments (see usage on stderr)");
    eprintln!("usage: zapp-deposit --txid <txid> [--vout <n>] [--amount <sat>] [--script <hex>] [--op-return <hex>] [--min-depth <n>] [--min-validated-depth <n> (default 1)] [--allow-pow-only-block (needs --min-validated-depth 0)] {IDENTITY_USAGE} [--no-require-identity] [--allow-demo] [--max-tip-age <s>] [--now <s>] [--network <name>] --image <hex> --vk <key file> --vk-sha256 <hex> [--headers-image <hex> --headers-vk <key file> --headers-vk-sha256 <hex>] <deposit.json>");
    eprintln!("       zapp-deposit build --core <core.hze> --image <hex> --vk <key file> --vk-sha256 <hex> --history <history.bin> [--headers <proof.hze>]... [--headers-image <hex> --headers-vk <key file> --headers-vk-sha256 <hex>] [--headers-file <file>] --height <h> --txid <txid> (--block <file> | --bitcoin-cli \"<cmd>\") [--tip-path <file>] [--network <name>] -o <deposit.json>");
    eprintln!("       zapp-deposit mmr-path <headers file> <tip height> <height>");
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
    eprintln!("zapp-deposit: {m}");
    exit(1);
}

fn unhex_or_die(s: &str) -> Vec<u8> {
    swiftsync_verify::unhex(s.trim()).unwrap_or_else(|| die("not hex"))
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a.first().map(|s| s.as_str()) {
        Some("build") => build_main(a[1..].to_vec()),
        Some("mmr-path") => build_main(a),
        _ => {
            // Any panic is a FAIL with exit 1: the documented outcomes are 0, 2 and 1 only.
            std::panic::set_hook(Box::new(|i| {
                use std::io::Write as _;
                let _ = writeln!(std::io::stdout(), "SWIFTSYNC_VERIFY_FAIL: {}", format!("{i}").escape_debug());
                exit(1);
            }));
            verify_main(a)
        }
    }
}

fn node(cmd: &str, args: &[&str]) -> String {
    let mut p = cmd.split_whitespace();
    let prog = p.next().unwrap_or_else(|| die("empty --bitcoin-cli"));
    let o = std::process::Command::new(prog).args(p).args(args).output().unwrap_or_else(|e| die(&format!("{prog}: {e}")));
    if !o.status.success() { die(&format!("{cmd} {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim())); }
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// Block hashes 0..=tip from a "height hex80" file that starts at genesis and links.
fn hashes_from(file: &str, tip: i64) -> Vec<[u8; 32]> {
    let mut out: Vec<[u8; 32]> = Vec::new();
    let text = std::fs::read_to_string(file).unwrap_or_else(|e| die(&format!("{file}: {e}")));
    for l in text.lines().filter(|l| !l.trim().is_empty()) {
        let mut f = l.split_whitespace();
        let h: i64 = f.next().and_then(|x| x.parse().ok()).unwrap_or_else(|| die("headers file: a line without a height"));
        if h > tip { break; }
        if h != out.len() as i64 { die(&format!("headers file: not consecutive from genesis at {h}")); }
        let b = unhex_or_die(f.next().unwrap_or_else(|| die("headers file: a line without a header")));
        if b.len() != 80 { die(&format!("headers file: header {h} is not 80 bytes")); }
        if let Some(p) = out.last() { if b[4..36] != p[..] { die(&format!("headers file: {h} does not link to {}", h - 1)); } }
        out.push(libzync::facts::header_hash(&b.try_into().unwrap()));
    }
    if out.len() as i64 != tip + 1 { die(&format!("headers file ends below the header tip {tip}")); }
    out
}

fn path(hs: &[[u8; 32]], h: u64) -> Vec<[u8; 32]> {
    zapp_headers::mmr::path(hs, h).unwrap_or_else(|| die(&format!("block {h} is not in 0..={}", hs.len() - 1)))
}

fn build_main(a: Vec<String>) {
    if a.first().map(|s| s.as_str()) == Some("mmr-path") {
        let n = |i: usize| -> u64 { a.get(i).and_then(|x| x.parse().ok()).unwrap_or_else(|| die("mmr-path <headers file> <tip> <height>")) };
        let hs = hashes_from(a.get(1).unwrap_or_else(|| die("mmr-path <headers file> <tip> <height>")), n(2) as i64);
        for p in path(&hs, n(3)) { println!("{}", hex(&p)); }
        return;
    }
    let get = |k: &str| a.iter().position(|x| x == k).and_then(|i| a.get(i + 1)).cloned();
    let all = |k: &str| a.windows(2).filter(|w| w[0] == k).map(|w| w[1].clone()).collect::<Vec<_>>();
    let need = |k: &str| get(k).unwrap_or_else(|| die(&format!("{k} is required (see the source header)")));
    let network = get("--network").unwrap_or("mainnet".into());
    let core = std::fs::read(need("--core")).unwrap_or_else(|e| die(&format!("--core: {e}")));
    let cs = core_state(&core).unwrap_or_else(|e| die(&e));
    let height: u32 = need("--height").parse().unwrap_or_else(|_| die("--height: not a height"));
    let mut txid = hex32(&need("--txid").to_lowercase()).unwrap_or_else(|| die("--txid: 64 hex digits"));
    txid.reverse();
    let headers: Vec<Vec<u8>> = all("--headers").iter().map(|f| std::fs::read(f).unwrap_or_else(|e| die(&format!("{f}: {e}")))).collect();
    let raw_block = match (get("--block"), get("--bitcoin-cli")) {
        (Some(f), _) => unhex_or_die(&std::fs::read_to_string(&f).unwrap_or_else(|e| die(&format!("{f}: {e}")))),
        (None, Some(cli)) => {
            let h = node(&cli, &["getblockhash", &height.to_string()]);
            unhex_or_die(&node(&cli, &["getblock", &h, "0"]))
        }
        _ => die("give --block <raw block hex file> or --bitcoin-cli \"<cmd>\""),
    };
    // the header chain's tip, from its last receipt's journal (the verifier checks the receipts)
    let htip: Option<HeaderState> = headers.last().map(|b| {
        let j = swiftsync_verify::proof_journal(b).unwrap_or_else(|e| die(&format!("header proof: {e}")));
        swiftsync_verify::decode_journal::<([u8; 32], HeaderState)>(&j).unwrap_or_else(|e| die(&format!("header journal: {e}"))).1
    });
    // --tip-path <file>: the core tip's MMR path, already made (`mmr-path`), instead of reading the headers file
    let read_path = |f: &str| -> Vec<[u8; 32]> {
        std::fs::read_to_string(f).unwrap_or_else(|e| die(&format!("{f}: {e}"))).lines().map(str::trim).filter(|l| !l.is_empty())
            .map(|l| hex32(&l.to_lowercase()).unwrap_or_else(|| die(&format!("{f}: not a 64-hex-digit hash per line")))).collect()
    };
    let given_tip_path = get("--tip-path").map(|f| read_path(&f));
    let hs = match (&htip, &given_tip_path) {
        (Some(s), None) => Some(hashes_from(&get("--headers-file").unwrap_or_else(|| die("header receipts need --headers-file or --tip-path")), s.height)),
        (Some(_), Some(_)) if height > cs.height => Some(hashes_from(&need("--headers-file"), htip.as_ref().unwrap().height)),
        _ => None,
    };
    let tip_path = match (&given_tip_path, &hs) { (Some(p), _) => p.clone(), (None, Some(h)) => path(h, cs.height as u64), _ => Vec::new() };
    let block_path = match &hs { Some(h) if height > cs.height => Some(path(h, height as u64)), _ => None };
    let history = get("--history").map(|f| std::fs::read(&f).unwrap_or_else(|e| die(&format!("{f}: {e}"))));
    let b = make_bundle(&network, &core, &headers, history.as_deref(), &raw_block, height, &txid, &tip_path, block_path.as_deref())
        .unwrap_or_else(|e| die(&e));
    // self-check with the verifier: the transaction at depth >= 1, a block above the core tip allowed (it is flagged)
    // as every zAPP resolves it (zapp_corecheck::image_for; "regtest" only under the tests-only regtest-test feature)
    let (img, hk) = keys(&network, get("--image"), get("--vk"), get("--vk-sha256"), get("--headers-vk"), get("--headers-vk-sha256")).unwrap_or_else(|e| die(&e));
    let himage = get("--headers-image").unwrap_or_default();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // (a self-check of the bundle's parts: the chain identity is the verifier's caller's, not the builder's)
    let ask = DepositAsk { txid, min_depth: 1, min_validated_depth: 0, allow_pow_only_block: true, require_identity: false, allow_demo: true, ..Default::default() };
    let rep = verify_deposit(&b, &ask, &network, img, &himage, hk.as_ref().map(|(b, p)| swiftsync_verify::EngineKey { bytes: b, pin: *p }).as_ref(), now);
    if rep.verdict == Verdict::Fail {
        for (_, w) in rep.checks.iter().filter(|c| !c.0) { eprintln!("  [FAIL] {w}"); }
        die("the bundle does not verify: not written");
    }
    let out = need("-o");
    let j = serde_json::to_string(&b).unwrap();
    std::fs::write(&out, &j).unwrap_or_else(|e| die(&format!("{out}: {e}")));
    let s = rep.statement.unwrap();
    eprintln!("zapp-deposit build: wrote {out} ({} bytes): tx at position {} of block {height}, {} confirmation(s) as of tip {} \
               ({} fully validated through {}){}", j.len(), b.block.index, s.confirmations, s.tip_height, s.validated_confirmations,
              s.validated_through, if s.block_fully_validated { "" } else { "; the block is ABOVE the core tip: proof of work only" });
}

/// `zapp-deposit --txid <txid> [...] <deposit.json>`: a deposit proof (docs/libzync/DEPOSIT_ZAPP.md). Every
/// expectation is the CALLER's (the txid, the output, the depth); the bundle (from `zapp-deposit build`) carries no txid and no
/// image id. Exit 0 OK, 2 DEMO (a valid statement on a chain below Core's minimum chain work), 1 FAIL. The verdict
/// line states what was shown: the block, the depth as of the tip (height, hash, chainwork), how much of it is fully
/// validated and how much proof of work only.
fn verify_main(a: Vec<String>) -> ! {
    use zapp_deposit::{disp, verify_deposit, DepositAsk, DepositBundle};
    let mut ask = DepositAsk::default();
    let (mut network, mut image_hex, mut himage, mut now, mut file, mut txid) =
        ("mainnet".to_string(), None, String::new(), None, None, None);
    let (mut vk, mut hvk): (Option<String>, Option<String>) = (None, None);
    let (mut vk_pin, mut hvk_pin): (Option<String>, Option<String>) = (None, None);
    let mut it = a.into_iter();
    let mut id = Identity::default();
    let num = |v: Option<String>, k: &str| -> u64 { v.and_then(|x| x.parse().ok()).unwrap_or_else(|| fail(&format!("{k}: not a number"))) };
    let hexarg = |v: Option<String>, k: &str| -> Vec<u8> { v.and_then(|x| swiftsync_verify::unhex(&x)).unwrap_or_else(|| fail(&format!("{k}: not hex"))) };
    while let Some(x) = it.next() {
        match x.as_str() {
            "--txid" => txid = it.next(),
            "--vout" => ask.vout = Some(u32::try_from(num(it.next(), "--vout")).unwrap_or_else(|_| fail("--vout: too large"))),
            "--amount" => ask.amount = Some(num(it.next(), "--amount")),
            "--script" => ask.script = Some(hexarg(it.next(), "--script")),
            "--op-return" => ask.op_return = Some(hexarg(it.next(), "--op-return")),
            "--min-depth" => ask.min_depth = num(it.next(), "--min-depth"),
            "--min-validated-depth" => ask.min_validated_depth = num(it.next(), "--min-validated-depth"),
            "--allow-pow-only-block" => ask.allow_pow_only_block = true,
            "--no-require-identity" => ask.require_identity = false,
            "--allow-demo" => ask.allow_demo = true,
            "--max-tip-age" => ask.max_tip_age = Some(num(it.next(), "--max-tip-age")),
            "--now" => now = Some(num(it.next(), "--now")),
            "--network" => network = it.next().unwrap_or_else(|| usage()),
            "--image" => image_hex = it.next(),
            "--headers-image" => himage = it.next().unwrap_or_else(|| usage()),
            "--vk" => vk = it.next(),
            "--vk-sha256" => vk_pin = it.next(),
            "--headers-vk" => hvk = it.next(),
            "--headers-vk-sha256" => hvk_pin = it.next(),
            f if id.take(f, &mut || it.next()).unwrap_or_else(|e| fail(&e)) => {}
            f if f.starts_with("--") => fail(&format!("unknown flag {}", esc(f))),
            f if file.is_none() => file = Some(f.to_string()),
            _ => usage(),
        }
    }
    let txid = txid.unwrap_or_else(|| fail("--txid <txid> (the transaction YOU ask about) is required"));
    let mut t = swiftsync_verify::hex32(&txid.to_lowercase()).unwrap_or_else(|| fail("--txid: not 64 hex digits"));
    t.reverse();
    ask.txid = t;
    let file = file.unwrap_or_else(|| usage());
    // the images as every zAPP resolves them (zapp_corecheck::image_for), with their keys (your own builds')
    let (image, hk) = keys(&network, image_hex, vk, vk_pin, hvk, hvk_pin).unwrap_or_else(|e| fail(&e));
    let hkey = hk.as_ref().map(|(b, p)| swiftsync_verify::EngineKey { bytes: b, pin: *p });
    let clock = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let now = match now { Some(t) if t <= clock => t, Some(_) => fail("--now must not be later than this clock"), None => clock };
    let b: DepositBundle = serde_json::from_slice(&std::fs::read(&file).unwrap_or_else(|e| fail(&format!("{}: {e}", esc(&file)))))
        .unwrap_or_else(|e| fail(&format!("{}: not a deposit bundle ({e})", esc(&file))));
    // chain identity (zapp-common): --mempool / --bitcoin-cli are asked for the block at the tip the depth is counted to
    // (the header chain's, or the core proof's), which the bundle gives, and their answer is checked as --expect is
    let parse = |e: &str| -> (i64, String) {
        let (h, x) = e.split_once(':').unwrap_or_else(|| fail("--expect <height>:<hash>"));
        (h.parse().unwrap_or_else(|_| fail("--expect: bad height")), x.to_string())
    };
    let mut extra = Vec::new();
    ask.expect_tip = id.expect.as_deref().map(parse);
    if id.needs_lookup() {
        // the tip to ask the sources about (read only for that: the statement is not used)
        let pre = verify_deposit(&b, &DepositAsk { expect_tip: None, require_identity: false, allow_demo: true, ..ask.clone() },
                                 &network, image, &himage, hkey.as_ref(), now);
        if let Some(s) = pre.statement.as_ref().filter(|_| pre.verdict != Verdict::Fail) {
            let (x, c) = zapp_common::expect_at(&id, &network, s.tip_height as u32, Some(&s.tip_work));
            ask.expect_tip = x.as_deref().map(parse);
            extra = c;
        }
    }
    let mut rep = verify_deposit(&b, &ask, &network, image, &himage, hkey.as_ref(), now);
    if extra.iter().any(|c| !c.0) { rep.verdict = Verdict::Fail; rep.accepted = false; rep.statement = None; }
    rep.checks.splice(0..0, extra);
    // a report on the tests-only regtest image says so on its FIRST line (zapps/corecheck's TEST_IMAGE_LINE, which the
    // core check puts first of its own lines, after the bundle's)
    #[cfg(feature = "regtest-test")]
    if let Some(i) = rep.checks.iter().position(|c| c.1 == format!("core: {}", zapp_corecheck::TEST_IMAGE_LINE)) {
        let c = rep.checks.remove(i);
        rep.checks.insert(0, c);
    }
    for (ok, what) in &rep.checks { println!("  [{}] {what}", if *ok { " ok " } else { "FAIL" }); }
    if rep.verdict == Verdict::Demo && rep.statement.is_none() {
        println!("SWIFTSYNC_VERIFY_DEMO: deposit NOT accepted: valid checks, but {}; the statement is withheld (--allow-demo shows it)",
                 if rep.checks.iter().any(|c| c.1.starts_with("chain identity NOT checked")) { "the tip was not compared with a source you trust" }
                 else { "the chain is below Core's minimum chain work or the core proof's checkpoint: a cheap fork can show this" });
        exit(2)
    }
    match (&rep.verdict, &rep.statement) {
        (Verdict::Fail, _) | (_, None) => { println!("SWIFTSYNC_VERIFY_FAIL: deposit: {} check(s) failed", rep.checks.iter().filter(|c| !c.0).count()); exit(1) }
        (v, Some(s)) => {
            let out = s.output.as_ref().map(|(n, v, spk)| format!(", output {n} = {v} sat to {}", hex(spk))).unwrap_or_default();
            let ident = if ask.expect_tip.is_some() { "" } else { "; chain identity NOT checked (give --expect <height>:<hash>, --mempool or --bitcoin-cli: a source you trust)" };
            let what = format!("deposit: transaction {}{out} is in block {} {}, {} confirmation(s) as of tip {} {} (chainwork {}); \
                                FULLY VALIDATED from genesis through block {} ({} of those confirmations){}{ident}",
                disp(&s.txid), s.height, disp(&s.block_hash), s.confirmations, s.tip_height, disp(&s.tip_hash), disp(&s.tip_work),
                s.validated_through, s.validated_confirmations,
                if s.block_fully_validated { if s.confirmations > s.validated_confirmations { "; blocks above it: PROOF OF WORK ONLY" } else { "" } }
                else { "; THE BLOCK ITSELF IS ABOVE THE FULLY VALIDATED TIP: PROOF OF WORK ONLY" });
            if rep.accepted && matches!(v, Verdict::Ok) { println!("SWIFTSYNC_VERIFY_OK: ACCEPTED: {what}"); exit(0) }
            println!("SWIFTSYNC_VERIFY_DEMO: NOT accepted (below Core's minimum chain work or the checkpoint, or the tip not compared): {what}"); exit(2)
        }
    }
}

/// The core image with its key, and the header-chain key (bytes, pin), from the flags: no engine image is published yet,
/// so the core image id and key are required, and each key is pinned by the CALLER (--vk-sha256, --headers-vk-sha256:
/// review R1-6 #1), never to itself.
fn keys(network: &str, image: Option<String>, vk: Option<String>, vk_pin: Option<String>, hvk: Option<String>,
        hvk_pin: Option<String>) -> Result<(&'static swiftsync_verify::Image, Option<(Vec<u8>, [u8; 32])>), String> {
    let (Some(image), Some(vk)) = (image, vk) else {
        return Err("no engine core image is published yet: give --image <hex>, --vk <key file> and --vk-sha256 <its pin>".into());
    };
    let pin = |p: &Option<String>| swiftsync_verify::hex32(p.as_deref().unwrap_or_default().trim()).unwrap_or_default();
    let b = swiftsync_verify::read_pinned_key(&vk, vk_pin.as_deref(), "--vk")?;
    let img = zapp_corecheck::with_key(zapp_corecheck::image_for(network, &image, false, false)?, &b, Some(&pin(&vk_pin)))?;
    let hk = match hvk {
        Some(f) => Some((swiftsync_verify::read_pinned_key(&f, hvk_pin.as_deref(), "--headers-vk")?, pin(&hvk_pin))),
        None => None,
    };
    Ok((img, hk))
}
