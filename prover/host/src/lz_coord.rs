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
use methods::METHOD_ID;

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
            let st: ChainState = serde_json::from_str(&t).map_err(|e| format!("{sp}: {e}"))?;
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
    let r = std::fs::read(format!("{out}/block_{h}.rcpt")).unwrap();
    crate::lz::write_atomic(&format!("range_{h}.bin"), &r);
    let _ = std::fs::remove_dir_all(&out);
    println!("leaf receipt for block {h} -> range_{h}.bin");
}

/// `verify-any <rcpt> [<lo>] [--check-end]` on a libzync host: verifies the receipt against this image and prints the
/// coordinator's RANGE-OK line. Without `<lo>` the range is reported as starting at hi (a leaf), which is checked the
/// same way. `--check-end` (the board passes it) also requires the END state to be the board's own (see below) and
/// adds `end=board` to the line; the board refuses a line without it, so an older host that ignores the flag fails
/// closed. A contributor's self-test has no board states and does not pass it.
pub(crate) fn verify_any_cmd(path: &str, lo: Option<u32>, check_end: bool) {
    if let Err(e) = verify_any(path, lo, check_end) {
        eprintln!("REJECT {e}");
        std::process::exit(1);
    }
}

fn verify_any(path: &str, lo: Option<u32>, check_end: bool) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let r: risc0_zkvm::Receipt = bincode::deserialize(&bytes).map_err(|e| format!("{path}: not a receipt: {e}"))?;
    // A FakeReceipt "verifies" whenever RISC0_DEV_MODE is set in this process's environment: refuse both, as
    // libzync-verify does, so a stray dev-mode variable on the board can never let a hand-written state in.
    if matches!(r.inner, risc0_zkvm::InnerReceipt::Fake(_)) { return Err("a dev-mode FakeReceipt is not a proof".into()); }
    if crate::verify::dev_mode_env() { return Err("RISC0_DEV_MODE is set in the verifier's environment: refusing to verify".into()); }
    // MISMATCH: the coordinator reads it as a different guest build (the common contributor mistake), not a forgery
    r.verify(METHOD_ID).map_err(|e| format!("does not verify against this coordinator's libzync image (METHOD_ID MISMATCH or invalid proof): {e}"))?;
    let st: ChainState = r.journal.decode().map_err(|e| format!("journal is not a ChainState: {e}"))?;
    if st.kind != KIND_CHAIN { return Err(format!("journal kind {:#x} is not a chain state", st.kind)); }
    if st.self_id != METHOD_ID { return Err("the state's self_id is not this image".into()); }
    if st.profile != crate::lz::expected_profile() {
        return Err(format!("profile {:#x}, this coordinator's image commits {:#x}", st.profile, crate::lz::expected_profile()));
    }
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
        if link_digest(&after) != link_digest(&st) || after.tip_hash != st.tip_hash {
            return Err(format!("the receipt ends on a state at block {} that is not the board's (its tip {}, the board's {}): \
                                not Bitcoin's chain", st.height, hex(&st.tip_hash), hex(&after.tip_hash)));
        }
    }
    println!("RANGE-OK lo={lo} hi={} in_tip={} out_tip={} out_leaves={} range_work=0 in_bhash={} out_bhash={} anchored={}{}",
             st.height, hex(&st.anchor_id), hex(&link_digest(&st)), st.utxo_leaves, hex(&before.tip_hash), hex(&st.tip_hash),
             (lo == 1) as u8, if check_end { " end=board" } else { "" });
    Ok(())
}
