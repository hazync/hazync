//! The liveness zAPP on the ENGINE chain (engine/VERIFIER.md, roadmap 2.5): the engine's proof of mainnet 1..200
//! (node format 2), liveness proofs made from ITS history (vectors/engine, `zapp-liveness prove`), each checked against
//! the verified engine state: the same claims and refusals as the RISC Zero chain's vectors/run.sh. `--features engine`.
#![cfg(feature = "engine")]
use swiftsync_verify::*;
use zapp_liveness::*;

const E: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../swiftsync-verify/tests/data/engine200");
const T9: &str = "0437cd7f8525ceed2324359c2d0ba26006d92d856a9c20fa0241106ee5a597c9";
const T170: &str = "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16";

fn rd(p: String) -> Vec<u8> {
    std::fs::read(&p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn state() -> (ChainState, u32) {
    let id = String::from_utf8(rd(format!("{E}/image_id"))).unwrap();
    let img = Image::by_network("mainnet").unwrap().with_method_id(id.trim()).unwrap().with_engine_key(rd(format!("{E}/vk.bin")));
    let r = verify(&Inputs { receipt: &rd(format!("{E}/proof.hze")), image: img, anchor: None, now: 1_800_000_000 });
    assert_eq!(r.verdict, Verdict::Demo, "{:?}", r.checks.iter().filter(|c| !c.0).collect::<Vec<_>>());
    (r.state.unwrap(), r.undo_from)
}

fn proof(n: &str) -> Vec<u8> {
    liveness_bytes(&rd(format!("{}/vectors/engine/{n}.hex", env!("CARGO_MANIFEST_DIR")))).unwrap()
}

fn check(cs: &ChainState, uf: u32, n: &str, q: String) -> Result<(String, String), String> {
    check_liveness(cs, uf, &proof(n), &LivenessAsk::parse(&q).unwrap())
}

#[test]
fn liveness_on_the_engine_chain() {
    let (cs, uf) = state();
    assert_eq!(cs.profile & PROFILE_NODE2, PROFILE_NODE2, "the engine chain is node format 2");
    for (n, q) in [
        ("b9_0_at200", format!("{T9}:0@200=spent:170:1:0")),
        ("b170_0_at200", format!("{T170}:0@tip=unspent,value=1000000000")),
        ("b170_1_at200", format!("{T170}:1@200=spent:181")),
        ("b170_1_at170", format!("{T170}:1@170=unspent")),
        ("b9_0_at200", format!("{T9}:0@200=spent,created=9,coinbase=1")),
    ] {
        let r = check(&cs, uf, n, q.clone());
        eprintln!("  {q}: {r:?}");
        assert!(r.is_ok(), "{n} {q}: {r:?}");
    }
    for (n, q, why) in [
        ("b9_0_at200", format!("{T9}:0@200=unspent"), "spent coin claimed unspent"),
        ("b170_0_at200", format!("{T170}:0@200=spent"), "unspent coin claimed spent"),
        ("b170_0_at200", format!("{T170}:0@200,value=1000000001"), "wrong amount"),
        ("b170_0_at200", format!("{T170}:0@200,script=51"), "wrong script"),
        ("b170_0_at200", format!("{T170}:1@200"), "another outpoint"),
        ("b170_0_at200", format!("{T170}:0@199"), "another height"),
        ("b9_0_at200", format!("{T9}:0@200=spent,created=10"), "another creation height"),
    ] {
        assert!(check(&cs, uf, n, q.clone()).is_err(), "{why}: {q} was accepted");
    }
    // a RISC Zero (format 1) liveness proof does not hold against the engine (format 2) state
    let old = liveness_bytes(&rd(format!("{}/vectors/b9_0_at200.hex", env!("CARGO_MANIFEST_DIR")))).unwrap();
    assert!(check_liveness(&cs, uf, &old, &LivenessAsk::parse(&format!("{T9}:0@200=spent:170:1:0")).unwrap()).is_err());
}
