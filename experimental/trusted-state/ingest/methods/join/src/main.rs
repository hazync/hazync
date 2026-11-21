//! EXPERIMENTAL Trusted State JOIN RECEIPT (docs/libzync/TRUSTED_STATE.md §11.3, §13.3). MEASUREMENT ONLY, its own small
//! image; nothing of it is in the core. Verifies three receipts, each by env::verify under an image id given here
//! (and committed): main@H (the core image, genesis-anchored), the ingest of the anchor (the combine image, whose
//! journal names its chunk and BIP30 images, checked against the ones given here), and the anchored lineage's tip T
//! (the core image). Rebuilds the anchor A from the ingest journal and main@H's header fields, requires the anchored
//! tip to rest on exactly A (anchor_id == link_digest(A)) and the ingest's coins to be main@H's live coins (H, tip,
//! value, live set), and commits the joined statement.
#![no_main]
extern crate alloc;
use alloc::vec::Vec;
use risc0_zkvm::guest::env;
use sha2::{Digest, Sha256};
risc0_zkvm::guest::entry!(main);

#[path = "../../../join_state.rs"]
#[allow(dead_code)]
mod join_state;
use join_state::*;

// cut bits of the `controls` build only (bip30/Cargo.toml)
const CUT_GENESIS: u32 = 1; // main@H's anchor_id is the genesis anchor given here
const CUT_TAIL: u32 = 2;    // the ingest journal names the chunk (and BIP30) images given here
const CUT_MATCH: u32 = 4;   // the ingest's (H, tip, value, live set) = main@H's
const CUT_LINK: u32 = 8;    // the anchored tip's anchor_id = link_digest(rebuilt A)
const CUT_SHAPE: u32 = 16;  // both states a v2 ChainState, the same profile, the tip above H, the tip not genesis-anchored

fn sha(b: &[u8]) -> [u8; 32] { Sha256::digest(b).into() }
fn wbytes(w: &[u32]) -> Vec<u8> { w.iter().flat_map(|x| x.to_le_bytes()).collect() }
fn link_digest(s: &JState) -> [u8; 32] {
    let mut z = s.clone();
    z.anchor_id = [0; 32];
    sha(&sha(&wbytes(&risc0_zkvm::serde::to_vec(&z).expect("ChainState serialises"))))
}
fn read_journal() -> Vec<u32> {
    let n: u32 = env::read();
    let mut v = alloc::vec![0u32; n as usize];
    if n > 0 { env::read_slice(&mut v); }
    v
}

fn main() {
    let c0 = env::cycle_count();
    #[cfg(feature = "controls")]
    let cut: u32 = env::read();
    #[cfg(not(feature = "controls"))]
    let cut: u32 = 0;
    let on = |bit: u32| cut & bit == 0;
    let main_image: [u32; 8] = env::read();
    let tip_image: [u32; 8] = env::read();
    let ingest_image: [u32; 8] = env::read();
    let chunk_image: [u32; 8] = env::read();
    let bip30_image: [u32; 8] = env::read(); // zero: the ingest built its BIP30 tree inline
    let genesis: [u8; 32] = env::read();
    let (jh, ji, jt) = (read_journal(), read_journal(), read_journal());
    env::verify(main_image, &jh).expect("main@H receipt");
    env::verify(ingest_image, &ji).expect("ingest receipt");
    env::verify(tip_image, &jt).expect("anchored tip receipt");
    let c1 = env::cycle_count();
    let main: JState = risc0_zkvm::serde::from_slice(&jh).expect("main@H: not a ChainState journal");
    let tip: JState = risc0_zkvm::serde::from_slice(&jt).expect("anchored tip: not a ChainState journal");
    let ing = decode_ingest(&ji).expect("not an ingest journal");
    if on(CUT_SHAPE) {
        for s in [&main, &tip] { assert!(s.kind == KIND_CHAIN && s.version == CHAIN_STATE_VERSION, "not a v2 ChainState"); }
        assert!(tip.profile == main.profile, "the two proofs are of other profiles");
        assert!(tip.height > ing.h, "the anchored tip is not above H");
        assert!(tip.anchor_id != genesis, "the anchored tip is genesis-anchored");
    }
    if on(CUT_GENESIS) { assert!(main.anchor_id == genesis, "main@H is not genesis-anchored"); }
    if on(CUT_TAIL) {
        let mut want: Vec<u32> = chunk_image.to_vec();
        let ok = ing.tail.len() >= 9 && ing.tail[..8] == want[..] && if bip30_image == [0; 8] { ing.tail.len() == 9 } else {
            want.push(ing.tail[8]);
            want.extend_from_slice(&bip30_image);
            ing.tail.len() == 18 && ing.tail[..17] == want[..]
        };
        assert!(ok, "the ingest journal does not name the chunk / BIP30 images given");
    }
    if on(CUT_MATCH) {
        assert!((ing.h, ing.tip, ing.value, ing.live_set, ing.live_hash) == (main.height, main.tip_hash, main.utxo_value, main.live_set, main.live_hash),
                "the ingest's (H, tip, value, live set) is not main@H's");
    }
    let a = rebuild_anchor(&ing, &main);
    let link = link_digest(&a);
    if on(CUT_LINK) { assert!(tip.anchor_id == link, "the anchored tip does not rest on the ingested anchor"); }
    let c2 = env::cycle_count();
    // the joined statement
    let mut out: Vec<u32> = Vec::new();
    let w8 = |o: &mut Vec<u32>, b: &[u8; 32]| for k in 0..8 { o.push(u32::from_le_bytes(b[4 * k..4 * k + 4].try_into().unwrap())); };
    for id in [&main_image, &tip_image, &ingest_image, &chunk_image, &bip30_image] { out.extend_from_slice(id); }
    w8(&mut out, &genesis);
    out.push(ing.h);
    w8(&mut out, &ing.tip);
    w8(&mut out, &link);
    out.push(tip.height);
    w8(&mut out, &tip.tip_hash);
    w8(&mut out, &tip.cum_work);
    out.push(tip.max_time);
    w8(&mut out, &sha(&wbytes(&jh)));
    w8(&mut out, &sha(&wbytes(&jt)));
    env::commit_slice(&out);
    let c3 = env::cycle_count();
    env::log(&alloc::format!("TSJ verify {} checks+rebuild+link {} commit {} H={} T={} cut={cut}", c1 - c0, c2 - c1, c3 - c2, ing.h, tip.height));
}
