//! libzync on the ORIGINAL coordinator (ROADMAP H-B e). The coordinator (`coordinator/server.py`) and its worker
//! (`coordinator/hazync`) drive a host binary through three commands; a libzync host answers the same three, so the
//! board, claims, folds and the frontier work unchanged:
//!
//! | command | original host | libzync host |
//! |---|---|---|
//! | `prove-range-bridge <h>` | block h from `$HAZYNC_BRIDGE_OUT/bundle_<h>.json` | a LEAF: block h from prep's `leaf_<h>.bin` |
//! | `fold-range <l> <r> <out>` | fold two adjacent ranges | `libzync-join` (mode 18) |
//! | `verify-any <rcpt> [<lo>]` | `RANGE-OK lo hi in_tip out_tip ...` | the same line, tips = link digests |
//!
//! The tips the coordinator chains ranges by (out_tip of one == in_tip of the next) are link digests: in_tip = the
//! receipt's `anchor_id` (the digest of S(lo-1)), out_tip = `link_digest` of its own state S(hi). A join's guest
//! already requires the right range's anchor to be the left's state, so two ranges chain on the board exactly when
//! they would join. The genesis tip (`GENESIS_TIP` for a libzync coordinator) is `libzync-genesis-tip`.
//!
//! A libzync receipt does not state the height its range starts at (only the digest of the state before it), so
//! `verify-any` takes the claimed `<lo>` and checks it: the state before lo is genesis for lo = 1, else the `prev` of
//! `$HAZYNC_BRIDGE_OUT/leaf_<lo>.bin` (the coordinator holds every bundle), and its digest must be the receipt's
//! anchor_id. The coordinator only verifies (decision_coordinator_serves_never_computes): a digest is a hash.

use crate::{hex, link_digest, ChainState, KIND_CHAIN};

fn bridge_dir() -> String {
    std::env::var("HAZYNC_BRIDGE_OUT").unwrap_or_else(|_| "/root/bridge_bundles".into())
}

/// The state before block `lo`: genesis for 1, else the `prev` of the block's leaf bundle, else the prep run's own
/// `state_<lo-1>.json` in `$HAZYNC_LZ_STATE_DIR`: the same state, which the run that wrote the bundle also saved.
/// The board prunes a leaf once its block is proved below the frontier, but a stored range starting there must stay
/// re-verifiable (the nightly check-proofs), so the small state file outlives the bundle.
fn state_before(lo: u32) -> Result<ChainState, String> {
    if lo == 1 { return Ok(crate::genesis_anchor()); }
    let p = format!("{}/leaf_{lo}.bin", bridge_dir());
    if std::path::Path::new(&p).exists() {
        let b = crate::prep::leaf_read(&p);
        if b.prev.height + 1 != lo { return Err(format!("{p} is for block {}, not {lo}", b.prev.height + 1)); }
        return Ok(b.prev);
    }
    if let Ok(d) = std::env::var("HAZYNC_LZ_STATE_DIR") {
        let sp = format!("{d}/state_{}.json", lo - 1);
        if let Ok(t) = std::fs::read_to_string(&sp) {
            let mut st: ChainState = serde_json::from_str(&t).map_err(|e| format!("{sp}: {e}"))?;
            crate::stamp(&mut st); // by era, as leaf_read does (docs/HANDOVER.md §5)
            if st.height + 1 != lo { return Err(format!("{sp} is the state at {}, not before block {lo}", st.height)); }
            return Ok(st);
        }
    }
    Err(format!("no leaf bundle {p} (nor a state_{}.json in HAZYNC_LZ_STATE_DIR): the state before block {lo} is unknown here", lo - 1))
}

/// `libzync-genesis-tip`: the in_tip of every range that starts at block 1 (the coordinator's GENESIS_TIP).
pub(crate) fn genesis_tip_cmd() { println!("{}", hex(&link_digest(&crate::genesis_anchor()))); }

/// `prove-range-bridge <h>` on a libzync host: a leaf proof of block h from `$HAZYNC_BRIDGE_OUT/leaf_<h>.bin`, written
/// where the worker expects it (`range_<h>.bin` in the working directory).
pub(crate) fn prove_bridge_cmd(h: u32) {
    let p = format!("{}/leaf_{h}.bin", bridge_dir());
    assert!(std::path::Path::new(&p).exists(), "{p}: no leaf bundle for block {h} (the coordinator serves it as the block's witness)");
    std::env::set_var("HAZYNC_LEAF_BUNDLE", &p);
    let out = format!("lz_leaf_{h}");
    crate::lz::lz_prove_range_cmd(h, h, &out, "leaf", &out);
    // the engine's leaf (block_<h>.ovm, a pair) as the board's one file: HZE1, checked by verify-core (STAGE3 3.2)
    let src = std::path::PathBuf::from(format!("{out}/block_{h}.ovm"));
    crate::engine_backend::with(&out, |e| e.pack(&src, std::path::Path::new(&format!("range_{h}.bin"))));
    let _ = std::fs::remove_dir_all(&out);
    println!("leaf engine proof for block {h} -> range_{h}.bin");
}

/// `verify-any <proof> [<lo>] [--check-end]` on a libzync host: verifies the engine proof (HZE1) under the board's key
/// (HAZYNC_ENGINE_VK + HAZYNC_ENGINE_VK_PIN) and prints the coordinator's RANGE-OK line. Without `<lo>` the range is
/// reported as starting at hi (a leaf), which is checked the same way. `--check-end` (the board passes it) also requires the END state to be the board's own (see below) and
/// adds `end=board` to the line; the board refuses a line without it, so an older host that ignores the flag fails
/// closed. A contributor's self-test has no board states and does not pass it.
pub(crate) fn verify_any_cmd(path: &str, lo: Option<u32>, check_end: bool) {
    if let Err(e) = verify_any(path, lo, check_end) {
        eprintln!("REJECT {e}");
        std::process::exit(1);
    }
}

fn verify_any(path: &str, lo: Option<u32>, check_end: bool) -> Result<(), String> {
    // The board takes engine proof files only, under its pinned key. A host with no key verifies nothing: a refusal
    // naming the configuration, never a verdict (RISC Zero receipts, the board's before CUTOVER C8, are v0.27.1's).
    let raw = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    match crate::engine_backend::board_key() {
        Some(key) => verify_any_engine(&raw, key?, lo, check_end),
        None => Err("this host has no engine key to verify with (set HAZYNC_ENGINE_VK and HAZYNC_ENGINE_VK_PIN: a coordinator \
                     configuration error); a RISC Zero receipt is checked with v0.27.1's verifier".into()),
    }
}

/// The checks the board needs beyond the proof itself, and the RANGE-OK line: the claimed start, the anchor (the state
/// before lo), the end (the board's own state at hi, `--check-end`), the era.
fn range_ok(st: &ChainState, image: &[u32; 8], lo: Option<u32>, check_end: bool, era_lo: u32, fake: bool) -> Result<(), String> {
    let lo = lo.unwrap_or(st.height);
    if lo == 0 || lo > st.height { return Err(format!("claimed start {lo} for a state at {}", st.height)); }
    let before = state_before(lo)?;
    if link_digest(&before) != st.anchor_id {
        return Err(format!("the receipt is not anchored on the state before block {lo} (anchor_id {}, that state's digest {})",
                           hex(&st.anchor_id), hex(&link_digest(&before))));
    }
    // The END must be Bitcoin's too. The start check alone lets a prover take the board's real state before block N,
    // mine its own block N on it (difficulty 1 below 32,256: seconds of work), prove that, and have it accepted as
    // block N: the image and the anchor are right, only the block is not Bitcoin's. The board's prep states come from
    // its own node, so the receipt's state at hi must be the board's state at hi, every field (anchor_id aside: it
    // records where this receipt started). Unknown here means refused: a range is accepted only up to what prep has seen.
    if check_end {
        let after = state_before(st.height + 1)
            .map_err(|e| format!("the board has no state at block {} to check the receipt's end against ({e})", st.height))?;
        if link_digest(&after) != link_digest(st) || after.tip_hash != st.tip_hash {
            return Err(format!("the receipt ends on a state at block {} that is not the board's (its tip {}, the board's {}): \
                                not Bitcoin's chain", st.height, hex(&st.tip_hash), hex(&after.tip_hash)));
        }
    }
    // A range of a later era must start inside it, or at genesis (the handover's [1..H]); a range that starts in a
    // predecessor's era and ends in this one can only be a base step on a predecessor-stamped state, which the lineage
    // check above already refuses. Stated anyway: the board keys eras on it.
    if lo != 1 && lo < era_lo {
        return Err(format!("range starts at {lo}, before image {}'s first block {era_lo}", hex_id(image)));
    }
    // image= and lineage= sit BEFORE anchored=: hazync-lz-zapp-follow.sh matches a line ENDING in `anchored=1`
    println!("RANGE-OK lo={lo} hi={} in_tip={} out_tip={} out_leaves={} range_work=0 in_bhash={} out_bhash={} image={} lineage={} anchored={}{}{}",
             st.height, hex(&st.anchor_id), hex(&link_digest(st)), st.utxo_leaves, hex(&before.tip_hash), hex(&st.tip_hash),
             hex_id(image), hex(&st.lineage), (lo == 1) as u8, if check_end { " end=board" } else { "" }, if fake { " fake=1" } else { "" });
    Ok(())
}

/// verify-any for an engine board: the HZE1 file verified as every reader verifies it (verify-core, under the pinned
/// key; the image is the one the key names), the journal a ChainState of that image with no lineage (an engine chain
/// from genesis), this board's profile, then the range checks (`range_ok`).
fn verify_any_engine(raw: &[u8], key: (Vec<u8>, [u8; 32]), lo: Option<u32>, check_end: bool) -> Result<(), String> {
    use hazync_engine_verify as ev;
    if !ev::is_engine_proof(raw) {
        return Err("not an engine proof file (HZE1): this coordinator verifies engine proofs (METHOD_ID MISMATCH: a RISC Zero receipt?)".into());
    }
    let (key, pin) = key;
    let vk = ev::decode_vk(&key).map_err(|e| format!("the board's engine key does not decode: {e}"))?;
    let id = ev::image_id_of(&vk);
    let (proof, journal) = ev::unpack(raw).map_err(|e| format!("not an engine proof file: {e}"))?;
    ev::verify_full_bytes(&key, &pin, proof, journal, &id).map_err(|e| engine_refusal(e, journal, &id))?;
    if journal.len() % 4 != 0 { return Err("journal is not whole words".into()); }
    let w: Vec<u32> = journal.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    let st: ChainState = hazync_journal::serde::from_slice(&w).map_err(|e| format!("journal is not a ChainState: {e}"))?;
    let image: [u32; 8] = core::array::from_fn(|i| u32::from_le_bytes(id[4 * i..4 * i + 4].try_into().unwrap()));
    if st.kind != KIND_CHAIN { return Err(format!("journal kind {:#x} is not a chain state", st.kind)); }
    if st.self_id != image { return Err("the state's self_id is not the image that verified it".into()); }
    if st.lineage != [0u8; 32] {
        return Err(format!("lineage {} is not zero: this board takes one engine image from genesis (no handover)", hex(&st.lineage)));
    }
    if st.profile != crate::lz::expected_profile() {
        return Err(format!("profile {:#x}, this coordinator's image commits {:#x}", st.profile, crate::lz::expected_profile()));
    }
    range_ok(&st, &image, lo, check_end, 1, false)
}

/// Why an engine proof did not verify, in the words the board sorts by (server.py verify_receipt): "MISMATCH" only for
/// a proof another guest build made (the common contributor mistake, which the worker reports as a build problem),
/// a forged, tampered or corrupt one as an invalid proof, and the board's own key as its configuration. Another build
/// is told apart by the proof's journal, read WITHOUT trusting it: a chain state naming another image. The verdict does
/// not depend on it (the proof is refused either way), only the reason (the RISC Zero board sorted by its receipt's
/// claimed image the same way). Found by the Gate 3 dry run (engine/results/board_dryrun): every failed engine proof
/// said "MISMATCH or invalid proof", so a tampered proof was reported as another build.
fn engine_refusal(e: hazync_engine_verify::Refusal, journal: &[u8], id: &[u8; 32]) -> String {
    use hazync_engine_verify::Refusal;
    if matches!(e, Refusal::VkPin | Refusal::Vk) {
        return format!("the board's engine key is not usable ({e}): a coordinator configuration error (HAZYNC_ENGINE_VK, HAZYNC_ENGINE_VK_PIN)");
    }
    let image: [u32; 8] = core::array::from_fn(|i| u32::from_le_bytes(id[4 * i..4 * i + 4].try_into().unwrap()));
    let other = (journal.len() % 4 == 0).then(|| journal.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect::<Vec<u32>>())
        .and_then(|w| hazync_journal::serde::from_slice::<ChainState, u32>(&w).ok())
        .filter(|st| st.kind == KIND_CHAIN && st.self_id != image);
    match other {
        Some(st) => format!("proved by image {} (its journal says so), not this coordinator's engine image {} (METHOD_ID MISMATCH: another guest build): {e}",
                            hex_id(&st.self_id), hex(id)),
        None => format!("not a valid proof of this coordinator's engine image {} (forged, tampered or corrupt): {e}", hex(id)),
    }
}

fn hex_id(w: &[u32; 8]) -> String {
    w.iter().flat_map(|x| x.to_le_bytes()).map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod engine_board_tests {
    //! verify-any on an engine board (engine/STAGE3.md 3.2), on the engine chain's proof of mainnet 1..200
    //! (swiftsync-verify/tests/data/engine200: HZE1, its key and pin) and another image's key (the header zAPP's).
    use super::{verify_any, verify_any_engine};
    const D: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../swiftsync-verify/tests/data/engine200/");
    const OTHER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../zapps/headers/tests/data/engine4031/");

    fn key(dir: &str) -> (Vec<u8>, [u8; 32]) {
        let k = std::fs::read(format!("{dir}vk.bin")).unwrap();
        let h = std::fs::read_to_string(format!("{dir}vk.sha256")).unwrap();
        let h = h.split_whitespace().next().unwrap();
        (k, core::array::from_fn(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap()))
    }
    fn proof() -> Vec<u8> { std::fs::read(format!("{D}proof.hze")).unwrap() }

    #[test]
    fn engine_verify_any_accepts_and_refuses() {
        // the range 1..200 from genesis, as the board checks it (no --check-end: the board's own states are not here)
        verify_any_engine(&proof(), key(D), Some(1), false).expect("block 1..200's engine proof is RANGE-OK");
        // the key not the pinned one
        let (k, _) = key(D);
        let e = verify_any_engine(&proof(), (k, [0u8; 32]), Some(1), false).unwrap_err();
        assert!(e.contains("configuration error") && !e.contains("MISMATCH"), "wrong pin: {e}");
        // another image's key (pinned correctly): the proof is not a valid proof under it, and not another BUILD's
        // chain proof either (its journal names this proof's own image, not the other key's)
        let e = verify_any_engine(&proof(), key(OTHER), Some(1), false).unwrap_err();
        assert!(e.contains("not this coordinator's engine image") || e.contains("not a valid proof of this coordinator's engine image"), "other image: {e}");
        // a journal byte changed: the public values no longer match; a FORGERY, never reported as another build
        let mut p = proof();
        let n = p.len();
        p[n - 5] ^= 1;
        let e = verify_any_engine(&p, key(D), Some(1), false).unwrap_err();
        assert!(e.contains("forged, tampered or corrupt") && !e.contains("MISMATCH"), "tampered journal: {e}");
        // a proof byte changed: the same
        let mut p = proof();
        p[4000] ^= 1;
        let e = verify_any_engine(&p, key(D), Some(1), false).unwrap_err();
        assert!(e.contains("forged, tampered or corrupt") && !e.contains("MISMATCH"), "tampered proof: {e}");
        // a chain proof whose journal names ANOTHER image (what another guest build's proof carries): MISMATCH, by the
        // journal's own self_id; built here from the real one with self_id changed (it does not verify either way)
        let (pr, jr) = hazync_engine_verify::unpack(&proof()).map(|(a, b)| (a.to_vec(), b.to_vec())).unwrap();
        let w: Vec<u32> = jr.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        let mut st: crate::ChainState = hazync_journal::serde::from_slice(&w).unwrap();
        st.self_id[0] ^= 1;
        let j2: Vec<u8> = hazync_journal::serde::to_vec(&st).unwrap().iter().flat_map(|x| x.to_le_bytes()).collect();
        let e = verify_any_engine(&hazync_engine_verify::pack(&pr, &j2), key(D), Some(1), false).unwrap_err();
        assert!(e.contains("METHOD_ID MISMATCH: another guest build") && e.contains("proved by image"), "another build: {e}");
        // claimed starts the state cannot have: past its height, zero, and one whose state before it is unknown here
        for lo in [201u32, 0] {
            let e = verify_any_engine(&proof(), key(D), Some(lo), false).unwrap_err();
            assert!(e.contains("claimed start"), "lo {lo}: {e}");
        }
        assert!(verify_any_engine(&proof(), key(D), Some(2), false).is_err(), "lo 2 accepted with no state before it");
        // --check-end with no board state at 200: refused (unknown means refused)
        let e = verify_any_engine(&proof(), key(D), Some(1), true).unwrap_err();
        assert!(e.contains("no state at block 200"), "check-end: {e}");
        // a file that is not HZE1 (a RISC Zero receipt starts with bincode's InnerReceipt tag: 1 = succinct)
        let mut r = vec![1u8, 0, 0, 0];
        r.extend_from_slice(&proof()[4..1000]);
        let e = verify_any_engine(&r, key(D), Some(1), false).unwrap_err();
        assert!(e.contains("not an engine proof file"), "not HZE1 on an engine board: {e}");
        // a truncated file
        let e = verify_any_engine(&proof()[..100], key(D), Some(1), false).unwrap_err();
        assert!(e.contains("not an engine proof file") || e.contains("forged, tampered or corrupt"), "truncated: {e}");
    }

    #[test]
    fn a_host_without_the_board_key_verifies_nothing() {
        if std::env::var("HAZYNC_ENGINE_VK").is_ok() { return; } // this test is about a host WITHOUT the engine key
        let e = verify_any(&format!("{D}proof.hze"), Some(1), false).unwrap_err();
        assert!(e.contains("no engine key") && e.contains("configuration error") && !e.contains("MISMATCH"), "{e}");
    }
}
