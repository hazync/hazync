//! EXPERIMENTAL Trusted State chunked ingest, the CHUNK (docs/libzync/TRUSTED_STATE.md §12). MEASUREMENT ONLY, its own
//! image. One contiguous run of coins of Core's snapshot, in file order, from a carried state S_in to S_out
//! (tsi::ChunkState): Core's hash_serialized_3 midstate, the ECMH partial sum, the coin log's append forest, the counting
//! trie's root (updated with a witness against S_in's root), the value, the BIP30 entry chain and the txid group being
//! read. Commits [SHA-256(S_in), SHA-256(S_out), SHA-256(header times), coins] and nothing else: the states are bound
//! only by the combine, which requires out_i == in_(i+1), in_0 == the initial state and out_last == the final state it
//! is given. The host computes every S_in natively (untrusted): a wrong one breaks the chain at the combine.
#![no_main]
extern crate alloc;
use alloc::vec::Vec;
use risc0_zkvm::guest::env;
use sha2::{Digest, Sha256};
risc0_zkvm::guest::entry!(main);

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
#[path = "../../../gcommon.rs"]
#[allow(dead_code)]
mod gcommon;

use gcommon::*;
use risc0_crypto::curves::secp256k1::Fq;
use risc0_crypto::BigInt;

fn main() {
    let c0 = env::cycle_count();
    let sin = read_section();
    let mut s = tsi::ChunkState::from_bytes(&sin).expect("S_in: not a canonical ChunkState");
    let d_in = s.digest();
    let times_b = read_section();
    let th = tsi::sha(&times_b);
    let times: Vec<u32> = times_b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    assert!(!times.is_empty(), "no header times");
    let h_tip = (times.len() - 1) as u32;
    let snap = read_section();
    swiftsync::install(false, read_section());
    let yh = read_section();
    let trk = read_section();
    let c1 = env::cycle_count();

    swiftsync::start(s.live_set, &s.live_hash);
    let mut src = Src { b: &snap, i: 0 };
    if s.started == 0 {
        // chunk 0: the file header, from the initial state only
        assert!(s == tsi::ChunkState::initial(), "a header can only be read from the initial state");
        assert!(src.take(5) == b"utxo\xff" && u16::from_le_bytes(src.take(2).try_into().unwrap()) == 2, "not a v2 snapshot");
        s.magic = src.take(4).try_into().unwrap();
        s.base = src.take(32).try_into().unwrap();
        s.coins_total = u64::from_le_bytes(src.take(8).try_into().unwrap());
        s.started = 1;
    }
    let mut mtp = alloc::vec![0u32; h_tip as usize + 1];
    let mut log = s.log_roots.clone();
    let mut ins: Vec<(ttree::Key, Option<coinlog::W>, u64)> = Vec::new();
    let (mut yi, mut ti, mut n) = (0usize, 0usize, 0u64);
    let seven = Fq::from_u32(7);
    let mut ser: Vec<u8> = Vec::with_capacity(128);
    while src.i < snap.len() {
        if s.g_left == 0 {
            // a new txid group: the one before it is complete
            s.close_group();
            let txid: [u8; 32] = src.take(32).try_into().unwrap();
            assert!(s.idx == 0 || txid > s.g_txid, "txids not ascending");
            let k = src.compact();
            assert!(k > 0 && k <= s.coins_total - s.idx, "bad group size");
            s.g_txid = txid;
            s.g_left = k;
            s.last_vout.clear();
        }
        let txid = s.g_txid;
        let vout64 = src.compact();
        assert!(vout64 < u32::MAX as u64, "vout out of range");
        let vout = vout64 as u32;
        let key = varint_bytes(vout64);
        assert!(s.last_vout.is_empty() || key > s.last_vout, "vouts not in Core's key order");
        s.last_vout = key;
        let code = src.varint();
        let (height, coinbase) = ((code >> 1) as u32, code & 1 == 1);
        assert!(height > 0 && height <= h_tip, "coin height outside 1..H");
        let v = decompress_amount(src.varint());
        let sz = src.varint();
        let spk: Vec<u8> = match sz {
            0 => [&[0x76u8, 0xa9, 20][..], src.take(20), &[0x88, 0xac]].concat(),
            1 => [&[0xa9u8, 20][..], src.take(20), &[0x87]].concat(),
            2 | 3 => [&[33u8, sz as u8][..], src.take(32), &[0xac]].concat(),
            4 | 5 => {
                let xb = src.take(32);
                let yb = &yh[32 * yi..32 * yi + 32];
                yi += 1;
                let x = Fq::from_bigint(BigInt(be_limbs(xb))).expect("x >= p");
                let y = Fq::from_bigint(BigInt(be_limbs(yb))).expect("y >= p");
                assert!(&y * &y == &(&(&x * &x) * &x) + &seven, "P2PK hint: y not on the curve");
                assert!((yb[31] & 1) as u64 == sz - 4, "P2PK hint: wrong parity");
                [&[0x41u8, 0x04][..], xb, yb, &[0xac]].concat()
            }
            _ => { let l = (sz - 6) as usize; assert!(l <= 10_000, "script over 10,000 bytes"); src.take(l).to_vec() }
        };
        let t = ti < trk.len() / 36 && trk[36 * ti..36 * ti + 32] == txid && u32::from_le_bytes(trk[36 * ti + 32..36 * ti + 36].try_into().unwrap()) == vout;
        ti += t as usize;
        // hash_serialized_3's stream: TxOutSer
        ser.clear();
        ser.extend_from_slice(&txid);
        ser.extend_from_slice(&vout.to_le_bytes());
        ser.extend_from_slice(&((height << 1) | coinbase as u32).to_le_bytes());
        ser.extend_from_slice(&v.to_le_bytes());
        if spk.len() < 253 { ser.push(spk.len() as u8); } else { ser.push(253); ser.extend_from_slice(&(spk.len() as u16).to_le_bytes()); }
        ser.extend_from_slice(&spk);
        s.h3.update(&ser);
        // the coin leaf
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
        let mut tk = [0u8; 64];
        tk[..32].copy_from_slice(&tsi::sha(&spk));
        tk[32..].copy_from_slice(&txid);
        for (b, x) in tk[32..36].iter_mut().zip(vout.to_le_bytes()) { *b ^= x; }
        ins.push((tk, None, v));
        s.value = s.value.checked_add(v).expect("value overflows");
        s.idx += 1;
        s.g_left -= 1;
        s.g_n += 1;
        s.g_flags |= coinbase as u32 | (t as u32) << 1;
        n += 1;
    }
    assert!(yi == yh.len() / 32 && ti == trk.len() / 36, "unused hints or tracked entries");
    s.log_roots = log;
    s.log_n += n;
    let (ls, lh) = swiftsync::total();
    s.live_set = ls;
    s.live_hash = lh;
    let c2 = env::cycle_count();
    // the counting trie: this chunk's coins into the trie at S_in's root (the witness must open exactly that root)
    let wit: Vec<u32> = read_section().chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    let perm: Vec<u32> = read_section().chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    assert!(wit.len() >= 9 && wit[1..9] == s.trie_root[..], "the trie witness is not of S_in's trie root");
    let c2b = env::cycle_count();
    s.trie_root = ttree::apply_stream_perm(&wit, &[], &ins, Some(&perm)).expect("trie update");
    let c3 = env::cycle_count();
    let d_out = s.digest();
    let mut out: Vec<u32> = Vec::with_capacity(26);
    for d in [d_in, d_out, th] { out.extend(d.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap()))); }
    out.extend_from_slice(&[n as u32, (n >> 32) as u32]);
    env::commit_slice(&out);
    let c4 = env::cycle_count();
    env::log(&alloc::format!("TSC read {} coins {} trie_read {} trie {} commit {} n={n} trie_wit_words={}", c1 - c0, c2 - c1, c2b - c2, c3 - c2b, c4 - c3, wit.len()));
}
