//! The assume checkers on the ENGINE chain (engine/VERIFIER.md, roadmap 2.5): the engine's proof of mainnet 1..200
//! verified, then Core v31.1.0's real UTXO snapshot at 200 (vectors/utxo_200.dat) checked against its live-coin hash,
//! and assumevalid on its tip. Neither input depends on the node format, so they are the RISC Zero chain's own vectors.
#![cfg(feature = "engine")]
use swiftsync_verify::*;
use zapp_assume::*;

const E: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../swiftsync-verify/tests/data/engine200");
const SNAP_200: &[u8] = include_bytes!("../vectors/utxo_200.dat");
const SNAP_199: &[u8] = include_bytes!("../vectors/utxo_199.dat");
const HDRS: &[u8] = include_bytes!("../vectors/headers_0_200.bin");

fn rd(p: String) -> Vec<u8> {
    std::fs::read(&p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn state() -> ChainState {
    let id = String::from_utf8(rd(format!("{E}/image_id"))).unwrap();
    let img = Image::by_network("mainnet").unwrap().with_method_id(id.trim()).unwrap().with_engine_key(rd(format!("{E}/vk.bin")));
    let r = verify(&Inputs { receipt: &rd(format!("{E}/proof.hze")), image: img, anchor: None, now: 1_800_000_000 });
    assert_eq!(r.verdict, Verdict::Demo);
    r.state.unwrap()
}

#[test]
fn assume_on_the_engine_chain() {
    let cs = state();
    let hc = HeaderChain::bind(HDRS, &cs.tip_hash, cs.height).unwrap();
    let r = check_snapshot(&cs, SNAP_200, &hc, &Tracked::new()).unwrap();
    eprintln!("  snapshot 200: {} coins, {} sat, hash_serialized_3 {}", r.coins, r.value, r.hash_serialized_3);
    assert_eq!((r.coins, r.value), (204, 1_000_000_000_000), "Core's UTXO set at 200 is the engine-proven set");
    // Core v31.1.0's own `gettxoutsetinfo hash_serialized_3` at 200 (the RISC Zero tests pin the same value)
    assert_eq!(r.hash_serialized_3, "a51f51aea7cf42fc9f1ab491532de766675e983fde7166fa6273a6d6a6439d67");
    // the snapshot at 199 is not the set at the proven tip
    assert!(check_snapshot(&cs, SNAP_199, &hc, &Tracked::new()).is_err());
    // one coin's amount changed: refused
    let mut bad = SNAP_200.to_vec();
    let n = bad.len() - 30;
    bad[n] ^= 1;
    assert!(check_snapshot(&cs, &bad[..], &hc, &Tracked::new()).is_err());
    let av = assume_valid(&cs, Some(&hc), None).unwrap();
    eprintln!("  assumevalid: {av:?}");
}
