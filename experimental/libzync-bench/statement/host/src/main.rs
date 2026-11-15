//! Design-B prototype bench (docs/libzync/STATEMENT_ZAPP.md §3). Execute mode: cycles, no proofs, no GPU.
//!
//!   statement-bench selftest              bip322_lite (the guest's check) against the bip322 crate, natively
//!   statement-bench real <n>              the REAL mainnet 1..200 proof (8888d59c) as the guest's assumption, its real
//!                                         block-200 trie, n fresh keys (P2WPKH and P2TR alternating, zero balances)
//!   statement-bench synth <n> <fillers>   a trie built here: <fillers> other scripts plus n of ours, 2 coins each; the
//!                                         guest's bench flag (no env::verify: this trie is no receipt's)
//! Keys are generated in memory for the run and never printed.
extern crate alloc;
#[path = "../../bip322_lite.rs"]
mod bip322_lite;

use bitcoin::{Address, Network, PrivateKey, Script};
use risc0_zkvm::{default_executor, ExecutorEnv, Receipt};
use statement_methods::{STATEMENT_GUEST_ELF, STATEMENT_GUEST_ID};
use swiftsync_verify::statement::scripts_for_key;

const VEC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../../swiftsync-verify/vectors/");

fn key() -> PrivateKey {
    use std::io::Read;
    loop {
        let mut b = [0u8; 32];
        std::fs::File::open("/dev/urandom").unwrap().read_exact(&mut b).unwrap();
        if let Ok(k) = bitcoin::secp256k1::SecretKey::from_slice(&b) { return PrivateKey::new(k, Network::Bitcoin); }
    }
}

/// scripts_for_key order: 0 P2PKH, 1 P2SH-P2WPKH, 2 P2WPKH, 3 P2TR
fn script(k: &PrivateKey, i: usize) -> Vec<u8> { scripts_for_key(k)[i].0.clone() }

fn sign(spk: &[u8], msg: &str, k: &PrivateKey) -> Vec<u8> {
    let a = Address::from_script(Script::from_bytes(spk), Network::Bitcoin).unwrap();
    let w = bip322::sign_simple(&a, msg, std::slice::from_ref(k), None).unwrap();
    let mut b = Vec::new();
    bitcoin::consensus::Encodable::consensus_encode(&w, &mut b).unwrap();
    b
}

fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }

/// The guest's message (it must match the guest's format! byte for byte, or every signature fails).
fn message(core_id: &[u32; 8], height: u32, tip: &[u8; 32], scripts: &[Vec<u8>], salt: &[u8; 32], total: u64, purpose: &str, nonce: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut lh = Sha256::new();
    for s in scripts { lh.update([s.len() as u8]); lh.update(s); }
    let list: [u8; 32] = lh.finalize().into();
    let commit: [u8; 32] = Sha256::digest([&salt[..], &list[..]].concat()).into();
    let id: Vec<u8> = core_id.iter().flat_map(|x| x.to_le_bytes()).collect();
    let mut t = *tip;
    t.reverse();
    format!("Hazync private statement of balance v1\nnetwork: mainnet\ncore image: {}\nblock: {height} {}\nscripts: {} commit {}\ntotal: {total} sat\npurpose: {purpose}\nnonce: {nonce}",
            hex(&id), hex(&t), scripts.len(), hex(&commit))
}

fn selftest() {
    let mut n = 0;
    for kind in [2usize, 3] {
        for _ in 0..20 {
            let k = key();
            let spk = script(&k, kind);
            let msg = "Hazync bip322_lite self-test";
            let w = bip322_lite::decode_witness(&sign(&spk, msg, &k)).expect("witness");
            assert_eq!(bip322_lite::verify_simple(&spk, msg.as_bytes(), &w), Ok(()), "a bip322-crate signature verifies");
            assert!(bip322_lite::verify_simple(&spk, b"another message", &w).is_err(), "another message");
            assert!(bip322_lite::verify_simple(&script(&key(), kind), msg.as_bytes(), &w).is_err(), "another key's script");
            let mut bad = w.clone();
            let last = bad[0].len() - 2;
            bad[0][last] ^= 1;
            assert!(bip322_lite::verify_simple(&spk, msg.as_bytes(), &bad).is_err(), "a changed signature");
            n += 1;
        }
    }
    // BIP-322 official vectors (as the bip322 crate's tests carry them): P2WPKH and P2TR, each with its message and another
    use base64::Engine;
    for (addr, m, sig) in [
        ("bc1qqthe0hz8klx90e7stf6shclhsvqd5ly96pn53v", "2V6TUTMSH4VQ3Z7WZWKYD7DFNH", "AkgwRQIhALC6hdfxNy1n45d7UXSskRBdfZW0Al259E1kDMpipdYkAiAJPfZqb+WurZuf1apU5xeE6Igui9dvt5tihQLDvxlY1AEhAqbnruyo677ktQjio7XOchO3w51Dh9AbRVngha5jtNfT"),
        ("bc1pcquvhrqv0q68t4m0hfq6tpn006qrskyc7yrqnp2uyrf2emg3wynsdjyk38", "PURVOQ544B6HUATVBJZN5EZJUU", "AUB6B2Rbupzua8LTQIF06516wzl+cwKy1be8RgoiW0riyXdKwe6GTz/5Hnb37m67pJwIKCh+D5jDueG6KpvYpmu8"),
    ] {
        let spk = Address::from_str_checked(addr).script_pubkey().to_bytes();
        let w = bip322_lite::decode_witness(&base64::engine::general_purpose::STANDARD.decode(sig).unwrap()).unwrap();
        assert_eq!(bip322_lite::verify_simple(&spk, m.as_bytes(), &w), Ok(()), "official vector {addr}");
        assert!(bip322_lite::verify_simple(&spk, b"Hello World", &w).is_err(), "official vector {addr}, another message");
    }
    println!("SELFTEST_OK {n} bip322-crate signatures verified by bip322_lite (P2WPKH, P2TR) and their changes refused; official P2WPKH + P2TR vectors OK");
}

trait Checked { fn from_str_checked(s: &str) -> Address; }
impl Checked for Address { fn from_str_checked(s: &str) -> Address { s.parse::<Address<_>>().unwrap().assume_checked() } }

struct Run { core_id: [u32; 8], journal: Vec<u8>, bench: bool, trie: libzync::trie_store::TrieStore, keys: Vec<(PrivateKey, Vec<u8>)>, receipt: Option<Receipt> }

fn execute(r: Run, label: &str) {
    let (purpose, nonce, salt) = ("audit 15.11.2026", "5be1c0de", [7u8; 32]);
    let mut keys = r.keys;
    keys.sort_by(|a, b| a.1.cmp(&b.1));
    let scripts: Vec<Vec<u8>> = keys.iter().map(|k| k.1.clone()).collect();
    let cs: swiftsync_verify::ChainState = {
        let w: Vec<u32> = r.journal.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        risc0_zkvm::serde::from_slice(&w).unwrap()
    };
    let root = r.trie.root();
    let mut total = 0;
    let proofs: Vec<Vec<u32>> = scripts.iter().map(|s| {
        let sh = libzync::trie_fact::script_hash(s);
        let p = r.trie.prove_addr(&sh);
        total += libzync::trie_fact::balance(&p, &root, &sh).unwrap().map_or(0, |b| b.value);
        p
    }).collect();
    let msg = message(&r.core_id, cs.height, &cs.tip_hash, &scripts, &salt, total, purpose, nonce);
    let entries: Vec<(Vec<u8>, Vec<u32>, Vec<u8>)> = keys.iter().zip(proofs).map(|((k, s), p)| (s.clone(), p, sign(s, &msg, k))).collect();
    let words: usize = entries.iter().map(|e| e.1.len()).sum();
    let mut b = ExecutorEnv::builder();
    if let Some(rc) = r.receipt { b.add_assumption(rc); }
    let env = b.write(&(r.core_id, r.journal, r.bench, purpose.to_string(), nonce.to_string(), salt, entries)).unwrap().build().unwrap();
    let t = std::time::Instant::now();
    let info = default_executor().execute(env, STATEMENT_GUEST_ELF).unwrap_or_else(|e| { eprintln!("execute: {e}"); std::process::exit(1) });
    let cycles = info.cycles();
    // the journal: total and script count (fields 10 and 11 of Out, after kind, core_id[8], lineage, anchor_id, height, ...)
    println!("{label} n={} total={total} sat, trie proof words {words}: {} user cycles ({:.0} per script), {} segments, executed in {:.1} s; image {}",
             scripts.len(), cycles, cycles as f64 / scripts.len() as f64, info.segments.len(), t.elapsed().as_secs_f64(),
             hex(&STATEMENT_GUEST_ID.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>()));
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let n = |i: usize| -> usize { a.get(i).and_then(|x| x.parse().ok()).expect("a number") };
    let rcpt = std::fs::read(format!("{VEC}mainnet_1_200_8888d59c.rcpt")).unwrap();
    let receipt: Receipt = bincode::deserialize(&rcpt).unwrap();
    let journal = receipt.journal.bytes.clone();
    let core_id = swiftsync_verify::image_words(swiftsync_verify::hex32("8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4").unwrap());
    match a.first().map(|s| s.as_str()) {
        Some("selftest") => selftest(),
        Some("real") => {
            let trie = libzync::trie_store::TrieStore::from_bytes(&std::fs::read(format!("{VEC}mainnet_200_8888d59c.trie")).unwrap()).unwrap();
            let keys = (0..n(1)).map(|i| { let k = key(); let s = script(&k, if i % 2 == 0 { 2 } else { 3 }); (k, s) }).collect();
            execute(Run { core_id, journal, bench: false, trie, keys, receipt: Some(receipt) }, "REAL");
        }
        Some("synth") => {
            let (count, fillers) = (n(1), n(2));
            let keys: Vec<(PrivateKey, Vec<u8>)> = (0..count).map(|i| { let k = key(); let s = script(&k, if i % 2 == 0 { 2 } else { 3 }); (k, s) }).collect();
            let mut trie = libzync::trie_store::TrieStore::new(true);
            let mut ins = Vec::new();
            for f in 0..fillers as u32 {
                let mut s = vec![0x00, 0x14];
                s.extend_from_slice(&libzync::trie_fact::script_hash(&f.to_le_bytes())[..20]);
                ins.push((libzync::trie_fact::coin_key(&s, &[1; 32], f), None, 1000 + f as u64));
            }
            for (i, (_, s)) in keys.iter().enumerate() {
                for j in 0..2u32 { ins.push((libzync::trie_fact::coin_key(s, &[2; 32], 2 * i as u32 + j), None, 50_000 + j as u64)); }
            }
            trie.apply(&[], &ins).unwrap();
            // the real journal with this trie's root in place of the real one (bench flag: the guest does not verify it)
            let w: Vec<u32> = journal.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
            let mut cs: swiftsync_verify::ChainState = risc0_zkvm::serde::from_slice(&w).unwrap();
            cs.trie_root = libzync::coinlog::h_of(&trie.root());
            let journal: Vec<u8> = risc0_zkvm::serde::to_vec(&cs).unwrap().iter().flat_map(|x| x.to_le_bytes()).collect();
            execute(Run { core_id, journal, bench: true, trie, keys, receipt: None }, &format!("SYNTH(fillers={fillers})"));
        }
        _ => { eprintln!("usage: statement-bench selftest | real <n> | synth <n> <fillers>"); std::process::exit(1) }
    }
}
