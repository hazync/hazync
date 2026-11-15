//! The coin-liveness zyncAPP (docs/libzync/LIVENESS_ZAPP.md): ONE outpoint's state after a height, checked against a
//! verified core proof and against the question the CALLER asks. A zAPP module of its own (docs/libzync/ZYNCAPPS.md §9):
//! it uses the core verifier `swiftsync_verify` as a library (receipt verification, chain state) and is not part of it.
//! Until 15.11.2026 these checks sat inside the core verifier (`swiftsync-verify --liveness`, the C ABI's and the
//! WASM's facts JSON `"liveness"`); the checks are unchanged, the entry points are this module's own.

use serde::{Deserialize, Serialize};
use swiftsync_verify::{hex, hex32, unhex, ChainState, Image, Report, Verdict, PROFILE_COINLOG};

/// Where a producer learns a coin's fields (block files or the user's node).
pub mod coin_source;

/// A liveness question (docs/libzync/LIVENESS_ZAPP.md), written by the CALLER, never taken from the fact server:
/// `<txid>:<vout>@<height|tip>[=unspent|=spent[:<h>[:<tx>:<vin>]]][,value=<sat>][,script=<hex>][,created=<h>][,coinbase=0|1]`,
/// txid as explorers print it, lower-case hex, numbers in decimal without a sign or leading zeros (one spelling per
/// question). The proof must be about exactly this outpoint at exactly this height; a given state, value, script,
/// creation height or coinbase flag must be the proof's. `created` pins WHICH coin: an outpoint created again over its
/// own spent coin (possible on mainnet from 1,983,702) has two leaves, one spent and one unspent, and without it both
/// `=spent` and `=unspent` can verify. Without `=...` the check reports the answer (CLI); the C ABI and WASM require it (they return only
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
    /// the creating block's height
    pub created: Option<u32>,
    pub coinbase: Option<bool>,
}

/// A decimal number as a question spells it: digits only, no leading zero unless it is 0.
fn dec<T: std::str::FromStr>(x: &str) -> Option<T> {
    let ok = !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit()) && (x == "0" || !x.starts_with('0'));
    if ok { x.parse().ok() } else { None }
}

/// Lower-case hex only.
fn lower_hex(x: &str) -> Option<Vec<u8>> {
    if x.bytes().any(|b| b.is_ascii_uppercase()) { None } else { unhex(x) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskState {
    Unspent,
    /// spent; optionally at this height, optionally by (tx, vin)
    Spent(Option<u32>, Option<(u32, u32)>),
}

impl LivenessAsk {
    pub fn parse(s: &str) -> Result<LivenessAsk, String> {
        let bad = |w: &str| format!("liveness question {s:?}: {w} (want <txid>:<vout>@<height|tip>[=unspent|=spent[:<h>[:<tx>:<vin>]]][,value=<sat>][,script=<hex>][,created=<h>][,coinbase=0|1])");
        let mut parts = s.split(',');
        let head = parts.next().unwrap_or("");
        let (q, st) = match head.split_once('=') { Some((q, st)) => (q, Some(st)), None => (head, None) };
        let (op, at) = q.split_once('@').ok_or_else(|| bad("no @<height|tip>"))?;
        let (t, v) = op.split_once(':').ok_or_else(|| bad("no :<vout>"))?;
        if t.len() != 64 || !t.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Err(bad("txid is not 64 lower-case hex digits"));
        }
        let mut txid = hex32(t).ok_or_else(|| bad("txid"))?;
        txid.reverse();
        let num = |x: &str| dec::<u32>(x);
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
        let (mut value, mut script, mut created, mut coinbase) = (None, None, None, None);
        for p in parts {
            match p.split_once('=') {
                Some(("value", x)) if value.is_none() => value = Some(dec::<u64>(x).ok_or_else(|| bad("value"))?),
                Some(("script", x)) if script.is_none() => script = Some(lower_hex(x).ok_or_else(|| bad("script is not lower-case hex"))?),
                Some(("created", x)) if created.is_none() => created = Some(num(x).ok_or_else(|| bad("created"))?),
                Some(("coinbase", x)) if coinbase.is_none() => coinbase = Some(match x { "0" => false, "1" => true, _ => return Err(bad("coinbase is 0 or 1")) }),
                _ => return Err(bad("unknown or repeated option")),
            }
        }
        Ok(LivenessAsk { txid, vout, at, state, value, script, created, coinbase })
    }
}

/// The answer token of a liveness proof: `unspent` or `spent:<h>:<tx>:<vin>`.
pub fn liveness_token(s: &libzync::liveness::State) -> String {
    match s {
        libzync::liveness::State::Unspent => "unspent".into(),
        libzync::liveness::State::Spent { height, tx, vin } => format!("spent:{height}:{tx}:{vin}"),
    }
}

/// A liveness proof as the C ABI and WASM take it: lower-case hex, exactly (no whitespace, no other spelling).
pub fn liveness_hex(s: &str) -> Result<Vec<u8>, String> {
    if s.is_empty() || s.bytes().any(|b| !(b.is_ascii_digit() || (b'a'..=b'f').contains(&b))) {
        return Err("liveness proof: not lower-case hex (one spelling per proof)".into());
    }
    unhex(s).ok_or_else(|| "liveness proof: not whole bytes of hex".into())
}

/// A liveness proof's bytes from a FILE's contents (the CLI): raw (starting "LZLV") or lower-case hex text.
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
    if ask.created.is_some_and(|h| h != p.coin.height) {
        return Err(format!("the proof is about the coin created at height {}, not the one asked", p.coin.height));
    }
    if ask.coinbase.is_some_and(|c| c != p.coin.coinbase) {
        return Err(format!("the coin {} created by a coinbase, not as asked", if p.coin.coinbase { "was" } else { "was not" }));
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
    let (token, line) = check_liveness(cs, undo_from, &liveness_hex(&f.proof)?, &ask)?;
    Ok(format!("{token}: {line}"))
}

/// The producer side (zync-path live / zync-serve /live): the liveness proof for `coin` after height `at` from a run's
/// history. The coin's fields come from a block source (the caller's node or block files) and are checked here: a
/// coin whose leaf the log does not hold is refused. Holds no secret and proves nothing by itself.
pub fn make_liveness(recs: &[libzync::history::Rec], tip: u32, at: u32, coin: &libzync::liveness::Coin) -> Result<Vec<u8>, String> {
    use libzync::liveness as lv;
    let r = libzync::history::replay(recs, at, tip).map_err(|e| e.to_string())?;
    let leaf = coin.leaf();
    // the latest leaf (a leaf hashes its creation height, so two equal leaves cannot exist; rposition says which wins)
    let pos = r.coins.iter().rposition(|c| *c == leaf)
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

/// The image a caller names (zapp_common::image_for, shared by every zAPP; cached, so a call leaks nothing new).
pub fn image_for(network: &str, image_hex: &str, coinlog: bool, product: bool) -> Result<&'static Image, String> {
    zapp_common::image_for(network, image_hex, coinlog, product)
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
    // the core receipt and the tip, as every zAPP checks them (zapp_common): a wrong `expect` is a FAIL line in the
    // report's checks, not one of the facts
    let core = zapp_common::verify_core(receipt, image, anchor, now, expect);
    let (mut rep, tip_checked) = (core.report, core.tip_checked);
    let mut out = Vec::new();
    if rep.verdict == Verdict::Fail {
        // a receipt that did not verify (its journal is the prover's word), or a tip that is not the block the caller
        // expects: nothing is checked against it
        for f in facts {
            out.push(Err(format!("liveness {:?}: NOT checked: the core check failed (the receipt, or its tip against expect)", f.ask)));
        }
    } else if let Some(cs) = rep.state.as_ref() {
        for f in facts {
            out.push(check_liveness_fact(cs, rep.undo_from, f, need_claim));
        }
    } else if !facts.is_empty() {
        out.push(Err("no verified state to check liveness facts against".to_string()));
    }
    if out.iter().any(|r| r.is_err()) || rep.checks.iter().any(|c| !c.0) {
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
    let mut coins = coin_source::candidates(src, txid, vout, height)?;
    // the LATEST coin first: an outpoint created again over its own spent coin names the live one by default; the
    // caller pins an older one with --height (and the verifier's question with `created=`)
    coins.sort_by(|a, b| b.height.cmp(&a.height));
    let mut last = "no candidate coin".to_string();
    for c in &coins {
        match make_liveness(recs, tip, at, c) {
            Ok(p) => return Ok(p),
            Err(e) => last = e,
        }
    }
    Err(last)
}
