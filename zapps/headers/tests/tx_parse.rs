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
    // and the same with a segwit witness added (marker, flag, one stack of one 2-byte item): the txid is the stripped form
    let mut w = t[..4].to_vec();
    w.extend([0x00, 0x01]);
    w.extend(&t[4..t.len() - 4]);
    w.extend([0x01, 0x02, 0xaa, 0xbb]);
    w.extend(&t[t.len() - 4..]);
    let (h, inc) = block_with(&w, dsha(&t));
    assert!(verify_tx_in_header(&h, &inc).is_ok(), "{:?}", verify_tx_in_header(&h, &inc));
    // the marker with every witness stack empty: Core refuses it ("Superfluous witness record"), and so does the parser
    let mut e = t[..4].to_vec();
    e.extend([0x00, 0x01]);
    e.extend(&t[4..t.len() - 4]);
    e.push(0);
    e.extend(&t[t.len() - 4..]);
    let (h, inc) = block_with(&e, dsha(&t));
    let err = verify_tx_in_header(&h, &inc).unwrap_err();
    assert!(err.contains("a witness section with every stack empty"), "{err}");
}

/// Compact sizes as Core reads them (libzync::facts::varint; review 21.11.2026): a count or length in a longer form
/// than its value needs, and a length past the end, are refused by name.
#[test]
fn compact_sizes_are_cores() {
    let t = legacy(7);
    let ok = |raw: &[u8]| { let (h, inc) = block_with(raw, dsha(raw)); verify_tx_in_header(&h, &inc) };
    assert!(ok(&t).is_ok(), "{:?}", ok(&t));
    let with = |at: usize, rep: &[u8]| { let mut v = t[..at].to_vec(); v.extend_from_slice(rep); v.extend_from_slice(&t[at + 1..]); v };
    // (the 5-byte forms keep each transaction off 64 bytes, which is refused before parsing)
    const C: &str = "a compact size not as Core reads it";
    assert_eq!(t[4], 1);
    let e = ok(&with(4, &[0xfe, 0x01, 0x00, 0x00, 0x00])).unwrap_err();
    assert!(e.contains(C), "a non-canonical input count: {e}");
    assert_eq!(t[41], 1);
    let e = ok(&with(41, &[0xfe, 0x01, 0x00, 0x00, 0x00])).unwrap_err();
    assert!(e.contains(C), "a non-canonical scriptSig length: {e}");
    let spk = t.len() - 4 - 2;
    assert_eq!(t[spk], 1);
    let e = ok(&with(spk, &[0xfe, 0x01, 0x00, 0x00, 0x00])).unwrap_err();
    assert!(e.contains(C), "a scriptPubKey length in the 5-byte form: {e}");
    // a witness stack count in a longer form than its value needs (fe 01 00 00 00 for one item of 2 bytes)
    let mut w = t[..4].to_vec();
    w.extend([0x00, 0x01]);
    w.extend(&t[4..t.len() - 4]);
    w.extend([0xfe, 0x01, 0x00, 0x00, 0x00, 0x02, 0xaa, 0xbb]);
    w.extend(&t[t.len() - 4..]);
    let (h, inc) = block_with(&w, dsha(&t));
    let e = verify_tx_in_header(&h, &inc).unwrap_err();
    assert!(e.contains(C), "a non-canonical witness count: {e}");
    // two inputs, the first stack empty and the second not: a witness record exists, so it is accepted
    let mut two = t[..4].to_vec();
    two.push(2);
    two.extend(&t[5..47]); // the first input (prevout, OP_0 scriptSig, sequence)
    let mut second = t[5..47].to_vec();
    second[0] ^= 0x55; // another previous txid
    two.extend(&second);
    two.extend(&t[47..t.len() - 4]); // the outputs
    let mut tw = two[..4].to_vec();
    tw.extend([0x00, 0x01]);
    tw.extend(&two[4..]);
    tw.extend([0x00, 0x01, 0x02, 0xaa, 0xbb]); // stack 1 empty, stack 2 one 2-byte item
    tw.extend(&t[t.len() - 4..]);
    two.extend(&t[t.len() - 4..]);
    assert_eq!(two.len(), 104);
    let (h, inc) = block_with(&tw, dsha(&two));
    assert!(verify_tx_in_header(&h, &inc).is_ok(), "one empty and one non-empty stack: {:?}", verify_tx_in_header(&h, &inc));
    // a scriptPubKey length past the end (5 bytes are left: the 1-byte script and the lock time; 7 asked)
    let e = ok(&with(spk, &[0x07])).unwrap_err();
    assert!(e.contains("a length past the end of the transaction"), "{e}");
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
