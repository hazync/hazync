//! The verdict paths of this zAPP's library, one test per way a FAIL could become DEMO or OK, or a DEMO OK
//! (docs/libzync/ZAPP_TESTING.md §3.5, the cargo-mutants pass of 17.11.2026): `verify_liveness` and
//! `check_liveness_fact` end to end on the real mainnet 1..200 proof (8888d59c, DEMO: below minimum work), and under the
//! tests-only `regtest-test` feature on the zAPP regtest harness's real receipt (image eb2cd424, OK with its tip
//! expected). Each assertion names the check it holds in place.

use swiftsync_verify::*;
use zapp_liveness::*;

const T9: &str = "0437cd7f8525ceed2324359c2d0ba26006d92d856a9c20fa0241106ee5a597c9";
const T170: &str = "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16";
const H200: &str = "000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320";

fn zapp_file(f: &str) -> Vec<u8> {
    std::fs::read(format!("{}/vectors/{f}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn receipt_1_200() -> Vec<u8> {
    std::fs::read(format!("{}/../../swiftsync-verify/vectors/mainnet_1_200_8888d59c.rcpt", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn image_1_200() -> &'static Image {
    image_for("mainnet", "8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4", false, false).unwrap()
}

/// A proof file's bytes as the C ABI and WASM take them: lower-case hex.
fn proof_hex(f: &str) -> String {
    hex(&liveness_bytes(&zapp_file(f)).unwrap())
}

fn fact(f: &str, ask: String) -> LivenessFact {
    LivenessFact { proof: proof_hex(f), ask }
}

fn state() -> (ChainState, u32) {
    let r = verify(&Inputs { receipt: &receipt_1_200(), image: image_1_200(), anchor: None, now: 1_800_000_000 });
    (r.state.unwrap(), r.undo_from)
}

/// `script=` and `coinbase=` given twice are refused, as `value=` and `created=` are (one spelling per question).
#[test]
fn repeated_script_or_coinbase_options_are_refused() {
    for bad in [format!("{T170}:0@200,script=00,script=00"), format!("{T170}:0@200,coinbase=0,coinbase=0")] {
        assert!(LivenessAsk::parse(&bad).unwrap_err().contains("repeated"), "{bad}");
    }
}

/// A state whose image has no coin log (profile bit 4 clear) proves nothing about a coin.
#[test]
fn a_chain_without_the_coin_log_is_refused() {
    let (mut cs, uf) = state();
    let ask = LivenessAsk::parse(&format!("{T170}:0@200=unspent")).unwrap();
    let b = liveness_bytes(&zapp_file("b170_0_at200.hex")).unwrap();
    check_liveness(&cs, uf, &b, &ask).unwrap();
    cs.profile &= !PROFILE_COINLOG;
    assert!(check_liveness(&cs, uf, &b, &ask).unwrap_err().contains("coin log"));
}

/// check_liveness_fact: a true claim holds (as "<token>: <line>"), a false one does not, and `need_claim` refuses a
/// question that claims nothing while accepting one that does.
#[test]
fn liveness_facts_hold_only_when_true() {
    let (cs, uf) = state();
    let ok = check_liveness_fact(&cs, uf, &fact("b9_0_at200.hex", format!("{T9}:0@200=spent:170:1:0")), true).unwrap();
    assert!(ok.starts_with("spent:170:1:0: coin ") && ok.contains("SPENT at height 170"), "{ok}");
    let e = check_liveness_fact(&cs, uf, &fact("b9_0_at200.hex", format!("{T9}:0@200=unspent")), true).unwrap_err();
    assert!(e.contains("the proof shows spent:170:1:0"), "{e}");
    let e = check_liveness_fact(&cs, uf, &fact("b9_0_at200.hex", format!("{T9}:0@200")), true).unwrap_err();
    assert!(e.contains("claims no answer"), "{e}");
    let ok = check_liveness_fact(&cs, uf, &fact("b9_0_at200.hex", format!("{T9}:0@200")), false).unwrap();
    assert!(ok.starts_with("spent:170:1:0: "), "{ok}");
    // the fact's proof: lower-case hex only; a question that does not parse
    let mut f = fact("b9_0_at200.hex", format!("{T9}:0@200=spent"));
    f.proof = f.proof.to_uppercase();
    assert!(check_liveness_fact(&cs, uf, &f, true).is_err());
    assert!(check_liveness_fact(&cs, uf, &fact("b9_0_at200.hex", format!("{T9}:0")), true).is_err());
}

/// verify_liveness on the real 1..200 proof: DEMO (below minimum work) with true facts, FAIL with any false one or a
/// receipt that does not verify, and the facts' results reported one per fact.
#[test]
fn verify_liveness_on_the_real_proof() {
    let r = receipt_1_200();
    let img = image_1_200();
    let now = 1_800_000_000;
    let good = [fact("b9_0_at200.hex", format!("{T9}:0@200=spent:170:1:0")), fact("b170_0_at200.hex", format!("{T170}:0@tip=unspent"))];
    let o = verify_liveness(&r, img, false, None, now, &good, None, true);
    assert_eq!(o.report.verdict, Verdict::Demo, "{:?} {:?}", o.report.checks, o.facts);
    assert!(o.facts.len() == 2 && o.facts.iter().all(|f| f.is_ok()), "{:?}", o.facts);
    assert!(!o.tip_checked);
    // the tip expected: still DEMO (low work), now with the tip checked
    let o = verify_liveness(&r, img, false, None, now, &good, Some(&format!("200:{H200}")), true);
    assert_eq!((o.report.verdict, o.tip_checked), (Verdict::Demo, true), "{:?}", o.report.checks);
    // one false fact among true ones: FAIL, the false one an Err
    let bad = [good[0].clone(), fact("b170_0_at200.hex", format!("{T170}:0@tip=spent"))];
    let o = verify_liveness(&r, img, false, None, now, &bad, None, true);
    assert_eq!(o.report.verdict, Verdict::Fail);
    assert!(o.facts[0].is_ok() && o.facts[1].is_err(), "{:?}", o.facts);
    // a fact that claims nothing where a claim is needed: FAIL
    let o = verify_liveness(&r, img, false, None, now, &[fact("b9_0_at200.hex", format!("{T9}:0@200"))], None, true);
    assert_eq!(o.report.verdict, Verdict::Fail);
    // a wrong expected tip: FAIL, and no fact is checked against that state
    let o = verify_liveness(&r, img, false, None, now, &good, Some(&format!("199:{H200}")), true);
    assert_eq!(o.report.verdict, Verdict::Fail);
    assert!(o.facts.iter().all(|f| f.as_ref().is_err_and(|e| e.contains("NOT checked"))), "{:?}", o.facts);
    // a receipt that does not verify: FAIL, nothing checked
    let mut t = r.clone();
    let n = t.len();
    t[n / 2] ^= 1;
    let o = verify_liveness(&t, img, false, None, now, &good, None, true);
    assert_eq!(o.report.verdict, Verdict::Fail);
    assert!(o.facts.iter().all(|f| f.is_err()), "{:?}", o.facts);
    // no facts: the core verdict as it is
    let o = verify_liveness(&r, img, false, None, now, &[], None, true);
    assert!(o.report.verdict == Verdict::Demo && o.facts.is_empty());
}

/// The OK verdict and its two demotions, on the regtest harness's real receipt (a's 112, GPU-proved, image eb2cd424;
/// Core's regtest has no minimum work, so a genesis-anchored proof reaches OK): OK only with facts that hold AND the
/// tip compared; DEMO with the tip not compared; OK with no facts at all (the core verdict, nothing to demote).
/// The block hash and the claim are Core's (scripts/zapp-regtest/oracle; zapps/liveness/test-regtest.sh).
#[cfg(feature = "regtest-test")]
#[test]
fn verify_liveness_ok_needs_the_tip_on_regtest() {
    const A112: &str = "67c1d2e718067bba4ffd07a10f2ce27f5e6b55702eb2c766fafbf36db6160574";
    const B112: &str = "4395baa44c0364ae2f6f415e439b4ce82a5d9e77cb9ad80bdf51eebb2406ce16";
    const X: &str = "e0b6ab75f1ce5f1dcf1a77ded4bf03eb962f50365ee8f77a4b269802dadd5a9f:1";
    let r = std::fs::read(format!("{}/../../scripts/zapp-regtest/receipts/a/block_112.rcpt", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let img = image_for("regtest", "", false, false).unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let x = [fact("regtest_x_spent_a112.hex", format!("{X}@112=spent:111:2:0"))];
    let ok = verify_liveness(&r, img, false, None, now, &x, Some(&format!("112:{A112}")), true);
    assert_eq!((ok.report.verdict, ok.tip_checked), (Verdict::Ok, true), "{:?} {:?}", ok.report.checks, ok.facts);
    // the tip not compared: at best DEMO
    let demo = verify_liveness(&r, img, false, None, now, &x, None, true);
    assert_eq!((demo.report.verdict, demo.tip_checked), (Verdict::Demo, false), "{:?}", demo.report.checks);
    // no facts: the core receipt's own OK, untouched
    assert_eq!(verify_liveness(&r, img, false, None, now, &[], None, true).report.verdict, Verdict::Ok);
    // the other branch's block 112 expected (the reorg): FAIL
    assert_eq!(verify_liveness(&r, img, false, None, now, &x, Some(&format!("112:{B112}")), true).report.verdict, Verdict::Fail);
    // a false claim with the tip compared: FAIL, not OK
    let lie = [fact("regtest_x_spent_a112.hex", format!("{X}@112=unspent"))];
    assert_eq!(verify_liveness(&r, img, false, None, now, &lie, Some(&format!("112:{A112}")), true).report.verdict, Verdict::Fail);
}
