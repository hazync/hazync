//! zapp-utreexo: check a Utreexo roots zyncAPP proof (docs/libzync/UTREEXO_ZAPP.md §6) and print the proven
//! accumulator in Floresta's `AssumeUtreexoValue` form.
//!
//!   zapp-utreexo verify <final run receipt> [--network mainnet|signet] [--expect <H>:<hash>] [--mempool | --mempool-url <base>]
//!                       [--bitcoin-cli "<cmd>"] [--expect-tip <T>:<hash>] [--image <hex>]
//!                       [--core-image <hex>] [--json] [--floresta] [--allow-demo]
//!   zapp-utreexo id
//!
//! `--network`: the network the proof must be for (default mainnet, as every zAPP CLI); a proof of another network is
//! FAIL, and the identity sources are asked about this network. `--expect`: block H from YOUR node's header chain (`bitcoin-cli getblockhash H`, or Floresta's own headers); the
//! chain-identity flags every zAPP CLI takes (zapps/common: `--mempool`, `--mempool-url`, `--bitcoin-cli`) are asked for
//! block H, and for the core tip T when it is above H (until 16.11.2026 `--expect` was
//! `--expect-at`). `--expect-tip`: the core tip T the same way (needed for OK when T > H; compared whenever given; with
//! `--mempool` / `--bitcoin-cli` too, those sources are also asked for block T and must agree with it, and the node's
//! chainwork there is compared with the proven one).
//! Exit 0 OK, 2 DEMO (valid, but the chain is not shown to be the network's: below minimum work or the checkpoint, or
//! block H / the tip not compared with your own; or the utreexo image is the unpinned built-in one), 1 FAIL or a usage
//! error.
//!
//! Output: the check lines and the verdict line on stdout. With `--json` or `--floresta`, stdout holds ONLY that
//! output (the checks and the verdict line go to stderr), and it is printed only on OK, or on DEMO with `--allow-demo`
//! (the JSON's `verdict` says which): an unchecked value is never emitted where a script would take it as proven. The
//! JSON names the `network` checked and where the `image` comes from (`image_source`: "builtin-unpinned", which caps
//! the verdict at DEMO and adds `image_label`, "builtin-pinned", or "caller" for an id named with `--image`); the
//! `--floresta` source starts with a comment line naming the network.
use swiftsync_verify::Verdict;
use zapp_common::Identity;
use zapp_utreexo::{verify_utreexo_on, IMAGE_BUILTIN_UNPINNED, IMAGE_CALLER, NETWORKS, UTREEXO_IMAGE_ID, UTREEXO_IMAGE_LABEL};

const USAGE: &str = "usage: zapp-utreexo verify <receipt> [--network mainnet|signet] [--expect H:hash] [--mempool | --mempool-url <base>] [--bitcoin-cli \"<cmd>\"] [--expect-tip T:hash] [--image hex] [--core-image hex] [--json] [--floresta] [--allow-demo] | id";

fn usage(m: &str) -> ! {
    eprintln!("zapp-utreexo: {m}\n{USAGE}");
    std::process::exit(1)
}

#[derive(Default)]
struct Opts {
    receipt: String,
    network: Option<String>,
    id: Identity,
    expect_tip: Option<String>,
    image: Option<String>,
    core_image: Option<String>,
    json: bool,
    floresta: bool,
    allow_demo: bool,
}

fn parse(a: &[String]) -> Opts {
    let mut o = Opts::default();
    let mut it = a.iter();
    let val = |it: &mut std::slice::Iter<String>, f: &str| -> String {
        match it.next() {
            Some(v) if !v.starts_with("--") => v.clone(),
            _ => usage(&format!("{f} needs a value")),
        }
    };
    let mut pos = Vec::new();
    while let Some(x) = it.next() {
        match x.as_str() {
            "--network" if o.network.is_some() => usage("--network: give it once"),
            "--network" => {
                let n = val(&mut it, x);
                if !NETWORKS.contains(&n.as_str()) { usage(&format!("--network {n:?}: mainnet or signet")) }
                o.network = Some(n)
            }
            "--expect-tip" if o.expect_tip.is_some() => usage("--expect-tip: give it once"),
            "--expect-tip" => o.expect_tip = Some(val(&mut it, x)),
            "--image" if o.image.is_some() => usage("--image: give it once"),
            "--image" => o.image = Some(val(&mut it, x)),
            "--core-image" if o.core_image.is_some() => usage("--core-image: give it once"),
            "--core-image" => o.core_image = Some(val(&mut it, x)),
            "--json" => o.json = true,
            "--floresta" => o.floresta = true,
            "--allow-demo" => o.allow_demo = true,
            f if o.id.take(f, &mut || match it.next() { Some(v) if !v.starts_with("--") => Some(v.clone()), _ => None })
                .unwrap_or_else(|e| usage(&e)) => {}
            f if f.starts_with("--") => usage(&format!("unknown flag {f}")),
            p => pos.push(p.to_string()),
        }
    }
    if pos.len() != 1 {
        usage("give exactly one receipt");
    }
    o.receipt = pos.remove(0);
    o
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a.first().map(|s| s.as_str()) {
        Some("id") if a.len() == 1 => println!("ZAPP_UTREEXO_IMAGE_ID {UTREEXO_IMAGE_ID} ({UTREEXO_IMAGE_LABEL})"),
        Some("verify") => {
            let o = parse(&a[1..]);
            let machine = o.json || o.floresta;
            // with machine output, everything human goes to stderr
            let say = |s: &str| if machine { eprintln!("{s}") } else { println!("{s}") };
            let fail = |m: String| -> ! { say(&format!("ZAPP_UTREEXO_FAIL: {m}")); std::process::exit(1) };
            let pe = |k: &str, v: &Option<String>| v.as_ref().map(|e| swiftsync_verify::parse_expect(e).unwrap_or_else(|| usage(&format!("{k}: want <height>:<64 hex digits>"))));
            let r = std::fs::read(&o.receipt).unwrap_or_else(|e| fail(format!("{}: {e}", o.receipt)));
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
            let net = o.network.as_deref().unwrap_or("mainnet");
            let image = o.image.as_deref().unwrap_or(UTREEXO_IMAGE_ID);
            let builtin = image.eq_ignore_ascii_case(UTREEXO_IMAGE_ID);
            if builtin {
                say(&format!("  image {UTREEXO_IMAGE_ID} (the built-in one: {UTREEXO_IMAGE_LABEL})"));
            }
            say(&format!("  network {net} (the proof must be for it{})", if o.network.is_none() { "; the default, --network names another" } else { "" }));
            // chain identity (zapp-common): --mempool / --bitcoin-cli are asked, about the network the CALLER named, for
            // block H (and for the core tip when it is above H), which the receipt names, only once it verifies; their
            // answers are checked as --expect (and --expect-tip, which they must agree with) are
            let (mut expect, mut expect_tip, mut extra) = (o.id.expect.clone(), o.expect_tip.clone(), Vec::new());
            if o.id.needs_lookup() {
                let pre = verify_utreexo_on(net, &r, image, o.core_image.as_deref(), now, None, None);
                if let Some(z) = pre.statement.as_ref().filter(|_| pre.verdict != Verdict::Fail) {
                    // the proof's chainwork is the core tip's: it is block H's only when H is the tip
                    let (x, c) = zapp_common::expect_at(&o.id, net, z.height, (z.height == z.core_height).then_some(&z.cum_work));
                    expect = x;
                    extra.extend(c);
                    // block T is asked whether or not --expect-tip is given: a given one stands in for --expect here, so
                    // the sources must agree with it (zapp-common's resolve), and the node's chainwork at T is compared
                    if z.core_height > z.height {
                        let (x, c) = zapp_common::expect_at(&Identity { expect: o.expect_tip.clone(), ..o.id.clone() }, net, z.core_height, Some(&z.cum_work));
                        expect_tip = x;
                        extra.extend(c);
                    }
                }
            }
            let (e1, e2) = (pe("--expect", &expect), pe("--expect-tip", &expect_tip));
            let mut rep = verify_utreexo_on(net, &r, image, o.core_image.as_deref(), now,
                                         e1.as_ref().map(|(h, x)| (*h, x.as_str())), e2.as_ref().map(|(h, x)| (*h, x.as_str())));
            if extra.iter().any(|c| !c.0) { rep.verdict = Verdict::Fail; }
            rep.checks.splice(0..0, extra);
            for (ok, what) in &rep.checks { say(&format!("  [{}] {what}", if *ok { " ok " } else { "FAIL" })); }
            let z = match (&rep.verdict, &rep.statement) {
                (Verdict::Fail, _) | (_, None) => fail(rep.checks.iter().find(|c| !c.0).map(|c| c.1.clone()).unwrap_or_default()),
                (_, Some(z)) => z,
            };
            let ok = rep.verdict == Verdict::Ok;
            let verdict = if ok { "ok" } else { "demo" };
            if machine && (ok || o.allow_demo) {
                if o.json {
                    println!("{}", serde_json::json!({"verdict": verdict, "network": rep.network, "identity_checked": rep.identity_checked,
                                                       "height": z.height, "block_hash": z.block_hash_hex(), "leaves": z.leaves,
                                                       "roots": z.roots_hex(), "empty_root_encoding": "\"empty\" (32 zero bytes in the journal)",
                                                       "n_live": z.n_live, "runs": z.runs, "core_height": z.core_height, "image": image.to_lowercase(),
                                                       "image_source": rep.image_source,
                                                       "image_label": (rep.image_source == Some(IMAGE_BUILTIN_UNPINNED)).then_some(UTREEXO_IMAGE_LABEL)}));
                }
                if o.floresta {
                    if o.json { println!("---"); }
                    println!("{}", z.floresta_rust(&rep.network));
                }
            } else if machine {
                say("  (no --json / --floresta output: the verdict is DEMO; --allow-demo prints it anyway, marked \"demo\")");
            }
            let line = format!("{} Utreexo accumulator (Floresta's) after block {} {} is leaves={} roots={} ({} live leaves; {} runs; core tip {})",
                               rep.network, z.height, z.block_hash_hex(), z.leaves, z.roots_hex().join(","), z.n_live, z.runs, z.core_height);
            if ok {
                let caller = if rep.image_source == Some(IMAGE_CALLER) { " (under the utreexo image you named; this checker does not know its code)" } else { "" };
                say(&format!("ZAPP_UTREEXO_OK: {line}{caller}"));
                std::process::exit(0)
            }
            say(&format!("ZAPP_UTREEXO_DEMO: {line} -- but {}", if rep.image_source != Some(IMAGE_BUILTIN_UNPINNED) { "the chain is not shown to be the network's (below minimum work or the checkpoint, or block H / the core tip not compared with your own header chain)" } else { "the utreexo image is the built-in one, not yet a pinned reproducible build (or the chain is not shown to be the network's)" }));
            std::process::exit(2)
        }
        _ => usage("unknown command"),
    }
}
