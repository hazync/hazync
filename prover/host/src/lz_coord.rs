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

/// The state before block `lo`: genesis for 1, else the `prev` of the block's leaf bundle.
fn state_before(lo: u32) -> Result<ChainState, String> {
    if lo == 1 { return Ok(crate::genesis_anchor()); }
    let p = format!("{}/leaf_{lo}.bin", bridge_dir());
    if !std::path::Path::new(&p).exists() { return Err(format!("no leaf bundle {p}: the state before block {lo} is unknown here")); }
    let b = crate::prep::leaf_read(&p);
    if b.prev.height + 1 != lo { return Err(format!("{p} is for block {}, not {lo}", b.prev.height + 1)); }
    Ok(b.prev)
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

/// `verify-any <rcpt> [<lo>]` on a libzync host: verifies the receipt against this image and prints the coordinator's
/// RANGE-OK line. Without `<lo>` the range is reported as starting at hi (a leaf), which is checked the same way.
pub(crate) fn verify_any_cmd(path: &str, lo: Option<u32>) {
    if let Err(e) = verify_any(path, lo) {
        eprintln!("REJECT {e}");
        std::process::exit(1);
    }
}

fn verify_any(path: &str, lo: Option<u32>) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let r: risc0_zkvm::Receipt = bincode::deserialize(&bytes).map_err(|e| format!("{path}: not a receipt: {e}"))?;
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
    println!("RANGE-OK lo={lo} hi={} in_tip={} out_tip={} out_leaves={} range_work=0 in_bhash={} out_bhash={} anchored={}",
             st.height, hex(&st.anchor_id), hex(&link_digest(&st)), st.utxo_leaves, hex(&before.tip_hash), hex(&st.tip_hash),
             (lo == 1) as u8);
    Ok(())
}
