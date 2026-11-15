//! `zapp-statement`: the balance statement zyncAPP's CLI (docs/libzync/STATEMENT_ZAPP.md), a zAPP module of its own
//! (docs/libzync/ZYNCAPPS.md §9) on the core verifier's library. The checker, and as subcommands the producer (until
//! 15.11.2026 the separate `zync-statement`):
//!
//!   zapp-statement [--network mainnet|signet] [--image <hex>] [--expect <height>:<hash>] [--mempool | --mempool-url <base>]
//!                  [--bitcoin-cli "<command>"] [--purpose <text> --nonce <text>] [--now <unix seconds>] <receipt> <statement.json>
//!
//! The core receipt is verified by `swiftsync_verify::verify`, the same checks as `swiftsync-verify` (real proof, image,
//! profile, anchor, block forest, time-too-new). Then the statement: its network, core image, height and block are the
//! proof's; each balance fact holds against the proof's trie root; the total is their sum; each script's BIP-322
//! signature verifies over the statement's message; purpose and nonce are the ones you give. The chain-identity flags
//! are every zAPP CLI's (zapp-common).
//!
//! Exit 0 `ZAPP_STATEMENT_OK`, 2 `ZAPP_STATEMENT_DEMO` (everything holds, but the chain's work is below Core's minimum,
//! the tip was not compared with a source you trust, or no --purpose / --nonce was given), 1 `ZAPP_STATEMENT_FAIL`.
//!
//! The producer: makes a balance statement for the checker.
//!
//!   zapp-statement draft --receipt <core receipt> --trie <state_<H>.trie> --purpose <text> --nonce <text>
//!                        [--network mainnet|signet] <scriptPubKey hex>...      a statement with no signatures (stdout)
//!   zapp-statement message <statement.json>                                    the exact text each script must sign
//!   zapp-statement sign <statement.json> --keys <file>                         signs the entries it holds keys for
//!   zapp-statement add-sig <statement.json> <scriptPubKey hex> simple|full|legacy <base64>
//!                                                                              a signature made by another wallet
//!   zapp-statement test-key <file> [--uncompressed]                                           a NEW random key, for tests: its WIF to
//!                                                                              <file> (mode 0600), its scripts to stdout
//!
//! `--keys`: one WIF private key per line (# comments). Each key signs for its P2WPKH, P2SH-P2WPKH and P2TR (key path,
//! no script tree) scripts in BIP-322 simple and its P2PKH and P2PK scripts in BIP-322 full (P2PK in the key's own form,
//! compressed or uncompressed, as the WIF says). Keys are never printed. A wallet
//! that signs BIP-322 messages itself (or Bitcoin Core `signmessage` for P2PKH: format legacy) signs the `message`
//! text, and `add-sig` files its signature. `sign` and `add-sig` rewrite the statement file in place.


use std::process::exit;
use swiftsync_verify::{unhex, verify, Image, Inputs, Verdict};
use zapp_common::Identity;
use zapp_statement::{self as statement, check_statement, Challenge, Statement};

fn fail(m: &str) -> ! {
    println!("ZAPP_STATEMENT_FAIL: {m}");
    exit(1)
}

fn die(m: &str) -> ! {
    eprintln!("zapp-statement: {m}");
    std::process::exit(1);
}

fn read_statement(p: &str) -> Statement {
    let b = std::fs::read(p).unwrap_or_else(|e| die(&format!("{p}: {e}")));
    serde_json::from_slice(&b).unwrap_or_else(|e| die(&format!("{p}: not a statement ({e})")))
}

fn write_statement(p: &str, st: &Statement) {
    std::fs::write(p, serde_json::to_string_pretty(st).unwrap() + "\n").unwrap_or_else(|e| die(&format!("{p}: {e}")));
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a.first().map(|s| s.as_str()) {
        Some("draft" | "message" | "sign" | "add-sig" | "test-key") => produce(a),
        _ => {
            std::panic::set_hook(Box::new(|i| {
                println!("ZAPP_STATEMENT_FAIL: {}", format!("{i}").escape_debug());
                exit(1);
            }));
            check(a)
        }
    }
}

/// The producer (draft, message, sign, add-sig, test-key).
fn produce(a: Vec<String>) {
    let flag = |name: &str| a.iter().position(|x| x == name).and_then(|i| a.get(i + 1)).cloned();
    match a.first().map(|s| s.as_str()) {
        Some("draft") => {
            let need = |n: &str| flag(n).unwrap_or_else(|| die(&format!("draft: {n} is required")));
            let (rp, tp, purpose, nonce) = (need("--receipt"), need("--trie"), need("--purpose"), need("--nonce"));
            let network = flag("--network").unwrap_or_else(|| "mainnet".into());
            let flags = ["--receipt", "--trie", "--purpose", "--nonce", "--network"];
            let mut scripts = Vec::new();
            let mut i = 1;
            while i < a.len() {
                if flags.contains(&a[i].as_str()) { i += 2; continue; }
                scripts.push(unhex(&a[i]).unwrap_or_else(|| die(&format!("{:?}: not a scriptPubKey in hex", a[i]))));
                i += 1;
            }
            if scripts.is_empty() { die("draft: give at least one scriptPubKey"); }
            let image = Image::by_network(&network).unwrap_or_else(|| die(&format!("unknown network {network:?}")));
            let receipt = std::fs::read(&rp).unwrap_or_else(|e| die(&format!("{rp}: {e}")));
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
            let rep = verify(&Inputs { receipt: &receipt, image, anchor: None, now });
            let cs = match (&rep.verdict, rep.state) {
                (Verdict::Fail, _) | (_, None) => die(&format!("{rp}: the receipt does not verify: {}",
                    rep.checks.iter().filter(|c| !c.0).map(|c| c.1.as_str()).collect::<Vec<_>>().join("; "))),
                (_, Some(cs)) => cs,
            };
            let tb = std::fs::read(&tp).unwrap_or_else(|e| die(&format!("{tp}: {e}")));
            let trie = libzync::trie_store::TrieStore::from_bytes(&tb).unwrap_or_else(|e| die(&format!("{tp}: not a trie snapshot ({e:?})")));
            let st = statement::draft(&cs, &trie, &scripts, &purpose, &nonce).unwrap_or_else(|e| die(&e));
            println!("{}", serde_json::to_string_pretty(&st).unwrap());
        }
        Some("message") => {
            let st = read_statement(a.get(1).unwrap_or_else(|| die("message <statement.json>")));
            print!("{}", statement::message(&st).unwrap_or_else(|e| die(&e)));
        }
        Some("sign") => {
            let p = a.get(1).unwrap_or_else(|| die("sign <statement.json> --keys <file>"));
            let kp = flag("--keys").unwrap_or_else(|| die("sign: --keys <file> is required"));
            let text = std::fs::read_to_string(&kp).unwrap_or_else(|e| die(&format!("{kp}: {e}")));
            let keys: Vec<bitcoin::PrivateKey> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#'))
                .enumerate()
                // the key itself is never echoed, only its line number
                .map(|(i, l)| bitcoin::PrivateKey::from_wif(l).unwrap_or_else(|_| die(&format!("{kp}: key {} is not a WIF private key", i + 1))))
                .collect();
            let mut st = read_statement(p);
            let n = statement::sign(&mut st, &keys).unwrap_or_else(|e| die(&e));
            write_statement(p, &st);
            let left = st.entries.iter().filter(|e| e.signature.is_empty()).count();
            eprintln!("zapp-statement: signed {n} entr{}; {left} still unsigned", if n == 1 { "y" } else { "ies" });
        }
        Some("add-sig") => {
            let (Some(p), Some(spk), Some(fmt), Some(sig)) = (a.get(1), a.get(2), a.get(3), a.get(4)) else {
                die("add-sig <statement.json> <scriptPubKey hex> simple|full|legacy <base64>")
            };
            let mut st = read_statement(p);
            let msg = statement::message(&st).unwrap_or_else(|e| die(&e));
            let want = unhex(spk).unwrap_or_else(|| die("scriptPubKey: not hex"));
            let net = st.network.clone();
            let e = st.entries.iter_mut().find(|e| unhex(&e.fact.script_pubkey).as_deref() == Some(&want[..]))
                .unwrap_or_else(|| die("no entry for that script"));
            // refuse a signature that would not verify, rather than file it
            statement::check_signature(&net, &want, fmt, sig, &msg).unwrap_or_else(|x| die(&x));
            e.format = fmt.clone();
            e.signature = sig.clone();
            write_statement(p, &st);
        }
        Some("test-key") => {
            use std::io::{Read, Write};
            use std::os::unix::fs::OpenOptionsExt;
            let p = a.get(1).unwrap_or_else(|| die("test-key <file>"));
            let k = loop {
                let mut b = [0u8; 32];
                std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).unwrap_or_else(|e| die(&e.to_string()));
                if let Ok(k) = bitcoin::secp256k1::SecretKey::from_slice(&b) {
                    let mut k = bitcoin::PrivateKey::new(k, bitcoin::Network::Bitcoin);
                    k.compressed = !a.iter().any(|x| x == "--uncompressed");
                    break k
                }
            };
            let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(p)
                .unwrap_or_else(|e| die(&format!("{p}: {e} (test-key never overwrites a file)")));
            writeln!(f, "{}", k.to_wif()).unwrap_or_else(|e| die(&e.to_string()));
            for ((s, _), name) in statement::scripts_for_key(&k).iter().zip(if k.compressed { &["p2pkh", "p2sh-p2wpkh", "p2wpkh", "p2tr", "p2pk"][..] } else { &["p2pkh", "p2pk"][..] }) {
                println!("{name} {}", swiftsync_verify::hex(s));
            }
        }
        _ => die("usage: zapp-statement draft|message|sign|add-sig (see the source header)"),
    }
}

/// The checker.
fn check(a: Vec<String>) {
    let mut it = a.into_iter();
    let (mut network, mut image_hex, mut id) = ("mainnet".to_string(), None, Identity::default());
    let (mut purpose, mut nonce, mut now_arg, mut files) = (None, None, None, Vec::new());
    let need = |it: &mut dyn Iterator<Item = String>, f: &str| it.next().unwrap_or_else(|| fail(&format!("{f} needs a value")));
    while let Some(a) = it.next() {
        match a.as_str() {
            "--network" => network = need(&mut it, &a),
            "--image" => image_hex = Some(need(&mut it, &a)),
            "--purpose" => purpose = Some(need(&mut it, &a)),
            "--nonce" => nonce = Some(need(&mut it, &a)),
            "--now" => now_arg = Some(need(&mut it, &a)),
            f if id.take(f, &mut || it.next()).unwrap_or_else(|e| fail(&e)) => {}
            f if f.starts_with("--") => fail(&format!("unknown flag {f:?} (see the source header)")),
            _ => files.push(a),
        }
    }
    let [receipt, stf] = files.as_slice() else { fail("give <receipt> <statement.json>") };
    let mut image = Image::by_network(&network).unwrap_or_else(|| fail(&format!("unknown network {network:?}")));
    if let Some(h) = &image_hex {
        image = match Image::by_id(&network, h) {
            Some(i) => i,
            None => image.with_method_id(&h.to_lowercase()).unwrap_or_else(|| fail("--image: not 64 hex digits")),
        };
    }
    let clock = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("clock").as_secs();
    let now = match now_arg.map(|v| v.parse::<u64>()) {
        None => clock,
        Some(Ok(t)) if t <= clock => t,
        _ => fail("--now: unix seconds, not later than this clock"),
    };
    let rb = std::fs::read(receipt).unwrap_or_else(|e| fail(&format!("{receipt:?}: {e}")));
    let st: Statement = std::fs::read(stf).map_err(|e| e.to_string())
        .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()))
        .unwrap_or_else(|e| fail(&format!("{stf:?}: not a balance statement ({e})")));
    let mut rep = verify(&Inputs { receipt: &rb, image, anchor: None, now });
    let mut tip_checked = false;
    let mut challenged = false;
    if let Some(cs) = rep.state.clone().filter(|_| rep.verdict != Verdict::Fail) {
        // chain identity (zapp-common: the same flags as every zAPP CLI and the core verifier's): each source asked for
        // the block at the proof's height and compared with the proven tip (and, with --bitcoin-cli, its chainwork)
        if !id.is_empty() {
            let (lines, _) = zapp_common::check_core_tip(&id, image.network, &cs);
            rep.checks.extend(lines);
            tip_checked = true;
        }
        let ch = Challenge { purpose: purpose.as_deref(), nonce: nonce.as_deref() };
        match check_statement(&cs, rep.undo_from, &st, &ch) {
            Ok(c) => { challenged = c.challenged; rep.checks.push((true, c.line)); }
            Err(e) => rep.checks.push((false, format!("statement: {e}"))),
        }
    } else {
        rep.checks.push((false, "statement NOT checked: the receipt did not verify".into()));
    }
    let failed = rep.checks.iter().filter(|c| !c.0).count();
    for (ok, what) in &rep.checks {
        println!("  [{}] {what}", if *ok { "ok" } else { "FAIL" });
    }
    if failed > 0 || rep.verdict == Verdict::Fail {
        println!("ZAPP_STATEMENT_FAIL: {failed} check(s) failed");
        exit(1);
    }
    let mut why = Vec::new();
    if rep.verdict == Verdict::Demo { why.push("the chain is below Core's minimum chainwork or the pinned checkpoint"); }
    if !tip_checked { why.push("the tip was not compared with a source you trust (--expect, --mempool, --bitcoin-cli)"); }
    if !challenged { why.push("the statement was not compared with your challenge (--purpose and --nonce): it may be an old one shown again"); }
    if why.is_empty() {
        println!("ZAPP_STATEMENT_OK: the statement holds at block {}", st.height);
        exit(0);
    }
    println!("ZAPP_STATEMENT_DEMO: the statement holds, but {}", why.join("; "));
    exit(2);
}
