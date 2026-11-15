//! The coin-liveness zyncAPP (docs/libzync/LIVENESS_ZAPP.md): ONE outpoint's state after a height, checked against a
//! verified core proof and against the question the CALLER asks. A zAPP module of its own (docs/libzync/ZYNCAPPS.md §9):
//! it uses the core verifier `swiftsync_verify` as a library (receipt verification, chain state) and is not part of it.
//! Until 15.11.2026 these checks sat inside the core verifier (`swiftsync-verify --liveness`, the C ABI's and the
//! WASM's facts JSON `"liveness"`); the checks are unchanged, the entry points are this module's own.

use serde::{Deserialize, Serialize};
use swiftsync_verify::{check_bundle, hex, hex32, unhex, verify, Bundle, ChainState, Image, Inputs, Report, Verdict, BUNDLE_MAGIC, PROFILE_COINLOG};

/// Where a producer learns a coin's fields (block files or the user's node).
pub mod coin_source;

/// A liveness question (docs/libzync/LIVENESS_ZAPP.md), written by the CALLER, never taken from the fact server:
/// `<txid>:<vout>@<height|tip>[=unspent|=spent[:<h>[:<tx>:<vin>]]][,value=<sat>][,script=<hex>]`, txid as explorers
/// print it. The proof must be about exactly this outpoint at exactly this height; a given state, value or script must
/// be the proof's. Without `=...` the check reports the answer (CLI); the C ABI and WASM require it (they return only
/// whether a claim holds).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LivenessAsk {
    /// internal byte order
    pub txid: [u8; 32],
    pub vout: u32,
    /// None: the proof's tip
    pub at: Option<u32>,
    pub state: Option<AskState>,
    pub value: Option<u64>,
    pub script: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskState {
    Unspent,
    /// spent; optionally at this height, optionally by (tx, vin)
    Spent(Option<u32>, Option<(u32, u32)>),
}

impl LivenessAsk {
    pub fn parse(s: &str) -> Result<LivenessAsk, String> {
        let bad = |w: &str| format!("liveness question {s:?}: {w} (want <txid>:<vout>@<height|tip>[=unspent|=spent[:<h>[:<tx>:<vin>]]][,value=<sat>][,script=<hex>])");
        let mut parts = s.trim().split(',');
        let head = parts.next().unwrap_or("");
        let (q, st) = match head.split_once('=') { Some((q, st)) => (q, Some(st)), None => (head, None) };
        let (op, at) = q.split_once('@').ok_or_else(|| bad("no @<height|tip>"))?;
        let (t, v) = op.split_once(':').ok_or_else(|| bad("no :<vout>"))?;
        if t.len() != 64 || !t.bytes().all(|b| b.is_ascii_hexdigit()) { return Err(bad("txid is not 64 hex digits")); }
        let mut txid = hex32(&t.to_lowercase()).ok_or_else(|| bad("txid"))?;
        txid.reverse();
        let num = |x: &str| x.parse::<u32>().ok().filter(|_| !x.starts_with('+') && !x.is_empty());
        let vout = num(v).ok_or_else(|| bad("vout"))?;
        let at = if at == "tip" { None } else { Some(num(at).ok_or_else(|| bad("height"))?) };
        let state = match st {
            None => None,
            Some("unspent") => Some(AskState::Unspent),
            Some(x) if x == "spent" || x.starts_with("spent:") => {
                let f: Vec<&str> = x.split(':').skip(1).collect();
                let n = |i: usize| num(f[i]).ok_or_else(|| bad("spent fields"));
                Some(match f.len() {
                    0 => AskState::Spent(None, None),
                    1 => AskState::Spent(Some(n(0)?), None),
                    3 => AskState::Spent(Some(n(0)?), Some((n(1)?, n(2)?))),
                    _ => return Err(bad("spent takes :<h> or :<h>:<tx>:<vin>")),
                })
            }
            Some(_) => return Err(bad("state is unspent or spent")),
        };
        let (mut value, mut script) = (None, None);
        for p in parts {
            match p.split_once('=') {
                Some(("value", x)) if value.is_none() => value = Some(x.parse::<u64>().map_err(|_| bad("value"))?),
                Some(("script", x)) if script.is_none() => script = Some(unhex(x).ok_or_else(|| bad("script is not hex"))?),
                _ => return Err(bad("unknown or repeated option")),
            }
        }
        Ok(LivenessAsk { txid, vout, at, state, value, script })
    }
}

/// The answer token of a liveness proof: `unspent` or `spent:<h>:<tx>:<vin>`.
pub fn liveness_token(s: &libzync::liveness::State) -> String {
    match s {
        libzync::liveness::State::Unspent => "unspent".into(),
        libzync::liveness::State::Spent { height, tx, vin } => format!("spent:{height}:{tx}:{vin}"),
    }
}

/// A liveness proof's bytes from a file's contents: raw (starting "LZLV") or hex text.
pub fn liveness_bytes(b: &[u8]) -> Result<Vec<u8>, String> {
    if b.starts_with(&libzync::liveness::MAGIC) { return Ok(b.to_vec()); }
    let t = std::str::from_utf8(b).map_err(|_| "liveness proof: neither raw (LZLV) nor hex text")?.trim();
    if t.bytes().any(|c| c.is_ascii_uppercase()) { return Err("liveness proof: hex must be lower case (one encoding per proof)".into()); }
    unhex(t).ok_or_else(|| "liveness proof: neither raw (LZLV) nor hex text".into())
}

/// Checks a liveness proof against a state that passed [`verify`] and against the CALLER's question. Ok: the answer
/// token and the line saying what is proven. Every field of the question is compared before the answer is given.
pub fn check_liveness(cs: &ChainState, undo_from: u32, proof: &[u8], ask: &LivenessAsk) -> Result<(String, String), String> {
    use libzync::liveness as lv;
    if cs.profile & PROFILE_COINLOG == 0 {
        return Err("the proof is not a coin-log chain: liveness needs the coin log".into());
    }
    let chain = lv::Chain { height: cs.height, utxo_roots: &cs.utxo_roots, utxo_leaves: cs.utxo_leaves, block_roots: &cs.block_roots,
                            block_leaves: cs.block_leaves, undo_from };
    let p = lv::decode(proof).map_err(|e| format!("liveness proof: {}", e.text()))?;
    // the question first: a true fact about another coin or height is not an answer (ZYNCAPPS.md §8, review 11-12)
    let mut shown = p.coin.txid;
    shown.reverse();
    let want_at = ask.at.unwrap_or(cs.height);
    if p.coin.txid != ask.txid || p.coin.vout != ask.vout {
        return Err(format!("the proof is about {}:{}, not the outpoint asked", hex(&shown), p.coin.vout));
    }
    if p.at != want_at {
        return Err(format!("the proof is about height {}, the question about {want_at}", p.at));
    }
    if ask.value.is_some_and(|v| v != p.coin.value) {
        return Err(format!("the coin holds {} sat, not the amount asked", p.coin.value));
    }
    if ask.script.as_ref().is_some_and(|s| *s != p.coin.script_pubkey) {
        return Err("the coin pays another scriptPubKey than the one asked".into());
    }
    lv::check(&p, &chain).map_err(|e| format!("liveness proof for {}:{} at {}: {}", hex(&shown), p.coin.vout, p.at, e.text()))?;
    let got = liveness_token(&p.state);
    let matches = match (ask.state, p.state) {
        (None, _) | (Some(AskState::Unspent), lv::State::Unspent) => true,
        (Some(AskState::Spent(h, by)), lv::State::Spent { height, tx, vin }) => h.is_none_or(|h| h == height) && by.is_none_or(|b| b == (tx, vin)),
        _ => false,
    };
    let c = &p.coin;
    let what = match p.state {
        lv::State::Unspent => format!("UNSPENT after block {}", p.at),
        lv::State::Spent { height, tx, vin } => format!("SPENT at height {height} by transaction {tx} input {vin} (state after block {})", p.at),
    };
    let line = format!("coin {}:{} ({} sat, script {}, created at height {}{}) is {what}{}", hex(&shown), c.vout, c.value,
                       hex(&c.script_pubkey), c.height, if c.coinbase { " by a coinbase" } else { "" },
                       if p.at < cs.height { format!(", opened in the proof's chain to {}", cs.height) } else { String::new() });
    if !matches {
        let claimed = match ask.state {
            Some(AskState::Spent(h, by)) => format!("spent{}{}", h.map(|h| format!(":{h}")).unwrap_or_default(),
                                                    by.map(|(t, v)| format!(":{t}:{v}")).unwrap_or_default()),
            _ => "unspent".into(),
        };
        return Err(format!("the question claims {claimed}, the proof shows {got}: {line}"));
    }
    Ok((got, line))
}

/// A liveness proof with the CALLER's question, as the C ABI and WASM take it in their facts JSON (`"liveness"`):
/// `{"proof": "<lower-case hex>", "ask": "<question>"}`. There the question must claim its answer (`=unspent` /
/// `=spent...`): those interfaces return only whether a claim holds.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct LivenessFact {
    pub proof: String,
    pub ask: String,
}

/// [`check_liveness`] on a [`LivenessFact`]; `need_claim`: refuse a question that does not claim its answer.
pub fn check_liveness_fact(cs: &ChainState, undo_from: u32, f: &LivenessFact, need_claim: bool) -> Result<String, String> {
    let ask = LivenessAsk::parse(&f.ask)?;
    if need_claim && ask.state.is_none() {
        return Err(format!("liveness question {:?} claims no answer (=unspent or =spent...): this interface returns only whether a claim holds", f.ask));
    }
    let (token, line) = check_liveness(cs, undo_from, &liveness_bytes(f.proof.as_bytes())?, &ask)?;
    Ok(format!("{token}: {line}"))
}

/// The producer side (zync-path live / zync-serve /live): the liveness proof for `coin` after height `at` from a run's
/// history. The coin's fields come from a block source (the caller's node or block files) and are checked here: a
/// coin whose leaf the log does not hold is refused. Holds no secret and proves nothing by itself.
pub fn make_liveness(recs: &[libzync::history::Rec], tip: u32, at: u32, coin: &libzync::liveness::Coin) -> Result<Vec<u8>, String> {
    use libzync::liveness as lv;
    let r = libzync::history::replay(recs, at, tip).map_err(|e| e.to_string())?;
    let leaf = coin.leaf();
    let pos = r.coins.iter().position(|c| *c == leaf)
        .ok_or("that coin is not in the coin log after that height (not yet created, never a leaf: unspendable, or created and spent in one block)")? as u64;
    let state = match r.spenders.get(&pos) {
        None => lv::State::Unspent,
        Some(&(height, tx, vin)) => lv::State::Spent { height, tx, vin },
    };
    let opening = (at < tip).then(|| lv::Opening {
        log_leaves: r.log.len(),
        log_value: r.value,
        log_roots: lv::roots_present(&r.log.roots()),
        block_hash: recs[at as usize].hash,
        undo: recs[at as usize].undo,
        block_path: libzync::history::block_path(&r.block_leaves, at as u64).0,
    });
    Ok(lv::encode(&lv::Proof { coin: coin.clone(), state, at, pos, path: r.log.path(pos), opening }))
}

/// The image a caller names, as the core verifier's WASM resolves it: `network` "mainnet" or "signet"; `image_hex`
/// empty for the network's built-in image, else a built-in image by id (with its own lineage) or the network's current
/// image re-pointed at that id; `coinlog` / `product` as the core's network bits 0x10 / 0x20. (No lineage text here:
/// an image the built-in table does not list, with its own handovers, is checked by the core verifier.)
pub fn image_for(network: &str, image_hex: &str, coinlog: bool, product: bool) -> Result<&'static Image, String> {
    let h = image_hex.trim().to_lowercase();
    let mut image = match Image::by_id(network, &h) {
        Some(i) if !h.is_empty() => i,
        _ => {
            let i = Image::by_network(network).ok_or_else(|| format!("unknown network {network:?}"))?;
            if h.is_empty() { i } else { i.with_method_id(&h).ok_or("image id is not 64 hex characters")? }
        }
    };
    if coinlog {
        image = image.with_coinlog();
    }
    if product {
        image = image.with_product();
    }
    Ok(image)
}

/// What [`verify_liveness`] found: the core report (its verdict now covering the liveness checks), whether the tip was
/// compared with the caller's `expect`, and one result per liveness fact (Ok: "<answer token>: <what is proven>").
pub struct Outcome {
    pub report: Report,
    pub tip_checked: bool,
    pub facts: Vec<Result<String, String>>,
}

/// The one implementation the CLI, the C ABI and the WASM module run: verify the core receipt (or .sspf bundle) with
/// the core verifier, compare its tip with `expect` ("<height>:<hash>", the network's block from a source the CALLER
/// trusts, never the fact server), then check each liveness fact against the verified state. A fact that does not hold,
/// or a receipt that does not verify, is FAIL; facts on a chain whose tip was not compared are at most DEMO.
/// `need_claim`: refuse a question that does not claim its answer (the C ABI and WASM return only whether claims hold).
pub fn verify_liveness(receipt: &[u8], image: &'static Image, anchor: Option<ChainState>, now: u64, facts: &[LivenessFact],
                       expect: Option<&str>, need_claim: bool) -> Outcome {
    let mut rep = if receipt.starts_with(BUNDLE_MAGIC) {
        match Bundle::decode(receipt) {
            Ok(b) => check_bundle(&b, image, anchor, now),
            Err(e) => Report { checks: vec![(false, format!("bundle: {e}"))], low_work: false, state: None, verdict: Verdict::Fail,
                               undo_from: swiftsync_verify::pin::NO_UNDO },
        }
    } else {
        verify(&Inputs { receipt, image, anchor, now })
    };
    let mut out = Vec::new();
    let tip_checked = match (expect.map(str::trim).filter(|e| !e.is_empty()), rep.state.as_ref()) {
        (Some(e), Some(cs)) => match swiftsync_verify::parse_expect(e) {
            Some((h, x)) if swiftsync_verify::identity_matches(h, &x, cs.height, &cs.tip_hash) => true,
            _ => { out.push(Err(format!("expect {e:?}: not the proof's tip"))); false }
        },
        _ => false,
    };
    if rep.verdict == Verdict::Fail {
        // the journal of a receipt that did not verify is the prover's word: nothing is checked against it
        for f in facts {
            out.push(Err(format!("liveness {:?}: NOT checked: the receipt did not verify", f.ask)));
        }
    } else if let Some(cs) = rep.state.as_ref() {
        for f in facts {
            out.push(check_liveness_fact(cs, rep.undo_from, f, need_claim));
        }
    } else if !facts.is_empty() {
        out.push(Err("no verified state to check liveness facts against".to_string()));
    }
    if out.iter().any(|r| r.is_err()) {
        rep.verdict = Verdict::Fail;
    }
    if rep.verdict == Verdict::Ok && !tip_checked && !facts.is_empty() {
        rep.verdict = Verdict::Demo;
    }
    Outcome { report: rep, tip_checked, facts: out }
}

/// The producer side (`zapp-liveness prove` / `zapp-liveness serve`): the proof for the outpoint `txid:vout` after
/// `at`, from a run's history and a block source, trying each candidate coin the source gives.
pub fn prove(recs: &[libzync::history::Rec], tip: u32, at: u32, src: &coin_source::Source, txid: &[u8; 32], vout: u32,
             height: Option<u32>) -> Result<Vec<u8>, String> {
    let coins = coin_source::candidates(src, txid, vout, height)?;
    let mut last = "no candidate coin".to_string();
    for c in &coins {
        match make_liveness(recs, tip, at, c) {
            Ok(p) => return Ok(p),
            Err(e) => last = e,
        }
    }
    Err(last)
}
