//! The balance-statement zAPP on the ENGINE chain (engine/VERIFIER.md, roadmap 2.5): the engine's proof of mainnet
//! 1..200 verified, its counting trie at 200 (vectors/engine_200.trie: native prep's, whose state equals the
//! engine-proven one but for self_id; node format 2), a statement by two fresh keys (their balance facts are absence
//! proofs) checked against the verified state.
use swiftsync_verify::*;
use zapp_statement::*;

const E: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../swiftsync-verify/tests/data/engine200");

fn rd(p: String) -> Vec<u8> {
    std::fs::read(&p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn state() -> (ChainState, u32) {
    let id = String::from_utf8(rd(format!("{E}/image_id"))).unwrap();
    let img = Image::by_network("mainnet").unwrap().with_method_id(id.trim()).unwrap().with_engine_key(rd(format!("{E}/vk.bin")));
    let r = verify(&Inputs { receipt: &rd(format!("{E}/proof.hze")), image: img, anchor: None, now: 1_800_000_000 });
    assert_eq!(r.verdict, Verdict::Demo);
    (r.state.unwrap(), r.undo_from)
}

fn trie(n: &str) -> libzync::trie_store::TrieStore {
    libzync::trie_store::TrieStore::from_bytes(&rd(format!("{}/vectors/{n}", env!("CARGO_MANIFEST_DIR")))).unwrap()
}

fn key() -> bitcoin::PrivateKey {
    use std::io::Read;
    loop {
        let mut b = [0u8; 32];
        std::fs::File::open("/dev/urandom").unwrap().read_exact(&mut b).unwrap();
        if let Ok(k) = bitcoin::secp256k1::SecretKey::from_slice(&b) { return bitcoin::PrivateKey::new(k, bitcoin::Network::Bitcoin); }
    }
}

const ASK: Challenge = Challenge { purpose: Some("engine test 26.11.2026"), nonce: Some("e0e1") };

#[test]
fn statement_on_the_engine_chain() {
    let (cs, uf) = state();
    let t = trie("engine_200.trie");
    assert_eq!(libzync::coinlog::h_of(&t.root()), cs.trie_root, "the trie is the engine-proven state's (its root)");
    let (k1, k2) = (key(), key());
    let scripts = vec![scripts_for_key(&k1)[2].0.clone(), scripts_for_key(&k2)[3].0.clone()];   // P2WPKH, P2TR
    let mut st = draft(&cs, &t, &scripts, ASK.purpose.unwrap(), ASK.nonce.unwrap()).unwrap();
    assert_eq!(sign(&mut st, &[k1, k2]).unwrap(), 2);
    let c = check_statement(&cs, uf, &st, &ASK).unwrap();
    eprintln!("  statement holds: total {} sat over {} scripts, {} coins", c.total, c.scripts, c.coins);
    assert_eq!((c.total, c.scripts, c.coins, c.challenged), (0, 2, 0, true));
    let mut bad = st.clone();
    bad.total = 1;
    assert!(check_statement(&cs, uf, &bad, &ASK).is_err(), "claiming 1 sat");
}
