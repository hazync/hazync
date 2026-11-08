//! Receipt checks: verify-range, verify-snark, verify-any, verify-chain, receipt-digest, and the
//! verifier's clock rule (time-too-new) shared with libzync-verify.

use bitcoin::hashes::Hash as _;
use hazync_utreexo::Forest;
use methods::METHOD_ID;
use crate::{ChainState, GENESIS_BITS, GENESIS_HASH, GENESIS_TIME, GENESIS_WORK, KIND_CHAIN, KIND_RANGE,
    RangeState, add256_host, arr, arr_u128, genesis_anchor, header_hash, hex, hx, normalize_host, rev,
    state_journal_bytes, verify_receipt, verify_receipt_ex, work_u128};

/// MAX_FUTURE_BLOCK_TIME (Core validation.h): 2 hours.
pub(crate) const MAX_FUTURE_BLOCK_TIME: u64 = 2 * 60 * 60;

/// The clock a verifier applies time-too-new against, and the raw HAZYNC_VERIFY_NOW value if that test-only
/// override is set. A verifier that sees the override never reports success (review 5): a stale export must
/// not quietly pass a future-dated chain, and an unparsable value falls back to the real clock but still counts.
pub(crate) fn verifier_clock() -> (u64, Option<String>) {
    let sys = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    match std::env::var("HAZYNC_VERIFY_NOW") {
        Ok(v) if !v.is_empty() => (v.parse().unwrap_or(sys), Some(v)),
        _ => (sys, None),
    }
}

/// RISC0_DEV_MODE as risc0 reads it: set and not empty / "0" / "false". Both verifiers refuse it (review 2, #16).
pub(crate) fn dev_mode_env() -> bool {
    std::env::var("RISC0_DEV_MODE").map(|v| !v.is_empty() && v != "0" && v != "false").unwrap_or(false)
}

/// Core's time-too-new (ContextualCheckBlockHeader) applied to a chain's largest timestamp on this clock.
pub(crate) fn time_ok(max_time: u32, now: u64) -> bool {
    max_time as u64 <= now + MAX_FUTURE_BLOCK_TIME
}

// Digest of a range's FULL boundary — everything `fold_range` binds at a seam (tip, UTXO roots+leaves,
// difficulty, and the MTP window). Chaining ranges on `out_bhash(k) == in_bhash(k+1)` reproduces the
// guest fold's seam check that tip-hash equality alone does NOT (a mid-chain range could otherwise
// fabricate its in-boundary UTXO set / in_time / MTP window). Roots are normalized so padding can't vary.
pub(crate) fn boundary_digest(height: u32, tip: &[u8; 32], roots: &[Option<[u8; 32]>], leaves: u64, nbits: u32, time: u32, epoch: u32, recent: &[u32], smt_root: &[u8; 32]) -> [u8; 32] {
    let mut m: Vec<u8> = Vec::new();
    // H9: bind the boundary's HEIGHT (out-boundary = hi, in-boundary = lo-1). RangeState.lo/hi are
    // committed in-circuit (prove_range sets lo=hi=w.height, the same value that selects the script
    // flags and coinbase subsidy; fold_range asserts hi+1==lo adjacency). Folding height into the seam
    // digest makes the coordinator's out_bhash(k)==in_bhash(k+1) chaining STRUCTURALLY require
    // hi(k)==lo(k+1)-1 — so a block mined onto the real tip but labelled a false (low) height, claiming a
    // larger subsidy / weaker flags, cannot chain even though its UTXO/difficulty/MTP boundary is valid.
    m.extend_from_slice(&height.to_le_bytes());
    m.extend_from_slice(tip);
    let nr = normalize_host(roots.to_vec());
    m.extend_from_slice(&(nr.len() as u32).to_le_bytes());
    for r in &nr {
        match r { Some(h) => { m.push(1); m.extend_from_slice(h); } None => m.push(0) }
    }
    m.extend_from_slice(&leaves.to_le_bytes());
    // AUDIT #3 F-3: the coinbase-SMT root is part of the boundary and MUST be in the seam digest.
    // Without it two ranges that disagree about BIP30 state but agree on everything else digest
    // IDENTICALLY, so the coordinator's out_bhash(k) == in_bhash(k+1) chaining cannot see the
    // disagreement. The guest's fold_range does assert SMT continuity, so folded proofs were always
    // safe — the hole was the verify-any chaining track the coordinator actually uses.
    m.extend_from_slice(smt_root);
    m.extend_from_slice(&nbits.to_le_bytes());
    m.extend_from_slice(&time.to_le_bytes());
    m.extend_from_slice(&epoch.to_le_bytes());
    m.extend_from_slice(&(recent.len() as u32).to_le_bytes());
    for t in recent { m.extend_from_slice(&t.to_le_bytes()); }
    header_hash(&m)
}

// Pin the FULL genesis in-boundary of a range proof. in_tip alone is not enough: in_epoch_start feeds
// the first retarget (block 2016) via calc_next_bits and propagates unchanged across fold seams, so an
// unpinned value forges that retarget's difficulty (up to 4x easier) and understates cum_work; in_roots
// must be the empty accumulator (in_leaves==0 alone permits phantom roots); in_recent/in_time feed MTP.
pub(crate) fn assert_genesis_in_boundary(rs: &RangeState) {
    assert_eq!(rs.lo, 1, "genesis-connected range must start at block 1");
    assert_eq!(rs.in_tip_hash, arr(rev(hx(GENESIS_HASH))), "in-boundary tip != genesis hash");
    assert_eq!(rs.in_leaves, 0, "in-boundary UTXO set not empty");
    assert_eq!(rs.in_nbits, GENESIS_BITS, "in-boundary nbits != genesis");
    assert_eq!(rs.in_epoch_start, GENESIS_TIME, "in-boundary epoch_start != genesis time");
    assert_eq!(normalize_host(rs.in_roots.clone()), normalize_host(Forest::new().roots()), "in-boundary UTXO roots != empty");
    assert_eq!(rs.in_recent, vec![GENESIS_TIME], "in-boundary recent-times != [genesis time]");
    assert_eq!(rs.in_time, GENESIS_TIME, "in-boundary prev-time != genesis time");
    // AUDIT #3 F-3. #54 added the coinbase-SMT root to the boundary and pinned it in `rangestate`;
    // this gate — behind verify-range, verify-snark, verify-any's conditional pin, and therefore the
    // coordinator's spine — did not get it. A spine folded from a FABRICATED genesis SMT root passed
    // verify-range. Same drift class as F-2, and the reason the shared predicate is called here rather
    // than the assertion being copied a third time.
    //
    // Compared against `empty_root()` directly rather than via `rangestate::is_genesis_anchored`,
    // because the host deliberately keeps its own `RangeState` mirror — `check-rangestate.sh` gates
    // that mirror field-for-field against the shared crate, which is what stops THIS copy drifting.
    // A tree of empty hashes does not fold to zero, so a zeroed field is not accidentally correct.
    assert_eq!(rs.in_smt_root, hazync_coinbase_smt::empty_root(),
        "in-boundary coinbase-SMT (BIP30) root is not the empty tree — the range claims genesis but \
         starts from a fabricated BIP30 history");
}

// `verify-range <bin>`: verify a range proof and PIN its leftmost boundary to the genesis anchor.
pub(crate) fn verify_range_cmd(bin: &str) {
    let r: risc0_zkvm::Receipt = bincode::deserialize(&std::fs::read(bin).expect("bin")).unwrap();
    verify_receipt(&r);
    let rs: RangeState = r.journal.decode().unwrap();
    assert!(rs.self_id == METHOD_ID, "self_id != METHOD_ID");
    assert!(rs.kind == KIND_RANGE, "receipt is not a RangeState (domain tag)"); // H8
    assert_eq!(rs.lo, 1, "range must start at block 1 (genesis-anchored)");
    assert_genesis_in_boundary(&rs);
    let mut total = arr_u128(GENESIS_WORK);
    add256_host(&mut total, &rs.range_work);
    println!(">>> RANGE PROOF [1..{}] VERIFIED — genesis-anchored, one succinct receipt.", rs.hi);
    println!("  out_tip_hash {}  range_work {}  total_cum_work {}  UTXO leaves {}",
        hex(&rs.out_tip_hash), work_u128(&rs.range_work), work_u128(&total), rs.out_leaves);
}

// `verify-snark <out.snark>`: verify a Groth16-wrapped range proof and PIN it to genesis.
//
// This deliberately repeats EVERY assertion `verify-range` makes. A wrap is only worth having if it
// is exactly as strong as what it replaces; a verifier that skipped the genesis pin because the
// receipt "came from" a folded proof would accept a fabricated-anchor range in a smaller package.
pub(crate) fn verify_snark_cmd(bin: &str) {
    let r: risc0_zkvm::Receipt = bincode::deserialize(&std::fs::read(bin).expect("bin")).unwrap();
    let claimed = r.journal.decode::<RangeState>().ok().map(|rs| rs.self_id);
    verify_receipt_ex(&r, claimed);
    let rs: RangeState = r.journal.decode().unwrap();
    assert!(rs.self_id == METHOD_ID, "self_id != METHOD_ID");
    assert!(rs.kind == KIND_RANGE, "receipt is not a RangeState (domain tag)"); // H8
    assert_eq!(rs.lo, 1, "range must start at block 1 (genesis-anchored)");
    assert_genesis_in_boundary(&rs);
    let mut total = arr_u128(GENESIS_WORK);
    add256_host(&mut total, &rs.range_work);
    let sz = std::fs::metadata(bin).map(|m| m.len()).unwrap_or(0);
    println!(">>> SNARK RANGE PROOF [1..{}] VERIFIED — genesis-anchored, {} bytes.", rs.hi, sz);
    println!("  out_tip_hash {}  range_work {}  total_cum_work {}  UTXO leaves {}",
        hex(&rs.out_tip_hash), work_u128(&rs.range_work), work_u128(&total), rs.out_leaves);
}

// Verify a range receipt WITHOUT the genesis assertion — the CPU check a coordinator runs on each
// submitted contribution. Confirms the STARK is valid and reports the committed [lo,hi] + boundary
// tips, so the coordinator can chain ranges (out_tip of k == in_tip of k+1) into a genesis-anchored
// frontier without doing any proving/folding itself.
pub(crate) fn verify_any_cmd(bin: &str) {
    let r: risc0_zkvm::Receipt = bincode::deserialize(&std::fs::read(bin).expect("bin")).unwrap();
    // The journal (public output) decodes without verification — read the proof's own committed guest id
    // first, so a verify failure can be classified as a build MISMATCH vs a genuinely INVALID proof.
    let claimed = r.journal.decode::<RangeState>().ok().map(|rs| rs.self_id);
    verify_receipt_ex(&r, claimed); // real STARK verification (distinguishes mismatch from forgery)
    let rs: RangeState = r.journal.decode().unwrap();
    assert!(rs.self_id == METHOD_ID, "self_id != METHOD_ID");
    assert!(rs.kind == KIND_RANGE, "receipt is not a RangeState (domain tag)"); // H8
    // If this range CLAIMS to connect to genesis, its full genesis in-boundary must be pinned — else a
    // prover fabricates the initial UTXO set / difficulty and the coordinator chains it into the frontier.
    if rs.in_tip_hash == arr(rev(hx(GENESIS_HASH))) {
        assert_genesis_in_boundary(&rs);
    }
    // Whether this receipt proves anything about the chain FROM GENESIS, as opposed to proving a
    // correct transition between two boundaries it states itself.
    //
    // Both conditions are required. `in_tip_hash == genesis` alone is what triggers the pin above,
    // but a receipt could carry the genesis in-tip while claiming lo != 1; the standalone verifier
    // (verifier/src/lib.rs) requires lo == 1 as one of its five assertions, so this agrees with it
    // rather than inventing a second, weaker notion of "anchored".
    let anchored = rs.in_tip_hash == arr(rev(hx(GENESIS_HASH))) && rs.lo == 1;
    // Expose FULL-boundary digests so the coordinator chains on `out_bhash(k) == in_bhash(k+1)` — the
    // complete seam check the guest fold does (tip + UTXO roots + leaves + difficulty + MTP window), not
    // just tip-hash. Without this a mid-chain range can fabricate its in-boundary UTXO set / difficulty.
    // in-boundary = the chain state after block lo-1; out-boundary = after block hi (H9 height binding).
    let in_bh = boundary_digest(rs.lo.saturating_sub(1), &rs.in_tip_hash, &rs.in_roots, rs.in_leaves, rs.in_nbits, rs.in_time, rs.in_epoch_start, &rs.in_recent, &rs.in_smt_root);
    let out_bh = boundary_digest(rs.hi, &rs.out_tip_hash, &rs.out_roots, rs.out_leaves, rs.out_nbits, rs.out_time, rs.out_epoch_start, &rs.out_recent, &rs.out_smt_root);
    // `anchored` is APPENDED, never inserted: the coordinator parses this line as
    // `dict(t.split("=",1) for t in line[len("RANGE-OK"):].split() if "=" in t)`, so a new key=value
    // token is picked up by older coordinators as an extra key and by newer ones as the flag. Moving
    // or renaming an existing token would not be safe; adding one is.
    println!("RANGE-OK lo={} hi={} in_tip={} out_tip={} out_leaves={} range_work={} in_bhash={} out_bhash={} anchored={}",
        rs.lo, rs.hi, hex(&rs.in_tip_hash), hex(&rs.out_tip_hash), rs.out_leaves, work_u128(&rs.range_work),
        hex(&in_bh), hex(&out_bh), if anchored { "yes" } else { "no" });
    // Say it in prose too. `RANGE-OK` reads as unqualified success, and for a mid-chain receipt that
    // is a stronger claim than the proof supports: it attests a correct transition between the
    // boundaries it states, NOT that those boundaries descend from the real genesis. A reader who
    // treats it as "this proves the chain from genesis" is wrong, and the exit code alone (0 here,
    // because the SNARK is valid) does not tell them so. Raised by external review 2026-08-01 (L-1).
    if !anchored {
        println!("NOTE: this range is NOT genesis-anchored — it proves a correct transition between");
        println!("      its own stated boundaries, not that they descend from the real genesis.");
        println!("      Anchoring is established by the connected chain (the board's frontier) or by");
        println!("      `verify-range` / `verify-chain`, which pin genesis. Use those to conclude more.");
    }
}

// `verify-chain <bin>`: verify a bare ChainState (mode-2/mode-5) receipt and PIN its committed anchor to
// the genesis anchor (S5). This is the chain-track analogue of verify-range's genesis in-boundary pin:
// without it an is_base=1 receipt built on a FABRICATED anchor (arbitrary height/UTXO/work/easy nbits) is
// journal-indistinguishable from a genuine genesis-anchored one. The expected anchor_id is the double-
// SHA256 of the canonical genesis-anchor journal — exactly what the guest commits in the base step.
pub(crate) fn verify_chain_cmd(bin: &str) {
    // A libzync ChainState is only meaningful with the profile, the block forest and the rest of libzync-verify's
    // checks; this command checks none of them, so it would call a chain that spends a
    // fabricated coin VERIFIED (review 5). Refuse rather than give a partial verdict.
    if methods::LIBZYNC {
        eprintln!("verify-chain: this is a libzync build; use `libzync-verify <receipt>`");
        std::process::exit(1);
    }
    let r: risc0_zkvm::Receipt = bincode::deserialize(&std::fs::read(bin).expect("bin")).unwrap();
    // Same refusals as libzync-verify (review 2, #16): a FakeReceipt "verifies" under RISC0_DEV_MODE.
    assert!(!matches!(r.inner, risc0_zkvm::InnerReceipt::Fake(_)), "dev-mode FakeReceipt is not a proof");
    assert!(!dev_mode_env(), "RISC0_DEV_MODE is set in this verifier's environment");
    let claimed = r.journal.decode::<ChainState>().ok().map(|cs| cs.self_id);
    verify_receipt_ex(&r, claimed); // real STARK verification (distinguishes build-mismatch from forgery)
    let cs: ChainState = r.journal.decode().unwrap();
    assert!(cs.self_id == METHOD_ID, "self_id != METHOD_ID");
    assert!(cs.kind == KIND_CHAIN, "receipt is not a ChainState (domain tag)"); // H8
    let expected_anchor = bitcoin::hashes::sha256d::Hash::hash(&state_journal_bytes(&genesis_anchor())).to_byte_array();
    assert_eq!(cs.anchor_id, expected_anchor, "chain NOT anchored at genesis (S5): anchor_id mismatch");
    // time-too-new on this verifier's clock, as libzync-verify does (the proof commits the largest timestamp).
    let (now, ovr) = verifier_clock();
    assert!(time_ok(cs.max_time, now),
            "time-too-new: largest block time {} > this clock {now} + {MAX_FUTURE_BLOCK_TIME} s", cs.max_time);
    if let Some(v) = ovr {
        eprintln!("verify-chain: HAZYNC_VERIFY_NOW={v} is set (test only): not reporting a verdict");
        std::process::exit(1);
    }
    println!(">>> CHAIN PROOF [genesis..{}] VERIFIED — genesis-anchored, self-authenticating.", cs.height);
    println!("  tip_hash {}  cum_work {}  UTXO leaves {}", hex(&cs.tip_hash), work_u128(&cs.cum_work), cs.utxo_leaves);
}

// Print a saved receipt's journal digest and verify it. The gate for segment distribution is that a
// distributed prove and a monolithic prove of the SAME chunk agree on this value -- proving that
// routing every segment across a wire changed nothing about what was proved.
//
// Printing the digest from `seg-distribute` alone proves nothing: it would agree with itself.
pub(crate) fn receipt_digest_cmd(path: &str) {
    use risc0_zkvm::sha::Digestible;
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let r: risc0_zkvm::Receipt = bincode::deserialize(&bytes).expect("deserialize receipt");
    match r.verify(METHOD_ID) {
        Ok(()) => println!("VERIFIED against METHOD_ID"),
        Err(e) => { println!("VERIFY FAILED: {e}"); std::process::exit(1); }
    }
    println!("journal_bytes {}", r.journal.bytes.len());
    println!("journal_digest {}", hex(r.journal.digest().as_bytes()));
}
