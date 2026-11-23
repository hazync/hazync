//! trieprep <getblock-3.json> <height> <addrs> <out.bin>: guest mode 17's input for one block, built the way Hazync's
//! measurement path does it (prover/host/src/lz.rs cl_synth_around + trie.rs trie_env): marks = every earlier coin the
//! block spends (key = coin_key, spender = (height, tx position, vin)), inserts = every coin it creates (Core-kept
//! scripts only; spent in the same block -> its spender), the old trie MODELLED around those keys
//! (trie_model::synth, sub 2, counts, seed 0x5eed+1). Also applies the update natively and prints the journal.
use libzync::{coinlog, trie_fact::coin_key, trie_model, ttree};
use std::collections::HashMap;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let j: serde_json::Value = serde_json::from_slice(&std::fs::read(&a[1]).unwrap()).unwrap();
    let height: u32 = a[2].parse().unwrap(); let addrs: u64 = a[3].parse().unwrap();
    let txs = j["tx"].as_array().unwrap();
    let rev = |h: &str| -> [u8; 32] { let mut b: [u8; 32] = hex::decode(h).unwrap().try_into().unwrap(); b.reverse(); b };
    // same-block spends: outpoint -> (tx position, vin)
    let mut spent_here: HashMap<([u8; 32], u32), (u32, u32)> = HashMap::new();
    let txids: Vec<[u8; 32]> = txs.iter().map(|t| rev(t["txid"].as_str().unwrap())).collect();
    let pos: HashMap<[u8; 32], u32> = txids.iter().enumerate().map(|(i, t)| (*t, i as u32)).collect();
    let mut marks: Vec<(ttree::Key, coinlog::W)> = vec![];
    for (ti, t) in txs.iter().enumerate().skip(1) {
        for (vi, v) in t["vin"].as_array().unwrap().iter().enumerate() {
            let pt = rev(v["txid"].as_str().unwrap()); let pv = v["vout"].as_u64().unwrap() as u32;
            let sp = coinlog::w_of(&coinlog::spender(height, ti as u32, vi as u32));
            if pos.contains_key(&pt) { spent_here.insert((pt, pv), (ti as u32, vi as u32)); continue; }
            let spk = hex::decode(v["prevout"]["scriptPubKey"]["hex"].as_str().unwrap()).unwrap();
            marks.push((coin_key(&spk, &pt, pv), sp));
        }
    }
    let mut ins: Vec<(ttree::Key, Option<coinlog::W>, u64)> = vec![];
    for (ti, t) in txs.iter().enumerate() {
        for (oi, o) in t["vout"].as_array().unwrap().iter().enumerate() {
            let spk = hex::decode(o["scriptPubKey"]["hex"].as_str().unwrap()).unwrap();
            if spk.first() == Some(&0x6a) || spk.len() > 10_000 { continue; }   // Core does not keep these
            let sats = (o["value"].as_f64().unwrap() * 1e8).round() as u64;
            let sp = spent_here.get(&(txids[ti], oi as u32)).map(|&(t2, v2)| coinlog::w_of(&coinlog::spender(height, t2, v2)));
            ins.push((coin_key(&spk, &txids[ti], oi as u32), sp, sats));
        }
    }
    // fresh-trie: TRIE_BUCKET=b keeps only the updates whose script hash starts with nibble b (one of 16 parts)
    if let Ok(b) = std::env::var("TRIE_BUCKET") {
        let b: u8 = b.parse().unwrap();
        marks.retain(|x| x.0[0] >> 4 == b);
        ins.retain(|x| x.0[0] >> 4 == b);
    }
    let rem: Vec<[u8; 64]> = marks.iter().map(|x| x.0).collect();
    let ink: Vec<[u8; 64]> = ins.iter().map(|x| x.0).collect();
    let (root, tr) = trie_model::synth(&rem, &ink, addrs, 2, true, 0x5eed + 1);
    let wit = trie_model::encode(true, &root, &tr);
    // trie_env, verbatim
    let ks: Vec<(coinlog::W, coinlog::W)> = marks.iter().map(|x| ttree::split(&x.0)).chain(ins.iter().map(|x| ttree::split(&x.0))).collect();
    let mut perm: Vec<u32> = (0..ks.len() as u32).collect();
    perm.sort_by(|&a, &c| ks[a as usize].cmp(&ks[c as usize]));
    let kw = |k: &ttree::Key| -> Vec<u32> { k.chunks_exact(4).map(|c| u32::from_be_bytes(c.try_into().unwrap())).collect() };
    let mut out: Vec<u32> = vec![wit.len() as u32, marks.len() as u32, ins.len() as u32, perm.len() as u32];
    out.extend(&wit);
    for (k, sp) in &marks { out.extend(kw(k)); out.extend_from_slice(sp); }
    for (k, sp, v) in &ins { out.extend(kw(k)); out.push(sp.is_some() as u32); out.extend_from_slice(&sp.unwrap_or([0; 8])); out.push(*v as u32); out.push((*v >> 32) as u32); }
    out.extend(&perm);
    let bytes: Vec<u8> = out.iter().flat_map(|w| w.to_le_bytes()).collect();
    std::fs::write(&a[4], &bytes).unwrap();
    let old: coinlog::W = wit[1..9].try_into().unwrap();
    let t0 = std::time::Instant::now();
    let new = ttree::apply_stream_perm(&wit, &marks, &ins, Some(&perm)).expect("native trie update");
    let jn = ttree::trie_journal(&old, &new, &ttree::list_digest(&marks, &ins));
    println!("marks={} inserts={} same_block_spends={} witness_words={} native_apply={:.3}s journal_sha256={}", marks.len(), ins.len(),
        spent_here.len(), wit.len(), t0.elapsed().as_secs_f64(),
        { use sha2::{Digest, Sha256}; hex::encode(Sha256::digest(jn.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>())) });
}
