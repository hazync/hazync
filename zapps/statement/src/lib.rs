//! Balance statement zyncAPP (docs/libzync/STATEMENT_ZAPP.md): "the coins at these scripts held X sat in total at block H,
//! and the holder of each script signed this statement". A statement is the counting-trie BALANCE fact of each script at
//! the proof's tip H, plus a BIP-322 signature by each script over one message that binds the statement: the network,
//! the core image, H and its block hash, the script list, the total, a purpose and a nonce. No new guest: the facts are
//! checked against the verified core state's trie root, the signatures by the rust-bitcoin project's `bip322` crate.
//!
//! What it shows: each script was paid the stated unspent coins in the proven chain at H, and someone who could sign
//! for each script at the time of signing signed this message. NOT that the signer controls the coins exclusively, nor
//! that the coins are still unspent after H (STATEMENT_ZAPP.md §4).

pub mod p2pk;

use swiftsync_verify::{check_trie_fact, hex, unhex, ChainState, TrieFact, PROFILE_SIGNET};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// The statement format this crate reads and writes.
pub const STATEMENT_VERSION: u32 = 1;
/// The first line of every statement message (domain separation from any other signed text).
pub const MESSAGE_HEADER: &str = "Hazync balance statement v1";
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

/// A purpose or nonce: 1 to [`MAX_FIELD`] bytes of printable ASCII (0x20 to 0x7e), not starting or ending with a space.
/// Printable ASCII, not "no control characters": Unicode line and paragraph separators, bidirectional and other format
/// characters would let a purpose show a wallet's user one line while the message says another. Surrounding spaces are
/// refused rather than trimmed, so every checker (CLI, WASM) compares the same bytes the signer signed.
fn field_ok(what: &str, s: &str) -> Result<(), String> {
    if s.is_empty() || s.len() > MAX_FIELD {
        return Err(format!("{what}: 1 to {MAX_FIELD} bytes"));
    }
    if !s.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        return Err(format!("{what}: printable ASCII only (no line breaks, tabs, or non-ASCII characters)"));
    }
    if s.starts_with(' ') || s.ends_with(' ') {
        return Err(format!("{what}: no leading or trailing spaces"));
    }
    Ok(())
}

/// 64 lower-case hex digits: how a statement writes the core image id and the block hash (checked before either is
/// printed or compared).
fn hex64_ok(what: &str, s: &str) -> Result<(), String> {
    if s.len() != 64 || !s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err(format!("{what}: want 64 lower-case hex digits, got {s:?}"));
    }
    Ok(())
}

/// Lower-case hex, decoded: how a statement writes every hex field. `unhex` reads either case (right for a hash a person
/// types), so without this a statement with one digit upper-cased held as the signed one, though it does not re-encode
/// to it (fuzz round 11-17, artifacts/statement_check/wrongok-uppercase-hex-in-fact-proof.json). A statement holds only
/// in the one encoding [`draft`] and [`sign`] write.
fn canonical_hex(what: &str, s: &str) -> Result<Vec<u8>, String> {
    match unhex(s) {
        Some(b) if hex(&b) == s => Ok(b),
        Some(_) => Err(format!("{what}: hex must be lower-case")),
        None => Err(format!("{what}: not hex")),
    }
}

/// The scripts of a statement, decoded, in entry order.
fn scripts_of(st: &Statement) -> Result<Vec<Vec<u8>>, String> {
    st.entries.iter().enumerate()
        .map(|(i, e)| canonical_hex(&format!("entry {i}: script_pubkey"), &e.fact.script_pubkey))
        .collect()
}

/// The exact text every script signs. Every field of the statement except the facts and signatures is in it; the
/// scripts are in it through [`script_list_hash`].
pub fn message(st: &Statement) -> Result<String, String> {
    field_ok("purpose", &st.purpose)?;
    field_ok("nonce", &st.nonce)?;
    hex64_ok("core_id", &st.core_id)?;
    hex64_ok("block_hash", &st.block_hash)?;
    if st.entries.is_empty() || st.entries.len() > MAX_ENTRIES {
        return Err(format!("1 to {MAX_ENTRIES} scripts"));
    }
    let scripts = scripts_of(st)?;
    Ok(format!(
        "{MESSAGE_HEADER}\nnetwork: {}\ncore image: {}\nblock: {} {}\nscripts: {} sha256 {}\ntotal: {} sat\npurpose: {}\nnonce: {}",
        st.network, st.core_id, st.height, st.block_hash, scripts.len(),
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
    use base64::Engine;
    // P2PK has no address, so the crate cannot take it: BIP-322 full, checked by p2pk.rs
    if p2pk::p2pk_key(script).is_some() {
        if format != "full" {
            return Err(format!("format {format:?}: a P2PK script signs in BIP-322 full only"));
        }
        let b = base64::engine::general_purpose::STANDARD.decode(sig.strip_prefix(bip322::FULL_SIGNATURE_PREFIX).unwrap_or(sig))
            .map_err(|_| "BIP-322 full: not base64".to_string())?;
        return p2pk::verify(script, msg.as_bytes(), &b);
    }
    // bip322 takes an address; scripts with no address form (bare multisig, non-standard) cannot sign here
    let addr = Address::from_script(Script::from_bytes(script), net)
        .map_err(|_| "this script has no address form (bare multisig or non-standard): the bip322 crate cannot check a signature for it".to_string())?
        .to_string();
    // BIP-341: a 65-byte key-path signature whose hash type byte is 0x00 is invalid (SIGHASH_DEFAULT is written only as a
    // 64-byte signature); the crate reads 0x00 as DEFAULT and accepts it, so refuse it here first
    if Script::from_bytes(script).is_p2tr() {
        let wit = match format {
            "simple" => base64::engine::general_purpose::STANDARD.decode(sig.strip_prefix(bip322::SIMPLE_SIGNATURE_PREFIX).unwrap_or(sig)).ok()
                .and_then(|b| bitcoin::consensus::deserialize::<bitcoin::Witness>(&b).ok()),
            "full" => base64::engine::general_purpose::STANDARD.decode(sig.strip_prefix(bip322::FULL_SIGNATURE_PREFIX).unwrap_or(sig)).ok()
                .and_then(|b| bitcoin::consensus::deserialize::<bitcoin::Transaction>(&b).ok())
                .and_then(|t| t.input.first().map(|i| i.witness.clone())),
            _ => None,
        };
        if wit.as_ref().and_then(|w| w.nth(0)).map_or(false, |s0| s0.len() == 65 && s0[64] == 0x00) {
            return Err("P2TR: a 65-byte signature with hash type 0x00 (invalid under BIP-341: DEFAULT is 64 bytes)".into());
        }
    }
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

/// Checks a statement against a chain state that passed [`swiftsync_verify::verify`]: it is about this proof's network, image, tip
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
    hex64_ok("core_id", &st.core_id)?;
    hex64_ok("block_hash", &st.block_hash)?;
    let core = core_id_of(cs);
    if st.core_id != core {
        return Err(format!("the statement names core image {:?}, the proof was made by {core}", st.core_id));
    }
    if st.height != cs.height {
        return Err(format!("the statement is about block {}, the proof's tip is {} (a statement is at the proof's tip)", st.height, cs.height));
    }
    let tip = display_hash(&cs.tip_hash);
    if st.block_hash != tip {
        return Err(format!("the statement names block {:?} at {}, the proof's tip is {tip}", st.block_hash, st.height));
    }
    // before any per-entry work (hex decoding, the message's script list)
    if st.entries.is_empty() || st.entries.len() > MAX_ENTRIES {
        return Err(format!("1 to {MAX_ENTRIES} scripts"));
    }
    let msg = message(st)?;
    if let Some(p) = ch.purpose {
        if p != st.purpose { return Err(format!("purpose {:?} is not the one asked for ({p:?})", st.purpose)); }
    }
    if let Some(n) = ch.nonce {
        if n != st.nonce { return Err(format!("nonce {:?} is not the one asked for ({n:?})", st.nonce)); }
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
        // the fields a balance fact does not read must be absent, and its proof lower-case hex: one encoding only
        if f.txid.is_some() || f.vout.is_some() || f.origin.is_some() {
            return Err(format!("entry {i}: a balance fact has no txid, vout or origin"));
        }
        canonical_hex(&format!("entry {i}: proof"), &f.proof)?;
        check_trie_fact(cs, undo_from, f).map_err(|x| format!("entry {i}: {x}"))?;
        let (c, v) = balance_of(want).map_err(|x| format!("entry {i}: expect {want:?}: {x}"))?;
        sum = sum.checked_add(v).ok_or("the values overflow")?;
        coins = coins.checked_add(c).ok_or("the coin counts overflow")?;
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

/// The recency check (`--max-age`): block `h` is at most `max_age` blocks behind the network's tip `tip` (from a source
/// the checker trusts). A statement is about block H; its coins may have moved in any block since, so how old H may be
/// is the checker's policy, and this is where it is applied. Ok: the line saying so.
pub fn check_age(h: u32, tip: u32, max_age: u32) -> Result<String, String> {
    if tip < h {
        return Err(format!("the network's tip {tip} is below the statement's block {h}: that source is behind the proof"));
    }
    let age = tip - h;
    if age > max_age {
        return Err(format!("block {h} is {age} blocks behind the network's tip {tip}, more than the {max_age} allowed (--max-age)"));
    }
    Ok(format!("block {h} is {age} block(s) behind the network's tip {tip} (at most {max_age} allowed)"))
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
        let f = swiftsync_verify::make_trie_fact(trie, "balance", spk, None, None)?;
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
/// P2TR (key path, BIP-86 tweak, no script tree) in BIP-322 simple, P2PKH and P2PK (the key's own form) in BIP-322 full.
/// Order: P2PKH, then (compressed keys only) P2SH-P2WPKH, P2WPKH, P2TR, then P2PK.
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
    out.push((p2pk::p2pk_script(key), "full"));
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
            let sig = if p2pk::p2pk_key(&spk).is_some() { p2pk::sign(k, msg.as_bytes())? } else {
            let addr = Address::from_script(Script::from_bytes(&spk), net).map_err(|e| e.to_string())?;
            match fmt {
                "simple" => bip322::sign_simple(&addr, &msg, std::slice::from_ref(k), None).map(|w| {
                    let mut b = Vec::new();
                    bitcoin::consensus::Encodable::consensus_encode(&w, &mut b).expect("vec write");
                    b
                }),
                _ => bip322::sign_full(&addr, &msg, std::slice::from_ref(k), None, bip322::LockParams::default()).map(|t| bitcoin::consensus::serialize(&t)),
            }.map_err(|e| format!("bip322 sign: {e}"))?
            };
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
    use swiftsync_verify::{verify, Image, Inputs, Verdict};
    use bitcoin::PrivateKey;

    const P2PK_BLOCK9: &str = "410411db93e1dcdb8a016b49840f8c53bc1eb68a382e97b1482ecad7b148a6909a5cb2e0eaddfb84ccf9744464f82e160bfa9b8b64f9d4c03f999b8643f656b412a3ac";

    fn vector(name: &str) -> Vec<u8> {
        let here = format!("{}/vectors/{name}", env!("CARGO_MANIFEST_DIR"));
        let core = format!("{}/../../swiftsync-verify/vectors/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read(&here).or_else(|_| std::fs::read(&core)).expect(name)
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

    /// Fuzz round 11-17 (artifacts/statement_check/wrongok-uppercase-hex-in-fact-proof.json): a statement holds only in
    /// the encoding the signer wrote. One hex digit upper-cased in a fact's proof or script, or a txid / vout / origin
    /// on a balance fact (fields it never reads), is refused, though each is the same fact to `unhex` and the trie.
    #[test]
    fn a_statement_holds_only_as_written() {
        let (cs, uf) = real();
        let (k1, k2) = (key(), key());
        let st = signed(&cs, &real_trie(), &[script(&k1, 2), script(&k2, 3)], &[k1, k2]);
        check_statement(&cs, uf, &st, &ASK).unwrap();
        let upper_first_letter = |s: &str| {
            let i = s.find(|c: char| c.is_ascii_lowercase()).expect("a hex letter");
            format!("{}{}{}", &s[..i], s[i..i + 1].to_uppercase(), &s[i + 1..])
        };
        let mut t = st.clone();
        t.entries[1].fact.proof = upper_first_letter(&t.entries[1].fact.proof);
        assert!(check_statement(&cs, uf, &t, &ASK).unwrap_err().contains("entry 1: proof: hex must be lower-case"));
        let mut t = st.clone();
        t.entries[0].fact.script_pubkey = upper_first_letter(&t.entries[0].fact.script_pubkey);
        assert!(check_statement(&cs, uf, &t, &ASK).unwrap_err().contains("entry 0: script_pubkey: hex must be lower-case"));
        let mut t = st.clone();
        t.entries[0].fact.txid = Some("00".repeat(32));
        assert!(check_statement(&cs, uf, &t, &ASK).unwrap_err().contains("no txid, vout or origin"));
        let mut t = st.clone();
        t.entries[1].fact.vout = Some(0);
        assert!(check_statement(&cs, uf, &t, &ASK).unwrap_err().contains("no txid, vout or origin"));
    }

    #[test]
    fn real_p2pk_coin_without_its_key() {
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
        assert!(e.contains("signs in BIP-322 full only"), "{e}");
        // a BIP-322 full P2PK signature by a key of ours, filed under block 9's script: the real fact holds, the
        // signature does not (no one here holds that key)
        let msg = message(&st).unwrap();
        use base64::Engine;
        st.entries[i].format = "full".into();
        st.entries[i].signature = base64::engine::general_purpose::STANDARD.encode(p2pk::sign(&key(), msg.as_bytes()).unwrap());
        let e = check_statement(&cs, uf, &st, &Challenge::default()).unwrap_err();
        assert!(e.contains("entry 1") && e.contains("P2PK: to_sign is not BIP-322's") || e.contains("does not verify"), "{e}");
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
        let mut t = st.clone(); t.purpose = "x\ntotal: 99 sat".into(); refuse(&t, &Challenge::default(), "printable ASCII");
        // Unicode line / paragraph separators and bidi or other format characters, a tab, surrounding spaces
        for bad in ["a\u{2028}total: 9 sat", "a\u{2029}b", "abc\u{202e}def", "zero\u{200b}width", "a\tb", "caf\u{e9}"] {
            let mut t = st.clone(); t.purpose = bad.into(); refuse(&t, &Challenge::default(), "printable ASCII");
            let mut t = st.clone(); t.nonce = bad.into(); refuse(&t, &Challenge::default(), "printable ASCII");
        }
        for bad in [" lead", "trail ", " "] {
            let mut t = st.clone(); t.purpose = bad.into(); refuse(&t, &Challenge::default(), "leading or trailing|1 to");
        }
        // core_id / block_hash: 64 lower-case hex only, refused before they are compared or printed (a line break in
        // either could forge a verdict line); the refusal prints them escaped
        for bad in ["8888D59C4B8C699FD4CC3AFAF8168FF2BAA1F6511B1DCEA1C23F041B2B2315F4", "ZAPP_STATEMENT_OK\n", ""] {
            let mut t = st.clone(); t.core_id = bad.into();
            let e = check_statement(&cs, uf, &t, &ASK).unwrap_err();
            assert!(e.contains("64 lower-case hex") && !e.contains('\n'), "{e:?}");
            let mut t = st.clone(); t.block_hash = bad.into();
            let e = check_statement(&cs, uf, &t, &ASK).unwrap_err();
            assert!(e.contains("64 lower-case hex") && !e.contains('\n'), "{e:?}");
        }
        // too many entries: refused before any of them is decoded
        let mut t = st.clone(); t.entries = vec![t.entries[0].clone(); MAX_ENTRIES + 1]; refuse(&t, &ASK, "1 to 10000 scripts");
        // a P2TR key-path signature of 65 bytes ending 0x00 (BIP-341 invalid). The crate signs ALL (65 bytes, 0x01), so a
        // SIGHASH_DEFAULT signature (64 bytes) is made here: valid as 64 bytes, and the crate alone also accepts it with
        // 0x00 appended (it reads 0x00 as DEFAULT); check_signature refuses that
        {
            use base64::Engine;
            use bitcoin::{hashes::Hash, key::TapTweak, sighash::{Prevouts, SighashCache, TapSighashType}};
            let i = st.entries.iter().position(|e| unhex(&e.fact.script_pubkey).unwrap()[0] == 0x51).unwrap();
            let spk = unhex(&st.entries[i].fact.script_pubkey).unwrap();
            let addr = bitcoin::Address::from_script(bitcoin::Script::from_bytes(&spk), bitcoin::Network::Bitcoin).unwrap();
            let msg = message(&st).unwrap();
            let tx = bip322::create_to_sign(&bip322::create_to_spend(&addr, &msg).unwrap(), None, bip322::LockParams::default()).unwrap().unsigned_tx;
            let prev = [bitcoin::TxOut { value: bitcoin::Amount::ZERO, script_pubkey: bitcoin::ScriptBuf::from_bytes(spk.clone()) }];
            let h = SighashCache::new(&tx).taproot_key_spend_signature_hash(0, &Prevouts::All(&prev), TapSighashType::Default).unwrap();
            let secp = bitcoin::secp256k1::Secp256k1::new();
            let kp = bitcoin::key::Keypair::from_secret_key(&secp, &k2.inner).tap_tweak(&secp, None).to_keypair();
            let sig = secp.sign_schnorr_no_aux_rand(&bitcoin::secp256k1::Message::from_digest(h.to_byte_array()), &kp);
            let enc = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(bitcoin::consensus::serialize(&bitcoin::Witness::from_slice(&[b.to_vec()])));
            let s64 = sig.as_ref().to_vec();
            let mut s65 = s64.clone();
            s65.push(0x00);
            assert_eq!(check_signature("mainnet", &spk, "simple", &enc(&s64), &msg), Ok(()), "a 64-byte DEFAULT signature is valid");
            assert!(bip322::verify_simple_encoded(&addr.to_string(), &msg, &enc(&s65)).is_ok(), "the crate alone takes 0x00 as DEFAULT");
            let mut t = st.clone();
            t.entries[i].signature = enc(&s65);
            refuse(&t, &ASK, "hash type 0x00");
        }
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
    fn recency() {
        assert!(check_age(200, 200, 0).unwrap().contains("0 block(s) behind"));
        assert!(check_age(200, 206, 6).is_ok());
        assert!(check_age(200, 207, 6).unwrap_err().contains("7 blocks behind"));
        assert!(check_age(200, 199, 6).unwrap_err().contains("behind the proof"));
    }

    #[test]
    fn message_is_stable() {
        // the signed text is part of the format: a change here breaks every statement signed before it
        let (cs, _) = real();
        let st = draft(&cs, &real_trie(), &[unhex(P2PK_BLOCK9).unwrap()], "audit", "n1").unwrap();
        assert_eq!(message(&st).unwrap(), "Hazync balance statement v1\nnetwork: mainnet\n\
            core image: 8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4\n\
            block: 200 000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320\n\
            scripts: 1 sha256 ".to_string() + &hex(&script_list_hash(&[unhex(P2PK_BLOCK9).unwrap()])) + "\ntotal: 2800000000 sat\npurpose: audit\nnonce: n1");
    }

    /// A key in its uncompressed form (P2PK coins of 2009 use one).
    fn uncompressed() -> PrivateKey { let mut k = key(); k.compressed = false; k }

    fn b64(b: &[u8]) -> String { use base64::Engine; base64::engine::general_purpose::STANDARD.encode(b) }
    fn unb64(s: &str) -> Vec<u8> { use base64::Engine; base64::engine::general_purpose::STANDARD.decode(s).unwrap() }

    #[test]
    fn p2pk_to_spend_and_to_sign_are_the_crates() {
        // p2pk.rs builds to_spend itself (the crate's takes an address): for every script the crate takes, the same txid,
        // and the same to_sign
        let k = key();
        for (spk, _) in scripts_for_key(&k).into_iter().filter(|(s, _)| p2pk::p2pk_key(s).is_none()) {
            let a = bitcoin::Address::from_script(bitcoin::Script::from_bytes(&spk), bitcoin::Network::Bitcoin).unwrap();
            for m in ["", "Hello World", "Hazync balance statement v1\nnonce: x"] {
                let c = bip322::create_to_spend(&a, m).unwrap();
                assert_eq!(p2pk::to_spend(&spk, m.as_bytes()).compute_txid(), c.compute_txid(), "to_spend {a} {m:?}");
                let cs = bip322::create_to_sign(&c, None, bip322::LockParams::default()).unwrap().unsigned_tx;
                assert_eq!(p2pk::to_sign(&spk, m.as_bytes()).unwrap().compute_txid(), cs.compute_txid(), "to_sign {a} {m:?}");
            }
        }
    }

    #[test]
    fn p2pk_both_key_forms_and_the_refusals() {
        let (kc, ku, kw) = (key(), uncompressed(), key());
        let (pc, pu) = (p2pk::p2pk_script(&kc), p2pk::p2pk_script(&ku));
        assert_eq!((pc.len(), pu.len()), (35, 67), "compressed and uncompressed P2PK");
        let owned = vec![(pc.clone(), &[5_000_000_000u64][..]), (pu.clone(), &[50_000][..]), (script(&kw, 2), &[7][..])];
        let (cs, uf, trie) = synthetic(&owned);
        let st = signed(&cs, &trie, &[pc.clone(), pu.clone(), script(&kw, 2)], &[kc, ku, kw]);
        let c = check_statement(&cs, uf, &st, &ASK).unwrap();
        assert_eq!((c.total, c.scripts), (5_000_050_007, 3));
        let msg = message(&st).unwrap();
        let ip = |spk: &Vec<u8>| st.entries.iter().position(|e| unhex(&e.fact.script_pubkey).as_ref() == Some(spk)).unwrap();
        for (spk, what) in [(&pc, "compressed"), (&pu, "uncompressed")] {
            let i = ip(spk);
            let tx_bytes = unb64(&st.entries[i].signature);
            assert_eq!(p2pk::verify(spk, msg.as_bytes(), &tx_bytes), Ok(()), "{what}");
            let tx: bitcoin::Transaction = bitcoin::consensus::deserialize(&tx_bytes).unwrap();
            let sig = tx.input[0].script_sig.instructions().next().unwrap().unwrap().push_bytes().unwrap().as_bytes().to_vec();
            let with_sig = |sig: &[u8]| -> Vec<u8> {
                let mut t = tx.clone();
                t.input[0].script_sig = bitcoin::script::Builder::new().push_slice(bitcoin::script::PushBytesBuf::try_from(sig.to_vec()).unwrap()).into_script();
                bitcoin::consensus::serialize(&t)
            };
            let with = |f: &dyn Fn(&mut bitcoin::Transaction)| -> Vec<u8> { let mut t = tx.clone(); f(&mut t); bitcoin::consensus::serialize(&t) };
            let refuse = |b: &[u8], m: &[u8], want: &str| {
                let e = p2pk::verify(spk, m, b).expect_err(want);
                assert!(e.contains(want), "{what}: want {want:?}, got {e:?}");
            };
            // a changed message, a changed signature byte, another sighash type
            refuse(&tx_bytes, b"another message", "to_sign is not BIP-322's");
            let mut s2 = sig.clone(); let n = s2.len(); s2[n - 3] ^= 1; refuse(&with_sig(&s2), msg.as_bytes(), "does not verify");
            for ty in [0x02u8, 0x03, 0x81] { let mut s2 = sig.clone(); *s2.last_mut().unwrap() = ty; refuse(&with_sig(&s2), msg.as_bytes(), "SIGHASH_ALL"); }
            // high S (n - s), the same signature otherwise: BIP-322 LOW_S
            let der = &sig[..sig.len() - 1];
            let mut compact = bitcoin::secp256k1::ecdsa::Signature::from_der(der).unwrap().serialize_compact();
            const N: [u8; 32] = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe,
                                 0xba, 0xae, 0xdc, 0xe6, 0xaf, 0x48, 0xa0, 0x3b, 0xbf, 0xd2, 0x5e, 0x8c, 0xd0, 0x36, 0x41, 0x41];
            let mut borrow = 0i16;
            for j in (0..32).rev() {
                let d = N[j] as i16 - compact[32 + j] as i16 - borrow;
                compact[32 + j] = d.rem_euclid(256) as u8;
                borrow = (d < 0) as i16;
            }
            let high = bitcoin::secp256k1::ecdsa::Signature::from_compact(&compact).unwrap();
            let mut hs = high.serialize_der().to_vec(); hs.push(1);
            refuse(&with_sig(&hs), msg.as_bytes(), "high-S");
            // not strictly DER (a padded length), a non-minimal push, two pushes, a witness, a lock time, version 2
            let mut bad = sig.clone(); bad[1] = bad[1].wrapping_add(1); refuse(&with_sig(&bad), msg.as_bytes(), "DER");
            let mut nm = vec![0x4c, sig.len() as u8]; nm.extend_from_slice(&sig);
            refuse(&with(&|t| t.input[0].script_sig = bitcoin::ScriptBuf::from_bytes(nm.clone())), msg.as_bytes(), "MINIMALDATA");
            refuse(&with(&|t| { let mut b = t.input[0].script_sig.to_bytes(); b.insert(0, 0x51); t.input[0].script_sig = bitcoin::ScriptBuf::from_bytes(b) }), msg.as_bytes(), "exactly one push");
            refuse(&with(&|t| t.input[0].witness.push([1u8])), msg.as_bytes(), "to_sign is not BIP-322's");
            refuse(&with(&|t| t.lock_time = bitcoin::absolute::LockTime::from_height(900_000).unwrap()), msg.as_bytes(), "to_sign is not BIP-322's");
            refuse(&with(&|t| t.version = bitcoin::transaction::Version(2)), msg.as_bytes(), "version 2");
            // another key's P2PK signature over the same message
            refuse(&p2pk::sign(&key(), msg.as_bytes()).unwrap(), msg.as_bytes(), "to_sign is not BIP-322's");
        }
        // the key forms swapped: the compressed key's signature does not verify for the uncompressed script of another key
        let e = check_statement(&cs, uf, &{ let mut t = st.clone(); let (a, b) = (ip(&pc), ip(&pu)); t.entries[a].signature = st.entries[b].signature.clone(); t }, &ASK).unwrap_err();
        assert!(e.contains("P2PK"), "{e}");
        // a hybrid key (0x06 / 0x07) fails STRICTENC; a P2PK entry in BIP-322 simple is refused
        let mut hy = pu.clone(); hy[1] = 0x06;
        assert!(p2pk::verify(&hy, msg.as_bytes(), &[]).unwrap_err().contains("STRICTENC"));
        let mut t = st.clone(); let i = ip(&pc); t.entries[i].format = "simple".into();
        assert!(check_statement(&cs, uf, &t, &ASK).unwrap_err().contains("full only"));
        let _ = b64(&[]);
    }

    // ---- verdict paths (docs/libzync/ZAPP_TESTING.md §3.5, the cargo-mutants pass of 17.11.2026): each test holds a
    // check in place that no test above reached

    /// script_list_hash is SHA-256 over each script as CompactSize(length) || bytes, its three length forms included:
    /// the preimage is written out here byte by byte, not through the function.
    #[test]
    fn script_list_hash_is_the_serialisation() {
        let sha = |b: &[u8]| -> [u8; 32] { Sha256::digest(b).into() };
        assert_eq!(script_list_hash(&[]), sha(&[]));
        let (a, b, c, d) = (vec![0x51u8], vec![7u8; 0xfc], vec![8u8; 0xfd], vec![9u8; 0x1_0000]);
        assert_eq!(script_list_hash(&[a.clone()]), sha(&[0x01, 0x51]));
        assert_eq!(script_list_hash(&[b.clone()]), sha(&[&[0xfc][..], &b].concat()));
        assert_eq!(script_list_hash(&[c.clone()]), sha(&[&[0xfd, 0xfd, 0x00][..], &c].concat()));
        assert_eq!(script_list_hash(&[d.clone()]), sha(&[&[0xfe, 0x00, 0x00, 0x01, 0x00][..], &d].concat()));
        assert_eq!(script_list_hash(&[a.clone(), c.clone()]), sha(&[&[0x01, 0x51, 0xfd, 0xfd, 0x00][..], &c].concat()));
        // so two script lists sign two messages
        assert_ne!(script_list_hash(&[a]), script_list_hash(&[vec![0x52]]));
    }

    /// purpose and nonce: 1 to MAX_FIELD bytes, both ends of the range; the script count: 1 to MAX_ENTRIES, both ends.
    #[test]
    fn message_field_and_entry_bounds() {
        let (cs, _) = real();
        let st = draft(&cs, &real_trie(), &[unhex(P2PK_BLOCK9).unwrap()], "audit", "n1").unwrap();
        for f in [0, 1] {
            let set = |v: String| { let mut t = st.clone(); if f == 0 { t.purpose = v } else { t.nonce = v } t };
            assert!(message(&set(String::new())).unwrap_err().contains("1 to 256 bytes"));
            assert!(message(&set("a".into())).is_ok());
            assert!(message(&set("a".repeat(MAX_FIELD))).is_ok());
            assert!(message(&set("a".repeat(MAX_FIELD + 1))).unwrap_err().contains("1 to 256 bytes"));
            assert!(message(&set("a".repeat(MAX_FIELD + 50))).is_err());
        }
        let with = |n: usize| { let mut t = st.clone(); t.entries = vec![st.entries[0].clone(); n]; t };
        assert!(message(&with(0)).unwrap_err().contains("1 to 10000 scripts"));
        assert!(message(&with(MAX_ENTRIES)).is_ok());
        assert!(message(&with(MAX_ENTRIES + 1)).unwrap_err().contains("1 to 10000 scripts"));
    }

    /// check_statement refuses a bad script count BEFORE any per-entry work (the message's own field checks included):
    /// with a bad purpose too, the count is the refusal; at exactly MAX_ENTRIES the count passes and the purpose is.
    #[test]
    fn the_script_count_is_refused_first() {
        let (cs, uf) = real();
        let st = draft(&cs, &real_trie(), &[unhex(P2PK_BLOCK9).unwrap()], "audit", "n1").unwrap();
        let with = |n: usize| { let mut t = st.clone(); t.purpose = String::new(); t.entries = vec![st.entries[0].clone(); n]; t };
        for n in [0, MAX_ENTRIES + 1] {
            let e = check_statement(&cs, uf, &with(n), &Challenge::default()).unwrap_err();
            assert!(e.contains("1 to 10000 scripts"), "{n}: {e}");
        }
        let e = check_statement(&cs, uf, &with(MAX_ENTRIES), &Challenge::default()).unwrap_err();
        assert!(e.contains("purpose"), "{e}");
    }

    /// `challenged` only when BOTH purpose and nonce were compared.
    #[test]
    fn challenged_needs_purpose_and_nonce() {
        let (cs, uf) = real();
        let k = key();
        let st = signed(&cs, &real_trie(), &[script(&k, 2)], &[k]);
        assert!(check_statement(&cs, uf, &st, &ASK).unwrap().challenged);
        assert!(!check_statement(&cs, uf, &st, &Challenge { purpose: ASK.purpose, nonce: None }).unwrap().challenged);
        assert!(!check_statement(&cs, uf, &st, &Challenge { purpose: None, nonce: ASK.nonce }).unwrap().challenged);
    }

    /// The BIP-341 0x00 refusal in BIP-322 FULL format as well as simple: the same 65-byte key-path signature ending 0x00
    /// as the witness of a to_sign transaction. The crate alone accepts it (it reads 0x00 as DEFAULT); check_signature
    /// does not. The 64-byte form of the same signature passes, so the refusal is the hash type's.
    #[test]
    fn p2tr_hash_type_zero_refused_in_full_format() {
        use base64::Engine;
        use bitcoin::{hashes::Hash, key::TapTweak, sighash::{Prevouts, SighashCache, TapSighashType}};
        let k = key();
        let spk = script(&k, 3);
        let addr = bitcoin::Address::from_script(bitcoin::Script::from_bytes(&spk), bitcoin::Network::Bitcoin).unwrap();
        let msg = "Hazync balance statement v1\nnonce: full-format";
        let mut tx = bip322::create_to_sign(&bip322::create_to_spend(&addr, msg).unwrap(), None, bip322::LockParams::default()).unwrap().unsigned_tx;
        let prev = [bitcoin::TxOut { value: bitcoin::Amount::ZERO, script_pubkey: bitcoin::ScriptBuf::from_bytes(spk.clone()) }];
        let h = SighashCache::new(&tx).taproot_key_spend_signature_hash(0, &Prevouts::All(&prev), TapSighashType::Default).unwrap();
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let kp = bitcoin::key::Keypair::from_secret_key(&secp, &k.inner).tap_tweak(&secp, None).to_keypair();
        let s64 = secp.sign_schnorr_no_aux_rand(&bitcoin::secp256k1::Message::from_digest(h.to_byte_array()), &kp).as_ref().to_vec();
        let mut s65 = s64.clone();
        s65.push(0x00);
        let mut full = |s: &[u8]| {
            tx.input[0].witness = bitcoin::Witness::from_slice(&[s.to_vec()]);
            base64::engine::general_purpose::STANDARD.encode(bitcoin::consensus::serialize(&tx))
        };
        let (f64, f65) = (full(&s64), full(&s65));
        assert_eq!(check_signature("mainnet", &spk, "full", &f64, msg), Ok(()), "the 64-byte DEFAULT signature, full format");
        assert!(bip322::verify_full_encoded(&addr.to_string(), msg, &f65).is_ok(), "the crate alone takes 0x00 as DEFAULT");
        assert!(check_signature("mainnet", &spk, "full", &f65, msg).unwrap_err().contains("hash type 0x00"));
    }

    /// p2pk_key: a 0x21 / 0x41 push of the WRONG length before OP_CHECKSIG is not P2PK.
    #[test]
    fn p2pk_key_needs_the_exact_key_length() {
        let s = |push: u8, n: usize| [&[push][..], &vec![2u8; n], &[0xac]].concat();
        assert_eq!(p2pk::p2pk_key(&s(0x21, 33)).map(|k| k.len()), Some(33));
        assert_eq!(p2pk::p2pk_key(&s(0x41, 65)).map(|k| k.len()), Some(65));
        for (push, n) in [(0x21, 32), (0x21, 34), (0x21, 65), (0x41, 64), (0x41, 66), (0x41, 33)] {
            assert_eq!(p2pk::p2pk_key(&s(push, n)), None, "push 0x{push:02x} of {n} bytes");
        }
    }
}
