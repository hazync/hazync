//! Deposit-proof tests on REAL receipts: the core proof of mainnet 0..200 (image 8888d59c, the v0.23.0-v0.25.0 image the
//! verifier keeps by id), the header-chain proofs 0..100 + 101..200 and 0..975,278 (image de54b94f), and real
//! transactions (block 170: the coinbase and Satoshi to Hal Finney; block 800,000: a segwit transaction and the coinbase's
//! OP_RETURN witness commitment).
use super::*;
use crate::headers::HEADERS_IMAGE_ID;

const CORE: &[u8] = include_bytes!("../../vectors/mainnet_1_200_8888d59c.rcpt");
const CORE_FLIPPED: &[u8] = include_bytes!("../../vectors/mainnet_1_200_8888d59c_flipped.rcpt");
const FAKE: &[u8] = include_bytes!("../../vectors/fake_block_2501.rcpt");
const HIST: &[u8] = include_bytes!("../../../experimental/libzync-bench/deposit/history_0_200.bin");
const BLOCK_170: &str = include_str!("../../../experimental/libzync-bench/deposit/block_170_raw.txt");
const HDR_TIP: &[u8] = include_bytes!("../../../experimental/libzync-bench/headers/headers_0_tip.rcpt");
const HDR_0_100: &[u8] = include_bytes!("../../../experimental/libzync-bench/headers/hdr_0_100.rcpt");
const HDR_101_200: &[u8] = include_bytes!("../../../experimental/libzync-bench/headers/hdr_101_200.rcpt");
const PATH_200: &str = include_str!("../../../experimental/libzync-bench/deposit/mmr_path_200_tip975278.txt");
const PATH_800000: &str = include_str!("../../../experimental/libzync-bench/deposit/mmr_path_800000_tip975278.txt");
const SEG: &str = include_str!("../../../experimental/libzync-bench/headers/tx/tx_seg.json");
/// block 170, position 1: Satoshi to Hal Finney, 10 BTC at output 0
const T170: &str = "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16";
const HAL_SPK: &str = "4104ae1a62fe09c5f51b13905f07f06b99a2f7159b2225f374cd378d71302fa28414e7aab37397f554a7df5f142c21c1b7303b8a0626f1baded5c72a704f7e6cd84cac";
const TIP: (i64, &str) = (975_278, "00000000000000000001d951592f7a06f0d24697e4f9c362ae173fced55583e3");

fn now() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() }
fn id(display: &str) -> [u8; 32] { let mut b = hex32(display).unwrap(); b.reverse(); b }
fn lines(t: &str) -> Vec<[u8; 32]> { t.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| hex32(l).unwrap()).collect() }
fn block_170() -> Vec<u8> { unhex(BLOCK_170.trim()).unwrap() }
fn txs_170() -> Vec<libzync::facts::TxView> { libzync::facts::parse_block(&block_170()).unwrap().1 }
fn coinbase_170() -> [u8; 32] { libzync::facts::txid(&txs_170()[0]) }

fn b_core(txid: &[u8; 32]) -> DepositBundle { make_bundle("mainnet", CORE, &[], Some(HIST), &block_170(), 170, txid, &[], None).unwrap() }
fn b_tip() -> DepositBundle {
    make_bundle("mainnet", CORE, &[HDR_TIP.to_vec()], Some(HIST), &block_170(), 170, &id(T170), &lines(PATH_200), None).unwrap()
}
/// A block ABOVE the core tip: block 800,000, from the path service's answer (tx_seg.json), at position `which`
/// (1: the segwit transaction, 0: the coinbase)
fn b_seg(coinbase: bool) -> (DepositBundle, [u8; 32]) {
    let v: serde_json::Value = serde_json::from_str(SEG).unwrap();
    let t = TxInclusion::from_json(v["txid"].as_str().unwrap(), &v).unwrap();
    let mut b = b_tip();
    let (raw, index, branch) = if coinbase { (t.coinbase.clone(), 0, t.coinbase_branch.clone()) } else { (t.raw.clone(), t.pos, t.branch.clone()) };
    let mut o = 0;
    let txid = libzync::facts::txid(&libzync::facts::parse_tx(&raw, &mut o).unwrap());
    b.block = DepositBlock {
        height: 800_000, header: hex(&t.header), forest: None, mmr_path: Some(lines(PATH_800000).iter().map(|x| hex(x)).collect()),
        tx: hex(&raw), index, branch: branch.iter().map(|x| hex(x)).collect(), coinbase: hex(&t.coinbase),
        coinbase_branch: t.coinbase_branch.iter().map(|x| hex(x)).collect(),
    };
    (b, txid)
}
fn ask(txid: [u8; 32]) -> DepositAsk { DepositAsk { txid, min_depth: 1, ..Default::default() } }
fn run(b: &DepositBundle, a: &DepositAsk) -> DepositReport {
    verify_deposit(b, a, "mainnet", Image::by_network("mainnet").unwrap(), HEADERS_IMAGE_ID, now())
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
    assert_eq!(s.core_image, "8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4");
    assert_eq!(s.headers_image, None);
    // the coinbase of the same block, through the same core proof
    assert_eq!(run(&b_core(&coinbase_170()), &ask(coinbase_170())).verdict, Verdict::Demo);
}

#[test]
fn bound_to_the_whole_header_chain_is_ok_and_splits_the_depth() {
    let a = DepositAsk { vout: Some(0), amount: Some(1_000_000_000), min_depth: 6, expect_tip: Some((TIP.0, TIP.1.into())), ..ask(id(T170)) };
    let r = run(&b_tip(), &a);
    assert_eq!(r.verdict, Verdict::Ok, "{:?}", r.checks.iter().filter(|c| !c.0).collect::<Vec<_>>());
    let s = r.statement.unwrap();
    assert_eq!((s.tip_height, disp(&s.tip_hash).as_str()), TIP);
    assert_eq!((s.confirmations, s.validated_confirmations, s.validated_through), (975_109, 31, 200));
    assert_eq!(s.headers_image.as_deref(), Some(HEADERS_IMAGE_ID));
    // the depth asked counts every confirmation; the validated depth only those at or below the core tip
    refused(&b_tip(), &DepositAsk { min_depth: 975_110, ..a.clone() }, "depth: 975109 confirmation(s)");
    assert_eq!(run(&b_tip(), &DepositAsk { min_validated_depth: 31, ..a.clone() }).verdict, Verdict::Ok);
    refused(&b_tip(), &DepositAsk { min_validated_depth: 32, ..a.clone() }, "validated depth: 31");
}

#[test]
fn bound_to_a_low_work_header_chain_is_demo() {
    // headers 0..200 in two chained runs; block 200 is a one-leaf peak of 201 leaves: an empty binding path
    let b = make_bundle("mainnet", CORE, &[HDR_0_100.to_vec(), HDR_101_200.to_vec()], Some(HIST), &block_170(), 170, &id(T170), &[], None).unwrap();
    let r = run(&b, &ask(id(T170)));
    assert_eq!(r.verdict, Verdict::Demo);
    assert!(r.checks.iter().any(|c| c.0 && c.1.starts_with("BINDING")));
    assert_eq!(r.statement.unwrap().confirmations, 31);
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
    let (mut b, t) = b_seg(false);
    b.header_receipts.clear(); b.core_tip_mmr_path.clear();
    refused(&b, &DepositAsk { allow_pow_only_block: true, ..ask(t) }, "above the core tip 200 and no header-chain proof");
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

/// THE 64-BYTE TRICK. Block 170 has two transactions, so its merkle root IS dsha(coinbase txid || T170). Those 64 bytes,
/// presented as a "transaction" at the root's level (an empty branch), hash to the merkle root: asked about that
/// "txid", the branch climbs to the header's root. Refused: a 64-byte transaction cannot be told apart from an inner node.
#[test]
fn the_64_byte_transaction_trick() {
    let mut inner = coinbase_170().to_vec();
    inner.extend_from_slice(&id(T170));
    let fake_txid = dsha(&inner);
    let good = b_core(&id(T170));
    let root: [u8; 32] = unhex(&good.block.header).unwrap()[36..68].try_into().unwrap();
    assert_eq!(fake_txid, root, "the 64 bytes hash to the merkle root");
    let mut b = good.clone();
    b.block.tx = hex(&inner); b.block.index = 0; b.block.branch.clear(); b.block.coinbase_branch.clear();
    refused(&b, &ask(fake_txid), "not exactly 64 bytes (64 bytes)");
    // and above the core tip (the header path) the same
    let (mut b, _) = b_seg(false);
    b.block.tx = hex(&inner);
    refused(&b, &DepositAsk { allow_pow_only_block: true, ..ask(fake_txid) }, "not exactly 64 bytes");
}

#[test]
fn a_block_above_the_core_tip_is_proof_of_work_only_and_flagged() {
    let (b, t) = b_seg(false);
    refused(&b, &ask(t), "PROOF OF WORK ONLY");
    let a = DepositAsk { allow_pow_only_block: true, min_depth: 6, ..ask(t) };
    let r = run(&b, &a);
    assert_eq!(r.verdict, Verdict::Ok, "{:?}", r.checks.iter().filter(|c| !c.0).collect::<Vec<_>>());
    let s = r.statement.unwrap();
    assert_eq!((s.height, s.block_fully_validated, s.validated_confirmations, s.confirmations), (800_000, false, 0, 175_279));
    refused(&b, &DepositAsk { min_validated_depth: 1, ..a.clone() }, "validated depth: 0");
    // its MMR path tampered, or missing
    let mut x = b.clone(); x.block.mmr_path.as_mut().unwrap()[3] = hex(&[1; 32]); refused(&x, &a, "is on the header chain");
    let mut x = b.clone(); x.block.mmr_path = None; refused(&x, &a, "needs its MMR path");
    // its merkle branch tampered
    let mut x = b.clone(); x.block.branch.swap(0, 1); refused(&x, &a, "transaction");
}

#[test]
fn an_op_return_payload() {
    // block 800,000's coinbase carries the witness commitment: OP_RETURN, one 36-byte push aa21a9ed || commitment
    let (b, t) = b_seg(true);
    let v: serde_json::Value = serde_json::from_str(SEG).unwrap();
    let cb = unhex(v["coinbase"].as_str().unwrap()).unwrap();
    let at = cb.windows(6).position(|w| w == [0x6a, 0x24, 0xaa, 0x21, 0xa9, 0xed]).unwrap();
    let payload = cb[at + 2..at + 38].to_vec();
    let a = DepositAsk { allow_pow_only_block: true, op_return: Some(payload.clone()), ..ask(t) };
    let r = run(&b, &a);
    assert_eq!(r.verdict, Verdict::Ok, "{:?}", r.checks.iter().filter(|c| !c.0).collect::<Vec<_>>());
    let (vout, value, spk) = r.statement.unwrap().output.unwrap();
    assert_eq!((value, spk.len(), &spk[2..]), (0, 38, payload.as_slice()));
    assert_eq!(run(&b, &DepositAsk { vout: Some(vout), ..a.clone() }).verdict, Verdict::Ok);
    let mut p = payload.clone(); p[10] ^= 1;
    refused(&b, &DepositAsk { op_return: Some(p), ..a.clone() }, "an output is an OP_RETURN carrying");
    refused(&b, &DepositAsk { op_return: Some(payload[..35].to_vec()), ..a.clone() }, "an output is an OP_RETURN carrying");
    refused(&b, &DepositAsk { vout: Some(0), ..a.clone() }, "output 0 is an OP_RETURN");
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
    let mut b = b_core(&id(T170)); b.core_receipt = hex(FAKE); refused(&b, &a, "dev-mode FakeReceipt");
    let mut b = b_core(&id(T170)); b.core_receipt = hex(CORE_FLIPPED); refused(&b, &a, "the core chain proof verifies");
    let mut b = b_tip(); b.header_receipts = vec![hex(FAKE)]; refused(&b, &a, "dev-mode FakeReceipt");
    let mut b = b_core(&id(T170)); b.core_receipt = "zz".into(); refused(&b, &a, "core receipt: not hex");
}

#[test]
fn the_binding_between_the_two_proofs() {
    let a = ask(id(T170));
    let mut b = b_tip(); b.core_tip_mmr_path[0] = hex(&[3; 32]); refused(&b, &a, "BINDING");
    let mut b = b_tip(); b.core_tip_mmr_path.clear(); refused(&b, &a, "BINDING");
    let mut b = b_core(&id(T170)); b.core_tip_mmr_path = vec![hex(&[3; 32])]; refused(&b, &a, "without header receipts");
    // a header chain that does not start at genesis, and one that stops below the core tip
    let mut b = b_tip(); b.header_receipts = vec![hex(HDR_101_200)]; b.core_tip_mmr_path.clear();
    refused(&b, &a, "the first run starts from genesis");
    let mut b = b_tip(); b.header_receipts = vec![hex(HDR_0_100)]; b.core_tip_mmr_path.clear();
    refused(&b, &a, "the header chain (100) reaches the core tip (200)");
}

#[test]
fn network_identity_and_freshness() {
    let a = ask(id(T170));
    let mut b = b_tip(); b.network = "signet".into(); refused(&b, &a, "the network asked about");
    let mut b = b_tip(); b.kind = "other".into(); refused(&b, &a, "a deposit bundle");
    let r = verify_deposit(&b_tip(), &a, "signet", Image::by_network("signet").unwrap(), HEADERS_IMAGE_ID, now());
    assert_eq!(r.verdict, Verdict::Fail);
    refused(&b_tip(), &DepositAsk { expect_tip: Some((TIP.0, "00".repeat(32))), ..a.clone() }, "chain identity");
    refused(&b_tip(), &DepositAsk { expect_tip: Some((TIP.0 - 1, TIP.1.into())), ..a.clone() }, "chain identity");
    // the tip's block time (975,278: 2026-11-10) against this clock
    refused(&b_tip(), &DepositAsk { max_tip_age: Some(60), ..a.clone() }, "is within 60 s");
    assert_eq!(run(&b_tip(), &DepositAsk { max_tip_age: Some(100 * 365 * 86_400), ..a.clone() }).verdict, Verdict::Ok);
}

/// The same two proofs wrapped in RISC Zero's Groth16 (tools/groth16-wrap on hazync-coord, 15.11.2026): the verifier
/// takes them unchanged (a chain proof is succinct or Groth16), and the bundle shrinks from ~913 KB to ~22 KB.
#[test]
fn groth16_wrapped_receipts_give_the_same_statement() {
    const CORE_G16: &[u8] = include_bytes!("../../../experimental/libzync-bench/deposit/mainnet_0_200_8888d59c_groth16.rcpt");
    const HDR_G16: &[u8] = include_bytes!("../../../experimental/libzync-bench/deposit/headers_0_975278_de54b94f_groth16.rcpt");
    assert_eq!(crate::receipt_kind(CORE_G16), Ok("groth16"));
    let b = make_bundle("mainnet", CORE_G16, &[HDR_G16.to_vec()], Some(HIST), &block_170(), 170, &id(T170), &lines(PATH_200), None).unwrap();
    let a = DepositAsk { vout: Some(0), amount: Some(1_000_000_000), expect_tip: Some((TIP.0, TIP.1.into())), ..ask(id(T170)) };
    let r = run(&b, &a);
    assert_eq!(r.verdict, Verdict::Ok, "{:?}", r.checks.iter().filter(|c| !c.0).collect::<Vec<_>>());
    assert_eq!(r.statement, run(&b_tip(), &a).statement);
    assert!(serde_json::to_string(&b).unwrap().len() < 25_000);
    // a Groth16 receipt with one byte flipped (past its 256-byte seal: the claim no longer matches it)
    let mut bad = CORE_G16.to_vec();
    let at = bad.len() / 3; bad[at] ^= 1;
    let mut x = b.clone(); x.core_receipt = hex(&bad);
    assert_eq!(run(&x, &a).verdict, Verdict::Fail);
}
