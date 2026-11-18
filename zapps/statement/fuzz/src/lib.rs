//! The balance statement zAPP's fuzz properties (docs/libzync/ZAPP_TESTING.md §3.4, §4.6), one function per target, so
//! the fuzz targets and the stable corpus replay (tests/replay.rs) run the same code. A property that fails panics.
//!
//! The reference is a real statement: the real mainnet 1..200 proof (8888d59c) and its block-200 trie, every script
//! type the signer covers (P2PKH, P2SH-P2WPKH, P2WPKH, P2TR, P2PK compressed and uncompressed), signed by fixed test
//! keys (zero balances: no key of ours was paid in 1..200). It is corpus/statement_check/signed.json, written once by
//! examples/mkseeds.rs and read from there (a BIP-340 signature need not be the same on a second signing).
//!
//! - `check`: a statement JSON checked against the real proof, with and without the challenge: one that holds is the
//!   reference statement, field for field.
//! - `fields`: purpose and nonce: `message` accepts exactly 1 to 256 bytes of printable ASCII without a leading or
//!   trailing space (STATEMENT_ZAPP.md, the doc of `field_ok`), and a statement whose purpose or nonce is not the
//!   signed one never holds, whatever challenge it is compared with.
//! - `p2pk`: `p2pk::verify` on any to_sign bytes: OK only for the reference signatures, under their own key and message.

use base64::Engine;
use std::sync::OnceLock;
use swiftsync_verify::{unhex, ChainState, Image, Inputs, Verdict};
use zapp_statement::{p2pk, Challenge, Statement};

pub const RCPT: &[u8] = include_bytes!("../../../../swiftsync-verify/vectors/mainnet_1_200_8888d59c.rcpt");
pub const TRIE: &[u8] = include_bytes!("../../vectors/mainnet_200_8888d59c.trie");
pub const IMAGE_8888: &str = "8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4";
pub const ASK: Challenge = Challenge { purpose: Some("loan application 15.11.2026"), nonce: Some("7f3a91c2") };
pub const REFERENCE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/corpus/statement_check/signed.json");

/// The real proof's verified state and undo_from, once.
pub fn state() -> &'static (ChainState, u32) {
    static S: OnceLock<(ChainState, u32)> = OnceLock::new();
    S.get_or_init(|| {
        let image = Image::by_id("mainnet", IMAGE_8888).unwrap();
        let r = swiftsync_verify::verify(&Inputs { receipt: RCPT, image, anchor: None, now: 1_800_000_000 });
        assert_eq!(r.verdict, Verdict::Demo, "{:?}", r.checks);
        (r.state.unwrap(), r.undo_from)
    })
}

/// Fixed TEST keys (public in this file: they hold nothing): two compressed, one uncompressed.
pub fn keys() -> Vec<bitcoin::PrivateKey> {
    [(0x11u8, true), (0x22, true), (0x33, false)].iter().map(|&(b, c)| {
        let k = bitcoin::secp256k1::SecretKey::from_slice(&[b; 32]).unwrap();
        if c { bitcoin::PrivateKey::new(k, bitcoin::Network::Bitcoin) } else { bitcoin::PrivateKey::new_uncompressed(k, bitcoin::Network::Bitcoin) }
    }).collect()
}

/// A fresh signed reference statement (examples/mkseeds.rs writes it once).
pub fn make_reference() -> Statement {
    let (cs, _) = state();
    let trie = libzync::trie_store::TrieStore::from_bytes(TRIE).unwrap();
    let ks = keys();
    let mut scripts: Vec<Vec<u8>> = zapp_statement::scripts_for_key(&ks[0]).into_iter().map(|x| x.0).collect();
    scripts.extend(zapp_statement::scripts_for_key(&ks[2]).into_iter().map(|x| x.0));
    let mut st = zapp_statement::draft(cs, &trie, &scripts, ASK.purpose.unwrap(), ASK.nonce.unwrap()).unwrap();
    assert_eq!(zapp_statement::sign(&mut st, &ks).unwrap(), st.entries.len());
    st
}

pub fn reference() -> &'static (Statement, serde_json::Value) {
    static R: OnceLock<(Statement, serde_json::Value)> = OnceLock::new();
    R.get_or_init(|| {
        let st: Statement = serde_json::from_slice(&std::fs::read(REFERENCE).unwrap_or_else(|e| panic!("{REFERENCE}: {e} (run examples/mkseeds.rs)"))).unwrap();
        let (cs, uf) = state();
        zapp_statement::check_statement(cs, *uf, &st, &ASK).unwrap_or_else(|e| panic!("the reference statement does not hold: {e}"));
        let v = serde_json::to_value(&st).unwrap();
        (st, v)
    })
}

/// (script, message, to_sign) of every P2PK entry of the reference.
pub fn p2pk_refs() -> &'static Vec<(Vec<u8>, String, Vec<u8>)> {
    static P: OnceLock<Vec<(Vec<u8>, String, Vec<u8>)>> = OnceLock::new();
    P.get_or_init(|| {
        let st = &reference().0;
        let msg = zapp_statement::message(st).unwrap();
        st.entries.iter().filter_map(|e| {
            let s = unhex(&e.fact.script_pubkey).unwrap();
            p2pk::p2pk_key(&s)?;
            let b = base64::engine::general_purpose::STANDARD.decode(e.signature.strip_prefix("ful").unwrap_or(&e.signature)).unwrap();
            Some((s, msg.clone(), b))
        }).collect()
    })
}

fn differing(v: &serde_json::Value, w: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    if let (Some(a), Some(b)) = (v.as_object(), w.as_object()) {
        for (k, x) in a { if b.get(k) != Some(x) { out.push(k.clone()); } }
        if let (Some(ea), Some(eb)) = (a.get("entries").and_then(|e| e.as_array()), b.get("entries").and_then(|e| e.as_array())) {
            for (i, (x, y)) in ea.iter().zip(eb).enumerate() {
                for (k, z) in x.as_object().into_iter().flatten() { if y.get(k) != Some(z) { out.push(format!("entries[{i}].{k}: {z} vs {}", y[k])); } }
            }
        }
    }
    out
}

/// The facts' hex with its case folded: `unhex` reads either case, so an upper-case digit in a fact is the same fact
/// (round 1 found exactly that; reported, artifacts/statement_check/). Signatures are base64 and keep their case.
fn fold_fact_case(v: &mut serde_json::Value) {
    fn lower(x: &mut serde_json::Value) {
        match x {
            serde_json::Value::String(s) => *s = s.to_lowercase(),
            serde_json::Value::Array(a) => a.iter_mut().for_each(lower),
            serde_json::Value::Object(o) => o.values_mut().for_each(lower),
            _ => {}
        }
    }
    if let Some(es) = v.get_mut("entries").and_then(|e| e.as_array_mut()) {
        for e in es { if let Some(f) = e.get_mut("fact") { lower(f); } }
    }
}

pub fn check(data: &[u8]) {
    let Ok(st) = serde_json::from_slice::<Statement>(data) else { return };
    let (cs, uf) = state();
    let (_, want) = reference();
    for ch in [ASK, Challenge::default()] {
        if zapp_statement::check_statement(cs, *uf, &st, &ch).is_ok() {
            let mut got = serde_json::to_value(&st).unwrap();
            fold_fact_case(&mut got);
            let mut want = want.clone();
            fold_fact_case(&mut want);
            assert!(got == want, "WRONG-OK: a statement other than the signed one holds (challenge given: {}): {:?}",
                    ch.purpose.is_some(), differing(&got, &want));
        }
    }
}

/// STATEMENT_ZAPP.md / `field_ok`'s doc: 1 to 256 bytes of printable ASCII (0x20..0x7e), no leading or trailing space.
fn field_spec(s: &str) -> bool {
    (1..=256).contains(&s.len()) && s.bytes().all(|b| (0x20..=0x7e).contains(&b)) && !s.starts_with(' ') && !s.ends_with(' ')
}

/// `<purpose> NUL <nonce>`.
pub fn fields(data: &[u8]) {
    let Ok(s) = std::str::from_utf8(data) else { return };
    let (p, n) = s.split_once('\0').unwrap_or((s, ASK.nonce.unwrap()));
    let (base, _) = reference();
    let mut st = base.clone();
    st.purpose = p.to_string();
    st.nonce = n.to_string();
    let m = zapp_statement::message(&st);
    assert_eq!(m.is_ok(), field_spec(p) && field_spec(n), "FIELD: purpose {p:?} nonce {n:?}: message gave {m:?}");
    let same = p == base.purpose && n == base.nonce;
    let (cs, uf) = state();
    for ch in [Challenge { purpose: Some(p), nonce: Some(n) }, ASK, Challenge::default()] {
        let r = zapp_statement::check_statement(cs, *uf, &st, &ch);
        assert!(r.is_err() || same, "WRONG-OK: purpose {p:?} nonce {n:?} hold under the signatures of {:?} {:?}", base.purpose, base.nonce);
    }
}

/// byte 0: 0 / 1 = the reference's P2PK entries (key and message fixed, the rest is to_sign); otherwise
/// `[u8 n][n bytes script][u16 LE m][m bytes message][to_sign]`.
pub fn p2pk(data: &[u8]) {
    let Some((&mode, rest)) = data.split_first() else { return };
    let refs = p2pk_refs();
    let (script, msg, ts): (Vec<u8>, Vec<u8>, &[u8]) = if (mode as usize) < refs.len() {
        let (s, m, _) = &refs[mode as usize];
        (s.clone(), m.as_bytes().to_vec(), rest)
    } else {
        let Some((&n, r)) = rest.split_first() else { return };
        if r.len() < n as usize + 2 { return; }
        let (s, r) = r.split_at(n as usize);
        let m = u16::from_le_bytes([r[0], r[1]]) as usize;
        let r = &r[2..];
        if r.len() < m { return; }
        (s.to_vec(), r[..m].to_vec(), &r[m..])
    };
    if p2pk::verify(&script, &msg, ts).is_ok() {
        assert!(refs.iter().any(|(s, m, t)| *s == script && m.as_bytes() == msg && t == ts),
                "WRONG-OK: p2pk::verify accepted a signature that is not one of the reference's ({} bytes of to_sign)", ts.len());
    }
}
