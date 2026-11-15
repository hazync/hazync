//! Statement of balance (docs/libzync/STATEMENT_ZAPP.md): "the coins at these scripts held X sat in total at block H,
//! and the holder of each script signed this statement". A statement is the counting-trie BALANCE fact of each script at
//! the proof's tip H, plus a BIP-322 signature by each script over one message that binds the statement: the network,
//! the core image, H and its block hash, the script list, the total, a purpose and a nonce. No new guest: the facts are
//! checked against the verified core state's trie root, the signatures by the rust-bitcoin project's `bip322` crate.
//!
//! What it shows: each script was paid the stated unspent coins in the proven chain at H, and someone who could sign
//! for each script at the time of signing signed this message. NOT that the signer controls the coins exclusively, nor
//! that the coins are still unspent after H (STATEMENT_ZAPP.md §4).

use crate::{check_trie_fact, hex, unhex, ChainState, TrieFact, PROFILE_SIGNET};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// The statement format this crate reads and writes.
pub const STATEMENT_VERSION: u32 = 1;
/// The first line of every statement message (domain separation from any other signed text).
pub const MESSAGE_HEADER: &str = "Hazync statement of balance v1";
/// Most scripts in one statement (each entry is a trie proof and a signature check).
pub const MAX_ENTRIES: usize = 10_000;
/// Longest purpose or nonce, in bytes.
pub const MAX_FIELD: usize = 256;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Statement {
    /// [`STATEMENT_VERSION`].
    pub version: u32,
    /// "mainnet" or "signet": the network the core image proves.
    pub network: String,
    /// The core image that proved the chain (64 hex digits, as `swiftsync-verify` prints image ids).
    pub core_id: String,
    /// H: the proof's tip height. Trie facts are at the tip, so H is the proof's tip.
    pub height: u32,
    /// The block at H, display order (as `getblockhash` prints it).
    pub block_hash: String,
    /// What the statement is for, chosen by whoever asked for it (an auditor, a counterparty).
    pub purpose: String,
    /// A value the asker chose, so an old statement cannot be shown again as a new one.
    pub nonce: String,
    /// The sum of the scripts' unspent values at H, in satoshis.
    pub total: u64,
    /// One per script, scripts in strictly increasing byte order (so no script is counted twice).
    pub entries: Vec<Entry>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Entry {
    /// A counting-trie "balance" fact for the script at H, with its claimed answer (`expect`).
    pub fact: TrieFact,
    /// "simple" (BIP-322 simple: a witness), "full" (BIP-322 full: the to_sign transaction) or "legacy" (P2PKH only:
    /// the signmessage format of BIP-137 / Bitcoin Core `signmessage`).
    pub format: String,
    /// Base64, as wallets print it (a BIP-322 "smp" / "ful" prefix is accepted). Empty in a draft.
    pub signature: String,
}

/// What the verifier asked for: the purpose and nonce it gave the holder. A statement checked without them proves the
/// same facts, but nothing stops it being an old statement shown again.
#[derive(Default, Clone, Copy)]
pub struct Challenge<'a> {
    pub purpose: Option<&'a str>,
    pub nonce: Option<&'a str>,
}

/// A statement that holds.
#[derive(Debug, Clone)]
pub struct Checked {
    pub total: u64,
    pub scripts: usize,
    pub coins: u64,
    /// Both purpose and nonce were compared with the verifier's challenge.
    pub challenged: bool,
    pub line: String,
}

/// The network a chain state proves, by its profile.
pub fn network_of(cs: &ChainState) -> &'static str {
    if cs.profile & PROFILE_SIGNET != 0 { "signet" } else if cs.profile & 8 != 0 { "regtest" } else { "mainnet" }
}

/// The image id that made a chain state, as printed (hex of the digest's bytes).
pub fn core_id_of(cs: &ChainState) -> String {
    hex(risc0_zkvm::sha::Digest::from(cs.self_id).as_bytes())
}

/// A block hash in display order.
pub fn display_hash(h: &[u8; 32]) -> String {
    let mut r = *h;
    r.reverse();
    hex(&r)
}

/// SHA-256 over the scripts in entry order, each as CompactSize(length) || bytes (its serialisation in a transaction).
pub fn script_list_hash(scripts: &[Vec<u8>]) -> [u8; 32] {
    let mut h = Sha256::new();
    for s in scripts {
        let n = s.len() as u64;
        match n {
            0..=0xfc => h.update([n as u8]),
            0xfd..=0xffff => { h.update([0xfd]); h.update((n as u16).to_le_bytes()) }
            _ => { h.update([0xfe]); h.update((n as u32).to_le_bytes()) }
        }
        h.update(s);
    }
    h.finalize().into()
}

/// A purpose or nonce: 1 to [`MAX_FIELD`] bytes, no control characters (a line break could forge a line of the message
/// a wallet shows its user).
fn field_ok(what: &str, s: &str) -> Result<(), String> {
    if s.is_empty() || s.len() > MAX_FIELD {
        return Err(format!("{what}: 1 to {MAX_FIELD} bytes"));
    }
    if s.chars().any(|c| c.is_control()) {
        return Err(format!("{what}: no control characters (line breaks, tabs)"));
    }
    Ok(())
}

/// The scripts of a statement, decoded, in entry order.
fn scripts_of(st: &Statement) -> Result<Vec<Vec<u8>>, String> {
    st.entries.iter().enumerate()
        .map(|(i, e)| unhex(&e.fact.script_pubkey).ok_or_else(|| format!("entry {i}: script_pubkey is not hex")))
        .collect()
}

/// The exact text every script signs. Every field of the statement except the facts and signatures is in it; the
/// scripts are in it through [`script_list_hash`].
pub fn message(st: &Statement) -> Result<String, String> {
    field_ok("purpose", &st.purpose)?;
    field_ok("nonce", &st.nonce)?;
    let scripts = scripts_of(st)?;
    Ok(format!(
        "{MESSAGE_HEADER}\nnetwork: {}\ncore image: {}\nblock: {} {}\nscripts: {} sha256 {}\ntotal: {} sat\npurpose: {}\nnonce: {}",
        st.network, st.core_id.to_lowercase(), st.height, st.block_hash.to_lowercase(), scripts.len(),
        hex(&script_list_hash(&scripts)), st.total, st.purpose, st.nonce))
}

/// A balance answer token (`never` or `<count>:<sat>`) as (count, sat).
fn balance_of(token: &str) -> Result<(u64, u64), String> {
    if token == "never" {
        return Ok((0, 0));
    }
    let (c, v) = token.split_once(':').ok_or("not a balance answer")?;
    Ok((c.parse().map_err(|_| "count")?, v.parse().map_err(|_| "value")?))
}

/// Checks one signature over `msg` by `script` (BIP-322 via the `bip322` crate). Only an unconditional proof is
/// accepted: valid with nLockTime 0 and nSequence 0 (BIP-322's "valid at time T and age S" for T = S = 0).
pub fn check_signature(network: &str, script: &[u8], format: &str, sig: &str, msg: &str) -> Result<(), String> {
    use bitcoin::{absolute::LockTime, Address, Network, Script, Sequence};
    let net = match network { "mainnet" => Network::Bitcoin, "signet" => Network::Signet, "regtest" => Network::Regtest,
                              n => return Err(format!("network {n:?}")) };
    if sig.is_empty() {
        return Err("no signature".into());
    }
    // bip322 takes an address; scripts with no address form (P2PK, bare multisig, non-standard) cannot sign here
    let addr = Address::from_script(Script::from_bytes(script), net)
        .map_err(|_| "this script has no address form (P2PK, bare multisig or non-standard): the bip322 crate cannot check a signature for it".to_string())?
        .to_string();
    let unconditional = |v: bip322::Verification| match v {
        bip322::Verification::Valid { time, age } if time == LockTime::ZERO && age == Sequence(0) => Ok(()),
        bip322::Verification::Valid { .. } => Err("the BIP-322 proof is valid only from a time or age (nLockTime / nSequence set): refused".to_string()),
        bip322::Verification::Inconclusive => Err("BIP-322 inconclusive: the verifier cannot interpret this script".to_string()),
    };
    match format {
        "simple" => unconditional(bip322::verify_simple_encoded(&addr, msg, sig).map_err(|e| format!("BIP-322 simple: {e}"))?),
        "full" => unconditional(bip322::verify_full_encoded(&addr, msg, sig).map_err(|e| format!("BIP-322 full: {e}"))?),
        "legacy" => bip322::verify_legacy_encoded(&addr, msg, sig).map_err(|e| format!("legacy (BIP-137): {e}")),
        f => Err(format!("format {f:?}: want simple, full or legacy")),
    }
}

/// Checks a statement against a chain state that passed [`crate::verify`]: it is about this proof's network, image, tip
/// height and block; the scripts are distinct; each fact is a balance fact for its script that holds against the
/// state's trie root; the total is their sum; the purpose and nonce are the challenge's (when given); and each script
/// signed the statement's message.
pub fn check_statement(cs: &ChainState, undo_from: u32, st: &Statement, ch: &Challenge) -> Result<Checked, String> {
    if st.version != STATEMENT_VERSION {
        return Err(format!("statement version {}: this verifier reads {STATEMENT_VERSION}", st.version));
    }
    let net = network_of(cs);
    if st.network != net {
        return Err(format!("the statement is for {:?}, the proof is of {net}", st.network));
    }
    let core = core_id_of(cs);
    if st.core_id.to_lowercase() != core {
        return Err(format!("the statement names core image {}, the proof was made by {core}", st.core_id));
    }
    if st.height != cs.height {
        return Err(format!("the statement is about block {}, the proof's tip is {} (a statement is at the proof's tip)", st.height, cs.height));
    }
    let tip = display_hash(&cs.tip_hash);
    if st.block_hash.to_lowercase() != tip {
        return Err(format!("the statement names block {} at {}, the proof's tip is {tip}", st.block_hash, st.height));
    }
    let msg = message(st)?;
    if let Some(p) = ch.purpose {
        if p != st.purpose { return Err(format!("purpose {:?} is not the one asked for ({p:?})", st.purpose)); }
    }
    if let Some(n) = ch.nonce {
        if n != st.nonce { return Err(format!("nonce {:?} is not the one asked for ({n:?})", st.nonce)); }
    }
    if st.entries.is_empty() || st.entries.len() > MAX_ENTRIES {
        return Err(format!("1 to {MAX_ENTRIES} scripts"));
    }
    let scripts = scripts_of(st)?;
    if let Some(i) = scripts.windows(2).position(|w| w[0] >= w[1]) {
        return Err(format!("entries {i} and {}: scripts must be distinct and in increasing byte order", i + 1));
    }
    let (mut sum, mut coins) = (0u64, 0u64);
    for (i, e) in st.entries.iter().enumerate() {
        let f = &e.fact;
        if f.kind != "balance" {
            return Err(format!("entry {i}: a {:?} fact, want balance", f.kind));
        }
        let want = f.expect.as_deref().ok_or_else(|| format!("entry {i}: the fact carries no answer (expect)"))?;
        check_trie_fact(cs, undo_from, f).map_err(|x| format!("entry {i}: {x}"))?;
        let (c, v) = balance_of(want).map_err(|x| format!("entry {i}: expect {want:?}: {x}"))?;
        sum = sum.checked_add(v).ok_or("the values overflow")?;
        coins += c;
    }
    if sum != st.total {
        return Err(format!("the statement's total is {} sat, its facts sum to {sum} sat", st.total));
    }
    for (i, (e, s)) in st.entries.iter().zip(&scripts).enumerate() {
        check_signature(net, s, &e.format, &e.signature, &msg)
            .map_err(|x| format!("entry {i} (script {}): {x}", e.fact.script_pubkey))?;
    }
    let challenged = ch.purpose.is_some() && ch.nonce.is_some();
    Ok(Checked {
        total: sum, scripts: scripts.len(), coins, challenged,
        line: format!("statement: {} script(s), {coins} unspent coin(s), {sum} sat in total at block {} {tip}; each script signed it; purpose {:?}, nonce {:?}{}",
                      scripts.len(), st.height, st.purpose, st.nonce,
                      if challenged { " (as asked)" } else { " (NOT compared with your challenge: give --purpose and --nonce)" }),
    })
}

/// The producer side: a draft statement (no signatures) for `scripts` at the tip of a trie snapshot that matches the
/// verified state `cs`. Each script's balance fact comes from the snapshot; the total is their sum.
pub fn draft(cs: &ChainState, trie: &libzync::trie_store::TrieStore, scripts: &[Vec<u8>], purpose: &str, nonce: &str)
             -> Result<Statement, String> {
    if trie.root() != libzync::coinlog::w_of(&cs.trie_root) {
        return Err("the trie snapshot's root is not the proof's trie root (a snapshot of another height or chain)".into());
    }
    let mut s: Vec<Vec<u8>> = scripts.to_vec();
    s.sort();
    s.dedup();
    let mut entries = Vec::new();
    let mut total = 0u64;
    for spk in &s {
        let f = crate::make_trie_fact(trie, "balance", spk, None, None)?;
        let (_, v) = balance_of(f.expect.as_deref().unwrap_or("never"))?;
        total = total.checked_add(v).ok_or("the values overflow")?;
        entries.push(Entry { fact: f, format: String::new(), signature: String::new() });
    }
    let st = Statement {
        version: STATEMENT_VERSION, network: network_of(cs).into(), core_id: core_id_of(cs), height: cs.height,
        block_hash: display_hash(&cs.tip_hash), purpose: purpose.into(), nonce: nonce.into(), total, entries,
    };
    message(&st)?;
    Ok(st)
}

/// The scripts a private key can sign for with this crate's signer, and the format each uses: P2WPKH, P2SH-P2WPKH and
/// P2TR (key path, BIP-86 tweak, no script tree) in BIP-322 simple, P2PKH in BIP-322 full.
pub fn scripts_for_key(key: &bitcoin::PrivateKey) -> Vec<(Vec<u8>, &'static str)> {
    use bitcoin::{key::Secp256k1, CompressedPublicKey, ScriptBuf};
    let secp = Secp256k1::new();
    let pk = key.public_key(&secp);
    let mut out = vec![(ScriptBuf::new_p2pkh(&pk.pubkey_hash()).to_bytes(), "full")];
    if let Ok(c) = CompressedPublicKey::try_from(pk) {
        let w = ScriptBuf::new_p2wpkh(&c.wpubkey_hash());
        out.push((ScriptBuf::new_p2sh(&w.script_hash()).to_bytes(), "simple"));
        out.push((w.to_bytes(), "simple"));
        let (x, _) = pk.inner.x_only_public_key();
        out.push((ScriptBuf::new_p2tr(&secp, x, None).to_bytes(), "simple"));
    }
    out
}

/// Signs every entry of `st` whose script one of `keys` can sign for (see [`scripts_for_key`]); returns how many.
pub fn sign(st: &mut Statement, keys: &[bitcoin::PrivateKey]) -> Result<usize, String> {
    use bitcoin::{Address, Network, Script};
    let msg = message(st)?;
    let net = match st.network.as_str() { "mainnet" => Network::Bitcoin, "signet" => Network::Signet, _ => Network::Regtest };
    let mut n = 0;
    for e in st.entries.iter_mut() {
        let spk = unhex(&e.fact.script_pubkey).ok_or("script_pubkey: not hex")?;
        for k in keys {
            let Some((_, fmt)) = scripts_for_key(k).into_iter().find(|(s, _)| *s == spk) else { continue };
            let addr = Address::from_script(Script::from_bytes(&spk), net).map_err(|e| e.to_string())?;
            let sig = match fmt {
                "simple" => bip322::sign_simple(&addr, &msg, std::slice::from_ref(k), None).map(|w| {
                    let mut b = Vec::new();
                    bitcoin::consensus::Encodable::consensus_encode(&w, &mut b).expect("vec write");
                    b
                }),
                _ => bip322::sign_full(&addr, &msg, std::slice::from_ref(k), None, bip322::LockParams::default()).map(|t| bitcoin::consensus::serialize(&t)),
            }.map_err(|e| format!("bip322 sign: {e}"))?;
            use base64::Engine;
            e.format = fmt.into();
            e.signature = base64::engine::general_purpose::STANDARD.encode(sig);
            n += 1;
            break;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    //! Against the real mainnet 1..200 proof (image 8888d59c, vectors/) and its block-200 trie snapshot, with keys
    //! generated here (never printed). The coins of 1..200 are all P2PK, which no key of ours can sign for, so the
    //! positive cases on the real trie are zero balances; the non-zero cases use a trie built here with coins for our
    //! scripts under the real proof's state (its trie root replaced: a state no receipt proves, labelled as such).
    use super::*;
    use crate::{verify, Image, Inputs, Verdict};
    use bitcoin::PrivateKey;

    const P2PK_BLOCK9: &str = "410411db93e1dcdb8a016b49840f8c53bc1eb68a382e97b1482ecad7b148a6909a5cb2e0eaddfb84ccf9744464f82e160bfa9b8b64f9d4c03f999b8643f656b412a3ac";

    fn vector(name: &str) -> Vec<u8> {
        std::fs::read(format!("{}/vectors/{name}", env!("CARGO_MANIFEST_DIR"))).expect(name)
    }

    /// A fresh random key from the OS (never printed).
    fn key() -> PrivateKey {
        use std::io::Read;
        loop {
            let mut b = [0u8; 32];
            std::fs::File::open("/dev/urandom").unwrap().read_exact(&mut b).unwrap();
            if let Ok(k) = bitcoin::secp256k1::SecretKey::from_slice(&b) { return PrivateKey::new(k, bitcoin::Network::Bitcoin); }
        }
    }

    fn script(k: &PrivateKey, kind: usize) -> Vec<u8> {
        // scripts_for_key order: 0 P2PKH, 1 P2SH-P2WPKH, 2 P2WPKH, 3 P2TR
        scripts_for_key(k)[kind].0.clone()
    }

    /// The real proof's verified state, and its undo_from.
    fn real() -> (ChainState, u32) {
        let r = vector("mainnet_1_200_8888d59c.rcpt");
        let image = Image::by_id("mainnet", "8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4").unwrap();
        let rep = verify(&Inputs { receipt: &r, image, anchor: None, now: 1_800_000_000 });
        assert_eq!(rep.verdict, Verdict::Demo, "1..200 is a real proof below the minimum work");
        (rep.state.unwrap(), rep.undo_from)
    }

    fn real_trie() -> libzync::trie_store::TrieStore {
        libzync::trie_store::TrieStore::from_bytes(&vector("mainnet_200_8888d59c.trie")).unwrap()
    }

    /// A trie with coins for `owned` (value per script) under the real state, its root put in place of the real one.
    fn synthetic(owned: &[(Vec<u8>, &[u64])]) -> (ChainState, u32, libzync::trie_store::TrieStore) {
        let (mut cs, uf) = real();
        let mut st = libzync::trie_store::TrieStore::new(true);
        let mut ins = Vec::new();
        for (i, (s, vals)) in owned.iter().enumerate() {
            for (j, v) in vals.iter().enumerate() {
                let txid = [i as u8 + 1; 32];
                ins.push((libzync::trie_fact::coin_key(s, &txid, j as u32), None, *v));
            }
        }
        // a spent coin too, which must not count
        ins.push((libzync::trie_fact::coin_key(&owned[0].0, &[0xee; 32], 0), Some([150, 1, 0, 0, 0, 0, 0, 0]), 7));
        st.apply(&[], &ins).unwrap();
        st.height = cs.height as u64;
        cs.trie_root = libzync::coinlog::h_of(&st.root());
        (cs, uf, st)
    }

    const ASK: Challenge = Challenge { purpose: Some("loan application 15.11.2026"), nonce: Some("7f3a91c2") };

    fn signed(cs: &ChainState, trie: &libzync::trie_store::TrieStore, scripts: &[Vec<u8>], keys: &[PrivateKey]) -> Statement {
        let mut st = draft(cs, trie, scripts, ASK.purpose.unwrap(), ASK.nonce.unwrap()).unwrap();
        assert_eq!(sign(&mut st, keys).unwrap(), st.entries.len(), "every entry signed");
        st
    }

    #[test]
    fn real_proof_zero_balances_hold() {
        let (cs, uf) = real();
        let (k1, k2) = (key(), key());
        let st = signed(&cs, &real_trie(), &[script(&k1, 2), script(&k2, 3)], &[k1, k2]);
        let c = check_statement(&cs, uf, &st, &ASK).unwrap();
        assert_eq!((c.total, c.scripts, c.coins, c.challenged), (0, 2, 0, true));
        // the same statement claiming 1 sat
        let mut t = st.clone();
        t.total = 1;
        assert!(check_statement(&cs, uf, &t, &ASK).unwrap_err().contains("sum to 0"));
    }

    #[test]
    fn real_p2pk_coins_cannot_sign() {
        let (cs, uf) = real();
        let k = key();
        let mut st = draft(&cs, &real_trie(), &[unhex(P2PK_BLOCK9).unwrap(), script(&k, 2)], "audit", "n").unwrap();
        assert_eq!(st.total, 2_800_000_000, "block 9's coin, from the real trie");
        assert_eq!(sign(&mut st, &[k]).unwrap(), 1, "our key signs only its own script");
        let e = check_statement(&cs, uf, &st, &Challenge::default()).unwrap_err();
        assert!(e.contains("no signature"), "{e}");
        // a signature made for our script, filed under the P2PK entry
        let our = st.entries.iter().find(|e| !e.signature.is_empty()).unwrap().clone();
        let i = st.entries.iter().position(|e| e.signature.is_empty()).unwrap();
        st.entries[i].format = our.format.clone();
        st.entries[i].signature = our.signature.clone();
        let e = check_statement(&cs, uf, &st, &Challenge::default()).unwrap_err();
        assert!(e.contains("no address form"), "{e}");
    }

    #[test]
    fn nonzero_statement_and_its_refusals() {
        let (k1, k2, k3, k4) = (key(), key(), key(), key());
        let owned = vec![(script(&k1, 2), &[50_000u64, 125_000][..]), (script(&k2, 3), &[2_100_000][..]),
                         (script(&k3, 1), &[999][..]), (script(&k4, 0), &[10][..])];
        let (cs, uf, trie) = synthetic(&owned);
        let scripts: Vec<Vec<u8>> = owned.iter().map(|o| o.0.clone()).collect();
        let keys = [k1, k2, k3, k4];
        let st = signed(&cs, &trie, &scripts, &keys);
        let c = check_statement(&cs, uf, &st, &ASK).unwrap();
        assert_eq!((c.total, c.scripts, c.coins), (2_276_009, 4, 5), "P2WPKH, P2TR, P2SH-P2WPKH, P2PKH (full)");
        assert!(!check_statement(&cs, uf, &st, &Challenge::default()).unwrap().challenged);

        let refuse = |t: &Statement, ch: &Challenge, want: &str| {
            let e = check_statement(&cs, uf, t, ch).expect_err(want);
            assert!(want.split('|').any(|w| e.contains(w)), "want {want:?}, got {e:?}");
        };
        // tampered total (and the same total re-signed by nobody: the signatures bind the old one)
        let mut t = st.clone(); t.total += 1; refuse(&t, &ASK, "sum to 2276009");
        // tampered fact answer
        let mut t = st.clone(); t.entries[0].fact.expect = Some("2:175001".into()); refuse(&t, &ASK, "the fact claims");
        // wrong height, wrong block, wrong core image, wrong network
        let mut t = st.clone(); t.height -= 1; refuse(&t, &ASK, "the proof's tip is 200");
        let mut t = st.clone(); t.block_hash = "00".repeat(32); refuse(&t, &ASK, "the proof's tip is");
        let mut t = st.clone(); t.core_id = "46666b6f7e3d1c1c3fa599a9bcf12209e0730d6a2fb747b8bf30fac667e43c1c".into(); refuse(&t, &ASK, "core image");
        let mut t = st.clone(); t.network = "signet".into(); refuse(&t, &ASK, "the proof is of mainnet");
        // a statement drafted against another tip: the same statement checked against a state one block on
        let mut later = cs.clone(); later.height += 1; assert!(check_statement(&later, uf, &st, &ASK).is_err());
        // replay: asked with another purpose or nonce; or the purpose rewritten to the one asked (signatures break)
        refuse(&st, &Challenge { purpose: Some("other purpose"), ..ASK }, "is not the one asked for");
        refuse(&st, &Challenge { nonce: Some("00000000"), ..ASK }, "is not the one asked for");
        let mut t = st.clone(); t.purpose = "other purpose".into();
        refuse(&t, &Challenge { purpose: Some("other purpose"), ..ASK }, "BIP-322");
        // wrong signature: another key's, or a signature over another message
        let mut t = st.clone(); t.entries[0].signature = st.entries[1].signature.clone(); refuse(&t, &ASK, "entry 0");
        let mut other = draft(&cs, &trie, &scripts, "audit", "different").unwrap();
        sign(&mut other, &keys).unwrap();
        let mut t = st.clone(); t.entries[2].signature = other.entries[2].signature.clone(); refuse(&t, &ASK, "entry 2");
        // a script not in the facts: entry 0's proof relabelled with a script of ours that has no fact
        let k5 = key();
        let mut t = st.clone(); t.entries[0].fact.script_pubkey = hex(&script(&k5, 2));
        t.entries.sort_by(|a, b| unhex(&a.fact.script_pubkey).cmp(&unhex(&b.fact.script_pubkey)));
        // (the relabelled path ends at a pruned node, or at an empty slot the proof does cover: "never paid")
        refuse(&t, &ASK, "does not hold|its proof gives \"never\"");
        // the same script twice (counted twice), out of order, a history fact, a fact with no answer
        let mut t = st.clone(); t.entries.push(t.entries[0].clone()); refuse(&t, &ASK, "distinct");
        let mut t = st.clone(); t.entries.swap(0, 1); refuse(&t, &ASK, "increasing");
        let mut t = st.clone(); t.entries[0].fact.kind = "history".into(); refuse(&t, &ASK, "want balance");
        let mut t = st.clone(); t.entries[0].fact.expect = None; refuse(&t, &ASK, "no answer");
        // a line break in the purpose would forge a line of the signed text
        let mut t = st.clone(); t.purpose = "x\ntotal: 99 sat".into(); refuse(&t, &Challenge::default(), "control characters");
        // a format the entry's signature is not in; an empty statement; another version
        let mut t = st.clone(); t.entries[0].format = "full".into(); refuse(&t, &ASK, "entry 0");
        let mut t = st.clone(); t.entries.clear(); refuse(&t, &ASK, "scripts");
        let mut t = st.clone(); t.version = 2; refuse(&t, &ASK, "version");
    }

    #[test]
    fn legacy_and_timelocked_signatures() {
        use base64::Engine;
        let k = key();
        let p2pkh = script(&k, 0);
        let (cs, uf, trie) = synthetic(&[(p2pkh.clone(), &[5_000][..])]);
        let mut st = draft(&cs, &trie, &[p2pkh.clone()], "audit", "n1").unwrap();
        let msg = message(&st).unwrap();
        let addr = bitcoin::Address::from_script(bitcoin::Script::from_bytes(&p2pkh), bitcoin::Network::Bitcoin).unwrap();
        // Bitcoin Core signmessage format (BIP-137): accepted for P2PKH
        let sig = bip322::sign_legacy(&addr, &msg, &k).unwrap();
        st.entries[0].format = "legacy".into();
        st.entries[0].signature = base64::engine::general_purpose::STANDARD.encode(sig.serialize());
        assert_eq!(check_statement(&cs, uf, &st, &Challenge::default()).unwrap().total, 5_000);
        // a BIP-322 full proof that is valid only from a lock time: refused
        let lp = bip322::LockParams { lock_time: bitcoin::absolute::LockTime::from_height(900_000).unwrap(), sequence: bitcoin::Sequence(0) };
        let tx = bip322::sign_full(&addr, &msg, &[k], None, lp).unwrap();
        st.entries[0].format = "full".into();
        st.entries[0].signature = base64::engine::general_purpose::STANDARD.encode(bitcoin::consensus::serialize(&tx));
        let e = check_statement(&cs, uf, &st, &Challenge::default()).unwrap_err();
        assert!(e.contains("valid only from a time"), "{e}");
    }

    #[test]
    fn a_failed_receipt_gives_no_state_to_check() {
        // a dev-mode FakeReceipt: verify() fails it before any journal is read (the CLI then checks no statement)
        let r = vector("fake_block_2501.rcpt");
        let rep = verify(&Inputs { receipt: &r, image: Image::by_network("mainnet").unwrap(), anchor: None, now: 1_800_000_000 });
        assert_eq!(rep.verdict, Verdict::Fail);
        assert!(rep.checks.iter().any(|(ok, w)| !ok && w.contains("FakeReceipt")), "{:?}", rep.checks);
    }

    #[test]
    fn message_is_stable() {
        // the signed text is part of the format: a change here breaks every statement signed before it
        let (cs, _) = real();
        let st = draft(&cs, &real_trie(), &[unhex(P2PK_BLOCK9).unwrap()], "audit", "n1").unwrap();
        assert_eq!(message(&st).unwrap(), "Hazync statement of balance v1\nnetwork: mainnet\n\
            core image: 8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4\n\
            block: 200 000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320\n\
            scripts: 1 sha256 ".to_string() + &hex(&script_list_hash(&[unhex(P2PK_BLOCK9).unwrap()])) + "\ntotal: 2800000000 sat\npurpose: audit\nnonce: n1");
    }
}
