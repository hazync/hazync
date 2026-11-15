//! Liveness proofs (docs/libzync/LIVENESS_ZAPP.md) against the REAL proof of mainnet 1..200 (image 8888d59c, risc0
//! 3.1.0, the core's swiftsync-verify/vectors/mainnet_1_200_8888d59c.rcpt) and the history of that run
//! (vectors/mainnet_1_200_history.bin). The proofs in vectors/ were written by `zync-path live ... --blocks
//! prover/testdata/libzync/mainnet`, now `zapp-liveness prove`.

use libzync::liveness as lv;
use swiftsync_verify::*;
use zapp_liveness::*;

const T9: &str = "0437cd7f8525ceed2324359c2d0ba26006d92d856a9c20fa0241106ee5a597c9";
const T170: &str = "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16";

/// The core verifier's vectors (the receipts this zAPP is checked against).
fn vec_path(f: &str) -> String {
    format!("{}/../../swiftsync-verify/vectors/{f}", env!("CARGO_MANIFEST_DIR"))
}

/// This zAPP's own vectors (liveness proofs, the run's history).
fn zapp_path(f: &str) -> String {
    format!("{}/vectors/{f}", env!("CARGO_MANIFEST_DIR"))
}

fn proof(f: &str) -> Vec<u8> {
    liveness_bytes(&std::fs::read(zapp_path(f)).unwrap()).unwrap()
}

fn verified(rcpt: &str) -> Report {
    let receipt = std::fs::read(vec_path(rcpt)).unwrap();
    let image = Image::by_id("mainnet", "8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4").unwrap();
    verify(&Inputs { receipt: &receipt, image, anchor: None, now: 1_800_000_000 })
}

fn state() -> (ChainState, u32) {
    let r = verified("mainnet_1_200_8888d59c.rcpt");
    assert_eq!(r.verdict, Verdict::Demo, "below minimum work: DEMO");
    (r.state.unwrap(), r.undo_from)
}

fn ask(s: &str) -> LivenessAsk {
    LivenessAsk::parse(s).unwrap()
}

/// (file, question, the answer token)
const HONEST: [(&str, &str, &str); 5] = [
    ("b9_0_at200.hex", "0437cd7f8525ceed2324359c2d0ba26006d92d856a9c20fa0241106ee5a597c9:0@200", "spent:170:1:0"),
    ("b9_0_at169.hex", "0437cd7f8525ceed2324359c2d0ba26006d92d856a9c20fa0241106ee5a597c9:0@169", "unspent"),
    ("b170_0_at200.hex", "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16:0@tip", "unspent"),
    ("b170_1_at200.hex", "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16:1@200", "spent:181:1:0"),
    ("b170_1_at170.hex", "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16:1@170", "unspent"),
];

#[test]
fn honest_proofs_hold_and_the_producer_is_deterministic() {
    let (cs, uf) = state();
    let recs = libzync::history::decode_all(&std::fs::read(zapp_path("mainnet_1_200_history.bin")).unwrap()).unwrap();
    for (f, q, want) in HONEST {
        let b = proof(f);
        let (got, line) = check_liveness(&cs, uf, &b, &ask(q)).unwrap_or_else(|e| panic!("{f}: {e}"));
        assert_eq!(got, want, "{f}: {line}");
        // the same question with its answer claimed also holds
        check_liveness(&cs, uf, &b, &ask(&format!("{q}={want}"))).unwrap();
        // the producer, given the coin, writes these exact bytes again
        let p = lv::decode(&b).unwrap();
        assert_eq!(make_liveness(&recs, 200, p.at, &p.coin).unwrap(), b, "{f}");
    }
    // the answers that matter for the uses: the first payment's input coin moved at 170; Hal's 10 BTC had not by 200
    let (_, l) = check_liveness(&cs, uf, &proof("b9_0_at200.hex"), &ask(&format!("{T9}:0@200=spent:170,value=5000000000"))).unwrap();
    assert!(l.contains("SPENT at height 170 by transaction 1 input 0"), "{l}");
}

#[test]
fn wrong_amount_or_script_is_refused() {
    let (cs, uf) = state();
    for (f, q, _) in HONEST {
        let p = lv::decode(&proof(f)).unwrap();
        let a = ask(q);
        // the proof itself carrying another amount / script / creation height / coinbase flag: another leaf
        for m in 0..4 {
            let mut w = p.clone();
            match m {
                0 => w.coin.value += 1,
                1 => w.coin.script_pubkey[1] ^= 1,
                2 => w.coin.height -= 1,
                _ => w.coin.coinbase = !w.coin.coinbase,
            }
            let e = check_liveness(&cs, uf, &lv::encode(&w), &a).unwrap_err();
            assert!(e.contains("not in the claimed state"), "{f} {m}: {e}");
        }
        // the question naming another amount or script than the coin's
        let e = check_liveness(&cs, uf, &proof(f), &ask(&format!("{q},value={}", p.coin.value + 1))).unwrap_err();
        assert!(e.contains("not the amount asked"), "{e}");
        let e = check_liveness(&cs, uf, &proof(f), &ask(&format!("{q},script=0014{}", "11".repeat(20)))).unwrap_err();
        assert!(e.contains("another scriptPubKey"), "{e}");
    }
}

#[test]
fn spent_claimed_unspent_and_unspent_claimed_spent_are_refused() {
    let (cs, uf) = state();
    // block 9's coin, spent at 170, presented as unspent at 200
    let mut p = lv::decode(&proof("b9_0_at200.hex")).unwrap();
    p.state = lv::State::Unspent;
    let e = check_liveness(&cs, uf, &lv::encode(&p), &ask(&format!("{T9}:0@200"))).unwrap_err();
    assert!(e.contains("not in the claimed state"), "{e}");
    // ... and with a different spender
    p.state = lv::State::Spent { height: 170, tx: 1, vin: 1 };
    assert!(check_liveness(&cs, uf, &lv::encode(&p), &ask(&format!("{T9}:0@200"))).is_err());
    // Hal's coin, unspent at 200, presented as spent
    let mut q = lv::decode(&proof("b170_0_at200.hex")).unwrap();
    for (h, tx, vin) in [(200, 1, 0), (171, 1, 0), (181, 1, 0)] {
        q.state = lv::State::Spent { height: h, tx, vin };
        assert!(check_liveness(&cs, uf, &lv::encode(&q), &ask(&format!("{T170}:0@200"))).is_err());
    }
    // the honest proof against a question claiming the other answer
    let e = check_liveness(&cs, uf, &proof("b170_0_at200.hex"), &ask(&format!("{T170}:0@200=spent"))).unwrap_err();
    assert!(e.contains("the proof shows unspent"), "{e}");
    let e = check_liveness(&cs, uf, &proof("b9_0_at200.hex"), &ask(&format!("{T9}:0@200=unspent"))).unwrap_err();
    assert!(e.contains("the proof shows spent:170:1:0"), "{e}");
    let e = check_liveness(&cs, uf, &proof("b9_0_at200.hex"), &ask(&format!("{T9}:0@200=spent:171"))).unwrap_err();
    assert!(e.contains("the proof shows"), "{e}");
    // a spender above the height asked about (the coin log after 169 still has it unspent)
    let mut r = lv::decode(&proof("b9_0_at169.hex")).unwrap();
    r.state = lv::State::Spent { height: 170, tx: 1, vin: 0 };
    assert!(check_liveness(&cs, uf, &lv::encode(&r), &ask(&format!("{T9}:0@169"))).is_err());
}

#[test]
fn an_outpoint_that_never_existed_has_no_proof() {
    let (cs, uf) = state();
    let recs = libzync::history::decode_all(&std::fs::read(zapp_path("mainnet_1_200_history.bin")).unwrap()).unwrap();
    let p = lv::decode(&proof("b170_0_at200.hex")).unwrap();
    // vout 2 of the first payment (it has two outputs), and a made-up txid, on every position of the log
    for coin in [lv::Coin { vout: 2, ..p.coin.clone() }, lv::Coin { txid: [0x42; 32], ..p.coin.clone() }] {
        assert!(make_liveness(&recs, 200, 200, &coin).is_err(), "the producer finds no such coin");
        let mut shown = coin.txid;
        shown.reverse();
        for pos in 0..cs.utxo_leaves {
            let w = lv::Proof { coin: coin.clone(), pos, ..p.clone() };
            assert!(check_liveness(&cs, uf, &lv::encode(&w), &ask(&format!("{}:{}@200", hex(&shown), coin.vout))).is_err(), "pos {pos}");
        }
    }
    // a coin that exists, asked about before its block: the coin log after 169 does not hold block 170's outputs
    assert!(make_liveness(&recs, 200, 169, &p.coin).is_err());
}

#[test]
fn heights_above_the_tip_and_mismatched_questions_are_refused() {
    let (cs, uf) = state();
    let mut p = lv::decode(&proof("b170_0_at200.hex")).unwrap();
    p.at = 201;
    let e = check_liveness(&cs, uf, &lv::encode(&p), &ask(&format!("{T170}:0@201"))).unwrap_err();
    assert!(e.contains("above the proof's tip"), "{e}");
    // an honest proof is not an answer to another question
    for q in [format!("{T170}:0@199"), format!("{T170}:1@200"), format!("{T9}:0@200")] {
        assert!(check_liveness(&cs, uf, &proof("b170_0_at200.hex"), &ask(&q)).is_err(), "{q}");
    }
    // a proof about 169 opened as if 169 were the tip, and a tip proof relabelled 169 without its opening
    let mut o = lv::decode(&proof("b9_0_at169.hex")).unwrap();
    o.opening = None;
    assert!(check_liveness(&cs, uf, &lv::encode(&o), &ask(&format!("{T9}:0@169"))).unwrap_err().contains("opening"));
    let mut t = lv::decode(&proof("b170_0_at200.hex")).unwrap();
    t.at = 199;
    assert!(check_liveness(&cs, uf, &lv::encode(&t), &ask(&format!("{T170}:0@199"))).is_err());
    // an undo hash on a block whose leaf has none (this image predates the commitment)
    let mut u = lv::decode(&proof("b9_0_at169.hex")).unwrap();
    u.opening.as_mut().unwrap().undo = Some([7; 32]);
    assert!(check_liveness(&cs, uf, &lv::encode(&u), &ask(&format!("{T9}:0@169"))).unwrap_err().contains("undo"));
}

/// Every bit of every real proof is bound (the #695 lesson): any single-bit change is refused by the proof check
/// alone, before any question is compared.
#[test]
fn every_byte_is_bound() {
    let (cs, uf) = state();
    let chain = lv::Chain { height: cs.height, utxo_roots: &cs.utxo_roots, utxo_leaves: cs.utxo_leaves, block_roots: &cs.block_roots,
                            block_leaves: cs.block_leaves, undo_from: uf };
    let mut n = 0;
    for (f, _, _) in HONEST {
        let b = proof(f);
        assert!(lv::verify(&b, &chain).is_ok());
        for i in 0..b.len() {
            for bit in 0..8 {
                let mut x = b.clone();
                x[i] ^= 1 << bit;
                assert!(lv::verify(&x, &chain).is_err(), "{f}: byte {i} bit {bit} accepted");
                n += 1;
            }
        }
        // and nothing may be appended or cut
        let mut l = b.clone();
        l.push(0);
        assert!(lv::verify(&l, &chain).is_err());
        assert!(lv::verify(&b[..b.len() - 1], &chain).is_err());
    }
    assert_eq!(n, 8 * HONEST.iter().map(|(f, _, _)| proof(f).len()).sum::<usize>());
    eprintln!("every_byte_is_bound: {n} single-bit flips, all refused");
}

/// A receipt that does not verify gives no state to check a proof against.
#[test]
fn a_dev_mode_or_tampered_receipt_is_refused() {
    for r in ["fake_block_2501.rcpt", "mainnet_1_200_8888d59c_flipped.rcpt"] {
        assert_eq!(verified(r).verdict, Verdict::Fail, "{r}");
    }
}

#[test]
fn questions_parse_strictly() {
    let a = ask(&format!("{T170}:1@200=spent:181:1:0,value=4000000000,script=00"));
    assert_eq!((a.vout, a.at, a.state, a.value, a.script), (1, Some(200), Some(AskState::Spent(Some(181), Some((1, 0)))), Some(4_000_000_000), Some(vec![0])));
    assert_eq!(ask(&format!("{T170}:0@tip")).at, None);
    for bad in [format!("{T170}:0"), format!("{T170}@1"), format!("{}:0@1", &T170[2..]), format!("{T170}:x@1"), format!("{T170}:0@1=gone"),
                format!("{T170}:0@1=spent:1:2"), format!("{T170}:0@1,value=1,value=2"), format!("{T170}:0@1,colour=red"), format!("{T170}:+1@1")] {
        assert!(LivenessAsk::parse(&bad).is_err(), "{bad}");
    }
    // the CLI reads a proof as raw bytes or LOWER-case hex only
    let b = proof("b170_0_at200.hex");
    assert_eq!(liveness_bytes(&b).unwrap(), b);
    assert!(liveness_bytes(hex(&b).to_uppercase().as_bytes()).is_err());
}

/// Timing (run with --ignored --nocapture): the receipt check once, then each proof's check.
#[test]
#[ignore]
fn timing() {
    let t = std::time::Instant::now();
    let (cs, uf) = state();
    eprintln!("receipt verify: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    for (f, q, _) in HONEST {
        let b = proof(f);
        let a = ask(q);
        let n = 20_000;
        let t = std::time::Instant::now();
        for _ in 0..n {
            check_liveness(&cs, uf, &b, &a).unwrap();
        }
        eprintln!("{f}: {} bytes, {:.1} us per check", b.len(), t.elapsed().as_secs_f64() * 1e6 / n as f64);
    }
}
