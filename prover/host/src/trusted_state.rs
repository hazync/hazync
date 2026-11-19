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

use crate::{ChainState, Smt};

/// The BIP30 tree of a Trusted State anchor from its `.smt` list (`<txid internal hex> <value>` per line), checked
/// against the anchor's smt_root.
pub(crate) fn smt_seed(path: &str, prev: &ChainState) -> Smt {
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
    assert_eq!(t.root(), prev.smt_root, "{path}: its {n} entries do not hash to the anchor's smt_root");
    eprintln!("TRUSTED STATE (experimental): BIP30 tree seeded from {path}: {n} entries, root = the anchor's smt_root");
    t
}
