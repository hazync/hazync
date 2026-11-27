//! `zapp-zeroconf`: the zero-confirmation zyncAPP's CLI (zapps/zeroconf), a zAPP module of its own on the core
//! verifier's library.
//!
//! ```text
//!   zapp-zeroconf <proof.hze> --vk <key file> --vk-sha256 <hex> --image <hex> --core-image <hex> --txid <txid>
//!                 [--network mainnet|signet] [--anchor state.json] [--now <unix seconds>]
//!                 [--expect <tip height>:<hash>] [--mempool | --mempool-url <base>] [--bitcoin-cli "<cmd>"] [--json]
//! ```
//! Checks that the proof says YOUR transaction (`--txid`, as explorers print it) is valid for the block after the proven
//! tip, with every coin it spends unspent there. The proof is an engine (OpenVM) proof file of the zeroconf image
//! `--image`, verified under the key file `--vk` from your own build of that image (its SHA-256 is the pin). The core
//! chain it rests on must be `--core-image`'s (an engine build). `--anchor`: a trusted state the core chain was proved
//! from instead of genesis (then at most DEMO). `--expect`, `--mempool`, `--bitcoin-cli` name the tip from your own
//! header chain (zapps/common); without one the verdict is at most DEMO.
//! Exit 0 OK, 2 DEMO, 1 FAIL. ⚠ Valid now, not final: a conflicting transaction can still confirm first.
use std::process::exit;
use swiftsync_verify::{ChainState, EngineKey, Verdict};
use zapp_common::{Identity, IDENTITY_USAGE};

fn usage() -> ! {
    println!("ZAPP_ZEROCONF_FAIL: bad arguments (see usage on stderr)");
    eprintln!("usage: zapp-zeroconf <proof.hze> --vk <key file> --vk-sha256 <hex> --image <hex> --core-image <hex> --txid <txid> [--network mainnet|signet] [--anchor state.json] [--now <unix seconds>] {IDENTITY_USAGE} [--json]");
    exit(1);
}

fn fail(msg: &str) -> ! {
    println!("ZAPP_ZEROCONF_FAIL: {msg}");
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
        let _ = writeln!(std::io::stdout(), "ZAPP_ZEROCONF_FAIL: {}", format!("{i}").escape_debug());
        exit(1);
    }));
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (mut network, mut vk, mut image, mut core, mut txid, mut anchor, mut now, mut json, mut id, mut files) =
        ("mainnet".to_string(), None, None, None, None, None, None, false, Identity::default(), Vec::new());
    let mut vk_pin: Option<String> = None;
    let mut it = a.into_iter();
    while let Some(x) = it.next() {
        let mut val = |f: &str| it.next().unwrap_or_else(|| fail(&format!("{f} needs a value")));
        match x.as_str() {
            "--network" => network = val("--network"),
            "--vk" => vk = Some(val("--vk")),
            "--vk-sha256" => vk_pin = Some(val("--vk-sha256")),
            "--image" => image = Some(val("--image")),
            "--core-image" => core = Some(val("--core-image")),
            "--txid" => txid = Some(val("--txid")),
            "--anchor" => anchor = Some(val("--anchor")),
            "--now" => now = Some(val("--now").parse::<u64>().unwrap_or_else(|_| fail("--now: not a number"))),
            "--json" => json = true,
            "-h" | "--help" => usage(),
            f if id.take(f, &mut || it.next()).unwrap_or_else(|e| fail(&e)) => {}
            f if f.starts_with("--") => fail(&format!("unknown flag {}", esc(f))),
            f => files.push(f.to_string()),
        }
    }
    let (Some(vk), Some(image), Some(core), Some(txid)) = (vk, image, core, txid) else { usage() };
    if files.len() != 1 { usage(); }
    let clock = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let now = match now { Some(t) if t <= clock => t, Some(_) => fail("--now must not be later than this clock"), None => clock };
    let rd = |f: &str| std::fs::read(f).unwrap_or_else(|e| fail(&format!("{}: {e}", esc(f))));
    // the key pinned by the CALLER (--vk-sha256; review R1-6 #1): never pinned to itself
    let file = rd(&files[0]);
    let key = swiftsync_verify::read_pinned_key(&vk, vk_pin.as_deref(), "--vk").unwrap_or_else(|e| fail(&esc(&e)));
    let pin: [u8; 32] = swiftsync_verify::hex32(vk_pin.as_deref().unwrap_or_default().trim()).unwrap_or_default();
    let key = EngineKey { bytes: &key, pin };
    // the trusted core image: the network's record re-pointed at --core-image (an engine build: node format 2)
    let img = zapp_corecheck::image_for(&network, &core, true, true).unwrap_or_else(|e| fail(&e));
    let anchor: Option<ChainState> = anchor.map(|p| serde_json::from_slice(&rd(&p)).unwrap_or_else(|e| fail(&format!("{}: not a ChainState: {e}", esc(&p)))));
    let run = |tip: Option<(u32, &str)>| zapp_zeroconf::verify_zeroconf(&file, &image, &key, img, anchor.as_ref(), now, &txid, tip);
    // chain identity (zapp-common): the sources are asked about the tip height the proof has SHOWN (it verified first)
    let (expect, extra) = if id.needs_lookup() {
        match run(None).statement {
            Some(z) => zapp_common::expect_at(&id, img.network, z.tip_height, Some(&z.cum_work)),
            None => (id.expect.clone(), Vec::new()),
        }
    } else {
        (id.expect.clone(), Vec::new())
    };
    let tip = expect.as_deref().map(|e| swiftsync_verify::parse_expect(e).unwrap_or_else(|| fail("--expect: want <height>:<64 hex digits>")));
    let mut r = run(tip.as_ref().map(|(h, x)| (*h, x.as_str())));
    if extra.iter().any(|c| !c.0) { r.verdict = Verdict::Fail; r.statement = None; }
    r.checks.splice(0..0, extra);
    let say = |s: &str| if json { eprintln!("{s}") } else { println!("{s}") };
    for (ok, what) in &r.checks { say(&format!("  [{}] {what}", if *ok { " ok " } else { "FAIL" })); }
    let code = match (&r.verdict, &r.statement) {
        (Verdict::Fail, _) | (_, None) => { say(&format!("ZAPP_ZEROCONF_FAIL: {} check(s) failed", r.checks.iter().filter(|c| !c.0).count())); 1 }
        (v, Some(z)) => {
            let what = format!("tx {} is valid for block {} on tip {} and spends only coins unspent there (fee {} sat, weight {}, sigops {})",
                               z.txid_hex(), z.tip_height + 1, z.tip_hex(), z.fee, z.weight, z.sigops);
            let ok = *v == Verdict::Ok;
            if json {
                println!("{}", serde_json::json!({"verdict": if ok { "OK" } else { "DEMO" }, "network": img.network, "txid": z.txid_hex(),
                    "wtxid": z.wtxid_hex(), "next_block": z.tip_height + 1, "tip": z.tip_hex(), "fee": z.fee, "weight": z.weight, "sigops": z.sigops}));
            }
            if ok {
                say(&format!("ZAPP_ZEROCONF_OK: {what}"));
            } else {
                say(&format!("ZAPP_ZEROCONF_DEMO: {what} -- but the core chain is not shown to be the network's (a given anchor, below minimum work or the checkpoint, or the tip not compared with your own header chain)"));
            }
            say("  ⚠ valid now, not final: a conflicting transaction can still confirm first");
            if ok { 0 } else { 2 }
        }
    };
    exit(code)
}
