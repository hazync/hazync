//! Extract real signature checks + tx bytes from one block (getblock verbosity 3 JSON).
//! Every extracted signature is verified natively with libsecp256k1 (as Core does:
//! lax DER, normalised low-S); anything that fails is NOT exported and is counted.
//!
//! Output (little-endian) bench input file:
//!   u32 n_ecdsa, then n_ecdsa x [msg32 | pubkey33 | sig64]
//!   u32 n_schnorr, then n_schnorr x [msg32 | xonly32 | sig64]
//!   u32 n_tx, then n_tx x [u32 len | stripped tx bytes]
//!   merkle_root32 (internal byte order, from the header)
use bitcoin::consensus::deserialize;
use bitcoin::hashes::Hash;
use bitcoin::secp256k1::{ecdsa, schnorr, Message, PublicKey, Secp256k1, XOnlyPublicKey};
use bitcoin::sighash::{Prevouts, SighashCache, TapSighashType};
use bitcoin::{Amount, Script, ScriptBuf, Transaction, TxOut};
use std::collections::BTreeMap;
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let j: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[1]).unwrap()).unwrap();
    let out_path = &args[2];
    let secp = Secp256k1::verification_only();
    let mut ec: Vec<[u8; 129]> = vec![];
    let mut sc: Vec<[u8; 128]> = vec![];
    let mut txs: Vec<Vec<u8>> = vec![];
    let mut skipped: BTreeMap<String, u64> = BTreeMap::new();
    let mut bad: BTreeMap<String, u64> = BTreeMap::new();
    for t in j["tx"].as_array().unwrap() {
        let tx: Transaction = deserialize(&hex::decode(t["hex"].as_str().unwrap()).unwrap()).unwrap();
        let mut stripped = tx.clone();
        for i in stripped.input.iter_mut() { i.witness.clear(); }
        txs.push(bitcoin::consensus::serialize(&stripped));
        if tx.is_coinbase() { continue; }
        let vins = t["vin"].as_array().unwrap();
        let prevouts: Vec<TxOut> = vins.iter().map(|v| {
            let p = &v["prevout"];
            TxOut {
                value: Amount::from_sat((p["value"].as_f64().unwrap() * 1e8).round() as u64),
                script_pubkey: ScriptBuf::from_bytes(hex::decode(p["scriptPubKey"]["hex"].as_str().unwrap()).unwrap()),
            }
        }).collect();
        let mut cache = SighashCache::new(&tx);
        for (i, v) in vins.iter().enumerate() {
            let ty = v["prevout"]["scriptPubKey"]["type"].as_str().unwrap().to_string();
            let spk = &prevouts[i].script_pubkey;
            let input = &tx.input[i];
            let wit: Vec<&[u8]> = input.witness.iter().collect();
            // (der_with_hashtype, pubkey, sighash) for ECDSA spends we support
            let mut ecd: Option<(Vec<u8>, Vec<u8>, [u8; 32])> = None;
            match ty.as_str() {
                "witness_v0_keyhash" if wit.len() == 2 => {
                    let ht = *wit[0].last().unwrap_or(&1) as u32;
                    let h = cache.p2wpkh_signature_hash(i, spk, prevouts[i].value, bitcoin::sighash::EcdsaSighashType::from_consensus(ht));
                    if let Ok(h) = h { ecd = Some((wit[0].to_vec(), wit[1].to_vec(), h.to_byte_array())); }
                }
                "scripthash" => {
                    let pushes: Vec<Vec<u8>> = input.script_sig.instructions().filter_map(|x| match x { Ok(bitcoin::script::Instruction::PushBytes(b)) => Some(b.as_bytes().to_vec()), _ => None }).collect();
                    let redeem = pushes.last().cloned().unwrap_or_default();
                    if redeem.len() == 22 && redeem[0] == 0 && redeem[1] == 20 && wit.len() == 2 {
                        let ht = *wit[0].last().unwrap_or(&1) as u32;
                        let rs = Script::from_bytes(&redeem);
                        let h = cache.p2wpkh_signature_hash(i, rs, prevouts[i].value, bitcoin::sighash::EcdsaSighashType::from_consensus(ht));
                        if let Ok(h) = h { ecd = Some((wit[0].to_vec(), wit[1].to_vec(), h.to_byte_array())); }
                    } else { *skipped.entry("p2sh_other".into()).or_default() += 1; }
                }
                "pubkeyhash" => {
                    let pushes: Vec<Vec<u8>> = input.script_sig.instructions().filter_map(|x| match x { Ok(bitcoin::script::Instruction::PushBytes(b)) => Some(b.as_bytes().to_vec()), _ => None }).collect();
                    if pushes.len() == 2 {
                        let ht = *pushes[0].last().unwrap_or(&1) as u32;
                        let h = cache.legacy_signature_hash(i, spk, ht);
                        if let Ok(h) = h { ecd = Some((pushes[0].clone(), pushes[1].clone(), h.to_byte_array())); }
                    }
                }
                "witness_v1_taproot" => {
                    let mut items = wit.clone();
                    if items.len() >= 2 && items.last().unwrap().first() == Some(&0x50) { items.pop(); }
                    if items.len() == 1 && (items[0].len() == 64 || items[0].len() == 65) {
                        let s = items[0];
                        let ht = if s.len() == 65 { TapSighashType::from_consensus_u8(s[64]).unwrap_or(TapSighashType::Default) } else { TapSighashType::Default };
                        if let Ok(h) = cache.taproot_key_spend_signature_hash(i, &Prevouts::All(&prevouts), ht) {
                            let xonly = XOnlyPublicKey::from_slice(&spk.as_bytes()[2..34]).unwrap();
                            let sig = schnorr::Signature::from_slice(&s[..64]).unwrap();
                            let msg = Message::from_digest(h.to_byte_array());
                            if secp.verify_schnorr(&sig, &msg, &xonly).is_ok() {
                                let mut r = [0u8; 128];
                                r[..32].copy_from_slice(&h.to_byte_array()); r[32..64].copy_from_slice(&xonly.serialize()); r[64..].copy_from_slice(&s[..64]);
                                sc.push(r);
                            } else { *bad.entry("schnorr".into()).or_default() += 1; }
                        }
                    } else { *skipped.entry("taproot_scriptpath".into()).or_default() += 1; }
                }
                other => { *skipped.entry(other.into()).or_default() += 1; }
            }
            if let Some((der, pk, h)) = ecd {
                let ok = (|| {
                    let mut sig = ecdsa::Signature::from_der_lax(&der[..der.len() - 1]).ok()?;
                    sig.normalize_s();
                    let pk = PublicKey::from_slice(&pk).ok()?;
                    secp.verify_ecdsa(&Message::from_digest(h), &sig, &pk).ok()?;
                    let mut r = [0u8; 129];
                    r[..32].copy_from_slice(&h); r[32..65].copy_from_slice(&pk.serialize()); r[65..].copy_from_slice(&sig.serialize_compact());
                    Some(r)
                })();
                match ok { Some(r) => ec.push(r), None => *bad.entry(format!("ecdsa_{ty}")).or_default() += 1 }
            }
        }
    }
    let mut root = hex::decode(j["merkleroot"].as_str().unwrap()).unwrap();
    root.reverse();
    let mut f = std::fs::File::create(out_path).unwrap();
    f.write_all(&(ec.len() as u32).to_le_bytes()).unwrap();
    for r in &ec { f.write_all(r).unwrap(); }
    f.write_all(&(sc.len() as u32).to_le_bytes()).unwrap();
    for r in &sc { f.write_all(r).unwrap(); }
    f.write_all(&(txs.len() as u32).to_le_bytes()).unwrap();
    for t in &txs { f.write_all(&(t.len() as u32).to_le_bytes()).unwrap(); f.write_all(t).unwrap(); }
    f.write_all(&root).unwrap();
    println!("height={} ecdsa_ok={} schnorr_ok={} txs={} stripped_bytes={}", j["height"], ec.len(), sc.len(), txs.len(), txs.iter().map(|t| t.len()).sum::<usize>());
    println!("skipped={skipped:?}");
    println!("failed_native_verify={bad:?}");
}
