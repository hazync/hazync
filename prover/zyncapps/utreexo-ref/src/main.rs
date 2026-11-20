//! utreexo-ref: the Utreexo accumulator after each block, computed by Floresta's own code (docs/libzync/UTREEXO_ZAPP.md §7).
//!
//! Per block, from genesis + 1 (Floresta starts from an empty Stump at height 0; the genesis coinbase is not a leaf):
//!   adds  = floresta_chain::proof_util::get_block_adds (the code Floresta's update_acc calls)
//!   dels  = every input that does not spend an output of the same block (process_proof's rule), as the leaf hash
//!           LeafData::_get_leaf_hashes of the coin it spends (the leaf data kept from its add)
//!   proof = rustreexo MemForest::prove(dels)
//!   acc   = floresta_chain Consensus::update_acc(acc, block, h, proof, dels)  (rustreexo Stump::modify)
//! and three cross-checks per block: our LeafData for each add hashes to get_block_adds' leaf; the MemForest's roots
//! equal the Stump's; at every --at height, the "zombie" fold (every leaf ever added, in order, a spent one as Empty,
//! folded with Stump::add's rule: no proofs, no deletions) equals the Stump. The BIP30 leaves of 91,722 and 91,812 must
//! hash to Floresta's UNSPENDABLE_BIP30_UTXO_* constants.
//!
//! usage: utreexo-ref json <block json dir> <stop> [--at h,h,...]       (prover/fetch_block_rpc.py's block_<h>.json)
//!        utreexo-ref hex  <file of one hex block per line, genesis first> <stop> [--at ...]
//! Prints `UTREEXO_REF height=<h> block=<hash> leaves=<n> roots=<hex|empty,...>` at each --at height and at stop.
use bitcoin::consensus::deserialize;
use bitcoin::hashes::{sha512_256, Hash, HashEngine};
use bitcoin::{Block, OutPoint};
use floresta_chain::pruned_utreexo::consensus::{Consensus, UNSPENDABLE_BIP30_UTXO_91722, UNSPENDABLE_BIP30_UTXO_91812};
use floresta_chain::{proof_util, LeafData};
use rustreexo::mem_forest::MemForest;
use rustreexo::node_hash::{AccumulatorHash, BitcoinNodeHash};
use rustreexo::stump::Stump;
use std::collections::{HashMap, HashSet};

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex")).collect()
}

fn put_varint(v: &mut Vec<u8>, n: u64) {
    match n {
        0..=0xfc => v.push(n as u8),
        0xfd..=0xffff => {
            v.push(0xfd);
            v.extend_from_slice(&(n as u16).to_le_bytes());
        }
        _ => {
            v.push(0xfe);
            v.extend_from_slice(&(n as u32).to_le_bytes());
        }
    }
}

fn rev32(s: &str) -> Vec<u8> {
    let mut b = unhex(s);
    b.reverse();
    b
}

/// prover/fetch_block_rpc.py's block_<h>.json -> the serialised block (with witnesses).
fn json_block(dir: &str, h: u32) -> Block {
    let p = format!("{dir}/block_{h}.json");
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap_or_else(|e| panic!("{p}: {e}"))).unwrap();
    assert_eq!(v["height"].as_u64(), Some(h as u64), "{p}");
    let mut b = Vec::new();
    b.extend_from_slice(&(v["version"].as_i64().unwrap() as i32).to_le_bytes());
    b.extend_from_slice(&rev32(v["prev"].as_str().unwrap()));
    b.extend_from_slice(&rev32(v["merkle"].as_str().unwrap()));
    b.extend_from_slice(&(v["time"].as_u64().unwrap() as u32).to_le_bytes());
    b.extend_from_slice(&(v["bits"].as_u64().unwrap() as u32).to_le_bytes());
    b.extend_from_slice(&(v["nonce"].as_u64().unwrap() as u32).to_le_bytes());
    let txs = v["txs"].as_array().unwrap();
    put_varint(&mut b, 1 + txs.len() as u64);
    b.extend_from_slice(&unhex(v["coinbase_hex"].as_str().unwrap()));
    for t in txs {
        b.extend_from_slice(&unhex(t["raw"].as_str().unwrap()));
    }
    deserialize(&b).unwrap_or_else(|e| panic!("{p}: {e}"))
}

/// Stump::add's rule, written out (rustreexo src/stump/mod.rs `add`): roots largest tree first, Empty as identity.
fn zombie_add(roots: &mut Vec<BitcoinNodeHash>, leaves: &mut u64, leaf: BitcoinNodeHash) {
    let mut node = leaf;
    let mut h = 0;
    while (*leaves >> h) & 1 == 1 {
        let r = roots.pop().unwrap();
        node = match (r.is_empty(), node.is_empty()) {
            (true, true) => BitcoinNodeHash::Empty,
            (true, false) => node,
            (false, true) => r,
            (false, false) => {
                // parent = SHA-512/256(left ‖ right), computed here rather than by AccumulatorHash::parent_hash
                let mut e = sha512_256::Hash::engine();
                e.input(&r[..]);
                e.input(&node[..]);
                BitcoinNodeHash::from(sha512_256::Hash::from_engine(e).to_byte_array())
            }
        };
        h += 1;
    }
    roots.push(node);
    *leaves += 1;
}

fn show(r: &[BitcoinNodeHash]) -> String {
    r.iter().map(|x| if x.is_empty() { "empty".to_string() } else { x.to_string() }).collect::<Vec<_>>().join(",")
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 4 {
        eprintln!("usage: utreexo-ref json <dir> <stop> [--at h,...] | hex <file> <stop> [--at h,...]");
        std::process::exit(2);
    }
    let stop: u32 = a[3].parse().expect("stop");
    let mut at: HashSet<u32> = a
        .iter()
        .position(|x| x == "--at")
        .map(|i| a[i + 1].split(',').map(|x| x.parse().expect("--at")).collect())
        .unwrap_or_default();
    at.insert(stop);
    let hex_lines: Vec<String> = if a[1] == "hex" {
        std::fs::read_to_string(&a[2]).unwrap().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
    } else {
        Vec::new()
    };
    let get = |h: u32| -> Block {
        match a[1].as_str() {
            "json" => json_block(&a[2], h),
            "hex" => deserialize(&unhex(&hex_lines[h as usize])).unwrap(),
            x => panic!("unknown input kind {x}"),
        }
    };
    let mut stump = Stump::new();
    let mut forest = MemForest::new();
    let mut live: HashMap<OutPoint, LeafData> = HashMap::new();
    let mut log: Vec<BitcoinNodeHash> = Vec::new(); // every leaf ever added, in order
    let mut pos_of: HashMap<BitcoinNodeHash, usize> = HashMap::new();
    let mut alive: Vec<bool> = Vec::new();
    let (mut n_adds, mut n_dels) = (0u64, 0u64);
    let t0 = std::time::Instant::now();
    for h in 1..=stop {
        let block = get(h);
        let hash = block.block_hash();
        let adds = proof_util::get_block_adds(&block, h, hash);
        // dels: process_proof's rule (an input spending an output of a transaction in this block is skipped)
        let in_block: HashMap<_, usize> = block.txdata.iter().map(|t| (t.compute_txid(), t.output.len())).collect();
        let mut del_sha = Vec::new();
        for tx in block.txdata.iter().skip(1) {
            for i in &tx.input {
                let p = i.previous_output;
                if in_block.get(&p.txid).is_some_and(|&n| (p.vout as usize) < n) {
                    continue;
                }
                let ld = live.remove(&p).unwrap_or_else(|| panic!("block {h}: spends {p}, not a live leaf"));
                del_sha.push(ld._get_leaf_hashes());
            }
        }
        let dels: Vec<BitcoinNodeHash> = del_sha.iter().map(|x| BitcoinNodeHash::from(x.to_byte_array())).collect();
        // our leaf data for each add, in get_block_adds' order, must hash to its leaf
        let spent: HashSet<OutPoint> = block.txdata.iter().flat_map(|t| t.input.iter().map(|i| i.previous_output)).collect();
        let mut k = 0;
        for tx in &block.txdata {
            let txid = tx.compute_txid();
            let cb = tx.is_coinbase();
            for (v, out) in tx.output.iter().enumerate() {
                let op = OutPoint::new(txid, v as u32);
                if Consensus::is_unspendable(&out.script_pubkey) || spent.contains(&op) {
                    continue;
                }
                let ld = LeafData { block_hash: hash, prevout: op, header_code: (h << 1) | cb as u32, utxo: out.clone() };
                assert_eq!(BitcoinNodeHash::from(ld._get_leaf_hashes().to_byte_array()), adds[k], "block {h}: leaf {k}");
                k += 1;
                if let Some(old) = live.insert(op, ld) {
                    // BIP30: 91,842 / 91,880 recreate the coinbases of 91,812 / 91,722; the old leaves stay for ever
                    let oh = old._get_leaf_hashes().to_byte_array();
                    assert!(oh == UNSPENDABLE_BIP30_UTXO_91722 || oh == UNSPENDABLE_BIP30_UTXO_91812, "block {h}: {op} overwritten");
                    println!("UTREEXO_REF_BIP30 height={h} outpoint={op} keeps the leaf of height {} for ever", old.header_code >> 1);
                }
            }
        }
        assert_eq!(k, adds.len(), "block {h}: add count");
        let proof = forest.prove(&dels).unwrap_or_else(|e| panic!("block {h}: prove: {e}"));
        stump = Consensus::update_acc(&stump, &block, h, proof, del_sha).unwrap_or_else(|e| panic!("block {h}: update_acc: {e:?}"));
        forest.modify(&adds, &dels).unwrap_or_else(|e| panic!("block {h}: MemForest: {e}"));
        let fr: Vec<BitcoinNodeHash> = forest.get_roots().iter().map(|r| r.get_data()).collect();
        assert_eq!(fr, stump.roots, "block {h}: MemForest roots != Stump roots");
        for d in &dels {
            alive[pos_of[d]] = false;
        }
        for x in &adds {
            pos_of.insert(*x, log.len());
            log.push(*x);
            alive.push(true);
        }
        n_adds += adds.len() as u64;
        n_dels += dels.len() as u64;
        if at.contains(&h) {
            let (mut zr, mut zn) = (Vec::new(), 0u64);
            for (x, &l) in log.iter().zip(&alive) {
                zombie_add(&mut zr, &mut zn, if l { *x } else { BitcoinNodeHash::Empty });
            }
            assert_eq!((zn, &zr), (stump.leaves, &stump.roots), "block {h}: the zombie fold != the Stump");
            println!(
                "UTREEXO_REF height={h} block={hash} leaves={} live={} adds={n_adds} dels={n_dels} roots={} (zombie fold equal; {:.1} s)",
                stump.leaves,
                alive.iter().filter(|x| **x).count(),
                show(&stump.roots),
                t0.elapsed().as_secs_f64()
            );
        }
    }
}
