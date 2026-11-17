//! The liveness zAPP's fuzz properties (docs/libzync/ZAPP_TESTING.md §3.4, §4.5), one function per target, so the fuzz
//! targets and the stable corpus replay (tests/replay.rs) run the same code. A property that fails panics.
//!
//! Three chains a proof is checked against, each with the proofs that are TRUE on it: the real mainnet 1..200 proof
//! (8888d59c) and vectors/b*.hex; the undo-commitment host run of 1..200 (vectors/undo_dev_*) with proofs made from its
//! history by the producer; the regtest 0..232 run (vectors/regtest_*) and regtest_cases.json's proofs.
//!
//! - `proof`: any bytes as an LZLV proof: no panic; a decoded proof re-encodes to the input; a proof that verifies on a
//!   chain is one of that chain's true proofs, byte for byte (a mutation of a true proof never verifies).
//! - `fact`: the facts JSON the C ABI and WASM take (`{"proof", "ask"}`): a fact that holds is a true proof, and the
//!   question is about that proof's outpoint and height, and every field it claims is the proof's.
//! - `ask`: the question grammar: one spelling per question (a question that parses is printed back as written, the
//!   options in any order).

use libzync::liveness as lv;
use std::sync::OnceLock;
use swiftsync_verify::{hex, ChainState, Image, Inputs, Verdict};
use zapp_liveness::{AskState, LivenessAsk, LivenessFact};

pub const RCPT: &[u8] = include_bytes!("../../../../swiftsync-verify/vectors/mainnet_1_200_8888d59c.rcpt");
pub const IMAGE_8888: &str = "8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4";

macro_rules! vector { ($f:literal) => { include_bytes!(concat!("../../vectors/", $f)) } }

pub const MAINNET_PROOFS: [&[u8]; 5] = [vector!("b9_0_at200.hex"), vector!("b9_0_at169.hex"), vector!("b170_0_at200.hex"),
                                        vector!("b170_1_at200.hex"), vector!("b170_1_at170.hex")];
const UNDO_HISTORY: &[u8] = vector!("undo_dev_1_200_history.bin");
const UNDO_STATE: &[u8] = vector!("undo_dev_1_200_state.json");
const REGTEST_STATE: &[u8] = vector!("regtest_0_232_state.json");
const REGTEST_CASES: &[u8] = vector!("regtest_cases.json");

pub struct TrueChain {
    pub name: &'static str,
    pub cs: ChainState,
    pub undo_from: u32,
    /// (proof bytes, the question it answers as the zAPP's tests ask it)
    pub proofs: Vec<(Vec<u8>, String)>,
}

/// The proofs true on `c`: the mainnet 1..200 proof and the undo host run are the same blocks, so a proof true on one is
/// true on the other.
fn true_on(c: &TrueChain) -> impl Iterator<Item = &Vec<u8>> {
    let all = chains();
    let same: Vec<&TrueChain> = if c.name == "regtest" { vec![c] } else { all.iter().filter(|x| x.name != "regtest").collect() };
    same.into_iter().flat_map(|x| x.proofs.iter().map(|(b, _)| b))
}

impl TrueChain {
    pub fn chain(&self) -> lv::Chain<'_> {
        lv::Chain { height: self.cs.height, utxo_roots: &self.cs.utxo_roots, utxo_leaves: self.cs.utxo_leaves,
                    block_roots: &self.cs.block_roots, block_leaves: self.cs.block_leaves, undo_from: self.undo_from }
    }
}

fn file_proof(b: &[u8]) -> Vec<u8> {
    zapp_liveness::liveness_bytes(b).unwrap()
}

fn question(p: &lv::Proof) -> String {
    let mut t = p.coin.txid;
    t.reverse();
    format!("{}:{}@{}={}", hex(&t), p.coin.vout, p.at, zapp_liveness::liveness_token(&p.state))
}

pub fn chains() -> &'static Vec<TrueChain> {
    static C: OnceLock<Vec<TrueChain>> = OnceLock::new();
    C.get_or_init(|| {
        let image = Image::by_id("mainnet", IMAGE_8888).unwrap();
        let r = swiftsync_verify::verify(&Inputs { receipt: RCPT, image, anchor: None, now: 1_800_000_000 });
        assert_eq!(r.verdict, Verdict::Demo, "{:?}", r.checks);
        let main = TrueChain {
            name: "mainnet", cs: r.state.unwrap(), undo_from: r.undo_from,
            proofs: MAINNET_PROOFS.iter().map(|f| { let b = file_proof(f); let q = question(&lv::decode(&b).unwrap()); (b, q) }).collect(),
        };
        // the undo host run: the producer's proofs for the same coins at the heights the zAPP's test opens them
        let cs: ChainState = serde_json::from_slice(UNDO_STATE).unwrap();
        let uf = swiftsync_verify::undo_from_of(&cs.self_id, &[]);
        let recs = libzync::history::decode_all(UNDO_HISTORY).unwrap();
        let mut proofs = Vec::new();
        for (f, at) in [(0, 169), (0, 170), (4, 170), (3, 181), (2, 199), (0, 200), (2, 200), (3, 200)] {
            let coin = lv::decode(&file_proof(MAINNET_PROOFS[f])).unwrap().coin;
            let b = zapp_liveness::make_liveness(&recs, 200, at, &coin).unwrap();
            let q = question(&lv::decode(&b).unwrap());
            proofs.push((b, q));
        }
        let undo = TrueChain { name: "undo_dev", cs, undo_from: uf, proofs };
        let cs: ChainState = serde_json::from_slice(REGTEST_STATE).unwrap();
        let uf = swiftsync_verify::undo_from_of(&cs.self_id, &[]);
        let cases: Vec<(String, String, String)> = serde_json::from_slice(REGTEST_CASES).unwrap();
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../vectors");
        let proofs = cases.iter().map(|(f, q, _)| (file_proof(&std::fs::read(format!("{dir}/{f}")).unwrap()), q.clone())).collect();
        let reg = TrueChain { name: "regtest", cs, undo_from: uf, proofs };
        for c in [&main, &undo, &reg] {
            for (b, q) in &c.proofs {
                lv::verify(b, &c.chain()).unwrap_or_else(|e| panic!("{}: a true proof ({q}) is refused: {}", c.name, e.text()));
            }
        }
        vec![main, undo, reg]
    })
}

pub fn proof(data: &[u8]) {
    let _ = zapp_liveness::liveness_bytes(data);
    if let Ok(s) = std::str::from_utf8(data) {
        if let Ok(b) = zapp_liveness::liveness_hex(s) {
            assert_eq!(hex(&b), s, "NONCANONICAL: liveness_hex accepted another spelling");
        }
    }
    let Ok(p) = lv::decode(data) else { return };
    assert!(lv::encode(&p) == data, "NONCANONICAL: decode accepted bytes that are not the encoding of what it read");
    for c in chains() {
        if lv::check(&p, &c.chain()).is_ok() {
            assert!(true_on(c).any(|b| b == data),
                    "WRONG-OK: on {}, a proof that is not one of the chain's true proofs verifies: {}", c.name, question(&p));
        }
    }
}

pub fn fact(data: &[u8]) {
    let Ok(f) = serde_json::from_slice::<LivenessFact>(data) else { return };
    for c in chains() {
        for need_claim in [true, false] {
            let Ok(line) = zapp_liveness::check_liveness_fact(&c.cs, c.undo_from, &f, need_claim) else { continue };
            let b = zapp_liveness::liveness_hex(&f.proof).unwrap();
            assert!(true_on(c).any(|t| *t == b), "WRONG-OK: on {}, a proof that is not true holds: {line}", c.name);
            let p = lv::decode(&b).unwrap();
            let a = LivenessAsk::parse(&f.ask).unwrap();
            assert!(!need_claim || a.state.is_some(), "WRONG-OK: a question without a claim holds where a claim is required");
            let state_ok = match (a.state, p.state) {
                (None, _) | (Some(AskState::Unspent), lv::State::Unspent) => true,
                (Some(AskState::Spent(h, by)), lv::State::Spent { height, tx, vin }) =>
                    h.map_or(true, |h| h == height) && by.map_or(true, |x| x == (tx, vin)),
                _ => false,
            };
            assert!(a.txid == p.coin.txid && a.vout == p.coin.vout && a.at.unwrap_or(c.cs.height) == p.at && state_ok
                    && a.value.map_or(true, |v| v == p.coin.value) && a.script.as_ref().map_or(true, |s| *s == p.coin.script_pubkey)
                    && a.created.map_or(true, |h| h == p.coin.height) && a.coinbase.map_or(true, |x| x == p.coin.coinbase),
                    "WRONG-OK: on {}, question {:?} holds against the proof of {}", c.name, f.ask, question(&p));
        }
    }
}

/// How the grammar (LIVENESS_ZAPP.md, the doc comment of LivenessAsk) spells a parsed question: head, then options.
fn render(a: &LivenessAsk) -> (String, Vec<String>) {
    let mut t = a.txid;
    t.reverse();
    let mut head = format!("{}:{}@{}", hex(&t), a.vout, a.at.map_or("tip".to_string(), |h| h.to_string()));
    match a.state {
        None => {}
        Some(AskState::Unspent) => head.push_str("=unspent"),
        Some(AskState::Spent(h, by)) => {
            head.push_str("=spent");
            if let Some(h) = h { head.push_str(&format!(":{h}")); }
            if let Some((tx, vin)) = by { head.push_str(&format!(":{tx}:{vin}")); }
        }
    }
    let mut opts = Vec::new();
    if let Some(v) = a.value { opts.push(format!("value={v}")); }
    if let Some(s) = &a.script { opts.push(format!("script={}", hex(s))); }
    if let Some(h) = a.created { opts.push(format!("created={h}")); }
    if let Some(c) = a.coinbase { opts.push(format!("coinbase={}", c as u8)); }
    (head, opts)
}

pub fn ask(data: &[u8]) {
    let Ok(s) = std::str::from_utf8(data) else { return };
    let Ok(a) = LivenessAsk::parse(s) else { return };
    let (head, mut opts) = render(&a);
    let mut parts = s.split(',');
    let given_head = parts.next().unwrap();
    let mut given: Vec<String> = parts.map(str::to_string).collect();
    given.sort();
    opts.sort();
    assert!(given_head == head && given == opts, "NONCANONICAL: question {s:?} parses as {head:?} {opts:?}");
    assert_eq!(LivenessAsk::parse(&[head].into_iter().chain(opts).collect::<Vec<_>>().join(",")).as_ref(), Ok(&a));
}
