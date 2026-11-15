//! `zync-statement`: makes a balance statement (docs/libzync/STATEMENT_ZAPP.md) for `zapp-statement`.
//!
//!   zync-statement draft --receipt <core receipt> --trie <state_<H>.trie> --purpose <text> --nonce <text>
//!                        [--network mainnet|signet] <scriptPubKey hex>...      a statement with no signatures (stdout)
//!   zync-statement message <statement.json>                                    the exact text each script must sign
//!   zync-statement sign <statement.json> --keys <file>                         signs the entries it holds keys for
//!   zync-statement add-sig <statement.json> <scriptPubKey hex> simple|full|legacy <base64>
//!                                                                              a signature made by another wallet
//!   zync-statement test-key <file> [--uncompressed]                                           a NEW random key, for tests: its WIF to
//!                                                                              <file> (mode 0600), its scripts to stdout
//!
//! `--keys`: one WIF private key per line (# comments). Each key signs for its P2WPKH, P2SH-P2WPKH and P2TR (key path,
//! no script tree) scripts in BIP-322 simple and its P2PKH and P2PK scripts in BIP-322 full (P2PK in the key's own form,
//! compressed or uncompressed, as the WIF says). Keys are never printed. A wallet
//! that signs BIP-322 messages itself (or Bitcoin Core `signmessage` for P2PKH: format legacy) signs the `message`
//! text, and `add-sig` files its signature. `sign` and `add-sig` rewrite the statement file in place.

use balance_statement::{self as statement, Statement};
use swiftsync_verify::{unhex, verify, Image, Inputs, Verdict};

fn die(m: &str) -> ! {
    eprintln!("zync-statement: {m}");
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
            eprintln!("zync-statement: signed {n} entr{}; {left} still unsigned", if n == 1 { "y" } else { "ies" });
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
        _ => die("usage: zync-statement draft|message|sign|add-sig (see the source header)"),
    }
}
