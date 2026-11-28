//! Deposit-proof tests on REAL engine proofs: the engine chain's proof of mainnet 1..200 and its history, the engine's
//! header-chain proof of 0..4,031 with block 200's MMR path to it, and real block 170 (Satoshi to Hal). Both proofs are
//! below Core's minimum work, so a real verdict is at best DEMO; the OK paths use the same proof under an image with no
//! minimum work (zero_work_img). The RISC Zero receipts' cases (the whole header chain to 975,278, a block above the
//! core tip, Groth16) went with the cutover (27.11.2026).
use super::*;

// The engine chain's proof of mainnet 1..200 and its history (node format 2), and the engine's header-chain proof of
// 0..4,031 with block 200's MMR path to that tip (engine proofs only since the cutover, 27.11.2026).
const CORE: &[u8] = include_bytes!("../../../swiftsync-verify/vectors/mainnet_1_200_engine.hze");
const CORE_FLIPPED: &[u8] = include_bytes!("../../../swiftsync-verify/vectors/mainnet_1_200_engine_proof_flipped.hze");
const CORE_VK: &[u8] = include_bytes!("../../../swiftsync-verify/vectors/mainnet_1_200_engine.vk");
const CORE_ID: &str = include_str!("../../../swiftsync-verify/vectors/mainnet_1_200_engine.image_id");
const HIST: &[u8] = include_bytes!("../../../swiftsync-verify/tests/data/engine200/history.bin");
const HDR: &[u8] = include_bytes!("../../headers/tests/data/engine4031/proof.hze");
const HDR_VK: &[u8] = include_bytes!("../../headers/tests/data/engine4031/vk.bin");
const HDR_ID: &str = include_str!("../../headers/tests/data/engine4031/image_id");
const PATH_200: &str = include_str!("../../headers/tests/data/engine4031/mmr_path_200_tip4031.txt");
const BLOCK_170: &str = include_str!("../../../experimental/libzync-bench/deposit/block_170_raw.txt");
const SEG: &str = include_str!("../../../experimental/libzync-bench/headers/tx/tx_seg.json");
/// block 170, position 1: Satoshi to Hal Finney, 10 BTC at output 0
const T170: &str = "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16";
const HAL_SPK: &str = "4104ae1a62fe09c5f51b13905f07f06b99a2f7159b2225f374cd378d71302fa28414e7aab37397f554a7df5f142c21c1b7303b8a0626f1baded5c72a704f7e6cd84cac";
const TIP: (i64, &str) = (4031, "00000000f037ad09d0b05ee66b8c1da83030abaf909d2b1bf519c3c7d2cd3fdf");
const H200: &str = "000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320";

fn now() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() }
fn id(display: &str) -> [u8; 32] { let mut b = hex32(display).unwrap(); b.reverse(); b }
fn lines(t: &str) -> Vec<[u8; 32]> { t.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| hex32(l).unwrap()).collect() }
fn block_170() -> Vec<u8> { unhex(BLOCK_170.trim()).unwrap() }
fn txs_170() -> Vec<libzync::facts::TxView> { libzync::facts::parse_block(&block_170()).unwrap().1 }
fn coinbase_170() -> [u8; 32] { libzync::facts::txid(&txs_170()[0]) }

fn b_core(txid: &[u8; 32]) -> DepositBundle { make_bundle("mainnet", CORE, &[], Some(HIST), &block_170(), 170, txid, &[], None).unwrap() }
/// Bound to the header chain 0..4,031 (below Core's minimum work: DEMO at best).
fn b_tip() -> DepositBundle {
    make_bundle("mainnet", CORE, &[HDR.to_vec()], Some(HIST), &block_170(), 170, &id(T170), &lines(PATH_200), None).unwrap()
}
/// The engine core image with its key.
fn core_img() -> &'static Image {
    Image::by_network("mainnet").unwrap().with_method_id(CORE_ID.trim()).unwrap().with_engine_key_unchecked(CORE_VK.to_vec())
}
/// The same image with no minimum work and no checkpoint: the only way a 200-block chain reaches OK, for the verdict's
/// OK paths (a state no network has, labelled as such).
fn zero_work_img() -> &'static Image {
    Box::leak(Box::new(Image { min_work_hex: "0000000000000000000000000000000000000000000000000000000000000000", checkpoint_height: 0, ..*core_img() }))
}
fn hdr_key() -> swiftsync_verify::EngineKey<'static> {
    use sha2::Digest as _;
    swiftsync_verify::EngineKey { bytes: HDR_VK, pin: sha2::Sha256::digest(HDR_VK).into() }
}
fn run_on(b: &DepositBundle, a: &DepositAsk, img: &'static Image) -> DepositReport {
    verify_deposit(b, a, "mainnet", img, HDR_ID.trim(), Some(&hdr_key()), now())
}
/// The checks' own tests read the statement of DEMO bundles and of OK ones without a tip source: the gating defaults
/// (require_identity on, allow_demo off) are tested on their own (the_verdict_gates_and_accepted).
fn ask(txid: [u8; 32]) -> DepositAsk { DepositAsk { txid, min_depth: 1, require_identity: false, allow_demo: true, ..Default::default() } }
fn run(b: &DepositBundle, a: &DepositAsk) -> DepositReport {
    run_on(b, a, core_img())
}
/// A refusal: verdict Fail, no statement, and a failed check that says `why`.
fn refused(b: &DepositBundle, a: &DepositAsk, why: &str) {
    let r = run(b, a);
    let failed: Vec<&String> = r.checks.iter().filter(|c| !c.0).map(|c| &c.1).collect();
    assert_eq!(r.verdict, Verdict::Fail, "want a refusal for {why:?}");
    assert!(r.statement.is_none());
    assert!(failed.iter().any(|w| w.contains(why)), "no failed check says {why:?}: {failed:?}");
}

#[test]
fn core_only_block_170_is_demo_and_says_what_it_shows() {
    let a = DepositAsk { vout: Some(0), amount: Some(1_000_000_000), script: unhex(HAL_SPK), min_depth: 31, ..ask(id(T170)) };
    let r = run(&b_core(&id(T170)), &a);
    assert_eq!(r.verdict, Verdict::Demo, "a 0..200 chain is below Core's minimum chain work");
    let s = r.statement.unwrap();
    assert_eq!((s.height, disp(&s.block_hash).as_str()), (170, "00000000d1145790a8694403d4063f323d499e655c83426834d4ce2f8dd4a2ee"));
    assert_eq!((s.confirmations, s.validated_confirmations, s.validated_through, s.block_fully_validated), (31, 31, 200, true));
    assert_eq!(s.output, Some((0, 1_000_000_000, unhex(HAL_SPK).unwrap())));
    assert_eq!(s.core_image, CORE_ID.trim());
    assert_eq!(s.headers_image, None);
    // the coinbase of the same block, through the same core proof: in the block, but 31 confirmations are not Core's
    // 100 (review R3-5 L5: a deposit names coins someone can spend, and a younger coinbase cannot be)
    refused(&b_core(&coinbase_170()), &ask(coinbase_170()), "coinbase not mature");
}

/// Review R3-5 L5: a coinbase deposit needs COINBASE_MATURITY (100) confirmations; at 3,862 (bound to the header chain)
/// it passes, at 31 (the core proof alone) it FAILs, and a non-coinbase transaction is not held to it.
#[test]
fn a_coinbase_deposit_needs_maturity() {
    let cb = coinbase_170();
    let tip = make_bundle("mainnet", CORE, &[HDR.to_vec()], Some(HIST), &block_170(), 170, &cb, &lines(PATH_200), None).unwrap();
    let a = DepositAsk { expect_tip: Some((TIP.0, TIP.1.into())), ..ask(cb) };
    let r = run(&tip, &a);
    assert_eq!(r.verdict, Verdict::Demo, "{:?}", r.checks.iter().filter(|c| !c.0).collect::<Vec<_>>());
    assert!(r.checks.iter().any(|c| c.0 && c.1.starts_with("coinbase maturity") && c.1.contains("3862 as of tip")), "{:?}", r.checks);
    refused(&b_core(&cb), &ask(cb), "coinbase not mature");
    let t = run(&b_core(&id(T170)), &ask(id(T170)));
    assert!(t.checks.iter().all(|c| !c.1.starts_with("coinbase maturity")), "a non-coinbase transaction is not held to maturity");
}

#[test]
fn bound_to_the_header_chain_splits_the_depth() {
    let a = DepositAsk { vout: Some(0), amount: Some(1_000_000_000), min_depth: 6, expect_tip: Some((TIP.0, TIP.1.into())), ..ask(id(T170)) };
    let r = run(&b_tip(), &a);
    assert_eq!(r.verdict, Verdict::Demo, "4,032 headers are below Core's minimum work: {:?}", r.checks.iter().filter(|c| !c.0).collect::<Vec<_>>());
    assert!(r.checks.iter().any(|c| c.0 && c.1.starts_with("BINDING")));
    let s = r.statement.unwrap();
    assert_eq!((s.tip_height, disp(&s.tip_hash).as_str()), TIP);
    assert_eq!((s.confirmations, s.validated_confirmations, s.validated_through), (3862, 31, 200));
    assert_eq!(s.headers_image.as_deref(), Some(HDR_ID.trim()));
    // the depth asked counts every confirmation; the validated depth only those at or below the core tip
    refused(&b_tip(), &DepositAsk { min_depth: 3863, ..a.clone() }, "depth: 3862 confirmation(s)");
    assert_eq!(run(&b_tip(), &DepositAsk { min_validated_depth: 31, ..a.clone() }).verdict, Verdict::Demo);
    refused(&b_tip(), &DepositAsk { min_validated_depth: 32, ..a.clone() }, "validated depth: 31");
    // header runs without the header image's key: refused
    let r = verify_deposit(&b_tip(), &a, "mainnet", core_img(), HDR_ID.trim(), None, now());
    assert_eq!(r.verdict, Verdict::Fail);
}

#[test]
fn the_transaction_is_the_callers() {
    // the bundle is a valid proof for T170; asked about the coinbase it is refused, and the reverse
    refused(&b_core(&id(T170)), &ask(coinbase_170()), "the one asked about");
    refused(&b_core(&coinbase_170()), &ask(id(T170)), "the one asked about");
    // the bundle has no txid to lie with
    let j = serde_json::to_value(b_core(&id(T170))).unwrap();
    assert!(j.get("txid").is_none() && j["block"].get("txid").is_none());
}

#[test]
fn wrong_output_amount_or_script() {
    let b = b_core(&id(T170));
    refused(&b, &DepositAsk { vout: Some(2), ..ask(id(T170)) }, "output 2 exists (the transaction has 2)");
    refused(&b, &DepositAsk { vout: Some(0), amount: Some(999_999_999), ..ask(id(T170)) }, "output 0 holds 999999999 sat");
    refused(&b, &DepositAsk { vout: Some(1), amount: Some(1_000_000_000), ..ask(id(T170)) }, "output 1 holds 1000000000 sat");
    let mut spk = unhex(HAL_SPK).unwrap();
    spk[5] ^= 1;
    refused(&b, &DepositAsk { vout: Some(0), script: Some(spk), ..ask(id(T170)) }, "output 0's scriptPubKey");
    refused(&b, &DepositAsk { amount: Some(1_000_000_000), ..ask(id(T170)) }, "without the output index");
    refused(&b, &DepositAsk { vout: Some(0), op_return: Some(vec![1]), ..ask(id(T170)) }, "not a data-only OP_RETURN");
}

#[test]
fn insufficient_depth() {
    let b = b_core(&id(T170));
    assert_ne!(run(&b, &DepositAsk { min_depth: 31, ..ask(id(T170)) }).verdict, Verdict::Fail);
    refused(&b, &DepositAsk { min_depth: 32, ..ask(id(T170)) }, "depth: 31 confirmation(s) as of tip 200 >= 32");
}

#[test]
fn a_block_not_in_the_chain() {
    let good = b_core(&id(T170));
    let a = ask(id(T170));
    // another height for the same block, a tampered coin state, no forest opening
    let mut b = good.clone(); b.block.height = 171; refused(&b, &a, "not in the proof's block forest");
    let mut b = good.clone(); b.block.forest.as_mut().unwrap().state = hex(&[7; 32]); refused(&b, &a, "not in the proof's block forest");
    let mut b = good.clone(); b.block.forest.as_mut().unwrap().path[0] = hex(&[7; 32]); refused(&b, &a, "not in the proof's block forest");
    let mut b = good.clone(); b.block.forest = None; refused(&b, &a, "give its block-forest opening");
    // a header that is not block 170's (one bit of the nonce)
    let mut h = unhex(&good.block.header).unwrap(); h[79] ^= 1;
    let mut b = good.clone(); b.block.header = hex(&h); refused(&b, &a, "not in the proof's block forest");
    // a block above the core tip with no header-chain proof
    let mut b = good.clone(); b.block.height = 201; b.block.forest = None; b.block.mmr_path = Some(vec![]);
    refused(&b, &DepositAsk { allow_pow_only_block: true, ..a.clone() }, "above the core tip 200 and no header-chain proof");
}

#[test]
fn a_tampered_branch() {
    let good = b_core(&id(T170));
    let a = ask(id(T170));
    let flip = |x: &str| { let mut v = unhex(x).unwrap(); v[0] ^= 1; hex(&v) };
    let mut b = good.clone(); b.block.branch[0] = flip(&b.block.branch[0]); refused(&b, &a, "is not at position 1 of block 170");
    let mut b = good.clone(); b.block.coinbase_branch[0] = flip(&b.block.coinbase_branch[0]); refused(&b, &a, "is not at position 1 of block 170");
    let mut b = good.clone(); b.block.index = 0; refused(&b, &a, "is not at position 0 of block 170");
    let mut b = good.clone(); b.block.branch.push(b.block.branch[0].clone()); refused(&b, &a, "is not at position 1 of block 170");
    // a raw transaction with one byte changed is another transaction, not at that position
    let mut b = good.clone(); let mut t = unhex(&b.block.tx).unwrap(); t[50] ^= 1; b.block.tx = hex(&t);
    refused(&b, &a, "is not at position 1 of block 170");
}

#[test]
fn op_return_payload_rules() {
    assert_eq!(op_return_payload(&[0x6a]), Some(vec![]));
    assert_eq!(op_return_payload(&[0x6a, 0x02, 1, 2, 0x01, 3]), Some(vec![1, 2, 3]));
    assert_eq!(op_return_payload(&[0x6a, 0x4c, 0x02, 1, 2]), Some(vec![1, 2]));
    assert_eq!(op_return_payload(&[0x6a, 0x4d, 0x01, 0x00, 9]), Some(vec![9]));
    assert_eq!(op_return_payload(&[0x6a, 0x4e, 0x01, 0, 0, 0, 9]), Some(vec![9]));
    assert_eq!(op_return_payload(&[0x6a, 0x00]), Some(vec![]));
    assert_eq!(op_return_payload(&[0x6a, 0x51]), None, "OP_1 is not a data push");
    assert_eq!(op_return_payload(&[0x6a, 0x03, 1, 2]), None, "a push past the end");
    assert_eq!(op_return_payload(&[0x6a, 0x4e, 0xff, 0xff, 0xff, 0xff]), None);
    assert_eq!(op_return_payload(&[0x51, 0x01, 1]), None, "not OP_RETURN");
    assert_eq!(op_return_payload(&[]), None);
}

#[test]
fn dev_mode_and_broken_receipts() {
    let a = ask(id(T170));
    let mut b = b_core(&id(T170)); b.core_receipt = hex(&[1, 0, 0, 0, 9, 9]); refused(&b, &a, "the core chain proof verifies");
    let mut b = b_core(&id(T170)); b.core_receipt = hex(CORE_FLIPPED); refused(&b, &a, "the core chain proof verifies");
    let mut f = HDR.to_vec(); f[4096] ^= 1;
    let mut b = b_tip(); b.header_receipts = vec![hex(&f)]; refused(&b, &a, "the header-chain proof verifies");
    let mut b = b_core(&id(T170)); b.core_receipt = "zz".into(); refused(&b, &a, "core receipt: not hex");
}

#[test]
fn the_binding_between_the_two_proofs() {
    let a = ask(id(T170));
    let mut b = b_tip(); b.core_tip_mmr_path[0] = hex(&[3; 32]); refused(&b, &a, "BINDING");
    let mut b = b_tip(); b.core_tip_mmr_path.clear(); refused(&b, &a, "BINDING");
    let mut b = b_core(&id(T170)); b.core_tip_mmr_path = vec![hex(&[3; 32])]; refused(&b, &a, "without header receipts");
    // the same header run twice (the second does not start where the first ended)
    let mut b = b_tip(); b.header_receipts = vec![hex(HDR), hex(HDR)];
    refused(&b, &a, "run 2 starts where run 1 ended");
}

#[test]
fn network_identity_and_freshness() {
    let a = ask(id(T170));
    let mut b = b_tip(); b.network = "signet".into(); refused(&b, &a, "the network asked about");
    let mut b = b_tip(); b.kind = "other".into(); refused(&b, &a, "a deposit bundle");
    let r = verify_deposit(&b_tip(), &a, "signet", Image::by_network("signet").unwrap(), HDR_ID.trim(), Some(&hdr_key()), now());
    assert_eq!(r.verdict, Verdict::Fail);
    refused(&b_tip(), &DepositAsk { expect_tip: Some((TIP.0, "00".repeat(32))), ..a.clone() }, "chain identity");
    refused(&b_tip(), &DepositAsk { expect_tip: Some((TIP.0 - 1, TIP.1.into())), ..a.clone() }, "chain identity");
    // the tip's block time (4,031: 2009) against this clock
    refused(&b_tip(), &DepositAsk { max_tip_age: Some(60), ..a.clone() }, "is within 60 s");
    assert_eq!(run(&b_tip(), &DepositAsk { max_tip_age: Some(100 * 365 * 86_400), ..a.clone() }).verdict, Verdict::Demo);
}

/// Review (11-15): a DEMO chain is a cheap difficulty-1 fork, so its statement is withheld unless asked for and is never
/// accepted; and an OK verdict needs the tip compared with a source the caller trusts unless the caller turns that off.
#[test]
fn the_verdict_gates_and_accepted() {
    let base = DepositAsk { txid: id(T170), vout: Some(0), amount: Some(1_000_000_000), ..Default::default() };
    assert!(base.require_identity && !base.allow_demo, "the defaults gate");
    let z = zero_work_img();
    let h200 = (200, H200.to_string());
    // enough work (the zero-work image), the tip expected: accepted
    let r = run_on(&b_core(&id(T170)), &DepositAsk { expect_tip: Some(h200.clone()), ..base.clone() }, z);
    assert_eq!((r.verdict, r.accepted, r.identity_checked, r.statement.is_some()), (Verdict::Ok, true, true, true));
    // no tip source: DEMO, not accepted, no statement
    let r = run_on(&b_core(&id(T170)), &base, z);
    assert_eq!((r.verdict, r.accepted, r.identity_checked, r.statement.is_some()), (Verdict::Demo, false, false, false));
    assert!(r.checks.iter().any(|c| c.0 && c.1.starts_with("chain identity NOT checked")));
    // ...unless the caller turns the requirement off: OK and accepted, the identity not checked
    let r = run_on(&b_core(&id(T170)), &DepositAsk { require_identity: false, ..base.clone() }, z);
    assert_eq!((r.verdict, r.accepted, r.identity_checked, r.statement.is_some()), (Verdict::Ok, true, false, true));
    // the core proof alone on the real image (below the minimum work and the 970,000 checkpoint), its tip expected:
    // DEMO, withheld
    let a = DepositAsk { expect_tip: Some(h200), ..base.clone() };
    let r = run(&b_core(&id(T170)), &a);
    assert_eq!((r.verdict, r.accepted, r.identity_checked, r.statement.is_some()), (Verdict::Demo, false, true, false));
    // allow_demo shows it, still not accepted
    let r = run(&b_core(&id(T170)), &DepositAsk { allow_demo: true, ..a.clone() });
    assert_eq!((r.verdict, r.accepted, r.statement.is_some()), (Verdict::Demo, false, true));
    // a failed check: nothing, whatever the flags
    let r = run_on(&b_core(&id(T170)), &DepositAsk { allow_demo: true, require_identity: false, min_depth: 32, ..base.clone() }, z);
    assert_eq!((r.verdict, r.accepted, r.identity_checked, r.statement.is_some()), (Verdict::Fail, false, false, false));
    // a wrong tip is a failed check, not a missing identity
    let r = run_on(&b_core(&id(T170)), &DepositAsk { expect_tip: Some((200, "00".repeat(32))), ..base.clone() }, z);
    assert_eq!((r.verdict, r.accepted), (Verdict::Fail, false));
}

/// Review (11-15): the fully validated path with the transaction in its witness serialisation (BIP 144: marker, flag,
/// a witness stack per input). The txid is over the serialisation without the witness, so the same statement holds;
/// the bytes after the witness are still bound (one appended byte is refused).
#[test]
fn a_witness_serialised_transaction_in_a_fully_validated_block() {
    let good = b_core(&id(T170));
    let raw = unhex(&good.block.tx).unwrap();
    let v = libzync::facts::parse_tx(&raw, &mut 0).unwrap();
    assert_eq!(v.stripped, raw, "the block's form has no witness");
    // version || 00 01 || inputs and outputs || one witness item of 3 bytes for its one input || lock time
    let mut w = raw[..4].to_vec();
    w.extend_from_slice(&[0x00, 0x01]);
    w.extend_from_slice(&raw[4..raw.len() - 4]);
    assert_eq!(v.inputs.len(), 1);
    w.extend_from_slice(&[0x01, 0x03, 0xaa, 0xbb, 0xcc]);
    w.extend_from_slice(&raw[raw.len() - 4..]);
    let mut b = good.clone();
    b.block.tx = hex(&w);
    let a = DepositAsk { vout: Some(0), amount: Some(1_000_000_000), ..ask(id(T170)) };
    let r = run(&b, &a);
    assert_eq!(r.verdict, Verdict::Demo, "{:?}", r.checks.iter().filter(|c| !c.0).collect::<Vec<_>>());
    assert_eq!(r.statement, run(&good, &a).statement, "the same statement as the block's own serialisation");
    let mut x = w.clone(); x.push(0);
    let mut b2 = good.clone(); b2.block.tx = hex(&x);
    assert_eq!(run(&b2, &a).verdict, Verdict::Fail);
}

/// The engine chain's block leaves commit the undo hash (0x05 leaves, docs/libzync/UNDO.md): block 170's forest opening
/// carries it, and a wrong one, or none, is refused.
#[test]
fn an_undo_leaf_block_of_the_engine_chain() {
    let b = b_core(&id(T170));
    let a = ask(id(T170));
    assert!(b.block.forest.as_ref().unwrap().undo.is_some());
    let mut x = b.clone(); x.block.forest.as_mut().unwrap().undo = Some(hex(&[1; 32]));
    refused(&x, &a, "not in the proof's block forest");
    let mut x = b.clone(); x.block.forest.as_mut().unwrap().undo = None;
    refused(&x, &a, "undo");
}

// ---- verdict paths (docs/libzync/ZAPP_TESTING.md §3.5, the cargo-mutants pass of 17.11.2026)

/// The "chain identity NOT checked" line appears only where it applies: a chain with the minimum work, no failed
/// check, the requirement on and no identity. Below the minimum work (the core proof alone) the DEMO is the work's,
/// and there is no such line. The validated-depth line appears only when a validated depth is asked (default 1).
#[test]
fn identity_and_validated_depth_lines_only_where_they_apply() {
    let base = DepositAsk { txid: id(T170), ..Default::default() };
    let z = zero_work_img();
    let note = |r: &DepositReport| r.checks.iter().any(|c| c.1.starts_with("chain identity NOT checked"));
    assert!(note(&run_on(&b_core(&id(T170)), &base, z)));
    let r = run(&b_core(&id(T170)), &base);
    assert_eq!(r.verdict, Verdict::Demo);
    assert!(!note(&r), "{:?}", r.checks);
    assert!(!note(&run_on(&b_core(&id(T170)), &DepositAsk { expect_tip: Some((200, H200.into())), ..base.clone() }, z)));
    assert!(!note(&run_on(&b_core(&id(T170)), &DepositAsk { require_identity: false, ..base.clone() }, z)));
    let r = run_on(&b_core(&id(T170)), &DepositAsk { min_depth: 32, ..base.clone() }, z);
    assert_eq!(r.verdict, Verdict::Fail);
    assert!(!note(&r), "{:?}", r.checks);
    let vd = |r: &DepositReport| r.checks.iter().filter(|c| c.1.starts_with("validated depth")).count();
    assert_eq!(vd(&run(&b_core(&id(T170)), &base)), 1);
    assert_eq!(vd(&run(&b_core(&id(T170)), &DepositAsk { min_validated_depth: 0, ..base.clone() })), 0);
}

/// OP_PUSHDATA1 / 2 / 4 after another push (not at byte 1): the length and the data are read where they are.
#[test]
fn op_return_pushdata_after_another_push() {
    assert_eq!(op_return_payload(&[0x6a, 0x01, 7, 0x4d, 0x02, 0x00, 8, 9]), Some(vec![7, 8, 9]));
    assert_eq!(op_return_payload(&[0x6a, 0x02, 5, 6, 0x4d, 0x01, 0x00, 9, 0x01, 4]), Some(vec![5, 6, 9, 4]));
    assert_eq!(op_return_payload(&[0x6a, 0x01, 7, 0x4e, 0x01, 0, 0, 0, 9]), Some(vec![7, 9]));
    assert_eq!(op_return_payload(&[0x6a, 0x01, 7, 0x4c, 0x01, 9, 0x00]), Some(vec![7, 9]));
    assert_eq!(op_return_payload(&[0x6a, 0x01, 7, 0x4d, 0x03, 0x00, 8, 9]), None, "a PUSHDATA2 past the end");
}

/// Review 11-18: a field the bundle format does not name is refused (top level, block, forest opening), and a block
/// carries exactly the opening its height takes (a forest opening at or below the core tip, an MMR path above it).
#[test]
fn unknown_fields_and_the_other_opening_are_refused() {
    let good = serde_json::to_value(b_core(&id(T170))).unwrap();
    assert!(serde_json::from_value::<DepositBundle>(good.clone()).is_ok());
    for (at, key) in [("", "extra"), ("block", "extra"), ("forest", "extra"), ("block", "mmr_pathh"), ("", "header_receipt")] {
        let mut v = good.clone();
        let o = match at { "" => &mut v, "block" => &mut v["block"], _ => &mut v["block"]["forest"] };
        o.as_object_mut().unwrap().insert(key.into(), serde_json::json!([]));
        let e = serde_json::from_value::<DepositBundle>(v).expect_err(&format!("{at}.{key} accepted"));
        assert!(e.to_string().contains("unknown field"), "{at}.{key}: {e}");
    }
    // a block at or below the core tip with an MMR path besides its forest opening
    let a = ask(id(T170));
    let mut b = b_tip(); b.block.mmr_path = Some(vec![hex(&[1; 32])]); refused(&b, &a, "an mmr_path is not part of such a bundle");
    let mut b = b_core(&id(T170)); b.block.mmr_path = Some(vec![]); refused(&b, &a, "an mmr_path is not part of such a bundle");
}

/// Review 11-18: above the core tip, inclusion is checked by the header zAPP's transaction parser and the outputs are
/// read by libzync's. The deposit binds the outputs with libzync's own stripped form, so a disagreement FAILS rather
/// than reading another transaction's outputs; this pins that the two agree on every real transaction here, in both
/// serialisations (the stripped form each derives hashes to the same txid).
#[test]
fn the_two_transaction_parsers_agree() {
    let v: serde_json::Value = serde_json::from_str(SEG).unwrap();
    let t = TxInclusion::from_json(v["txid"].as_str().unwrap(), &v).unwrap();
    let mut cases = vec![(t.raw.clone(), t.pos, t.branch.clone(), t.header, t.coinbase.clone(), t.coinbase_branch.clone()),
                         (t.coinbase.clone(), 0, t.coinbase_branch.clone(), t.header, t.coinbase.clone(), t.coinbase_branch.clone())];
    let (header, txs) = libzync::facts::parse_block(&block_170()).unwrap();
    let ids: Vec<[u8; 32]> = txs.iter().map(libzync::facts::txid).collect();
    for (i, x) in txs.iter().enumerate() {
        cases.push((x.stripped.clone(), i as u32, libzync::facts::merkle_branch(&ids, i), header, txs[0].stripped.clone(),
                    libzync::facts::merkle_branch(&ids, 0)));
    }
    let mut witness = 0;
    for (raw, pos, branch, header, coinbase, coinbase_branch) in cases {
        let mut o = 0;
        let lz = libzync::facts::parse_tx(&raw, &mut o).filter(|_| o == raw.len()).expect("libzync parses it");
        if lz.stripped != raw { witness += 1; }
        let txid = libzync::facts::txid(&lz);
        let hash = dsha(&header);
        let inc = TxInclusion { header, txid, raw, pos, branch, coinbase, coinbase_branch };
        assert!(zapp_headers::verify_tx_in_header(&hash, &inc).is_ok(), "the header parser derives another txid at position {pos}");
    }
    assert!(witness >= 1, "no witness-serialised case");
}
