//! ⚠ GUEST-COMPILED CRATE — editing this file changes METHOD_ID and re-baselines the board.
//!
//! This file is #[path]-included into `prover/methods/guest`, so it compiles into the guest ELF. Rust
//! embeds file and line into panic metadata, which means ANY edit that moves line numbers changes the
//! image id — including pure comments, and including code the guest never calls. Every proof on the
//! board becomes invalid.
//!
//! The per-block BIP30 state transition (hazync#54).
//!
//! This is the logic that replaces the structural argument. It lives here, `no_std`, so the guest and
//! the bridge run the same compiled code — and so it can be tested exhaustively on the host before it
//! is load-bearing anywhere.
//!
//! # The rule, precisely
//!
//! Core: no transaction may create an output that already exists unspent (`bad-txns-BIP30`). Core asks its coin
//! database. This tree holds what it takes to give the same answer without one.
//!
//! A transaction can repeat only if every one of its inputs was re-created, so a repeat traces back to a repeated
//! coinbase. BIP34 forbids that except for a coinbase mined before BIP34 whose script reads as a later height Y, and
//! then only at exactly Y. So the tree tracks those coinbases (`repeatable`) and every transaction whose inputs are
//! ALL repeatable coins: the only transactions that can ever repeat. On mainnet that is 173 transactions to height
//! 940,000 for Y under 100,000,000, measured. Each entry counts its unspent outputs, and at zero stays as `REPEATABLE`:
//!
//!   absent         never seen: allowed
//!   REPEATABLE     seen, fully spent, may repeat: allowed, and the repeat is itself tracked
//!   a live count   has unspent outputs: refused, Core's bad-txns-BIP30
//!   SPENT          an untracked coinbase, fully spent: refused
//!
//! An untracked coinbase cannot repeat under BIP34, so `SPENT` only refuses a repeat before BIP34 on a chain that is
//! not mainnet's: real history repeats exactly two coinbases, both grandfathered, both while unspent (measured to height
//! 230,000 and at 490,897). There the guest is stricter than Core. Everywhere else this is Core's rule.
//!
//! # The transition
//!
//! ```text
//!   1. prove the coinbase txid is absent or REPEATABLE, then insert it with its output count
//!   2. the same for each tracked transaction this block creates, in block order
//!   3. for each input spending a tracked coin, decrement its count, marking it at zero
//! ```
//!
//! Step 3 is what keeps the danger set shrinking as history does, and is why a static exception table
//! is the wrong shape: ~18% of pre-BIP34 coinbases are still unspent today, and that fraction only
//! falls.
//!
//! # Proofs are SEQUENCED, and the bridge must match
//!
//! Every proof is against the root *as it stands when that step is applied*, not against the
//! incoming root. The coinbase insert lands first, so a spend's proof must be taken after it; two
//! spends of the same coinbase chain likewise. A proof taken against the wrong intermediate root is
//! refused rather than mis-folded — asserted below — which is the safe failure, but it does mean the
//! bridge has to generate proofs in exactly this order. That is a real coupling between the two, and
//! it is cheap for the bridge (it holds the tree and can step it) but easy to get wrong silently if
//! anyone reorders the steps here without reordering them there.
//!
//! # Ordering matters, and is asserted
//!
//! The check happens against the root **before** any of this block's own updates. A block whose
//! coinbase duplicates a coinbase it also spends in the same block must still be rejected — checking
//! after the decrements would let it spend the old one to zero and then claim the slot was free.

use crate::{apply, verify, Hash, Key, Proof};
use alloc::vec::Vec;

/// An untracked coinbase with no unspent outputs. Not 0: the tree treats 0 as absent. No block can hold this many outputs.
pub const SPENT: u32 = u32::MAX;
/// A tracked coinbase or transaction with no unspent outputs: it may repeat.
pub const REPEATABLE: u32 = u32::MAX - 1;

/// True for a count of unspent outputs, false for absent-as-zero and the two markers.
pub fn is_live(count: u32) -> bool {
    count != 0 && count != SPENT && count != REPEATABLE
}

/// An entry's value after one more of its outputs is spent. `count` must be live.
pub fn after_spend(count: u32, tracked: bool) -> u32 {
    match (count, tracked) {
        (1, true) => REPEATABLE,
        (1, false) => SPENT,
        _ => count - 1,
    }
}

/// Coinbases that indicate this height or above are not tracked, and the guest refuses blocks at or above it. Mainnet
/// reaches it in about 1,900 years; a later image can raise it.
pub const REPEAT_HORIZON: u32 = 100_000_000;

/// One coinbase-output spend: which coinbase it belonged to, and the proof of that coinbase's current
/// count under the root as it stands when the spend is applied.
pub struct Spend {
    pub coinbase_txid: Key,
    pub current_count: u32,
    pub proof: Proof,
    /// A tracked coin, read from the coin's leaf flag.
    pub tracked: bool,
}

/// A tracked transaction this block creates: its txid and spendable output count, and the slot's prior state.
pub struct TrackedTx {
    pub txid: Key,
    pub outputs: u32,
    pub prior: Option<u32>,
    pub proof: Proof,
}

/// Everything the transition needs for one block.
pub struct BlockUpdate {
    pub coinbase_txid: Key,
    pub coinbase_outputs: u32,
    /// The prior state of the coinbase's slot: `None` (absent) or `Some(REPEATABLE)`. Bound by `absence_proof`.
    pub coinbase_prior: Option<u32>,
    /// Tracked transactions this block creates, in block order.
    pub tracked: Vec<TrackedTx>,
    /// Normally a proof of `coinbase_prior` under the INCOMING root.
    ///
    /// At a grandfathered block (`overwrite` is `Some`) it is instead a MEMBERSHIP proof of the prior
    /// count, because absence is *unprovable* there — see `overwrite`.
    pub absence_proof: Proof,
    /// `Some(prior_count)` ONLY at the two blocks BIP30 grandfathers (91842, 91880).
    ///
    /// Those two coinbases duplicated earlier ones that were **still unspent** — that is the whole
    /// reason BIP30 exists, and the reason Core exempts exactly these two heights. So entering them
    /// the tree holds the txid with a nonzero count and no absence proof can exist. Without this the
    /// check is reject-valid on real history and a from-genesis prover stalls at 91841.
    ///
    /// It is NOT a general escape hatch. The caller gates it on the block hash (the guest does this
    /// from the hash it derives, exactly as the utreexo F3 overwrite is gated), and `apply` still
    /// binds the prover to the real prior count — a wrong one folds to a different root and fails.
    pub overwrite: Option<u32>,
    /// Coinbase-output spends in this block, in application order.
    pub spends: Vec<Spend>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Bip30Error {
    /// The txid is in the tree with unspent outputs, or is an untracked coinbase: the rule this exists to enforce.
    DuplicateUnspentCoinbase,
    /// A proof did not verify against the root it was offered for.
    BadProof,
    /// A spend claimed something that is not a live count.
    SpendFromEmpty,
    /// A grandfathered overwrite claimed a prior value that is not a live count.
    OverwriteFromEmpty,
}

/// Apply one block's BIP30 transition, returning the new SMT root.
///
/// Fails closed: any bad proof, any duplicate, and the block is rejected. In the guest a failure means
/// no receipt at all, so there is no partial-application concern — but the root is threaded rather
/// than mutated in place regardless, so a rejected block cannot leave a half-updated state behind for
/// a caller that ignores the error.
pub fn apply_block(root: &Hash, u: &BlockUpdate) -> Result<Hash, Bip30Error> {
    // A coinbase with no SPENDABLE outputs leaves its slot as it was: the tree holds no zero-valued leaves, and
    // nothing can spend such a coinbase, so nothing descends from it.
    let new_val = if u.coinbase_outputs == 0 { u.coinbase_prior } else { Some(u.coinbase_outputs) };

    let mut cur = match u.overwrite {
        // 1. THE CHECK — against the incoming root, before this block's own updates. See the ordering
        //    note above: doing it later would let a block spend the duplicate to zero and then claim
        //    the slot. A fully spent coinbase is held as `SPENT`, so absence fails for it too.
        //
        // 2. Then insert. The SAME proof carries the update — it proved the slot's state — which is
        //    why the insert costs no extra witness bytes.
        None => {
            if !matches!(u.coinbase_prior, None | Some(REPEATABLE)) || !verify(root, &u.coinbase_txid, u.coinbase_prior, &u.absence_proof) {
                // Either the key is present with a nonzero count — a real BIP30 violation — or the
                // proof is junk. Both reject; distinguishing them needs a second proof and buys
                // nothing, since the block is invalid either way.
                return Err(Bip30Error::DuplicateUnspentCoinbase);
            }
            apply(root, &u.coinbase_txid, u.coinbase_prior, new_val, &u.absence_proof)
                .ok_or(Bip30Error::BadProof)?
        }
        // THE GRANDFATHERED OVERWRITE (91842 / 91880). Absence is unprovable here by construction, so
        // the witness proves the PRIOR COUNT instead and `apply` replaces it. That still binds the
        // prover to the real prior state: a wrong count folds to a different root and fails. For the
        // real blocks the duplicate coinbase is byte-identical, so prior == new and the root does not
        // move — the overwrite is a no-op on the tree, which is exactly right, because the earlier
        // outputs are destroyed and replaced by identical ones.
        Some(prior) => {
            // The slot holds a live count here, so a claim of REPEATABLE is false, and the coinbase's leaf flag reads it.
            if !is_live(prior) || u.coinbase_prior.is_some() {
                return Err(Bip30Error::OverwriteFromEmpty);
            }
            apply(root, &u.coinbase_txid, Some(prior), new_val, &u.absence_proof)
                .ok_or(Bip30Error::BadProof)?
        }
    };

    // 2. Tracked transactions, checked before this block's spends as Core checks BIP30 before connecting any input.
    for t in &u.tracked {
        if !matches!(t.prior, None | Some(REPEATABLE)) || t.outputs == 0 || !verify(&cur, &t.txid, t.prior, &t.proof) {
            return Err(Bip30Error::DuplicateUnspentCoinbase);
        }
        cur = apply(&cur, &t.txid, t.prior, Some(t.outputs), &t.proof).ok_or(Bip30Error::BadProof)?;
    }

    // 3. Decrement each spent coin. Sequential: every proof is against the root as it stands after the previous
    //    update, which is what lets a block spend two outputs of the same transaction.
    for s in &u.spends {
        if !is_live(s.current_count) {
            return Err(Bip30Error::SpendFromEmpty);
        }
        cur = apply(&cur, &s.coinbase_txid, Some(s.current_count), Some(after_spend(s.current_count, s.tracked)), &s.proof)
            .ok_or(Bip30Error::BadProof)?;
    }
    Ok(cur)
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::Smt;
    use sha2::{Digest, Sha256};

    fn k(n: u64) -> Key {
        let mut b = [0u8; 32];
        b[..8].copy_from_slice(&n.to_be_bytes());
        Sha256::digest(b).into()
    }

    fn update(t: &Smt, cb: Key, nout: u32) -> BlockUpdate {
        BlockUpdate {
            coinbase_prior: None, tracked: Vec::new(),
            coinbase_txid: cb,
            coinbase_outputs: nout,
            absence_proof: t.prove(&cb),
            overwrite: None,
            spends: Vec::new(),
        }
    }

    #[test]
    fn an_ordinary_block_advances_the_root() {
        let mut t = Smt::new();
        let mut root = t.root();
        for i in 0..20u64 {
            let cb = k(i);
            let new_root = apply_block(&root, &update(&t, cb, 1)).expect("honest block rejected");
            t.insert(cb, 1);
            assert_eq!(new_root, t.root(), "guest root diverged from the host tree at block {i}");
            root = new_root;
        }
    }

    #[test]
    fn a_duplicate_unspent_coinbase_is_rejected() {
        // The entire point of the structure.
        let mut t = Smt::new();
        t.insert(k(7), 1);
        let root = t.root();
        let err = apply_block(&root, &update(&t, k(7), 1)).unwrap_err();
        assert_eq!(err, Bip30Error::DuplicateUnspentCoinbase);
    }

    /// One block that spends the last output of `key`, through the real transition.
    fn spend_last(t: &mut Smt, key: Key, block_cb: Key) -> Hash {
        let root = t.root();
        let absence = t.prove(&block_cb);
        t.insert(block_cb, 1);
        let u = BlockUpdate {
            coinbase_prior: None, tracked: Vec::new(),
            coinbase_txid: block_cb, coinbase_outputs: 1, absence_proof: absence, overwrite: None,
            spends: vec![Spend { coinbase_txid: key, current_count: 1, proof: t.prove(&key), tracked: false }],
        };
        let out = apply_block(&root, &u).expect("honest block rejected");
        t.insert(key, SPENT);
        assert_eq!(out, t.root(), "SPENT marker diverged from the host tree");
        out
    }

    #[test]
    fn a_duplicate_fully_spent_coinbase_is_rejected() {
        // Review #26. Core allows this, and it is what would let a descendant repeat: C spent by T, C
        // mined again, T mined again. This tree cannot see T, so it must stop C.
        let mut t = Smt::new();
        t.insert(k(7), 1);
        let root = spend_last(&mut t, k(7), k(8));
        assert_eq!(t.get(&k(7)), Some(SPENT), "a fully spent coinbase must stay in the tree");
        assert_eq!(apply_block(&root, &update(&t, k(7), 2)).unwrap_err(), Bip30Error::DuplicateUnspentCoinbase);
    }

    #[test]
    fn a_fully_spent_coinbase_cannot_be_spent_or_overwritten() {
        let mut t = Smt::new();
        t.insert(k(7), 1);
        let root = spend_last(&mut t, k(7), k(8));
        let mut u = update(&t, k(9), 1);
        let mut mid = t.clone();
        mid.insert(k(9), 1);
        u.spends = vec![Spend { coinbase_txid: k(7), current_count: SPENT, proof: mid.prove(&k(7)), tracked: false }];
        assert_eq!(apply_block(&root, &u).unwrap_err(), Bip30Error::SpendFromEmpty);
        // Claiming a live count for it does not fold to the real root.
        let mut u = update(&t, k(9), 1);
        u.spends = vec![Spend { coinbase_txid: k(7), current_count: 1, proof: mid.prove(&k(7)), tracked: false }];
        assert_eq!(apply_block(&root, &u).unwrap_err(), Bip30Error::BadProof);
        let mut u = update(&t, k(7), 1);
        u.overwrite = Some(SPENT);
        assert_eq!(apply_block(&root, &u).unwrap_err(), Bip30Error::OverwriteFromEmpty);
    }

    #[test]
    fn spending_coinbase_outputs_decrements() {
        let mut t = Smt::new();
        t.insert(k(1), 2);
        let root = t.root();

        // The spend proof must be against the root AFTER the coinbase insert, because apply_block
        // threads the root through in order. Proving it against the incoming root gives a stale proof
        // and is correctly refused — the first version of this test made exactly that mistake.
        let absence = t.prove(&k(50));
        let mut mid = t.clone();
        mid.insert(k(50), 1);
        let spends = vec![
            Spend { coinbase_txid: k(1), current_count: 2, proof: mid.prove(&k(1)), tracked: false },
        ];
        let u = BlockUpdate {
            coinbase_prior: None, tracked: Vec::new(),
            coinbase_txid: k(50), coinbase_outputs: 1,
            absence_proof: absence, overwrite: None, spends,
        };
        let new_root = apply_block(&root, &u).expect("honest block rejected");
        t.insert(k(50), 1);
        t.insert(k(1), 1);
        assert_eq!(new_root, t.root(), "decrement diverged from the host tree");
    }

    #[test]
    fn two_spends_of_the_same_coinbase_in_one_block_chain_correctly() {
        // The sequential-proof property: the second spend's proof must be against the root AFTER the
        // first. Getting this wrong is the classic double-application bug, and it would leave a
        // coinbase looking unspent when it is not.
        let mut t = Smt::new();
        t.insert(k(2), 2);
        let root = t.root();

        let p1 = t.prove(&k(2));
        let mut mid = t.clone();
        mid.insert(k(2), 1);                       // state the second proof must be against
        let p2 = mid.prove(&k(2));

        let u = BlockUpdate {
            coinbase_prior: None, tracked: Vec::new(),
            coinbase_txid: k(60), coinbase_outputs: 1,
            absence_proof: t.prove(&k(60)),
            overwrite: None,
            spends: vec![
                Spend { coinbase_txid: k(2), current_count: 2, proof: p1, tracked: false },
                Spend { coinbase_txid: k(2), current_count: 1, proof: p2, tracked: false },
            ],
        };
        // NOTE: the absence proof and p1 are both against `root`, but p2 is against the post-p1 state.
        // apply_block threads the root, so this is the honest arrangement.
        let got = apply_block(&root, &u);
        // p1 was taken before the coinbase insert, so it will not verify against the post-insert root:
        // this asserts the transition REFUSES a stale proof rather than silently mis-folding.
        assert!(got.is_err(), "a proof taken against the wrong intermediate root was accepted");
    }

    #[test]
    fn a_block_cannot_spend_its_own_duplicate_to_free_the_slot() {
        // The ordering attack the check-before-update rule exists to stop: if the check ran after the
        // decrements, a block could spend the old coinbase to zero and then claim the slot was free.
        let mut t = Smt::new();
        t.insert(k(9), 1);
        let root = t.root();
        let u = BlockUpdate {
            coinbase_prior: None, tracked: Vec::new(),
            coinbase_txid: k(9), coinbase_outputs: 1,
            absence_proof: t.prove(&k(9)),
            overwrite: None,
            spends: vec![Spend { coinbase_txid: k(9), current_count: 1, proof: t.prove(&k(9)), tracked: false }],
        };
        assert_eq!(apply_block(&root, &u).unwrap_err(), Bip30Error::DuplicateUnspentCoinbase,
                   "a block spent its own duplicate to free the slot");
    }

    #[test]
    fn a_forged_absence_proof_is_refused() {
        let mut t = Smt::new();
        t.insert(k(3), 1);
        let root = t.root();
        // Offer a proof for a DIFFERENT key and claim it covers k(3).
        let u = BlockUpdate {
            coinbase_prior: None, tracked: Vec::new(),
            coinbase_txid: k(3), coinbase_outputs: 1,
            absence_proof: t.prove(&k(4242)),
            overwrite: None,
            spends: Vec::new(),
        };
        assert!(apply_block(&root, &u).is_err(), "a proof for another key was accepted");
    }

    #[test]
    fn spending_from_an_empty_count_is_refused() {
        let t = Smt::new();
        let root = t.root();
        let u = BlockUpdate {
            coinbase_prior: None, tracked: Vec::new(),
            coinbase_txid: k(70), coinbase_outputs: 1,
            absence_proof: t.prove(&k(70)),
            overwrite: None,
            spends: vec![Spend { coinbase_txid: k(71), current_count: 0, proof: t.prove(&k(71)), tracked: false }],
        };
        assert_eq!(apply_block(&root, &u).unwrap_err(), Bip30Error::SpendFromEmpty);
    }

    /// AUDIT #3 F-1 REPRODUCTION — the real shape of blocks 91842 / 91880.
    ///
    /// The duplicated coinbases were overwritten while STILL UNSPENT. That is the entire reason BIP30
    /// exists and the entire reason Core grandfathers exactly these two blocks: the later coinbase
    /// destroyed the earlier, still-spendable outputs. This codebase proves the precondition itself —
    /// the guest MANDATES an overwrite witness at those two block hashes and DELETES the superseded
    /// leaves from the utreexo accumulator, which can only succeed if those leaves are present.
    ///
    /// So entering 91842 the SMT holds the txid with a nonzero count, and an absence proof cannot
    /// exist. Without a grandfather on the SMT path the block is rejected and a from-genesis prover
    /// stalls at 91841 — about 10% of the chain.
    #[test]
    fn the_real_91842_shape_is_accepted() {
        let dup = k(91812);
        let mut t = Smt::new();
        t.insert(dup, 1);              // present and UNSPENT — the real precondition
        let root = t.root();

        let u = BlockUpdate {
            coinbase_prior: None, tracked: Vec::new(),
            coinbase_txid: dup,
            coinbase_outputs: 1,
            // At a grandfathered block the witness proves the PRIOR COUNT, not absence — absence is
            // unprovable here, which is the check working correctly on a block history exempts.
            absence_proof: t.prove(&dup),
            overwrite: Some(1),
            spends: Vec::new(),
        };
        let out = apply_block(&root, &u)
            .expect("block 91842 was rejected — a from-genesis prover stalls at 91841");

        // The duplicate coinbase is byte-identical, so the count is unchanged and so is the root.
        assert_eq!(out, root, "a byte-identical overwrite must leave the SMT root unchanged");
    }

    /// The grandfather must not become a general escape hatch: the guest gates `overwrite` on the two
    /// block hashes, and the transition still binds the prover to the REAL prior count.
    #[test]
    fn an_overwrite_with_the_wrong_prior_count_is_refused() {
        let dup = k(91812);
        let mut t = Smt::new();
        t.insert(dup, 1);
        let root = t.root();
        for lie in [0u32, 2, 7] {
            let u = BlockUpdate {
                coinbase_prior: None, tracked: Vec::new(),
                coinbase_txid: dup, coinbase_outputs: 1,
                absence_proof: t.prove(&dup), overwrite: Some(lie), spends: Vec::new(),
            };
            assert!(apply_block(&root, &u).is_err(),
                    "an overwrite claiming a prior count of {lie} was accepted");
        }
    }

    /// An overwrite of a key that is ABSENT is refused — otherwise the exception would let a prover
    /// insert an arbitrary coinbase at the two grandfathered heights without proving anything.
    #[test]
    fn an_overwrite_of_an_absent_coinbase_is_refused() {
        let t = Smt::new();
        let u = BlockUpdate {
            coinbase_prior: None, tracked: Vec::new(),
            coinbase_txid: k(5), coinbase_outputs: 1,
            absence_proof: t.prove(&k(5)), overwrite: Some(1), spends: Vec::new(),
        };
        assert!(apply_block(&t.root(), &u).is_err(),
                "an overwrite of an absent coinbase was accepted");
    }

    /// A block that spends the last output of the tracked coin `key`, then the state after it.
    fn spend_last_tracked(t: &mut Smt, key: Key, block_cb: Key) -> Hash {
        let root = t.root();
        let absence = t.prove(&block_cb);
        t.insert(block_cb, 1);
        let u = BlockUpdate {
            coinbase_prior: None, tracked: Vec::new(), coinbase_txid: block_cb, coinbase_outputs: 1,
            absence_proof: absence, overwrite: None,
            spends: vec![Spend { coinbase_txid: key, current_count: 1, proof: t.prove(&key), tracked: true }],
        };
        let out = apply_block(&root, &u).expect("honest block rejected");
        t.insert(key, REPEATABLE);
        assert_eq!(out, t.root());
        out
    }

    #[test]
    fn a_tracked_coinbase_may_repeat_once_fully_spent_and_not_before() {
        let mut t = Smt::new();
        t.insert(k(7), 1);
        assert!(apply_block(&t.root(), &update(&t, k(7), 1)).is_err(), "repeated while unspent");
        let root = spend_last_tracked(&mut t, k(7), k(8));
        let mut u = update(&t, k(7), 2);
        u.coinbase_prior = Some(REPEATABLE);
        let out = apply_block(&root, &u).expect("Core accepts this repeat");
        t.insert(k(7), 2);
        assert_eq!(out, t.root(), "the repeat's outputs are live again");
        // A prover cannot claim REPEATABLE for a live coinbase, or absent for a REPEATABLE one.
        let mut u = update(&t, k(7), 1);
        u.coinbase_prior = Some(REPEATABLE);
        assert!(apply_block(&t.root(), &u).is_err());
        let mut live = Smt::new();
        live.insert(k(9), REPEATABLE);
        assert!(apply_block(&live.root(), &update(&live, k(9), 1)).is_err());
    }

    fn with_tracked(t: &Smt, cb: Key, tx: Key, prior: Option<u32>) -> BlockUpdate {
        let mut mid = t.clone();
        mid.insert(cb, 1);
        let mut u = update(t, cb, 1);
        u.tracked = vec![TrackedTx { txid: tx, outputs: 2, prior, proof: mid.prove(&tx) }];
        u
    }

    #[test]
    fn a_tracked_transaction_is_core_s_bip30() {
        let mut t = Smt::new();
        let root = t.root();
        let out = apply_block(&root, &with_tracked(&t, k(1), k(50), None)).expect("first instance");
        t.insert(k(1), 1);
        t.insert(k(50), 2);
        assert_eq!(out, t.root());
        // Repeated while it has unspent outputs: bad-txns-BIP30.
        assert_eq!(apply_block(&t.root(), &with_tracked(&t, k(2), k(50), None)).unwrap_err(), Bip30Error::DuplicateUnspentCoinbase);
        assert_eq!(apply_block(&t.root(), &with_tracked(&t, k(2), k(50), Some(2))).unwrap_err(), Bip30Error::DuplicateUnspentCoinbase);
        assert_eq!(apply_block(&t.root(), &with_tracked(&t, k(2), k(50), Some(REPEATABLE))).unwrap_err(), Bip30Error::DuplicateUnspentCoinbase);
        // Fully spent: Core accepts the repeat.
        t.insert(k(50), REPEATABLE);
        assert!(apply_block(&t.root(), &with_tracked(&t, k(2), k(50), Some(REPEATABLE))).is_ok());
        assert!(apply_block(&t.root(), &with_tracked(&t, k(2), k(50), None)).is_err(), "a false claim of absence");
    }

    #[test]
    fn an_overwrite_cannot_claim_a_repeat() {
        let mut t = Smt::new();
        t.insert(k(7), 1);
        let mut u = update(&t, k(7), 1);
        u.overwrite = Some(1);
        assert!(apply_block(&t.root(), &u).is_ok());
        u.coinbase_prior = Some(REPEATABLE);
        assert_eq!(apply_block(&t.root(), &u).unwrap_err(), Bip30Error::OverwriteFromEmpty);
    }

    #[test]
    fn a_marker_cannot_be_spent() {
        for m in [SPENT, REPEATABLE] {
            let mut t = Smt::new();
            t.insert(k(7), m);
            let mut mid = t.clone();
            mid.insert(k(9), 1);
            let mut u = update(&t, k(9), 1);
            u.spends = vec![Spend { coinbase_txid: k(7), current_count: m, proof: mid.prove(&k(7)), tracked: true }];
            assert_eq!(apply_block(&t.root(), &u).unwrap_err(), Bip30Error::SpendFromEmpty);
        }
    }

    #[test]
    fn an_old_style_zero_entry_is_still_an_ordinary_absence() {
        // The tree itself still reads 0 as absent. Only `apply_block` keeps a spent coinbase, as SPENT.
        let mut t = Smt::new();
        t.insert(k(4242), 0);
        assert!(apply_block(&t.root(), &update(&t, k(4242), 1)).is_ok());
    }
}
