//! zapp-utreexo: check a Utreexo roots zyncAPP proof (docs/libzync/UTREEXO_ZAPP.md §6) and print the proven
//! accumulator in Floresta's `AssumeUtreexoValue` form.
//!
//!   zapp-utreexo verify <final run receipt> [--expect-at <H>:<hash>] [--expect-tip <T>:<hash>] [--image <hex>]
//!                       [--core-image <hex>] [--json] [--floresta]
//!   zapp-utreexo id
//!
//! `--expect-at`: block H from YOUR node's header chain (`bitcoin-cli getblockhash H`, or Floresta's own headers).
//! Exit 0 OK, 2 DEMO (valid, but the chain is not shown to be the network's: below minimum work or the checkpoint, or
//! block H not compared with your own), 1 FAIL.
use swiftsync_verify::Verdict;
use zapp_utreexo::{verify_utreexo, UTREEXO_IMAGE_ID};

fn flag<'a>(a: &'a [String], f: &str) -> Option<&'a String> {
    a.iter().position(|x| x == f).and_then(|i| a.get(i + 1))
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a.first().map(|s| s.as_str()) {
        Some("id") => println!("ZAPP_UTREEXO_IMAGE_ID {UTREEXO_IMAGE_ID}"),
        Some("verify") if a.len() >= 2 => {
            let fail = |m: String| -> ! { println!("ZAPP_UTREEXO_FAIL: {m}"); std::process::exit(1) };
            let r = std::fs::read(&a[1]).unwrap_or_else(|e| fail(format!("{}: {e}", a[1])));
            let pe = |k: &str| flag(&a, k).map(|e| swiftsync_verify::parse_expect(e).unwrap_or_else(|| fail(format!("{k}: want <height>:<64 hex digits>"))));
            let (e1, e2) = (pe("--expect-at"), pe("--expect-tip"));
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
            let image = flag(&a, "--image").map(|s| s.as_str()).unwrap_or(UTREEXO_IMAGE_ID);
            let rep = verify_utreexo(&r, image, flag(&a, "--core-image").map(|s| s.as_str()), now,
                                     e1.as_ref().map(|(h, x)| (*h, x.as_str())), e2.as_ref().map(|(h, x)| (*h, x.as_str())));
            for (ok, what) in &rep.checks { println!("  [{}] {what}", if *ok { " ok " } else { "FAIL" }); }
            let z = match (&rep.verdict, &rep.statement) {
                (Verdict::Fail, _) | (_, None) => fail(rep.checks.iter().find(|c| !c.0).map(|c| c.1.clone()).unwrap_or_default()),
                (_, Some(z)) => z,
            };
            if a.iter().any(|x| x == "--json") {
                println!("{}", serde_json::json!({"height": z.height, "block_hash": z.block_hash_hex(), "leaves": z.leaves,
                                                   "roots": z.roots_hex(), "runs": z.runs, "core_height": z.core_height}));
            }
            if a.iter().any(|x| x == "--floresta") {
                println!("{}", z.floresta_rust());
            }
            let line = format!("Floresta's Utreexo accumulator after block {} {} is leaves={} roots={} ({} live leaves; {} runs; core tip {})",
                               z.height, z.block_hash_hex(), z.leaves, z.roots_hex().join(","), z.n_live, z.runs, z.core_height);
            match rep.verdict {
                Verdict::Ok => { println!("ZAPP_UTREEXO_OK: {line}"); std::process::exit(0) }
                _ => {
                    println!("ZAPP_UTREEXO_DEMO: {line} -- but the chain is not shown to be the network's (below minimum work or the checkpoint, or block H / the core tip not compared with your own header chain)");
                    std::process::exit(2)
                }
            }
        }
        _ => {
            eprintln!("usage: zapp-utreexo verify <receipt> [--expect-at H:hash] [--expect-tip T:hash] [--image hex] [--core-image hex] [--json] [--floresta] | id");
            std::process::exit(1)
        }
    }
}
