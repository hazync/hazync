//! `verify_tx_in_header`'s transaction parser on synthetic blocks (docs/libzync/ZAPP_TESTING.md §3.5, the cargo-mutants
//! pass of 17.11.2026): the segwit marker is read only as 0x00 0x01 right after the version, and the parser's refusals
//! are the ones it names. A block here is two transactions, a coinbase and one other, with its header's merkle root
//! computed from them; no proof of work (verify_tx_in_header checks the transaction against the header it is given).

use sha2::{Digest, Sha256};
use zapp_headers::{verify_tx_in_header, TxInclusion};

fn dsha(b: &[u8]) -> [u8; 32] {
    Sha256::digest(Sha256::digest(b)).into()
}

/// A legacy coinbase: one input spending the null outpoint, one output (65 bytes, not 64).
fn coinbase() -> Vec<u8> {
    let mut t = vec![1, 0, 0, 0, 1];
    t.extend([0u8; 32]);
    t.extend([0xff; 4]);
    t.extend([4, 3, 1, 2, 3]); // scriptSig: push the height
    t.extend([0xff; 4]);
    t.push(1);
    t.extend(50_0000_0000u64.to_le_bytes());
    t.extend([1, 0x51]);
    t.extend([0; 4]);
    t
}

/// A legacy transaction with one input whose previous txid starts with `first`, one OP_TRUE output.
fn legacy(first: u8) -> Vec<u8> {
    let mut t = vec![1, 0, 0, 0, 1];
    let mut prev = [7u8; 32];
    prev[0] = first;
    t.extend(prev);
    t.extend(0u32.to_le_bytes());
    t.extend([1, 0x00]); // scriptSig: OP_0 (62 bytes in all; 65 with a witness: neither is 64)
    t.extend([0xff; 4]);
    t.push(1);
    t.extend(1000u64.to_le_bytes());
    t.extend([1, 0x51]);
    t.extend([0; 4]);
    t
}

/// The inclusion of `raw` at position 1 of a two-transaction block; `txid` is what the caller asks about.
fn block_with(raw: &[u8], txid: [u8; 32]) -> ([u8; 32], TxInclusion) {
    let cb = coinbase();
    let cb_id = dsha(&cb);
    let root = dsha(&[cb_id, txid].concat());
    let mut header = [0u8; 80];
    header[0] = 4;
    header[36..68].copy_from_slice(&root);
    (dsha(&header), TxInclusion { header, txid, raw: raw.to_vec(), pos: 1, branch: vec![cb_id], coinbase: cb, coinbase_branch: vec![txid] })
}

/// A legacy transaction with ONE input (byte 4 is 0x01) whose previous txid starts 0x01 (byte 5): bytes 4..6 are
/// 0x01 0x01, not the segwit marker 0x00 0x01. It is accepted, parsed as legacy (its txid is its whole serialisation).
#[test]
fn a_legacy_input_starting_01_is_not_segwit() {
    let t = legacy(0x01);
    assert_eq!((t[4], t[5]), (0x01, 0x01));
    let (h, inc) = block_with(&t, dsha(&t));
    assert!(verify_tx_in_header(&h, &inc).is_ok(), "{:?}", verify_tx_in_header(&h, &inc));
    // and the same with a segwit witness added (marker, flag, one empty witness stack): the txid is the stripped form
    let mut w = t[..4].to_vec();
    w.extend([0x00, 0x01]);
    w.extend(&t[4..t.len() - 4]);
    w.push(0);
    w.extend(&t[t.len() - 4..]);
    let (h, inc) = block_with(&w, dsha(&t));
    assert!(verify_tx_in_header(&h, &inc).is_ok(), "{:?}", verify_tx_in_header(&h, &inc));
}

/// The parser's own refusals, by name: six bytes 01000000 0001 are a transaction with no inputs (not a segwit marker
/// with nothing after it); a count above the transaction's length is refused as such; a count equal to it is read
/// until the bytes run out.
#[test]
fn parser_refusals_name_their_reason() {
    let six = [1u8, 0, 0, 0, 0, 1];
    let (h, inc) = block_with(&six, dsha(&six));
    let e = verify_tx_in_header(&h, &inc).unwrap_err();
    assert!(e.contains("no inputs"), "{e}");
    // 0xfd 0xffff inputs in a 100-byte transaction
    let mut big = vec![1u8, 0, 0, 0, 0xfd, 0xff, 0xff];
    big.resize(100, 0);
    let (h, inc) = block_with(&big, dsha(&big));
    let e = verify_tx_in_header(&h, &inc).unwrap_err();
    assert!(e.contains("a count larger than the transaction"), "{e}");
    // exactly as many inputs as the transaction has bytes (300): not refused by the count, refused when it ends
    let mut eq = vec![1u8, 0, 0, 0, 0xfd];
    eq.extend(300u16.to_le_bytes());
    eq.resize(300, 0);
    let (h, inc) = block_with(&eq, dsha(&eq));
    let e = verify_tx_in_header(&h, &inc).unwrap_err();
    assert!(e.contains("ends early"), "{e}");
}
