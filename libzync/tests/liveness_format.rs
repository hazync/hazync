//! The liveness proof format (src/liveness.rs), the cases cargo-mutants found no test for (docs/libzync/ZAPP_TESTING.md
//! §3.5, 17.11.2026): every coin flag survives the one encoding, a path is at most 63 hashes, and a bad flag byte is
//! refused as such (not only by the canonical re-encoding behind it). Synthetic proofs: decode checks the form only.
#![cfg(feature = "liveness")]

use libzync::liveness::*;

fn proof(coinbase: bool, tracked: bool, path: usize, opening: bool) -> Proof {
    Proof {
        coin: Coin { txid: [7; 32], vout: 1, value: 5_000, script_pubkey: vec![0x51], height: 9, coinbase, tracked, creation_mtp: 77 },
        state: State::Unspent,
        at: 9,
        pos: 3,
        path: vec![[1; 32]; path],
        opening: opening.then(|| Opening { log_leaves: 1, log_value: 5_000, log_roots: vec![[2; 32]], block_hash: [3; 32], undo: None,
                                           block_path: vec![[4; 32]; 2] }),
    }
}

/// Every combination of the coinbase and tracked flags, with and without an opening, decodes to itself.
#[test]
fn every_flag_round_trips() {
    for cb in [false, true] {
        for tr in [false, true] {
            for op in [false, true] {
                let p = proof(cb, tr, 2, op);
                assert_eq!(decode(&encode(&p)), Ok(p), "coinbase {cb} tracked {tr} opening {op}");
            }
        }
    }
}

/// A path of 63 hashes (a tree of 2^63 leaves) is read; 64 is refused before its hashes are read.
#[test]
fn paths_up_to_63() {
    let p = proof(false, false, 63, false);
    assert_eq!(decode(&encode(&p)), Ok(p));
    let b = encode(&proof(false, false, 64, false));
    assert_eq!(decode(&b), Err(Error::Decode("path longer than any tree")));
}

/// The flag byte (offset 6): an unknown bit, or the undo bit without the opening bit, is refused by the flag check.
#[test]
fn bad_flags_are_refused_by_name() {
    let b = encode(&proof(false, false, 2, false));
    for f in [0x10u8, 0x80, 0x08, 0x08 | 0x01] {
        let mut x = b.clone();
        x[6] = f;
        assert_eq!(decode(&x), Err(Error::Decode("unknown flag bits")), "flags 0x{f:02x}");
    }
}

/// Each refusal has its own text (what the zAPPs print).
#[test]
fn errors_say_what_failed() {
    let all = [Error::AboveTip, Error::Opening, Error::Undo, Error::Block, Error::NotInLog, Error::Decode("truncated")];
    for (i, e) in all.iter().enumerate() {
        assert!(e.text().len() > 8, "{e:?}");
        assert!(all[i + 1..].iter().all(|o| o.text() != e.text()), "{e:?}");
    }
    assert!(Error::AboveTip.text().contains("above the proof's tip"));
    assert_eq!(Error::Decode("truncated").text(), "truncated");
}

/// A scriptPubKey of up to 10,000 bytes (Core's MAX_SCRIPT_SIZE) is read; one byte more is refused by the length check.
#[test]
fn scripts_up_to_10000_bytes() {
    let mut p = proof(false, false, 2, false);
    p.coin.script_pubkey = vec![0x51; 10_000];
    assert_eq!(decode(&encode(&p)), Ok(p.clone()));
    p.coin.script_pubkey.push(0x51);
    assert_eq!(decode(&encode(&p)), Err(Error::Decode("scriptPubKey over 10,000 bytes (unspendable, never a coin)")));
}

/// An opening below the tip on the REAL mainnet 1..200 run of the ENGINE chain (its history,
/// swiftsync-verify/tests/data/engine200/history.bin, node format 2, and zapps/liveness/vectors/engine: its proof of block
/// 9's coin after block 169, unspent then): the tip's block forest opens the coin log at 169. The engine chain's leaves
/// carry the undo commitment from block 1 (undo_from 1), so the opening carries an undo hash. Refused: no undo hash, or
/// another, where the leaves carry one; an undo hash where the image's leaves carry none (undo_from u32::MAX); another
/// block hash. (Until Cutover C4 this read the RISC Zero run's history, mainnet_1_200_history.bin, whose leaves carried
/// none; that run's files went with C4.)
#[test]
fn an_opening_below_the_tip_on_the_real_run() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
    let recs = libzync::history::decode_all(&std::fs::read(format!("{root}/swiftsync-verify/tests/data/engine200/history.bin")).unwrap()).unwrap();
    let t = libzync::history::replay(&recs, 200, 200).unwrap();
    let utxo_roots = t.log.roots();
    let (_, block_roots) = libzync::history::block_path(&t.block_leaves, 0);
    let chain = |undo_from: u32| Chain { height: 200, utxo_roots: &utxo_roots, utxo_leaves: t.log.len(), block_roots: &block_roots,
                                         block_leaves: t.block_leaves.len() as u64, undo_from };
    let text = std::fs::read_to_string(format!("{root}/zapps/liveness/vectors/engine/b9_0_at169.hex")).unwrap();
    let text = text.trim();
    let b: Vec<u8> = (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect();
    let p = verify(&b, &chain(1)).unwrap();
    assert!(p.at == 169 && p.state == State::Unspent && p.opening.as_ref().is_some_and(|o| o.undo.is_some()), "{p:?}");
    assert_eq!(check(&p, &chain(u32::MAX)), Err(Error::Undo));
    let mut w = p.clone();
    w.opening.as_mut().unwrap().undo = None;
    assert_eq!(check(&w, &chain(1)), Err(Error::Undo));
    let mut w = p.clone();
    w.opening.as_mut().unwrap().undo.as_mut().unwrap()[0] ^= 1;
    assert!(check(&w, &chain(1)).is_err());
    let mut w = p.clone();
    w.opening.as_mut().unwrap().block_hash[0] ^= 1;
    assert_eq!(check(&w, &chain(1)), Err(Error::Block));
}
