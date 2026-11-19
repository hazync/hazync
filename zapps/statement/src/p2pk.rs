//! BIP-322 full-format signatures for P2PK scripts (`<pubkey> OP_CHECKSIG`), which the `bip322` crate does not cover
//! (it takes an address, and P2PK has none). The ONLY hand-written signature code in this zyncAPP; kept minimal:
//!
//! - to_spend is BIP-322's, with the P2PK script as `message_challenge`. Built here because the crate's
//!   `create_to_spend` takes an address; the same construction, and the tests check that it gives the crate's txid for
//!   scripts the crate does take (P2WPKH, P2TR, P2PKH).
//! - to_sign is the crate's own `create_to_sign` on that to_spend.
//! - the check: to_sign is exactly that transaction (one input spending to_spend:0, no witness, nSequence 0, nLockTime 0,
//!   version 0, one 0-sat OP_RETURN output) with a scriptSig of exactly one minimal push, the signature; the signature
//!   ends in SIGHASH_ALL, is strictly DER-encoded and low-S (BIP-322 required rules: SIGHASH_ALL, LOW_S, STRICTENC,
//!   MINIMALDATA, CLEANSTACK), and verifies by libsecp over the legacy sighash of input 0 with the P2PK script as
//!   scriptCode. The key is compressed (0x02 / 0x03, 33 bytes) or uncompressed (0x04, 65 bytes); hybrid keys (0x06 /
//!   0x07) fail STRICTENC.
//! - a scriptSig other than one push (e.g. a NOP before it) would satisfy the script, but is refused here: this checker
//!   understands only the one-push form (BIP-322 lets a validator without a full interpreter stop at "inconclusive";
//!   a statement refuses both).

use bitcoin::{
    absolute::LockTime, consensus::Decodable, hashes::Hash, script::Instruction, secp256k1, sighash::SighashCache,
    transaction::Version, Amount, EcdsaSighashType, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
};

/// The public key of a P2PK script, or None if the script is not P2PK (33- or 65-byte key push, then OP_CHECKSIG).
pub fn p2pk_key(script: &[u8]) -> Option<&[u8]> {
    match script {
        [0x21, k @ .., 0xac] if k.len() == 33 => Some(k),
        [0x41, k @ .., 0xac] if k.len() == 65 => Some(k),
        _ => None,
    }
}

/// BIP-322's to_spend for any scriptPubKey (the crate builds the same from an address).
pub fn to_spend(script: &[u8], message: &[u8]) -> Transaction {
    let h = bip322::tagged_hash(bip322::BIP322_TAG, message);
    Transaction {
        version: Version(0),
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint { txid: bitcoin::Txid::all_zeros(), vout: 0xFFFF_FFFF },
            script_sig: bitcoin::script::Builder::new().push_int(0).push_slice(h).into_script(),
            sequence: Sequence(0),
            witness: Witness::new(),
        }],
        output: vec![TxOut { value: Amount::ZERO, script_pubkey: ScriptBuf::from_bytes(script.to_vec()) }],
    }
}

/// The unsigned to_sign for `script` and `message`: the crate's `create_to_sign` on [`to_spend`].
pub fn to_sign(script: &[u8], message: &[u8]) -> Result<Transaction, String> {
    let psbt = bip322::create_to_sign(&to_spend(script, message), None, bip322::LockParams::default()).map_err(|e| e.to_string())?;
    Ok(psbt.unsigned_tx)
}

fn sighash(tx: &Transaction, script: &[u8]) -> secp256k1::Message {
    let h = SighashCache::new(tx)
        .legacy_signature_hash(0, bitcoin::Script::from_bytes(script), EcdsaSighashType::All.to_u32())
        .expect("input 0 exists");
    secp256k1::Message::from_digest(h.to_byte_array())
}

/// Signs `message` for the P2PK script of `key` (compressed or uncompressed, as the key says): the to_sign transaction,
/// consensus-encoded (base64 it for a statement entry of format "full").
pub fn sign(key: &bitcoin::PrivateKey, message: &[u8]) -> Result<Vec<u8>, String> {
    let secp = secp256k1::Secp256k1::new();
    let script = p2pk_script(key);
    let mut tx = to_sign(&script, message)?;
    let sig = secp.sign_ecdsa(&sighash(&tx, &script), &key.inner); // libsecp signs low-S
    let mut b = sig.serialize_der().to_vec();
    b.push(EcdsaSighashType::All as u8);
    tx.input[0].script_sig = bitcoin::script::Builder::new()
        .push_slice(bitcoin::script::PushBytesBuf::try_from(b).map_err(|e| e.to_string())?).into_script();
    Ok(bitcoin::consensus::serialize(&tx))
}

/// The P2PK script of a key, in the key's own form (compressed or not).
pub fn p2pk_script(key: &bitcoin::PrivateKey) -> Vec<u8> {
    let pk = key.public_key(&secp256k1::Secp256k1::new()).to_bytes();
    let mut s = vec![pk.len() as u8];
    s.extend_from_slice(&pk);
    s.push(0xac);
    s
}

/// Checks a BIP-322 full signature (`to_sign`, consensus-encoded) by the P2PK `script` over `message`.
pub fn verify(script: &[u8], message: &[u8], to_sign_bytes: &[u8]) -> Result<(), String> {
    let key = p2pk_key(script).ok_or("not a P2PK script")?;
    // STRICTENC for the key
    match (key.len(), key[0]) {
        (33, 0x02 | 0x03) | (65, 0x04) => {}
        _ => return Err("P2PK: the key is not a strictly encoded public key (STRICTENC)".into()),
    }
    let pk = secp256k1::PublicKey::from_slice(key).map_err(|_| "P2PK: the key is not on the curve")?;
    let mut r = to_sign_bytes;
    let tx = Transaction::consensus_decode_from_finite_reader(&mut r).map_err(|e| format!("P2PK: to_sign does not decode ({e})"))?;
    if !r.is_empty() {
        return Err("P2PK: bytes after to_sign".into());
    }
    // exactly the crate's to_sign but for the scriptSig
    let mut want = to_sign(script, message)?;
    if tx.version == Version(2) {
        return Err("P2PK: to_sign version 2 (a time-locked proof): refused".into());
    }
    want.input[0].script_sig = tx.input.first().map(|i| i.script_sig.clone()).unwrap_or_default();
    if tx != want {
        return Err("P2PK: to_sign is not BIP-322's for this script and message (input, output, version, nLockTime, nSequence or witness)".into());
    }
    // the scriptSig: one push, minimally encoded (MINIMALDATA), nothing else (CLEANSTACK, push-only)
    let ss = &tx.input[0].script_sig;
    let mut it = ss.instructions_minimal();
    let sig = match (it.next(), it.next()) {
        (Some(Ok(Instruction::PushBytes(p))), None) => p.as_bytes().to_vec(),
        (Some(Err(_)), _) => return Err("P2PK: the scriptSig push is not minimally encoded (MINIMALDATA)".into()),
        _ => return Err("P2PK: the scriptSig must be exactly one push, the signature".into()),
    };
    let (ty, der) = sig.split_last().ok_or("P2PK: empty signature")?;
    if *ty != EcdsaSighashType::All as u8 {
        return Err(format!("P2PK: sighash type 0x{ty:02x}, BIP-322 requires SIGHASH_ALL (0x01)"));
    }
    let s = secp256k1::ecdsa::Signature::from_der(der).map_err(|_| "P2PK: the signature is not strictly DER-encoded (STRICTENC)")?;
    let mut low = s;
    low.normalize_s();
    if low != s {
        return Err("P2PK: high-S signature (BIP-322 requires LOW_S)".into());
    }
    secp256k1::Secp256k1::verification_only().verify_ecdsa(&sighash(&tx, script), &s, &pk)
        .map_err(|_| "P2PK: the signature does not verify".to_string())
}
