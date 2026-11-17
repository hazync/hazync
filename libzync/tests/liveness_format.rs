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
