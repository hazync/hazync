//! zapp-utreexo: check a Utreexo roots zyncAPP proof (docs/libzync/UTREEXO_ZAPP.md §6) and print the proven
//! accumulator in Floresta's `AssumeUtreexoValue` form.
//!
//!   zapp-utreexo verify <final run receipt> [--expect-at <H>:<hash>] [--expect-tip <T>:<hash>] [--image <hex>]
//!                       [--core-image <hex>] [--json] [--floresta] [--allow-demo]
//!   zapp-utreexo id
//!
//! `--expect-at`: block H from YOUR node's header chain (`bitcoin-cli getblockhash H`, or Floresta's own headers);
//! `--expect-tip`: the core tip T the same way (needed for OK when T > H; compared whenever given).
//! Exit 0 OK, 2 DEMO (valid, but the chain is not shown to be the network's: below minimum work or the checkpoint, or
//! block H / the tip not compared with your own), 1 FAIL or a usage error.
//!
//! Output: the check lines and the verdict line on stdout. With `--json` or `--floresta`, stdout holds ONLY that
//! output (the checks and the verdict line go to stderr), and it is printed only on OK, or on DEMO with `--allow-demo`
//! (the JSON's `verdict` says which): an unchecked value is never emitted where a script would take it as proven.
use swiftsync_verify::Verdict;
use zapp_utreexo::{verify_utreexo, UTREEXO_IMAGE_ID, UTREEXO_IMAGE_LABEL};

const USAGE: &str = "usage: zapp-utreexo verify <receipt> [--expect-at H:hash] [--expect-tip T:hash] [--image hex] [--core-image hex] [--json] [--floresta] [--allow-demo] | id";

fn usage(m: &str) -> ! {
    eprintln!("zapp-utreexo: {m}\n{USAGE}");
    std::process::exit(1)
}

#[derive(Default)]
struct Opts {
    receipt: String,
    expect_at: Option<String>,
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
            "--expect-at" => o.expect_at = Some(val(&mut it, x)),
            "--expect-tip" => o.expect_tip = Some(val(&mut it, x)),
            "--image" => o.image = Some(val(&mut it, x)),
            "--core-image" => o.core_image = Some(val(&mut it, x)),
            "--json" => o.json = true,
            "--floresta" => o.floresta = true,
            "--allow-demo" => o.allow_demo = true,
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
            let (e1, e2) = (pe("--expect-at", &o.expect_at), pe("--expect-tip", &o.expect_tip));
            let r = std::fs::read(&o.receipt).unwrap_or_else(|e| fail(format!("{}: {e}", o.receipt)));
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
            let image = o.image.as_deref().unwrap_or(UTREEXO_IMAGE_ID);
            if o.image.is_none() {
                say(&format!("  image {UTREEXO_IMAGE_ID} (the built-in default: {UTREEXO_IMAGE_LABEL})"));
            }
            let rep = verify_utreexo(&r, image, o.core_image.as_deref(), now,
                                     e1.as_ref().map(|(h, x)| (*h, x.as_str())), e2.as_ref().map(|(h, x)| (*h, x.as_str())));
            for (ok, what) in &rep.checks { say(&format!("  [{}] {what}", if *ok { " ok " } else { "FAIL" })); }
            let z = match (&rep.verdict, &rep.statement) {
                (Verdict::Fail, _) | (_, None) => fail(rep.checks.iter().find(|c| !c.0).map(|c| c.1.clone()).unwrap_or_default()),
                (_, Some(z)) => z,
            };
            let ok = rep.verdict == Verdict::Ok;
            let verdict = if ok { "ok" } else { "demo" };
            if machine && (ok || o.allow_demo) {
                if o.json {
                    println!("{}", serde_json::json!({"verdict": verdict, "network": z.network_name(), "identity_checked": rep.identity_checked,
                                                       "height": z.height, "block_hash": z.block_hash_hex(), "leaves": z.leaves,
                                                       "roots": z.roots_hex(), "empty_root_encoding": "\"empty\" (32 zero bytes in the journal)",
                                                       "n_live": z.n_live, "runs": z.runs, "core_height": z.core_height, "image": image}));
                }
                if o.floresta {
                    if o.json { println!("---"); }
                    println!("{}", z.floresta_rust());
                }
            } else if machine {
                say("  (no --json / --floresta output: the verdict is DEMO; --allow-demo prints it anyway, marked \"demo\")");
            }
            let line = format!("{} Utreexo accumulator (Floresta's) after block {} {} is leaves={} roots={} ({} live leaves; {} runs; core tip {})",
                               z.network_name(), z.height, z.block_hash_hex(), z.leaves, z.roots_hex().join(","), z.n_live, z.runs, z.core_height);
            if ok {
                say(&format!("ZAPP_UTREEXO_OK: {line}"));
                std::process::exit(0)
            }
            say(&format!("ZAPP_UTREEXO_DEMO: {line} -- but the chain is not shown to be the network's (below minimum work or the checkpoint, or block H / the core tip not compared with your own header chain)"));
            std::process::exit(2)
        }
        _ => usage("unknown command"),
    }
}
