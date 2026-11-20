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
    // one spelling per question (review 15.11.2026): lower-case txid and script, no sign, no leading zeros
    let a = ask(&format!("{T9}:0@200,created=9,coinbase=1"));
    assert_eq!((a.created, a.coinbase), (Some(9), Some(true)));
    for bad in [format!("{}:0@1", T170.to_uppercase()), format!("{T170}:01@1"), format!("{T170}:0@01"), format!("{T170}:0@+1"),
                format!("{T170}:0@1,value=+5"), format!("{T170}:0@1,value=05"), format!("{T170}:0@1,script=0A"), format!("{T170}:0@1=spent:0170"),
                format!("{T170}:0@1,created=09"), format!("{T170}:0@1,coinbase=2"), format!("{T170}:0@1,created=1,created=1"), format!(" {T170}:0@1")] {
        assert!(LivenessAsk::parse(&bad).is_err(), "{bad}");
    }
    assert_eq!(ask(&format!("{T170}:0@0")).at, Some(0), "0 itself is fine");
    // the CLI reads a proof FILE as raw bytes or LOWER-case hex; the C ABI / WASM take exact lower-case hex only
    let b = proof("b170_0_at200.hex");
    assert_eq!(liveness_bytes(&b).unwrap(), b);
    assert!(liveness_bytes(hex(&b).to_uppercase().as_bytes()).is_err());
    assert_eq!(liveness_hex(&hex(&b)).unwrap(), b);
    for bad in [format!("{}\n", hex(&b)), format!(" {}", hex(&b)), hex(&b).to_uppercase(), String::from_utf8_lossy(&b).to_string(), String::new()] {
        assert!(liveness_hex(&bad).is_err());
    }
}

/// Item 1 of the 15.11.2026 review: `created=` / `coinbase=` pin the coin, compared before the answer.
#[test]
fn created_and_coinbase_pin_the_coin() {
    let (cs, uf) = state();
    let b9 = proof("b9_0_at200.hex");
    check_liveness(&cs, uf, &b9, &ask(&format!("{T9}:0@200=spent,created=9,coinbase=1"))).unwrap();
    let e = check_liveness(&cs, uf, &b9, &ask(&format!("{T9}:0@200=spent,created=10"))).unwrap_err();
    assert!(e.contains("created at height 9, not the one asked"), "{e}");
    let e = check_liveness(&cs, uf, &b9, &ask(&format!("{T9}:0@200,coinbase=0"))).unwrap_err();
    assert!(e.contains("coinbase"), "{e}");
    check_liveness(&cs, uf, &proof("b170_0_at200.hex"), &ask(&format!("{T170}:0@200=unspent,created=170,coinbase=0"))).unwrap();
}

/// A synthetic chain: block 1's coinbase output (BIP30-TRACKED, flag bit 1) is spent at 2, and the SAME transaction
/// is mined again at 3 (the shape possible on mainnet from 1,983,702). Two leaves, one outpoint.
fn recreated() -> (tempdir::Dir, Vec<libzync::history::Rec>, ChainState, lv::Coin, lv::Coin) {
    use libzync::history::Rec;
    let blk = |h: u32| -> serde_json::Value { serde_json::from_slice(&std::fs::read(format!("{}/../../prover/testdata/libzync/mainnet/block_{h}.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap() };
    let cb = blk(9)["coinbase_hex"].as_str().unwrap().to_string();
    let raw = unhex(&cb).unwrap();
    let mut o = 0;
    let t = libzync::facts::parse_tx(&raw, &mut o).unwrap();
    let txid = libzync::facts::txid(&t);
    let d = tempdir::Dir::new("lv-recreated");
    let times = [vec![1_231_006_505u32], vec![1_231_006_505, 1_231_469_665], vec![1_231_006_505, 1_231_469_665, 1_231_469_744]];
    for (h, rt) in [(1u32, &times[0]), (3, &times[2])] {
        let j = serde_json::json!({ "height": h, "coinbase_hex": cb, "txs": [], "recent_times": rt });
        std::fs::write(d.0.join(format!("block_{h}.json")), j.to_string()).unwrap();
    }
    let coin = |h: u32, mtp: u32| lv::Coin { txid, vout: 0, value: t.outputs[0].0, script_pubkey: t.outputs[0].1.clone(), height: h,
                                              coinbase: true, tracked: true, creation_mtp: mtp };
    let (old, new) = (coin(1, 1_231_006_505), coin(3, 1_231_469_665));
    let v = old.value;
    let recs = vec![
        Rec { height: 0, hash: [10; 32], value: 0, flips: vec![], appends: vec![], undo: None },
        Rec { height: 1, hash: [11; 32], value: v, flips: vec![], appends: vec![old.leaf()], undo: None },
        Rec { height: 2, hash: [12; 32], value: 0, flips: vec![(0, 1, 0)], appends: vec![], undo: None },
        Rec { height: 3, hash: [13; 32], value: v, flips: vec![], appends: vec![new.leaf()], undo: None },
    ];
    let r = libzync::history::replay(&recs, 3, 3).unwrap();
    let mut cs = genesis_anchor(Image::by_id("mainnet", "8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4").unwrap());
    cs.profile |= PROFILE_COINLOG;
    cs.height = 3;
    cs.utxo_roots = r.log.roots();
    cs.utxo_leaves = r.log.len();
    cs.utxo_value = v;
    cs.block_roots = libzync::history::block_path(&r.block_leaves, 0).1;
    cs.block_leaves = r.block_leaves.len() as u64;
    (d, recs, cs, old, new)
}

mod tempdir {
    pub struct Dir(pub std::path::PathBuf);
    impl Dir {
        pub fn new(tag: &str) -> Dir {
            let p = std::env::temp_dir().join(format!("{tag}-{}-{:?}", std::process::id(), std::thread::current().id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Dir(p)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
}

#[test]
fn a_recreated_outpoint_is_pinned_by_created_and_the_producer_names_the_latest() {
    let (d, recs, cs, old, new) = recreated();
    let uf = swiftsync_verify::pin::NO_UNDO;
    let mut shown = new.txid;
    shown.reverse();
    let op = hex(&shown);
    // the producer, asked by outpoint only, gives the LATEST coin (unspent); pinned to 1, the old (spent) one
    let src = coin_source::Source::Dir(d.0.clone());
    let p_new = prove(&recs, 3, 3, &src, &new.txid, 0, None).unwrap();
    assert_eq!(lv::decode(&p_new).unwrap().coin, new);
    let p_old = prove(&recs, 3, 3, &src, &new.txid, 0, Some(1)).unwrap();
    assert_eq!(lv::decode(&p_old).unwrap().coin, old);
    assert!(new.tracked && lv::decode(&p_new).unwrap().coin.tracked, "the BIP30-tracked reading is the one the log holds");
    // without created=, BOTH answers verify: the hole the review found
    assert_eq!(check_liveness(&cs, uf, &p_new, &ask(&format!("{op}:0@3=unspent"))).unwrap().0, "unspent");
    assert_eq!(check_liveness(&cs, uf, &p_old, &ask(&format!("{op}:0@3=spent"))).unwrap().0, "spent:2:1:0");
    // with it, only the coin asked about
    assert!(check_liveness(&cs, uf, &p_new, &ask(&format!("{op}:0@3=unspent,created=3"))).is_ok());
    assert!(check_liveness(&cs, uf, &p_old, &ask(&format!("{op}:0@3=spent,created=3"))).unwrap_err().contains("not the one asked"));
    assert!(check_liveness(&cs, uf, &p_old, &ask(&format!("{op}:0@3=spent,created=1"))).is_ok());
    // tracked=true is bound like every other field: the same coin read untracked is another leaf
    let mut w = lv::decode(&p_new).unwrap();
    w.coin.tracked = false;
    assert!(check_liveness(&cs, uf, &lv::encode(&w), &ask(&format!("{op}:0@3"))).unwrap_err().contains("not in the claimed state"));
    let chain = lv::Chain { height: 3, utxo_roots: &cs.utxo_roots, utxo_leaves: cs.utxo_leaves, block_roots: &cs.block_roots,
                            block_leaves: cs.block_leaves, undo_from: uf };
    assert_eq!(flips_refused(&p_new, &chain) + flips_refused(&p_old, &chain), 8 * (p_new.len() + p_old.len()));
    // below the tip: after block 2 the old coin is spent and the new one does not exist yet
    let p2 = prove(&recs, 3, 2, &src, &new.txid, 0, None).unwrap();
    assert_eq!(lv::decode(&p2).unwrap().coin, old, "the latest coin AFTER block 2 is the old one");
    assert_eq!(check_liveness(&cs, uf, &p2, &ask(&format!("{op}:0@2=spent:2:1:0,created=1"))).unwrap().0, "spent:2:1:0");
}

/// Every single-bit flip of `b` refused by the proof check alone; returns how many were tried.
fn flips_refused(b: &[u8], chain: &lv::Chain) -> usize {
    assert!(lv::verify(b, chain).is_ok());
    let mut n = 0;
    for i in 0..b.len() {
        for bit in 0..8 {
            let mut x = b.to_vec();
            x[i] ^= 1 << bit;
            assert!(lv::verify(&x, chain).is_err(), "byte {i} bit {bit} accepted");
            n += 1;
        }
    }
    n
}

/// The 0x05 block leaf (with the undo hash, docs/libzync/UNDO.md) opened on REAL host data: a mainnet 1..200 run of a
/// development image with the undo commitment (`vectors/undo_dev_1_200_*`, an execute-mode run on hazync-coord,
/// 14.11.2026, its receipts dev-mode, so the chain state is the host's JSON, not a verified receipt). A v0.26.0
/// (46666b6f) proof with history waits on the board's first run.
#[test]
fn undo_leaf_openings_on_host_data() {
    let recs = libzync::history::decode_all(&std::fs::read(zapp_path("undo_dev_1_200_history.bin")).unwrap()).unwrap();
    assert!(recs[1..].iter().all(|r| r.undo.is_some()) && recs[0].undo.is_none());
    let cs: ChainState = serde_json::from_slice(&std::fs::read(zapp_path("undo_dev_1_200_state.json")).unwrap()).unwrap();
    let uf = swiftsync_verify::undo_from_of(&cs.self_id, &[]);
    assert_eq!((cs.height, uf), (200, 1));
    let chain = lv::Chain { height: cs.height, utxo_roots: &cs.utxo_roots, utxo_leaves: cs.utxo_leaves, block_roots: &cs.block_roots,
                            block_leaves: cs.block_leaves, undo_from: uf };
    let mut n = 0;
    for (f, at, want) in [("b9_0_at169.hex", 169, "unspent"), ("b9_0_at200.hex", 170, "spent:170:1:0"), ("b170_1_at170.hex", 170, "unspent"),
                          ("b170_1_at200.hex", 181, "spent:181:1:0"), ("b170_0_at200.hex", 199, "unspent")] {
        let coin = lv::decode(&proof(f)).unwrap().coin;
        let b = make_liveness(&recs, 200, at, &coin).unwrap();
        let p = lv::decode(&b).unwrap();
        assert!(p.opening.as_ref().unwrap().undo.is_some(), "a 0x05 leaf is opened with its undo hash");
        let mut shown = coin.txid;
        shown.reverse();
        assert_eq!(check_liveness(&cs, uf, &b, &ask(&format!("{}:{}@{at}", hex(&shown), coin.vout))).unwrap().0, want, "{f} at {at}");
        // the same opening without its undo hash, or under a pre-undo rule, is refused
        let mut w = p.clone();
        w.opening.as_mut().unwrap().undo = None;
        assert!(lv::verify(&lv::encode(&w), &chain).is_err());
        assert!(lv::verify(&b, &lv::Chain { undo_from: swiftsync_verify::pin::NO_UNDO, ..chain_copy(&chain) }).is_err());
        n += flips_refused(&b, &chain);
    }
    eprintln!("undo openings: {n} single-bit flips, all refused");
}

fn chain_copy<'a>(c: &lv::Chain<'a>) -> lv::Chain<'a> {
    lv::Chain { height: c.height, utxo_roots: c.utxo_roots, utxo_leaves: c.utxo_leaves, block_roots: c.block_roots, block_leaves: c.block_leaves,
                undo_from: c.undo_from }
}

/// Above block 200: the regtest base chain of the consensus rig (Core 28.0 regtest, -testactivationheight=bip34@110),
/// 0..232, proved in execute mode on hazync-coord (15.11.2026) by a development image with the undo commitment; the
/// proofs in vectors/regtest_* were written by `zapp-liveness prove ... --bitcoin-cli` against a copy of that node.
#[test]
fn above_block_200_on_regtest() {
    let cs: ChainState = serde_json::from_slice(&std::fs::read(zapp_path("regtest_0_232_state.json")).unwrap()).unwrap();
    let uf = swiftsync_verify::undo_from_of(&cs.self_id, &[]);
    assert_eq!(cs.height, 232);
    let chain = lv::Chain { height: cs.height, utxo_roots: &cs.utxo_roots, utxo_leaves: cs.utxo_leaves, block_roots: &cs.block_roots,
                            block_leaves: cs.block_leaves, undo_from: uf };
    let cases: Vec<(String, String, String)> = serde_json::from_slice(&std::fs::read(zapp_path("regtest_cases.json")).unwrap()).unwrap();
    assert!(cases.len() >= 3);
    let mut n = 0;
    for (f, q, want) in &cases {
        let b = proof(f);
        assert_eq!(&check_liveness(&cs, uf, &b, &ask(q)).unwrap_or_else(|e| panic!("{f}: {e}")).0, want, "{f}");
        n += flips_refused(&b, &chain);
    }
    eprintln!("regtest above 200: {} proofs, {n} single-bit flips, all refused", cases.len());
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

/// Review 11-18: the facts JSON of the C ABI and WASM is `{"liveness": [...]}` and nothing else. Before
/// `facts_from_json` they read `v.get("liveness")` and dropped every other key unread (a misspelt key beside the real
/// one). The entry points' own cases: ffi/test_liveness.c and wasm/test-liveness.mjs (`facts JSON:`).
#[test]
fn the_facts_json_takes_only_its_documented_key() {
    let f = r#"{"proof": "00", "ask": "q"}"#;
    let ok = facts_from_json(format!(r#"{{"liveness": [{f}]}}"#).as_bytes()).unwrap();
    assert_eq!((ok.len(), ok[0].proof.as_str(), ok[0].ask.as_str()), (1, "00", "q"));
    for (j, why) in [
        (format!(r#"{{"liveness": [{f}], "livenes": [{f}]}}"#), "unknown field `livenes`"),
        (format!(r#"{{"livenes": [{f}]}}"#), "unknown field `livenes`"),
        (format!(r#"{{"liveness": [{f}], "extra": 1}}"#), "unknown field `extra`"),
        (format!(r#"{{"liveness": [{f}], "expect": "200:00"}}"#), "\"expect\" inside the facts JSON is refused"),
        // in any form, null too (review 11-18 round 2): the old code refused any "expect" key
        (format!(r#"{{"liveness": [{f}], "expect": null}}"#), "\"expect\" inside the facts JSON is refused"),
        (format!(r#"{{"expect": null, "liveness": [{f}]}}"#), "\"expect\" inside the facts JSON is refused"),
        ("{}".to_string(), "has no \"liveness\""),
        (format!("[{f}]"), "not an object"),
        ("null".to_string(), "not an object"),
        (format!("[[{f}]]"), "not an object"),
        (r#"{"liveness": [["00", "q"]]}"#.to_string(), "each liveness fact is an object"),
        (r#"{"liveness": []}"#.to_string(), "no liveness facts"),
        (r#"{"liveness": [{"proof": "00", "ask": "q", "extra": 1}]}"#.to_string(), "unknown field `extra`"),
        (r#"{"liveness": [{"proof": "00", "ask": "q"}], "liveness": []}"#.to_string(), "duplicate field `liveness`"),
        (r#"{"liveness": [{"proof": "00", "proof": "01", "ask": "q"}]}"#.to_string(), "duplicate field `proof`"),
        (format!(r#"{{"expect": "200:00", "liveness": [{f}], "expect": "200:00"}}"#), "\"expect\" inside the facts JSON is refused"),
    ] {
        let e = facts_from_json(j.as_bytes()).unwrap_err();
        assert!(e.contains(why), "{j}: {e}");
    }
}
