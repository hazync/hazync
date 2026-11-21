//! EXPERIMENTAL Trusted State snapshot ingest (docs/libzync/TRUSTED_STATE.md §6). MEASUREMENT ONLY: its own image,
//! never a core mode. It builds, inside the zkVM, the anchor `ts-anchor build` builds natively from a Core
//! `dumptxoutset` snapshot at H: Core's hash_serialized_3, the coin leaves (creation MTP from the headers, the
//! BIP30-tracked bit from a list), the live-coin ECMH (the core guest's own swiftsync.rs, host-hinted), the coin log's
//! roots (libzync coinlog.rs), the counting trie's root (libzync ttree.rs, every coin inserted into the empty trie), the
//! BIP30 tree's root (coinbase-smt roots.rs) and the block forest. The host checks every committed value against
//! ts-anchor's anchor; each phase's cycles are logged (`TSI <phase> <cycles>`).
//!
//! What it does NOT do, by design of the measurement: cum_work (one 256-bit division per header; the header-chain
//! zAPP already proves chainwork), and Core's loader's full strictness (zapp-assume's reader is the reference).
#![no_main]
extern crate alloc;
use alloc::vec::Vec;
use risc0_zkvm::guest::env;
use sha2::{Digest, Sha256};
risc0_zkvm::guest::entry!(main);

// The core guest's ECMH and point arithmetic, unchanged (#[path], as the core guest compiles them).
#[path = "../../../../../../prover/methods/guest/src/batch_ecdsa.rs"]
#[allow(dead_code)]
mod batch_ecdsa;
#[path = "../../../../../../prover/methods/guest/src/swiftsync.rs"]
#[allow(dead_code)]
mod swiftsync;
#[path = "../../../../../../libzync/src/coinlog.rs"]
#[allow(dead_code)]
mod coinlog;
#[path = "../../../../../../libzync/src/ttree.rs"]
#[allow(dead_code)]
mod ttree;
#[path = "../../../tsi.rs"]
#[allow(dead_code)]
mod tsi;
#[path = "../../../../../../coinbase-smt/src/roots.rs"]
#[allow(dead_code)]
mod smt;

use coinlog::W;
use risc0_crypto::curves::secp256k1::Fq;
use risc0_crypto::BigInt;

const UNKNOWN_STATE_TAG: &[u8] = b"hazync trusted-state: the coin state after this block is not known (below the anchor)";
const REPEATABLE: u32 = u32::MAX - 1;

fn sha(b: &[u8]) -> [u8; 32] { Sha256::digest(b).into() }
fn dsha(b: &[u8]) -> [u8; 32] { sha(&sha(b)) }

fn read_u32() -> u32 { env::read() }
fn read_bytes(n: usize) -> Vec<u8> {
    let mut v = alloc::vec![0u8; n.div_ceil(4) * 4];
    if n > 0 { env::read_slice(&mut v); }
    v.truncate(n);
    v
}
fn read_words(n: usize) -> Vec<u32> {
    let mut v = alloc::vec![0u32; n];
    if n > 0 { env::read_slice(&mut v); }
    v
}

struct Src<'a> { b: &'a [u8], i: usize }
impl<'a> Src<'a> {
    fn take(&mut self, n: usize) -> &'a [u8] { let s = &self.b[self.i..self.i + n]; self.i += n; s }
    fn u8(&mut self) -> u8 { self.take(1)[0] }
    fn compact(&mut self) -> u64 {
        match self.u8() {
            c @ 0..=252 => c as u64,
            253 => u16::from_le_bytes(self.take(2).try_into().unwrap()) as u64,
            254 => u32::from_le_bytes(self.take(4).try_into().unwrap()) as u64,
            _ => u64::from_le_bytes(self.take(8).try_into().unwrap()),
        }
    }
    /// Core's VARINT (VarIntMode::DEFAULT)
    fn varint(&mut self) -> u64 {
        let mut n: u64 = 0;
        loop {
            let ch = self.u8();
            assert!(n <= u64::MAX >> 7, "VARINT too large");
            n = (n << 7) | (ch & 0x7f) as u64;
            if ch & 0x80 != 0 { n += 1; } else { return n; }
        }
    }
}

/// Core's DecompressAmount.
fn decompress_amount(mut x: u64) -> u64 {
    if x == 0 { return 0; }
    x -= 1;
    let mut e = x % 10;
    x /= 10;
    let mut n;
    if e < 9 { let d = (x % 9) + 1; x /= 9; n = x * 10 + d; } else { n = x + 1; }
    while e > 0 { n *= 10; e -= 1; }
    n
}

fn be_limbs(b: &[u8]) -> [u32; 8] {
    let mut l = [0u32; 8];
    for k in 0..8 { l[k] = u32::from_be_bytes(b[28 - 4 * k..32 - 4 * k].try_into().unwrap()); }
    l
}

fn median(v: &[u32]) -> u32 { let mut s = v.to_vec(); s.sort_unstable(); s[s.len() / 2] }

fn stump_add(roots: &mut Vec<Option<[u8; 32]>>, leaf: [u8; 32]) {
    let mut node = leaf;
    let mut h = 0;
    while h < roots.len() && roots[h].is_some() {
        let l = roots[h].take().unwrap();
        let mut p = [0u8; 65];
        p[0] = 1;
        p[1..33].copy_from_slice(&l);
        p[33..].copy_from_slice(&node);
        node = sha(&p);
        h += 1;
    }
    if h == roots.len() { roots.push(None); }
    roots[h] = Some(node);
}

fn main() {
    let c0 = env::cycle_count();
    let mut out: Vec<u32> = Vec::new();
    // ---- A: headers 0..H, linked by hash; times and the MTP table
    let h_tip = read_u32();
    let hdr = read_bytes(80 * (h_tip as usize + 1));
    let mut hashes: Vec<[u8; 32]> = Vec::with_capacity(h_tip as usize + 1);
    let mut times: Vec<u32> = Vec::with_capacity(h_tip as usize + 1);
    for (i, h) in hdr.chunks_exact(80).enumerate() {
        if i > 0 { assert!(h[4..36] == hashes[i - 1], "header {i} does not link"); }
        hashes.push(dsha(h));
        times.push(u32::from_le_bytes(h[68..72].try_into().unwrap()));
    }
    let tip_hash = hashes[h_tip as usize];
    let c1 = env::cycle_count();

    // ---- B: the snapshot, coin by coin
    let ns = read_u32() as usize;
    let snap = read_bytes(ns);
    let ne = read_u32() as usize;
    swiftsync::install(false, read_bytes(ne));
    let np = read_u32() as usize;
    let yh = read_bytes(32 * np);
    let nt = read_u32() as usize;
    let trk = read_bytes(36 * nt);
    let c1b = env::cycle_count();
    let mut s = Src { b: &snap, i: 0 };
    assert!(s.take(5) == b"utxo\xff" && u16::from_le_bytes(s.take(2).try_into().unwrap()) == 2, "not a v2 snapshot");
    let magic: [u8; 4] = s.take(4).try_into().unwrap();
    assert!(s.take(32) == tip_hash, "the snapshot's base block is not the headers' tip");
    let count = u64::from_le_bytes(s.take(8).try_into().unwrap());
    let mut h3 = Sha256::new();
    let mut mtp = alloc::vec![0u32; h_tip as usize + 1];
    let mut log: Vec<Option<W>> = Vec::new();
    let mut ins: Vec<(ttree::Key, Option<W>, u64)> = Vec::with_capacity(count as usize);
    let mut groups: Vec<([u8; 32], u32, bool, bool)> = Vec::new(); // txid, live coins, coinbase, tracked
    let (mut value, mut seen, mut yi, mut ti) = (0u64, 0u64, 0usize, 0usize);
    let mut last: Option<[u8; 32]> = None;
    let seven = Fq::from_u32(7);
    while seen < count {
        let txid: [u8; 32] = s.take(32).try_into().unwrap();
        assert!(last.map_or(true, |p| txid > p), "txids not ascending");
        last = Some(txid);
        let k = s.compact();
        assert!(k > 0 && k <= count - seen, "bad group");
        for _ in 0..k {
            let vout = s.compact() as u32;
            let code = s.varint();
            let (height, coinbase) = ((code >> 1) as u32, code & 1 == 1);
            assert!(height > 0 && height <= h_tip, "coin height outside 1..H");
            let v = decompress_amount(s.varint());
            let n = s.varint();
            let spk: Vec<u8> = match n {
                0 => [&[0x76u8, 0xa9, 20][..], s.take(20), &[0x88, 0xac]].concat(),
                1 => [&[0xa9u8, 20][..], s.take(20), &[0x87]].concat(),
                2 | 3 => [&[33u8, n as u8][..], s.take(32), &[0xac]].concat(),
                4 | 5 => {
                    // uncompressed P2PK: y from the host, checked on the curve and of the stored parity
                    let xb = s.take(32);
                    let yb = &yh[32 * yi..32 * yi + 32];
                    yi += 1;
                    let x = Fq::from_bigint(BigInt(be_limbs(xb))).expect("x >= p");
                    let y = Fq::from_bigint(BigInt(be_limbs(yb))).expect("y >= p");
                    assert!(&y * &y == &(&(&x * &x) * &x) + &seven, "P2PK hint: y not on the curve");
                    assert!((yb[31] & 1) as u64 == n - 4, "P2PK hint: wrong parity");
                    [&[0x41u8, 0x04][..], xb, yb, &[0xac]].concat()
                }
                _ => { let l = (n - 6) as usize; assert!(l <= 10_000, "script over 10,000 bytes"); s.take(l).to_vec() }
            };
            // tracked: the host's list, in the snapshot's order, all of it used
            let t = ti < nt && trk[36 * ti..36 * ti + 32] == txid && u32::from_le_bytes(trk[36 * ti + 32..36 * ti + 36].try_into().unwrap()) == vout;
            ti += t as usize;
            // Core's hash_serialized_3 (TxOutSer)
            h3.update(txid);
            h3.update(vout.to_le_bytes());
            h3.update(((height << 1) | coinbase as u32).to_le_bytes());
            h3.update(v.to_le_bytes());
            h3.update(match spk.len() { l @ 0..=252 => alloc::vec![l as u8], l => { let mut o = alloc::vec![253u8]; o.extend_from_slice(&(l as u16).to_le_bytes()); o } });
            h3.update(&spk);
            // the coin leaf (UTREEXO_FORMAT.md §2)
            let hu = height as usize;
            if mtp[hu] == 0 { mtp[hu] = median(&times[hu.saturating_sub(11)..hu]); }
            let mut lh = Sha256::new();
            lh.update([0u8]);
            lh.update(txid);
            lh.update(vout.to_le_bytes());
            lh.update(v.to_le_bytes());
            lh.update((spk.len() as u32).to_le_bytes());
            lh.update(&spk);
            lh.update(height.to_le_bytes());
            lh.update([coinbase as u8 | (t as u8) << 1]);
            lh.update(mtp[hu].to_le_bytes());
            let leaf: [u8; 32] = lh.finalize().into();
            swiftsync::add(&leaf);
            // the coin log: append (coinlog.rs parent, level-tagged)
            let mut node = coinlog::w_of(&leaf);
            let mut l = 0usize;
            while l < log.len() && log[l].is_some() {
                let mut o = [0u32; 8];
                coinlog::parent_w(l as u32 + 1, &log[l].take().unwrap(), &node, &mut o);
                node = o;
                l += 1;
            }
            if l == log.len() { log.push(None); }
            log[l] = Some(node);
            // the trie key: SHA-256(scriptPubKey) ‖ txid with vout XORed into its first 4 bytes
            let mut key = [0u8; 64];
            key[..32].copy_from_slice(&sha(&spk));
            key[32..].copy_from_slice(&txid);
            for (b, x) in key[32..36].iter_mut().zip(vout.to_le_bytes()) { *b ^= x; }
            ins.push((key, None, v));
            value = value.checked_add(v).expect("value overflows");
            match groups.last_mut() {
                Some(g) if g.0 == txid => { g.1 += 1; g.2 |= coinbase; g.3 |= t; }
                _ => groups.push((txid, 1, coinbase, t)),
            }
        }
        seen += k;
    }
    assert!(s.i == snap.len() && yi == np && ti == nt, "trailing bytes, or unused hints / tracked entries");
    let hash3 = sha(&h3.finalize());
    let (live_set, live_hash) = swiftsync::total();
    let c2 = env::cycle_count();

    // ---- C: the counting trie, every coin inserted into the empty trie
    let nw = read_u32() as usize;
    let wit = read_words(nw);
    let perm = read_words(ins.len());
    // the core's condition on the mode (prover/methods/guest/src/main.rs: the mode is the witness's first word, an empty
    // trie's root is zero in both modes), and the witness must open the EMPTY trie: every coin goes into a trie of
    // nothing else, so a witness of another root cannot carry coins the snapshot does not have
    assert!(wit.first() == Some(&1), "trie: the witness is not in counts mode");
    assert!(wit.len() >= 9 && wit[1..9] == [0u32; 8], "trie: the witness is not of the empty trie");
    let c2b = env::cycle_count();
    let troot = ttree::apply_stream_perm(&wit, &[], &ins, Some(&perm)).expect("trie");
    let c3 = env::cycle_count();

    // ---- D: the BIP30 tree: live coinbases and tracked transactions with their live counts, then REPEATABLE slots
    let nr = read_u32() as usize;
    let rep = read_bytes(32 * nr);
    // 0: one proven roots.rs `apply` per entry (the core's per-insert path); 1: tsi::smt_bulk over the sorted entries
    let smt_mode = read_u32();
    let npw = read_u32() as usize;
    let pw = read_words(npw);
    let c3b = env::cycle_count();
    if (1..=3).contains(&smt_mode) {
        assert!(npw == 0, "bulk mode takes no proofs");
        // the entries in key order: the coinbase / tracked groups (txids ascending, the snapshot's order) merged with the
        // REPEATABLE txids (ascending); strictly increasing overall, so no key twice and no REPEATABLE txid with live coins
        let mut e: Vec<([u8; 32], u32)> = groups.iter().filter(|g| g.2 || g.3).map(|g| (g.0, g.1)).collect();
        e.extend(rep.chunks_exact(32).map(|r| (<[u8; 32]>::try_from(r).unwrap(), REPEATABLE)));
        e.sort_unstable_by(|x, y| x.0.cmp(&y.0));
        assert!(e.windows(2).all(|w| w[0].0 < w[1].0), "BIP30 entries: a key twice");
        let root = if smt_mode == 1 {
            let empty = smt::empty_hashes();
            tsi::smt_bulk(&e, 0, &empty, &|k, v| smt::hash_leaf(k, v), &|l, r| smt::hash_node(l, r))
        } else if smt_mode == 2 {
            // 2: the same tree with the leaf and node hashes as hand-padded compressions (tsi::leaf_raw / node_raw)
            let empty = tsi::empty_raw();
            tsi::smt_bulk(&e, 0, &empty, &tsi::leaf_raw, &tsi::node_raw)
        } else {
            // 3: the same, each block one direct call of the SHA accelerator (tsi::leaf_sys / node_sys)
            let empty = tsi::empty_sys();
            tsi::smt_bulk(&e, 0, &empty, &tsi::leaf_sys, &tsi::node_sys)
        };
        let c4 = env::cycle_count();
        finish(c0, c1, c1b, c2, c2b, c3, c3b, c4, h_tip, tip_hash, hash3, magic, seen, value, live_set, live_hash, troot, root, e.len() as u32, &log, &hashes, out);
        return;
    }
    let mut root = smt::empty_root();
    let mut pi = 0usize;
    let mut next_proof = || -> smt::Proof {
        let mut bitmap = [0u8; 32];
        for k in 0..8 { bitmap[4 * k..4 * k + 4].copy_from_slice(&pw[pi + k].to_le_bytes()); }
        let n = pw[pi + 8] as usize;
        let siblings = (0..n).map(|j| { let mut h = [0u8; 32]; for k in 0..8 { h[4 * k..4 * k + 4].copy_from_slice(&pw[pi + 9 + 8 * j + k].to_le_bytes()); } h }).collect();
        pi += 9 + 8 * n;
        smt::Proof { bitmap, siblings }
    };
    let mut n_smt = 0u32;
    for g in groups.iter().filter(|g| g.2 || g.3) {
        root = smt::apply(&root, &g.0, None, Some(g.1), &next_proof()).expect("BIP30 tree insert");
        n_smt += 1;
    }
    for r in rep.chunks_exact(32) {
        let t: [u8; 32] = r.try_into().unwrap();
        assert!(groups.binary_search_by(|g| g.0.cmp(&t)).is_err(), "a repeatable txid has live coins");
        root = smt::apply(&root, &t, None, Some(REPEATABLE), &next_proof()).expect("BIP30 tree insert");
        n_smt += 1;
    }
    assert!(pi == pw.len(), "unused BIP30 proofs");
    let c4 = env::cycle_count();

    finish(c0, c1, c1b, c2, c2b, c3, c3b, c4, h_tip, tip_hash, hash3, magic, seen, value, live_set, live_hash, troot, root, n_smt, &log, &hashes, out);
}

#[allow(clippy::too_many_arguments)]
fn finish(c0: u64, c1: u64, c1b: u64, c2: u64, c2b: u64, c3: u64, c3b: u64, c4: u64, h_tip: u32, tip_hash: [u8; 32], hash3: [u8; 32],
          magic: [u8; 4], seen: u64, value: u64, live_set: u32, live_hash: [u32; 16], troot: W, root: [u8; 32], n_smt: u32,
          log: &[Option<W>], hashes: &[[u8; 32]], mut out: Vec<u32>) {
    // ---- E: the block forest (H+1 leaves) and the commit
    let mut roots: Vec<Option<[u8; 32]>> = log.iter().map(|r| r.map(|w| coinlog::h_of(&w))).collect();
    while roots.last() == Some(&None) { roots.pop(); }
    let mut st = Vec::new();
    st.push(4u8);
    st.extend_from_slice(&seen.to_le_bytes());
    st.extend_from_slice(&value.to_le_bytes());
    for r in roots.iter().rev().flatten() { st.extend_from_slice(r); }
    let st_h = sha(&st);
    let unknown = sha(UNKNOWN_STATE_TAG);
    let mut bf: Vec<Option<[u8; 32]>> = Vec::new();
    for (i, h) in hashes.iter().enumerate() {
        let mut p = [0u8; 69];
        p[0] = 2;
        p[1..33].copy_from_slice(h);
        p[33..37].copy_from_slice(&(i as u32).to_le_bytes());
        p[37..].copy_from_slice(if i as u32 == h_tip { &st_h } else { &unknown });
        stump_add(&mut bf, sha(&p));
    }
    while bf.last() == Some(&None) { bf.pop(); }
    let c5 = env::cycle_count();
    let w8 = |o: &mut Vec<u32>, b: &[u8; 32]| for k in 0..8 { o.push(u32::from_le_bytes(b[4 * k..4 * k + 4].try_into().unwrap())); };
    out.push(h_tip);
    w8(&mut out, &tip_hash);
    w8(&mut out, &hash3);
    out.push(u32::from_le_bytes(magic));
    out.extend_from_slice(&[seen as u32, (seen >> 32) as u32, value as u32, (value >> 32) as u32, live_set]);
    out.extend_from_slice(&live_hash);
    w8(&mut out, &coinlog::h_of(&troot));
    w8(&mut out, &root);
    out.push(n_smt);
    for rs in [&roots, &bf] {
        out.push(rs.len() as u32);
        for r in rs.iter() { match r { Some(h) => { out.push(1); w8(&mut out, h); } None => out.push(0) } }
    }
    env::commit_slice(&out);
    let c6 = env::cycle_count();
    env::log(&alloc::format!("TSI headers {} read {} coins {} trie_read {} trie {} smt_read {} smt {} forest {} commit {} n={seen} smt_n={n_smt} h={h_tip}",
                             c1 - c0, c1b - c1, c2 - c1b, c2b - c2, c3 - c2b, c3b - c3, c4 - c3b, c5 - c4, c6 - c5));
}
