//! The deposit zAPP's fuzz properties (docs/libzync/ZAPP_TESTING.md §3.4, §4.7), one function per target, so the fuzz
//! targets and the stable corpus replay (tests/replay.rs) run the same code. A property that fails panics.
//!
//! - `bundle`: `{"ask": <n>, "bundle": <a deposit bundle>}`, the bundle's receipts written as "@CORE" / "@HDR_TIP"
//!   (the real core proof of 1..200 and the real header proof 0..975,278, substituted here, so the fuzzer mutates the
//!   rest of the bundle instead of 450 kB of receipt hex). The references are the zAPP's own real bundles (block 170
//!   under the core proof alone and bound to the header chain; block 800,000 proof of work only). A bundle that gives a
//!   statement is a reference bundle for that question (hex compared case-insensitively: the format takes either case).
//! - `tx`: the transaction parser (libzync::facts::parse_tx, which the deposit reads its outputs with) and
//!   `op_return_payload`: no panic; a parsed transaction's stripped serialisation parses to itself; the OP_RETURN payload
//!   equals a reference written from the function's doc comment.

use std::sync::OnceLock;
use swiftsync_verify::{hex, hex32, unhex, Image, Verdict};
use zapp_deposit::{DepositAsk, DepositBlock, DepositBundle};
use zapp_headers::TxInclusion;

macro_rules! bench { ($f:literal) => { include_bytes!(concat!("../../../../experimental/libzync-bench/", $f)) } }
// the engine chain's proof of mainnet 1..200 and its history; the engine's header-chain proof 0..4,031 (named "@HDR_TIP"
// in the corpus) and block 200's MMR path to its tip (engine proofs only since 27.11.2026)
pub const CORE: &[u8] = include_bytes!("../../../../swiftsync-verify/vectors/mainnet_1_200_engine.hze");
const CORE_VK: &[u8] = include_bytes!("../../../../swiftsync-verify/vectors/mainnet_1_200_engine.vk");
const CORE_ID: &str = include_str!("../../../../swiftsync-verify/vectors/mainnet_1_200_engine.image_id");
pub const HDR_TIP: &[u8] = include_bytes!("../../../headers/tests/data/engine4031/proof.hze");
const HDR_VK: &[u8] = include_bytes!("../../../headers/tests/data/engine4031/vk.bin");
const HDR_ID: &str = include_str!("../../../headers/tests/data/engine4031/image_id");
const HIST: &[u8] = include_bytes!("../../../../swiftsync-verify/tests/data/engine200/history.bin");
const BLOCK_170: &[u8] = bench!("deposit/block_170_raw.txt");
const PATH_200: &[u8] = include_bytes!("../../../headers/tests/data/engine4031/mmr_path_200_tip4031.txt");
const SEG: &[u8] = bench!("headers/tx/tx_seg.json");
pub const T170: &str = "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16";
pub const HAL_SPK: &str = "4104ae1a62fe09c5f51b13905f07f06b99a2f7159b2225f374cd378d71302fa28414e7aab37397f554a7df5f142c21c1b7303b8a0626f1baded5c72a704f7e6cd84cac";
pub const NOW: u64 = 1_800_000_000;

fn id(display: &str) -> [u8; 32] { let mut b = hex32(display).unwrap(); b.reverse(); b }
fn lines(t: &[u8]) -> Vec<[u8; 32]> {
    std::str::from_utf8(t).unwrap().lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| hex32(l).unwrap()).collect()
}

pub struct Refs {
    /// the questions, by index
    pub asks: Vec<DepositAsk>,
    /// (the asks it answers, the bundle with its receipts as placeholders, as JSON)
    pub bundles: Vec<(Vec<usize>, serde_json::Value)>,
}

fn placeholders(b: &DepositBundle) -> serde_json::Value {
    let mut v = serde_json::to_value(b).unwrap();
    v["core_receipt"] = "@CORE".into();
    if let Some(a) = v["header_receipts"].as_array_mut() {
        for x in a.iter_mut() { *x = "@HDR_TIP".into(); }
    }
    v
}

pub fn refs() -> &'static Refs {
    static R: OnceLock<Refs> = OnceLock::new();
    R.get_or_init(|| {
        let block = unhex(std::str::from_utf8(BLOCK_170).unwrap().trim()).unwrap();
        let txs = libzync::facts::parse_block(&block).unwrap().1;
        let cb170 = libzync::facts::txid(&txs[0]);
        let t170 = id(T170);
        let core = |txid: &[u8; 32]| zapp_deposit::make_bundle("mainnet", CORE, &[], Some(HIST), &block, 170, txid, &[], None).unwrap();
        let tip = |txid: &[u8; 32]| zapp_deposit::make_bundle("mainnet", CORE, &[HDR_TIP.to_vec()], Some(HIST), &block, 170, txid,
                                                              &lines(PATH_200), None).unwrap();
        let v: serde_json::Value = serde_json::from_slice(SEG).unwrap();
        let t = TxInclusion::from_json(v["txid"].as_str().unwrap(), &v).unwrap();
        // the txids of block 800,000's segwit transaction and coinbase (asks 2 and 3: no bundle reaches that block now)
        let txid_of = |raw: &[u8]| { let mut o = 0; libzync::facts::txid(&libzync::facts::parse_tx(raw, &mut o).unwrap()) };
        let (seg_txid, seg_cbid) = (txid_of(&t.raw), txid_of(&t.coinbase));
        let base = |txid: [u8; 32]| DepositAsk { txid, min_depth: 1, require_identity: false, allow_demo: true, ..Default::default() };
        let pow = |txid: [u8; 32]| DepositAsk { allow_pow_only_block: true, min_validated_depth: 0, ..base(txid) };
        let asks = vec![
            base(t170),
            base(cb170),
            pow(seg_txid),
            pow(seg_cbid),
            DepositAsk { vout: Some(0), amount: Some(1_000_000_000), script: unhex(HAL_SPK), ..base(t170) },
            DepositAsk { expect_tip: Some((4031, "00000000f037ad09d0b05ee66b8c1da83030abaf909d2b1bf519c3c7d2cd3fdf".into())),
                         require_identity: true, ..base(t170) },   // allow_demo: a 4,032-header chain is DEMO
        ];
        let bundles = vec![
            (vec![0, 4], placeholders(&core(&t170))),
            (vec![0, 4, 5], placeholders(&tip(&t170))),
            (vec![1], placeholders(&core(&cb170))),
        ];
        Refs { asks, bundles }
    })
}

fn lower(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::String(s) => s.to_lowercase().into(),
        serde_json::Value::Array(a) => a.iter().map(lower).collect(),
        serde_json::Value::Object(o) => o.iter().map(|(k, x)| (k.clone(), lower(x))).collect(),
        x => x.clone(),
    }
}

fn substitute(v: &mut serde_json::Value) {
    if v["core_receipt"] == "@CORE" { v["core_receipt"] = hex(CORE).into(); }
    if let Some(a) = v.get_mut("header_receipts").and_then(|a| a.as_array_mut()) {
        for x in a.iter_mut() { if *x == "@HDR_TIP" { *x = hex(HDR_TIP).into(); } }
    }
}

pub fn bundle(data: &[u8]) {
    let _ = run(data);
}

/// The seeds' positive check: whether `data` gives a statement (examples/mkseeds.rs requires it of every reference).
pub fn gives_statement(data: &[u8]) -> bool {
    run(data) == Some(true)
}

fn run(data: &[u8]) -> Option<bool> {
    let v = serde_json::from_slice::<serde_json::Value>(data).ok()?;
    let r = refs();
    let n = v["ask"].as_u64().filter(|n| (*n as usize) < r.asks.len())?;
    let mut full = v["bundle"].clone();
    if !full.is_object() { return None; }
    substitute(&mut full);
    let b = serde_json::from_value::<DepositBundle>(full).ok()?;
    use sha2::Digest as _;
    // once: with_engine_key leaks an Image per call
    static IMG: OnceLock<&'static Image> = OnceLock::new();
    let img = *IMG.get_or_init(|| Image::by_network("mainnet").unwrap().with_method_id(CORE_ID.trim()).unwrap().with_engine_key(CORE_VK.to_vec()));
    let hk = swiftsync_verify::EngineKey { bytes: HDR_VK, pin: sha2::Sha256::digest(HDR_VK).into() };
    let rep = zapp_deposit::verify_deposit(&b, &r.asks[n as usize], "mainnet", img, HDR_ID.trim(), Some(&hk), NOW);
    assert!(rep.statement.is_none() || rep.verdict != Verdict::Fail, "a statement with a FAIL verdict");
    assert!(!rep.accepted || rep.verdict == Verdict::Ok, "accepted without an OK verdict");
    if rep.statement.is_some() || rep.accepted {
        // compare what was parsed (defaults filled; an unknown field is refused by the parse), with the receipts as placeholders again
        let got = lower(&placeholders(&b));
        let core_ok = b.core_receipt == hex(CORE);
        let hdr_ok = b.header_receipts.iter().all(|x| *x == hex(HDR_TIP));
        assert!(core_ok && hdr_ok && r.bundles.iter().any(|(asks, w)| asks.contains(&(n as usize)) && lower(w) == got),
                "WRONG-OK: ask {n}: a bundle that is not a reference bundle gives a statement ({:?})", rep.verdict);
    }
    Some(rep.statement.is_some())
}

/// OP_RETURN's payload as the doc of `op_return_payload` states it (written from the text, not the code).
fn payload_ref(spk: &[u8]) -> Option<Vec<u8>> {
    let (first, mut rest) = spk.split_first()?;
    if *first != 0x6a { return None; }
    let mut out = Vec::new();
    while let Some((&op, r)) = rest.split_first() {
        let (len, r) = match op {
            0 => (0usize, r),
            1..=75 => (op as usize, r),
            76 => (*r.first()? as usize, &r[1..]),
            77 => (u16::from_le_bytes(r.get(..2)?.try_into().ok()?) as usize, &r[2..]),
            78 => (u32::from_le_bytes(r.get(..4)?.try_into().ok()?) as usize, &r[4..]),
            _ => return None,
        };
        if r.len() < len { return None; }
        out.extend_from_slice(&r[..len]);
        rest = &r[len..];
    }
    Some(out)
}

pub fn tx(data: &[u8]) {
    assert_eq!(zapp_deposit::op_return_payload(data), payload_ref(data), "OP_RETURN payload differs from the documented rule");
    let mut o = 0;
    let Some(v) = libzync::facts::parse_tx(data, &mut o) else { return };
    assert!(o <= data.len(), "parse_tx read past the end");
    let mut o2 = 0;
    let w = libzync::facts::parse_tx(&v.stripped, &mut o2).expect("the stripped serialisation of a parsed transaction does not parse");
    assert_eq!(o2, v.stripped.len(), "the stripped serialisation has bytes left over");
    assert_eq!((&w.stripped, &w.inputs, &w.outputs), (&v.stripped, &v.inputs, &v.outputs), "stripped form parses differently");
    for (_, spk) in &v.outputs { assert_eq!(zapp_deposit::op_return_payload(spk), payload_ref(spk)); }
}
