//! EXPERIMENTAL Trusted State (docs/libzync/TRUSTED_STATE.md), host side only, behind the `trusted-state` cargo
//! feature (off in every release and CI host). The guest is NOT changed: an anchored lineage is the core image's own
//! base step (`is_base = 1`) on an anchor built from a Core snapshot by `experimental/trusted-state/anchor`
//! (`ts-anchor build`), whose `anchor_id` the guest commits as for any anchor.
//!
//! The one thing the host cannot do for such an anchor without this module is its BIP30 tree: the prover either replays
//! the tree from block files 1..lo-1 (which a snapshot node does not have) or, for a demonstration anchor, seeds it
//! with the coinbases the range spends and REWRITES the anchor's smt_root, which changes the anchor's link digest. With
//! `HAZYNC_TS_SMT_SEED=<anchor>.smt` the tree is built from the anchor's own entry list instead and must hash to the
//! anchor's smt_root, so the anchor (and its anchor_id) is used exactly as built.

use crate::Smt;

/// A BIP30 tree from an entry list (`<txid internal hex> <value>` per line), required to hash to `root`: the anchor's
/// `.smt` (`root` = the anchor's smt_root) or, on a resume (HAZYNC_RESUME=1), the `state_<k>.smt` the run wrote beside
/// block k's receipt (`root` = that receipt's smt_root, which the guest proved; so the file is trusted for nothing).
pub(crate) fn smt_seed(path: &str, root: &[u8; 32], whose: &str) -> Smt {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("HAZYNC_TS_SMT_SEED {path}: {e}"));
    let mut t = Smt::new();
    let mut n = 0usize;
    for (i, l) in text.lines().enumerate() {
        let l = l.trim();
        if l.is_empty() { continue; }
        let (k, v) = l.split_once(' ').unwrap_or_else(|| panic!("{path} line {}: want <txid hex> <value>", i + 1));
        let k: [u8; 32] = crate::hx(k).try_into().unwrap_or_else(|_| panic!("{path} line {}: a 32-byte txid", i + 1));
        let v: u32 = v.trim().parse().unwrap_or_else(|_| panic!("{path} line {}: value", i + 1));
        assert!(t.get(&k).is_none(), "{path} line {}: txid listed twice", i + 1);
        t.insert(k, v);
        n += 1;
    }
    assert_eq!(&t.root(), root, "{path}: its {n} entries do not hash to {whose} smt_root");
    eprintln!("TRUSTED STATE (experimental): BIP30 tree from {path}: {n} entries, root = {whose} smt_root");
    t
}

/// The tree's entries in `smt_seed`'s format, written atomically (a resume must never read a short file).
pub(crate) fn smt_save(path: &str, t: &Smt) {
    let mut s = String::with_capacity(t.len() * 76);
    for (k, v) in t.entries() { s += &crate::hex(&k); s.push(' '); s += &v.to_string(); s.push('\n'); }
    crate::lz::write_atomic(path, s.as_bytes());
}
