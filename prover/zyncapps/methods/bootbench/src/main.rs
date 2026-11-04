//! E1 bootstrap measurement (absence/README.md): the per-unit costs of building both keyed trees ONCE from the coin log
//! at a height, without sorting in the zkVM. Execute mode only. Three passes over synthetic data:
//!   A. a spent coin-log leaf: folded into the coin-log forest (the root is checked against ChainState in a real proof);
//!   B. a live coin: its coin leaf from the preimage (facts::coin_leaf's layout), folded; its outpoint and script keys;
//!      each key's element hash added to that tree's multiset sum;
//!   C. one sorted tree entry (host-sorted): strictly after the previous, leaf live(key, next), folded into the tree's
//!      forest, element hash added to the multiset sum (must equal B's: the sorted list IS the live set).
//! Input (read_slice): [nA, nB, nC], nA x 8 words, nB x (txid 8, vout, value lo, value hi, height, flags, mtp, spk 22 bytes
//! in 6 words), nC x 16 words (sorted keys).
#![no_main]
extern crate alloc;
use alloc::vec::Vec;
use risc0_zkvm::guest::env;
risc0_zkvm::guest::entry!(main);

#[path = "../../../../../libzync/src/coinlog.rs"]
#[allow(dead_code)]
mod coinlog;
#[path = "../../../../../libzync/src/itree.rs"]
#[allow(dead_code)]
mod itree;
use coinlog::{parent_w, W};

struct Forest(Vec<Option<W>>);
impl Forest {
    fn push(&mut self, mut node: W) {
        let mut l = 0usize;
        while l < self.0.len() && self.0[l].is_some() {
            let left = self.0[l].take().unwrap();
            let mut up = [0u32; 8];
            parent_w(l as u32 + 1, &left, &node, &mut up);
            node = up;
            l += 1;
        }
        if l == self.0.len() { self.0.push(None); }
        self.0[l] = Some(node);
    }
}

fn add(acc: &mut W, e: &W) {
    let mut c = 0u64;
    for i in 0..8 { let s = acc[i] as u64 + e[i] as u64 + c; acc[i] = s as u32; c = s >> 32; }
}

fn elem(k: &itree::Key) -> W {
    let mut a = [0u32; 8];
    let mut b = [0u32; 8];
    for i in 0..8 {
        a[i] = u32::from_be_bytes(k[4 * i..4 * i + 4].try_into().unwrap());
        b[i] = u32::from_be_bytes(k[32 + 4 * i..36 + 4 * i].try_into().unwrap());
    }
    let mut o = [0u32; 8];
    parent_w(61, &a, &b, &mut o);
    o
}

fn main() {
    use sha2::{Digest, Sha256};
    let mut hdr = [0u32; 3];
    env::read_slice(&mut hdr);
    let (na, nb, nc) = (hdr[0] as usize, hdr[1] as usize, hdr[2] as usize);
    let mut log = Forest(Vec::new());
    // A
    let mut wa = alloc::vec![0u32; 8 * na];
    env::read_slice(&mut wa);
    let c0 = env::cycle_count();
    for c in wa.chunks_exact(8) { log.push(c.try_into().unwrap()); }
    let c1 = env::cycle_count();
    // B
    let mut wb = alloc::vec![0u32; 21 * nb];
    env::read_slice(&mut wb);
    let c2 = env::cycle_count();
    let (mut so, mut ss) = ([0u32; 8], [0u32; 8]);
    for c in wb.chunks_exact(21) {
        let txid: [u8; 32] = core::array::from_fn(|i| (c[i / 4] >> (8 * (i % 4))) as u8);
        let spk: [u8; 22] = core::array::from_fn(|i| (c[15 + i / 4] >> (8 * (i % 4))) as u8);
        let mut h = Sha256::new();
        h.update([0u8]); h.update(txid); h.update(c[8].to_le_bytes());
        h.update(((c[10] as u64) << 32 | c[9] as u64).to_le_bytes());
        h.update(22u32.to_le_bytes()); h.update(spk);
        h.update(c[11].to_le_bytes()); h.update([c[12] as u8]); h.update(c[13].to_le_bytes());
        let leaf: [u8; 32] = h.finalize().into();
        log.push(coinlog::w_of(&leaf));
        let mut h = Sha256::new(); h.update(txid); h.update(c[8].to_le_bytes());
        let ok: [u8; 32] = h.finalize().into();
        let sh: [u8; 32] = Sha256::digest(spk).into();
        let mut ko = [0u8; 64]; ko[..32].copy_from_slice(&ok);
        let mut ks = [0u8; 64]; ks[..32].copy_from_slice(&sh); ks[32..].copy_from_slice(&ok);
        add(&mut so, &elem(&ko));
        add(&mut ss, &elem(&ks));
    }
    let c3 = env::cycle_count();
    // C
    let mut wc = alloc::vec![0u32; 16 * nc];
    env::read_slice(&mut wc);
    let c4 = env::cycle_count();
    let keys: Vec<itree::Key> = wc.chunks_exact(16).map(|c| {
        let mut k = [0u8; 64];
        for (j, x) in c.iter().enumerate() { k[4 * j..4 * j + 4].copy_from_slice(&x.to_be_bytes()); }
        k
    }).collect();
    let mut tree = Forest(Vec::new());
    let mut sc = [0u32; 8];
    for i in 0..keys.len() {
        let next = if i + 1 < keys.len() { keys[i + 1] } else { itree::MAX_KEY };
        assert!(next > keys[i], "not strictly sorted");
        tree.push(itree::live(&keys[i], &next));
        add(&mut sc, &elem(&keys[i]));
    }
    let c5 = env::cycle_count();
    env::log(&alloc::format!("BOOTBENCH A {na} {} per {:.0} | B {nb} {} per {:.0} | C {nc} {} per {:.0}",
        c1 - c0, (c1 - c0) as f64 / na.max(1) as f64, c3 - c2, (c3 - c2) as f64 / nb.max(1) as f64,
        c5 - c4, (c5 - c4) as f64 / nc.max(1) as f64));
    for r in log.0.iter().chain(tree.0.iter()) { env::commit_slice(&r.unwrap_or([0; 8])); }
    env::commit_slice(&so); env::commit_slice(&ss); env::commit_slice(&sc);
}
