//! BIP-322 "simple" verification for P2WPKH and P2TR (key path, SIGHASH_DEFAULT / ALL), hand-written for the design-B
//! PROTOTYPE guest only (docs/libzync/STATEMENT_ZAPP.md §3). no_std; SHA-256 by `sha2`, RIPEMD-160 by `ripemd`, curve by
//! `k256`. The shipped design (A) does not use this file: it verifies with the rust-bitcoin `bip322` crate. This file is
//! cross-checked against that crate in the host (`statement-bench selftest`): every signature the crate makes verifies
//! here, and a changed message, key or signature does not.
//!
//! to_spend / to_sign are BIP-322's: to_spend spends (0^32, 0xffffffff) with scriptSig OP_0 PUSH32 tagged_hash(message)
//! and pays 0 to the script; to_sign (version 0, locktime 0) spends to_spend:0 (sequence 0) and pays 0 to OP_RETURN.

use alloc::vec::Vec;
use sha2::{Digest, Sha256};

pub fn sha256(b: &[u8]) -> [u8; 32] { Sha256::digest(b).into() }
pub fn dsha256(b: &[u8]) -> [u8; 32] { sha256(&sha256(b)) }

pub fn tagged(tag: &str, parts: &[&[u8]]) -> [u8; 32] {
    let t = sha256(tag.as_bytes());
    let mut h = Sha256::new();
    h.update(t);
    h.update(t);
    for p in parts { h.update(p); }
    h.finalize().into()
}

fn compact(n: usize, out: &mut Vec<u8>) {
    match n {
        0..=0xfc => out.push(n as u8),
        0xfd..=0xffff => { out.push(0xfd); out.extend_from_slice(&(n as u16).to_le_bytes()) }
        _ => { out.push(0xfe); out.extend_from_slice(&(n as u32).to_le_bytes()) }
    }
}

/// The txid (internal order) of BIP-322's to_spend for `script` and `message`.
pub fn to_spend_txid(script: &[u8], message: &[u8]) -> [u8; 32] {
    let mh = tagged("BIP0322-signed-message", &[message]);
    let mut t = Vec::with_capacity(100 + script.len());
    t.extend_from_slice(&0u32.to_le_bytes()); // version 0
    t.push(1);
    t.extend_from_slice(&[0u8; 32]);
    t.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
    t.push(34);
    t.push(0x00); // OP_0
    t.push(0x20); // PUSH32
    t.extend_from_slice(&mh);
    t.extend_from_slice(&0u32.to_le_bytes()); // sequence 0
    t.push(1);
    t.extend_from_slice(&0u64.to_le_bytes());
    compact(script.len(), &mut t);
    t.extend_from_slice(script);
    t.extend_from_slice(&0u32.to_le_bytes()); // locktime 0
    dsha256(&t)
}

/// to_sign's single output: 0 sat to OP_RETURN, serialised.
const OUTPUT: [u8; 10] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 0x6a];

#[derive(Debug, PartialEq, Eq)]
pub enum Bip322Err { Script, Witness, Key, Signature }

/// BIP-322 simple: `witness` (the stack, decoded) proves `script` signed `message`. P2WPKH and P2TR key path only.
pub fn verify_simple(script: &[u8], message: &[u8], witness: &[Vec<u8>]) -> Result<(), Bip322Err> {
    let txid = to_spend_txid(script, message);
    let mut outpoint = [0u8; 36];
    outpoint[..32].copy_from_slice(&txid);
    let seq = 0u32.to_le_bytes();
    if script.len() == 22 && script[0] == 0x00 && script[1] == 0x14 {
        // P2WPKH: [DER sig || 0x01, compressed pubkey], BIP-143 digest, SIGHASH_ALL only
        let [sig, pk] = witness else { return Err(Bip322Err::Witness) };
        if pk.len() != 33 { return Err(Bip322Err::Key) }
        let h160: [u8; 20] = { use ripemd::Ripemd160; Ripemd160::digest(sha256(pk)).into() };
        if h160[..] != script[2..] { return Err(Bip322Err::Key) }
        let (ty, der) = sig.split_last().ok_or(Bip322Err::Signature)?;
        if *ty != 0x01 { return Err(Bip322Err::Signature) }
        let mut code = Vec::with_capacity(26);
        code.extend_from_slice(&[0x19, 0x76, 0xa9, 0x14]);
        code.extend_from_slice(&script[2..]);
        code.extend_from_slice(&[0x88, 0xac]);
        let mut p = Vec::with_capacity(160);
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(&dsha256(&outpoint));
        p.extend_from_slice(&dsha256(&seq));
        p.extend_from_slice(&outpoint);
        p.extend_from_slice(&code);
        p.extend_from_slice(&0u64.to_le_bytes());
        p.extend_from_slice(&seq);
        p.extend_from_slice(&dsha256(&OUTPUT));
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(&1u32.to_le_bytes());
        let digest = dsha256(&p);
        use k256::ecdsa::{signature::hazmat::PrehashVerifier, Signature, VerifyingKey};
        let s = Signature::from_der(der).map_err(|_| Bip322Err::Signature)?;
        if s.normalize_s().is_some() { return Err(Bip322Err::Signature) } // libsecp refuses high S
        let vk = VerifyingKey::from_sec1_bytes(pk).map_err(|_| Bip322Err::Key)?;
        vk.verify_prehash(&digest, &s).map_err(|_| Bip322Err::Signature)
    } else if script.len() == 34 && script[0] == 0x51 && script[1] == 0x20 {
        // P2TR key path: [64-byte sig] (DEFAULT) or [65-byte sig || 0x01] (ALL), BIP-341 digest
        let [sig] = witness else { return Err(Bip322Err::Witness) };
        let ty = match sig.len() { 64 => 0u8, 65 if sig[64] == 0x01 => 1, _ => return Err(Bip322Err::Signature) };
        let mut spks = Vec::with_capacity(35);
        compact(script.len(), &mut spks);
        spks.extend_from_slice(script);
        let digest = tagged("TapSighash", &[
            &[0u8], &[ty], &0u32.to_le_bytes(), &0u32.to_le_bytes(),
            &sha256(&outpoint), &sha256(&0u64.to_le_bytes()), &sha256(&spks), &sha256(&seq), &sha256(&OUTPUT),
            &[0u8], &0u32.to_le_bytes(),
        ]);
        let vk = k256::schnorr::VerifyingKey::from_bytes(&script[2..]).map_err(|_| Bip322Err::Key)?;
        let s = k256::schnorr::Signature::try_from(&sig[..64]).map_err(|_| Bip322Err::Signature)?;
        vk.verify_raw(&digest, &s).map_err(|_| Bip322Err::Signature)
    } else {
        Err(Bip322Err::Script)
    }
}

/// A consensus-encoded witness (as BIP-322 simple signatures carry it, base64-decoded) as its stack.
pub fn decode_witness(b: &[u8]) -> Option<Vec<Vec<u8>>> {
    fn rd(b: &[u8], i: &mut usize) -> Option<usize> {
        let x = *b.get(*i)?; *i += 1;
        Some(match x {
            0xfd => { let v = u16::from_le_bytes(b.get(*i..*i + 2)?.try_into().ok()?) as usize; *i += 2; v }
            0xfe => { let v = u32::from_le_bytes(b.get(*i..*i + 4)?.try_into().ok()?) as usize; *i += 4; v }
            0xff => return None,
            n => n as usize,
        })
    }
    let mut i = 0;
    let n = rd(b, &mut i)?;
    if n > 16 { return None }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let l = rd(b, &mut i)?;
        out.push(b.get(i..i + l)?.to_vec());
        i += l;
    }
    (i == b.len()).then_some(out)
}
