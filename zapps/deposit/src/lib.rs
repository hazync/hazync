//! The deposit-proof zyncAPP (docs/libzync/DEPOSIT_ZAPP.md): one bundle a bridge, rollup or other chain's operator
//! checks to learn "transaction T (and optionally its output v, with this amount and scriptPubKey, or an OP_RETURN
//! payload) is in block h, under at least N blocks, on a chain whose every block from genesis to the core tip was fully
//! validated, with total work W, as of tip X".
//!
//! It is verifier-side composition of two proofs that already exist (design (a) of DEPOSIT_ZAPP.md §2):
//!   1. the CORE chain proof ([`swiftsync_verify::verify`]): every block 0..=Hc fully validated, and a block forest with one leaf
//!      per block. The transaction's block is opened in that forest ([`swiftsync_verify::check_tx_fact`]), so it is one of the
//!      fully validated blocks;
//!   2. optionally the HEADER-CHAIN proof ([`zapp_headers::verify_headers`]): proof of work and Core's header rules
//!      0..=Ht, and an MMR of every block hash. The core tip's hash must be the MMR's leaf at Hc (the BINDING): both
//!      chains are hash chains from genesis, so the header chain's blocks 0..=Hc are the core's, and Hc+1..=Ht add
//!      burial that is PROOF OF WORK ONLY.
//!
//! Rules kept from the header zAPP's review: the transaction asked about is the CALLER's ([`DepositAsk::txid`]); the
//! bundle carries no txid, the raw bytes must hash to the asked one. A statement is returned only when no check failed.
//!
//! A zAPP module of its own (docs/libzync/ZYNCAPPS.md §9), on the core verifier (`swiftsync_verify`) and the header-chain
//! zAPP (`zapp_headers`) as libraries; not part of either. Until 15.11.2026 this was `swiftsync_verify::deposit`, the
//! CLI's `swiftsync-verify deposit`, `zync-deposit` and the WASM's `verifyDeposit`; the checks are unchanged.
use zapp_headers::{self as headers, HeaderState, TxInclusion};
use swiftsync_verify::{check_tx_fact, hex, hex32, unhex, verify, ChainState, Image, Inputs, TxFact, Verdict};
use serde::{Deserialize, Serialize};

pub const BUNDLE_KIND: &str = "hazync-deposit";
pub const BUNDLE_VERSION: u32 = 1;

/// The deposit proof object (JSON). Hashes are hex of their INTERNAL byte order (as the block forest and the MMR use
/// them), raw data is plain hex. It carries no txid and no image ids: both come from the verifier's caller.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DepositBundle {
    pub kind: String,
    pub version: u32,
    pub network: String,
    /// The core chain proof (a succinct receipt, bincode, hex).
    pub core_receipt: String,
    /// The header-chain proof, as one receipt or a chain of runs from genesis (hex each). Empty: no burial beyond the
    /// core tip.
    #[serde(default)]
    pub header_receipts: Vec<String>,
    /// The core tip's MMR path in the header chain's MMR (lowest first). Needed exactly when header receipts are given.
    #[serde(default)]
    pub core_tip_mmr_path: Vec<String>,
    pub block: DepositBlock,
}

/// The deposit's block and transaction. A block at or below the core tip is opened in the core proof's block forest
/// (`forest`); one above it only in the header MMR (`mmr_path`), and then it is PROOF OF WORK ONLY.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DepositBlock {
    pub height: u32,
    /// The 80-byte header, hex.
    pub header: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forest: Option<ForestOpening>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mmr_path: Option<Vec<String>>,
    /// The raw transaction (with or without its witness), its position in the block and its merkle branch.
    pub tx: String,
    pub index: u32,
    pub branch: Vec<String>,
    /// The block's raw coinbase and ITS branch: they pin the tree's depth (the 64-byte transaction ambiguity).
    pub coinbase: String,
    pub coinbase_branch: Vec<String>,
}

/// The block's leaf in the core proof's block forest: its coin-state digest after the block, its undo hash (for a
/// block whose leaf commits one, docs/libzync/UNDO.md) and its path.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ForestOpening {
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo: Option<String>,
    pub path: Vec<String>,
}

/// What the CALLER asks: never read from the bundle. `Default` requires the chain identity and withholds a DEMO
/// statement (`require_identity` true, `allow_demo` false).
#[derive(Clone, Debug)]
pub struct DepositAsk {
    /// The transaction, internal byte order.
    pub txid: [u8; 32],
    /// An output of it, and what it must hold.
    pub vout: Option<u32>,
    pub amount: Option<u64>,
    pub script: Option<Vec<u8>>,
    /// An OP_RETURN output carrying exactly this payload (the data of its pushes, concatenated); at `vout` if given,
    /// else at any output. The push boundaries are not compared (one push "abcd" and two pushes "ab" "cd" carry the
    /// same payload): `script` compares the scriptPubKey byte for byte.
    pub op_return: Option<Vec<u8>>,
    /// Confirmations (the block itself counts 1), fully validated or proof of work only.
    pub min_depth: u64,
    /// Confirmations within the fully validated part (blocks at or below the core tip). Default 1: the deposit's own
    /// block is fully validated, its burial may be proof of work only (a valid block on top costs a miner the same work
    /// as an invalid one, so validating the burial adds next to nothing; validation's job is to rule out a fake deposit
    /// in an invalid block). 0 drops the check; on its own it changes nothing, since a block above the core tip is
    /// refused unless `allow_pow_only_block` (accepting one takes both: allow_pow_only_block and 0).
    pub min_validated_depth: u64,
    /// Accept a block ABOVE the core tip (proof of work only). Off by default; needs `min_validated_depth` 0 too.
    pub allow_pow_only_block: bool,
    /// The tip (height, hash as explorers print it) from a source the caller trusts.
    pub expect_tip: Option<(i64, String)>,
    /// The tip's block time must be within this many seconds of `now` (a stale proof is refused).
    pub max_tip_age: Option<u64>,
    /// The verdict can be Ok only when the tip was compared with a source the caller trusts (`expect_tip`). On by
    /// default; off is an explicit choice of the caller's (for example, one that compares the tip by other means).
    pub require_identity: bool,
    /// Return the statement for a DEMO verdict too. A DEMO chain is below Core's minimum chain work (or, for a core
    /// proof alone, below its pinned checkpoint): a difficulty-1 fork of it costs little to mine, so its statement is
    /// cheap to forge and is withheld unless asked for.
    pub allow_demo: bool,
}

impl Default for DepositAsk {
    fn default() -> Self {
        DepositAsk { txid: [0; 32], vout: None, amount: None, script: None, op_return: None, min_depth: 0,
                     min_validated_depth: 1, allow_pow_only_block: false, expect_tip: None, max_tip_age: None,
                     require_identity: true, allow_demo: false }
    }
}

/// What the bundle showed: set when the verdict is Ok, or Demo with `allow_demo` (never when a check failed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepositStatement {
    /// internal byte order
    pub txid: [u8; 32],
    pub height: u32,
    /// internal byte order
    pub block_hash: [u8; 32],
    /// (vout, amount, scriptPubKey) of the output checked, if any.
    pub output: Option<(u32, u64, Vec<u8>)>,
    pub tip_height: i64,
    pub tip_hash: [u8; 32],
    /// little-endian, as the journals hold it
    pub tip_work: [u8; 32],
    /// The core tip: every block 0..=validated_through was fully validated.
    pub validated_through: u32,
    pub confirmations: u64,
    pub validated_confirmations: u64,
    /// false: the block itself is above the core tip (proof of work only; needs allow_pow_only_block).
    pub block_fully_validated: bool,
    pub core_image: String,
    pub headers_image: Option<String>,
}

pub struct DepositReport {
    pub checks: Vec<(bool, String)>,
    pub verdict: Verdict,
    /// THE field to gate on: the verdict is Ok (every check passed, the tip proof has Core's minimum chain work) and
    /// the tip was compared with a source the caller trusts, or the caller turned that requirement off.
    pub accepted: bool,
    /// Whether the tip was compared with the caller's source (`expect_tip`) and matched.
    pub identity_checked: bool,
    pub statement: Option<DepositStatement>,
}

impl DepositReport {
    fn check(&mut self, ok: bool, what: impl Into<String>) -> bool { self.checks.push((ok, what.into())); ok }
    fn fail(mut self, what: impl Into<String>) -> DepositReport { self.check(false, what); self.verdict = Verdict::Fail; self }
}

/// A hash as explorers print it (reversed, hex).
pub fn disp(b: &[u8; 32]) -> String { b.iter().rev().map(|x| format!("{x:02x}")).collect() }

fn dsha(b: &[u8]) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    Sha256::digest(Sha256::digest(b)).into()
}

fn hashes(v: &[String], what: &str) -> Result<Vec<[u8; 32]>, String> {
    v.iter().map(|x| hex32(&x.to_lowercase()).ok_or(format!("{what}: not 64-hex-digit hashes"))).collect()
}

/// The data an OP_RETURN scriptPubKey carries: OP_RETURN (0x6a) followed by data pushes only (OP_0, 1..75-byte pushes,
/// OP_PUSHDATA1/2/4), their data concatenated. None for any other script (an OP_1..OP_16 or other opcode, a push that
/// runs past the end). The push boundaries are lost in the concatenation, as is which push opcode carried each part:
/// a caller that needs the exact script compares it byte for byte (`DepositAsk::script`).
pub fn op_return_payload(spk: &[u8]) -> Option<Vec<u8>> {
    if spk.first() != Some(&0x6a) { return None; }
    let (mut i, mut out) = (1usize, Vec::new());
    while i < spk.len() {
        let op = spk[i];
        i += 1;
        let n = match op {
            0x00..=0x4b => op as usize,
            0x4c => { let n = *spk.get(i)? as usize; i += 1; n }
            0x4d => { let n = u16::from_le_bytes(spk.get(i..i + 2)?.try_into().ok()?) as usize; i += 2; n }
            0x4e => { let n = u32::from_le_bytes(spk.get(i..i + 4)?.try_into().ok()?) as usize; i += 4; n }
            _ => return None,
        };
        out.extend_from_slice(spk.get(i..i.checked_add(n)?)?);
        i += n;
    }
    Some(out)
}

/// Check a deposit bundle against the caller's question. `network`, `core_image` and `headers_image` are the
/// caller's (the bundle's own `network` must agree); `now` is the caller's clock (unix seconds).
pub fn verify_deposit(b: &DepositBundle, ask: &DepositAsk, network: &str, core_image: &'static Image, headers_image: &str,
                      now: u64) -> DepositReport {
    let mut rep = DepositReport { checks: Vec::new(), verdict: Verdict::Fail, accepted: false, identity_checked: false, statement: None };
    if !rep.check(b.kind == BUNDLE_KIND && b.version == BUNDLE_VERSION,
                  format!("a deposit bundle, version {BUNDLE_VERSION} (kind {:?}, version {})", b.kind, b.version)) {
        return rep;
    }
    if !rep.check(b.network == network && core_image.network == network,
                  format!("the bundle is for {:?}, the network asked about {network:?}", b.network)) {
        return rep;
    }

    // 1. The core chain proof: full validation 0..=Hc.
    let Some(core_bytes) = unhex(&b.core_receipt) else { return rep.fail("core receipt: not hex") };
    let cr = verify(&Inputs { receipt: &core_bytes, image: core_image, anchor: None, now });
    for (ok, w) in &cr.checks { rep.checks.push((*ok, format!("core: {w}"))); }
    let (cs, undo_from) = match (&cr.verdict, &cr.state) {
        (Verdict::Fail, _) | (_, None) => return rep.fail("the core chain proof verifies (see the core: checks)"),
        (_, Some(cs)) => (cs.clone(), cr.undo_from),
    };
    let core_ok = cr.verdict == Verdict::Ok;

    // 2. The header-chain proof, bound to the core tip.
    let mut hs: Option<HeaderState> = None;
    let mut headers_ok = false;
    if !b.header_receipts.is_empty() {
        if network != "mainnet" { return rep.fail("the header-chain zAPP is mainnet only"); }
        let Some(bytes) = b.header_receipts.iter().map(|x| unhex(x)).collect::<Option<Vec<_>>>() else {
            return rep.fail("header receipts: not hex");
        };
        let refs: Vec<&[u8]> = bytes.iter().map(|x| x.as_slice()).collect();
        let hr = headers::verify_headers(&refs, headers_image, now, None, None);
        for (ok, w) in &hr.checks { rep.checks.push((*ok, format!("headers: {w}"))); }
        let s = match (&hr.verdict, hr.state) {
            (Verdict::Fail, _) | (_, None) => return rep.fail("the header-chain proof verifies (see the headers: checks)"),
            (_, Some(s)) => s,
        };
        headers_ok = hr.verdict == Verdict::Ok;
        let path = match hashes(&b.core_tip_mmr_path, "core_tip_mmr_path") { Ok(p) => p, Err(e) => return rep.fail(e) };
        if !rep.check(s.height >= cs.height as i64,
                      format!("the header chain ({}) reaches the core tip ({})", s.height, cs.height)) {
            return rep;
        }
        if !rep.check(headers::verify_inclusion(&s, cs.height as i64, &cs.tip_hash, &path),
                      format!("BINDING: the core tip {} {} is the header chain's block at that height (its MMR path, {} \
                               hashes, leads to a committed peak): the header chain's blocks 0..={} are the fully \
                               validated ones", cs.height, disp(&cs.tip_hash), path.len(), cs.height)) {
            return rep;
        }
        hs = Some(s);
    } else if !b.core_tip_mmr_path.is_empty() {
        return rep.fail("core_tip_mmr_path given without header receipts");
    }

    // 3. The transaction in its block.
    let d = &b.block;
    let Some(header) = unhex(&d.header).and_then(|h| <[u8; 80]>::try_from(h).ok()) else { return rep.fail("block header: not 80 bytes of hex") };
    let Some(raw) = unhex(&d.tx) else { return rep.fail("tx: not hex") };
    let Some(coinbase) = unhex(&d.coinbase) else { return rep.fail("coinbase: not hex") };
    let block_hash = dsha(&header);
    // An early refusal of the plainest form only. What closes the 64-byte ambiguity (CVE-2012-2459's family; BIP 54) is
    // in the inclusion checks below: the txid preimage (the serialisation WITHOUT witness) must not be 64 bytes, and the
    // tree depth is pinned by the block's coinbase and its branch (libzync::facts::tx_in_block, verify_tx_in_header).
    // A witness form of more than 64 bytes whose stripped form is 64 bytes passes this line and is refused there.
    if !rep.check(raw.len() != 64, format!("the transaction as given is not exactly 64 bytes ({} bytes)", raw.len())) {
        return rep;
    }
    let full = d.height <= cs.height;
    let view = if full {
        let Some(fo) = &d.forest else { return rep.fail(format!("block {} is at or below the core tip {}: give its block-forest opening", d.height, cs.height)) };
        let f = TxFact { height: d.height, header: d.header.clone(), state: fo.state.clone(), undo: fo.undo.clone(),
                         block_path: fo.path.clone(), tx: d.tx.clone(), index: d.index, branch: d.branch.clone(),
                         coinbase: d.coinbase.clone(), coinbase_branch: d.coinbase_branch.clone() };
        match check_tx_fact(&cs, undo_from, &f) {
            Err(e) => return rep.fail(format!("the transaction is in a fully validated block: {e}")),
            Ok((txid, view)) => {
                if !rep.check(txid == ask.txid, format!("the raw transaction is {}, the one asked about {}", disp(&txid), disp(&ask.txid))) {
                    return rep;
                }
                rep.check(true, format!("block {} {} is in the core proof's block forest (FULLY VALIDATED), and transaction {} \
                                         is at position {} of it (tree depth {} by the block's coinbase)",
                                        d.height, disp(&block_hash), disp(&txid), d.index, d.branch.len()));
                view
            }
        }
    } else {
        let Some(s) = &hs else {
            return rep.fail(format!("block {} is above the core tip {} and no header-chain proof is given", d.height, cs.height));
        };
        if !rep.check(ask.allow_pow_only_block,
                      format!("block {} is at or below the fully validated tip {} (it is above it: PROOF OF WORK ONLY; a \
                               caller may accept that with allow_pow_only_block)", d.height, cs.height)) {
            return rep;
        }
        let Some(mp) = &d.mmr_path else { return rep.fail("a block above the core tip needs its MMR path") };
        let mp = match hashes(mp, "mmr_path") { Ok(p) => p, Err(e) => return rep.fail(e) };
        if !rep.check(headers::verify_inclusion(s, d.height as i64, &block_hash, &mp),
                      format!("block {} {} is on the header chain (MMR path, {} hashes): PROOF OF WORK ONLY",
                              d.height, disp(&block_hash), mp.len())) {
            return rep;
        }
        let (Ok(branch), Ok(cb_branch)) = (hashes(&d.branch, "branch"), hashes(&d.coinbase_branch, "coinbase_branch")) else {
            return rep.fail("branch / coinbase_branch: not 64-hex-digit hashes");
        };
        let t = TxInclusion { header, txid: ask.txid, raw: raw.clone(), pos: d.index, branch, coinbase, coinbase_branch: cb_branch };
        if let Err(e) = headers::verify_tx_in_header(&block_hash, &t) {
            return rep.fail(format!("transaction {} is in block {}: {e}", disp(&ask.txid), d.height));
        }
        rep.check(true, format!("transaction {} is at position {} of block {} (tree depth {} by the block's coinbase)",
                                disp(&ask.txid), d.index, d.height, d.branch.len()));
        let mut o = 0;
        match libzync::facts::parse_tx(&raw, &mut o).filter(|_| o == raw.len()) {
            Some(v) => v,
            None => return rep.fail("tx: not one transaction"),
        }
    };

    // 4. The output asked about. The outputs are parsed from the bytes that hash to the txid, so they are bound.
    let mut output = None;
    if let Some(v) = ask.vout {
        let Some((value, spk)) = view.outputs.get(v as usize) else {
            return rep.fail(format!("output {v} exists (the transaction has {})", view.outputs.len()));
        };
        output = Some((v, *value, spk.clone()));
        if let Some(a) = ask.amount { rep.check(*value == a, format!("output {v} holds {a} sat (it holds {value})")); }
        if let Some(sc) = &ask.script {
            rep.check(spk == sc, format!("output {v}'s scriptPubKey is {} (it is {})", hex(sc), hex(spk)));
        }
        if let Some(p) = &ask.op_return {
            rep.check(op_return_payload(spk).as_deref() == Some(p.as_slice()),
                      format!("output {v} is an OP_RETURN carrying {} ({})", hex(p),
                              op_return_payload(spk).map(|x| format!("it carries {}", hex(&x))).unwrap_or("it is not a data-only OP_RETURN".into())));
        }
    } else {
        if ask.amount.is_some() || ask.script.is_some() {
            return rep.fail("an amount or scriptPubKey is asked without the output index (give vout)");
        }
        if let Some(p) = &ask.op_return {
            match view.outputs.iter().position(|(_, spk)| op_return_payload(spk).as_deref() == Some(p.as_slice())) {
                Some(i) => {
                    output = Some((i as u32, view.outputs[i].0, view.outputs[i].1.clone()));
                    rep.check(true, format!("output {i} is an OP_RETURN carrying {}", hex(p)));
                }
                None => { rep.check(false, format!("an output is an OP_RETURN carrying {}", hex(p))); }
            }
        }
    }

    // 5. Depth, as of the tip.
    let (tip_height, tip_hash, tip_work, tip_time) = match &hs {
        Some(s) => (s.height, s.hash, s.work, s.time as u64),
        None => (cs.height as i64, cs.tip_hash, cs.cum_work, cs.prev_time as u64),
    };
    let confirmations = (tip_height - d.height as i64 + 1).max(0) as u64;
    let validated_confirmations = if full { (cs.height - d.height + 1) as u64 } else { 0 };
    rep.check(confirmations >= ask.min_depth.max(1),
              format!("depth: {confirmations} confirmation(s) as of tip {tip_height} >= {} asked ({validated_confirmations} \
                       fully validated, {} proof of work only)", ask.min_depth.max(1), confirmations - validated_confirmations));
    if ask.min_validated_depth > 0 {
        rep.check(validated_confirmations >= ask.min_validated_depth,
                  format!("validated depth: {validated_confirmations} fully validated confirmation(s) >= {} asked{}",
                          ask.min_validated_depth,
                          if full { "" } else { " (the block itself is above the core tip: accepting it takes \
                                                 allow_pow_only_block AND min_validated_depth 0)" }));
    }
    let mut identity = false;
    if let Some((h, x)) = &ask.expect_tip {
        identity = *h == tip_height && disp(&tip_hash) == x.to_ascii_lowercase();
        rep.check(identity, format!("chain identity: the tip is block {h} {x} (from a source the caller trusts)"));
    }
    if let Some(age) = ask.max_tip_age {
        rep.check(tip_time.saturating_add(age) >= now, format!("the tip's block time {tip_time} is within {age} s of this clock {now}"));
    }

    let failed = rep.checks.iter().any(|(ok, _)| !ok);
    // Ok: the TIP proof shows Core's minimum chain work (the header chain when given and bound, else the core proof,
    // whose Ok also needs its pinned checkpoint). Below it: Demo, a valid statement on a chain not shown to be the network's.
    let enough = if hs.is_some() { headers_ok } else { core_ok };
    // Not a failed check: without the identity the chain may be valid, but is not shown to be the network's.
    if !failed && enough && ask.require_identity && !identity {
        rep.checks.push((true, "chain identity NOT checked: the verdict is at best DEMO (give the tip from a source you \
                                trust, or turn require_identity off)".into()));
    }
    rep.verdict = if failed { Verdict::Fail }
                  else if enough && (identity || !ask.require_identity) { Verdict::Ok }
                  else { Verdict::Demo };
    rep.identity_checked = !failed && identity;
    rep.accepted = rep.verdict == Verdict::Ok;
    if !failed && (rep.verdict == Verdict::Ok || ask.allow_demo) {
        rep.statement = Some(DepositStatement {
            txid: ask.txid, height: d.height, block_hash, output, tip_height, tip_hash, tip_work,
            validated_through: cs.height, confirmations, validated_confirmations, block_fully_validated: full,
            core_image: core_image_used(&core_bytes, core_image).method_id_hex.to_string(), headers_image: hs.as_ref().map(|_| headers_image.to_string()),
        });
    }
    rep
}

/// The core image the receipt was checked against: the one asked for, or the earlier built-in image of its chain that the
/// receipt names. The same substitution `swiftsync_verify::verify` makes (it asks `earlier_image_named_by` under the same
/// receipt_kind gate), asked of the core library here rather than read back from a check string, so the core's Report
/// stays as it is (a zAPP never changes the core verifier).
fn core_image_used(receipt: &[u8], asked: &'static Image) -> &'static Image {
    if swiftsync_verify::receipt_kind(receipt).is_err() {
        return asked;
    }
    bincode::deserialize::<risc0_zkvm::Receipt>(receipt).ok()
        .and_then(|r| swiftsync_verify::earlier_image_named_by(&r, asked))
        .unwrap_or(asked)
}

/// Build a bundle for transaction `txid` (internal order) of `raw_block` (as `getblock <hash> 0` prints it, the block
/// at `height`). `history`: the core proving run's history.bin, for a block at or below the core tip (its leaf in the
/// block forest). `core_tip_mmr_path` (with header receipts) and `block_mmr_path` (a block above the core tip): MMR
/// paths in the header chain's MMR ([`zapp_headers::mmr::path`]). Nothing here is trusted: [`verify_deposit`] checks it all.
#[allow(clippy::too_many_arguments)]
pub fn make_bundle(network: &str, core: &[u8], header_receipts: &[Vec<u8>], history: Option<&[u8]>, raw_block: &[u8],
                   height: u32, txid: &[u8; 32], core_tip_mmr_path: &[[u8; 32]], block_mmr_path: Option<&[[u8; 32]]>)
                   -> Result<DepositBundle, String> {
    let cs = core_state(core)?;
    let (header, txs) = libzync::facts::parse_block(raw_block).ok_or("the block does not parse")?;
    let ids: Vec<[u8; 32]> = txs.iter().map(libzync::facts::txid).collect();
    let index = ids.iter().position(|i| i == txid).ok_or("the transaction is not in that block")?;
    let bhash = libzync::facts::header_hash(&header);
    let (forest, mmr_path) = if height <= cs.height {
        use libzync::history::{block_path, decode_all, replay};
        let recs = decode_all(history.ok_or("a block at or below the core tip needs the core run's history.bin")?)?;
        let r = replay(&recs, cs.height, cs.height)?;
        if recs[height as usize].hash != bhash { return Err("the block is not the proven chain's at that height (history hash differs)".into()); }
        let (path, roots) = block_path(&r.block_leaves, height as u64);
        if roots != cs.block_roots { return Err("history.bin is not this core proof's run (its block forest differs)".into()); }
        (Some(ForestOpening { state: hex(&r.states[height as usize]), undo: r.undos[height as usize].map(|u| hex(&u)),
                              path: path.iter().map(|x| hex(x)).collect() }), None)
    } else {
        (None, Some(block_mmr_path.ok_or("a block above the core tip needs its MMR path")?.iter().map(|x| hex(x)).collect()))
    };
    Ok(DepositBundle {
        kind: BUNDLE_KIND.into(), version: BUNDLE_VERSION, network: network.into(), core_receipt: hex(core),
        header_receipts: header_receipts.iter().map(|x| hex(x)).collect(),
        core_tip_mmr_path: core_tip_mmr_path.iter().map(|x| hex(x)).collect(),
        block: DepositBlock {
            height, header: hex(&header), forest, mmr_path, tx: hex(&txs[index].stripped), index: index as u32,
            branch: libzync::facts::merkle_branch(&ids, index).iter().map(|x| hex(x)).collect(),
            coinbase: hex(&txs[0].stripped),
            coinbase_branch: libzync::facts::merkle_branch(&ids, 0).iter().map(|x| hex(x)).collect(),
        },
    })
}

/// The core tip's state, for a tool that builds a bundle (it must already have verified the receipt).
pub fn core_state(receipt: &[u8]) -> Result<ChainState, String> {
    let r: risc0_zkvm::Receipt = bincode::deserialize(receipt).map_err(|e| format!("core receipt: {e}"))?;
    r.journal.decode().map_err(|e| format!("core journal: {e}"))
}

#[cfg(test)]
mod tests;
