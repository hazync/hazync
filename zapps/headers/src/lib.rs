//! The header-chain zyncAPP (ROADMAP K1, N3): what a verifier checks on its receipts.
//!
//! The guest (`prover/zyncapps/methods/headers`) runs Core's header checks over a run of mainnet headers and commits
//! `(sha256 of its input state, its output state)`. A run from genesis must start from the one empty state, so a chain
//! of runs is valid from genesis when the first run's input digest is the empty state's and every later run's input
//! digest is the digest of the state the run before it ended in. That is how the proof follows the tip: new headers
//! are a short run chained to the last one.
//!
//! Verdict, as for the core proof: `Ok` when the chain's work reaches Core's minimum chain work for mainnet, `Demo`
//! below it, `Fail` on any failed check. Chain identity (that this is the network's chain, not only a valid one) is
//! the tip hash, compared with a source the verifier trusts (`expect`).
//!
//! The state carries the peaks of a Merkle Mountain Range of every block hash 0..=height (`crate::mmr`, the guest's
//! own file), so a verified chain also answers "is block X at height h on this chain" ([`verify_inclusion`]) and,
//! with the block's 80-byte header, the raw transaction and its merkle branch, and the block's raw coinbase and its
//! branch (which pin the tree's depth), "is transaction T in a block on this chain" ([`verify_tx_in_header`]). The MMR root ([`HeaderState::mmr_root`]) is derived from the committed peaks, not
//! stored: it is a short name for the peaks, to compare or publish.
//!
//! A zAPP module of its own (docs/libzync/ZYNCAPPS.md §9): it uses the core verifier `swiftsync_verify` as a library
//! (receipt checks, images, work) and is not part of it. Until 15.11.2026 this was `swiftsync_verify::headers` and the
//! CLI's `swiftsync-verify headers`, the WASM's `verifyHeaders`; the checks are unchanged.

/// The header-chain zyncAPP's Merkle Mountain Range of block hashes: the guest's own file (one source for the guest,
/// this verifier and the zyncapp host's path builder).
pub mod mmr {
    include!("../../../prover/zyncapps/methods/headers/src/mmr.rs");
}
extern crate alloc;
use swiftsync_verify::{decode_journal, decode_receipt, dev_mode_env, receipt_kind, work_at_least, Verdict, IMAGES, MAX_FUTURE_BLOCK_TIME};
use risc0_zkvm::sha::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// The header-chain image id this verifier trusts by default: the `zyncapp-headers` line of reproduce/LIBZYNC_IDS (the
/// reproducible build; reproduce/pin-libzync-ids.py sets it). `--image <hex>` replaces it for a reader who rebuilt it.
pub const HEADERS_IMAGE_ID: &str = "de54b94f40d750c746aafd8f306988791a074e424e81e411702312d4c528bf07";

/// The guest's state, field for field and in order (risc0 serde): the last header's height, hash (internal byte
/// order), chainwork (little-endian), nBits, time, its retarget epoch's first time, the last <= 11 times, the
/// largest time seen, and the MMR peaks of the block hashes 0..=height (left to right, largest first; crate::mmr).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct HeaderState {
    pub height: i64,
    pub hash: [u8; 32],
    pub work: [u8; 32],
    pub bits: u32,
    pub time: u32,
    pub epoch_first_time: u32,
    pub recent_times: Vec<u32>,
    pub max_time: u32,
    pub mmr_peaks: Vec<[u8; 32]>,
}

impl HeaderState {
    /// The state before genesis: the only state a run from genesis may start from.
    pub fn empty() -> Self {
        HeaderState { height: -1, hash: [0; 32], work: [0; 32], bits: 0, time: 0, epoch_first_time: 0,
                      recent_times: Vec::new(), max_time: 0, mmr_peaks: Vec::new() }
    }
    /// The MMR's leaf count: blocks 0..=height (0 below genesis). In u64, so even height i64::MAX does not overflow.
    pub fn leaves(&self) -> u64 { if self.height < 0 { 0 } else { self.height as u64 + 1 } }
    /// The MMR root: the committed peaks bagged right to left (crate::mmr::root). Pair it with the height.
    pub fn mmr_root(&self) -> [u8; 32] { crate::mmr::root(&self.mmr_peaks) }
    pub fn mmr_root_hex(&self) -> String { self.mmr_root().iter().map(|b| format!("{b:02x}")).collect() }
    /// What the guest commits for an input state: sha256 of risc0's serde words, as little-endian bytes.
    pub fn digest(&self) -> [u8; 32] {
        let w = risc0_zkvm::serde::to_vec(self).expect("a HeaderState always serialises");
        Sha256::digest(w.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>()).into()
    }
    /// The tip hash as block explorers print it (reversed, hex).
    pub fn hash_hex(&self) -> String { self.hash.iter().rev().map(|b| format!("{b:02x}")).collect() }
    pub fn work_hex(&self) -> String { self.work.iter().rev().map(|b| format!("{b:02x}")).collect() }
}

pub struct HeadersReport {
    pub checks: Vec<(bool, String)>,
    /// The state the last run ended in, when every receipt decoded.
    pub state: Option<HeaderState>,
    pub verdict: Verdict,
    /// What an `include` showed, set only when NO check failed (verdict Ok or Demo: read it with the verdict): the
    /// block's height and hash, and the txid of the transaction found in it. The txid is the one the CALLER asked about
    /// (TxInclusion::txid) and the raw transaction hashed to it; nothing here comes from the path service's word.
    pub included: Option<Included>,
    /// The tip was compared with an `expect` (from a source the caller trusts) and matched, in a report that is not a
    /// FAIL. The verdict is about the PROOF (valid, and enough work for Ok); a caller that says "the network's chain"
    /// must also require this, as every zAPP's CLI and WASM output does (an Ok without it is shown as DEMO there:
    /// [`HeadersReport::gated`]).
    pub tip_checked: bool,
}

impl HeadersReport {
    /// The verdict a caller shows: Ok only when the tip was also compared with a trusted source; a valid chain with
    /// enough work whose identity was NOT checked is Demo (a different chain with as much work is not ruled out), as in
    /// every other zAPP (review 11-16).
    pub fn gated(&self) -> Verdict {
        match self.verdict { Verdict::Ok if !self.tip_checked => Verdict::Demo, v => v }
    }
}

/// A block (and optionally a transaction in it) shown to be on the verified chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Included {
    pub height: i64,
    /// internal byte order
    pub hash: [u8; 32],
    /// internal byte order
    pub txid: Option<[u8; 32]>,
}

impl Included {
    pub fn hash_hex(&self) -> String { disp(&self.hash) }
    pub fn txid_hex(&self) -> Option<String> { self.txid.as_ref().map(disp) }
}

/// A hash as explorers print it (reversed, hex).
fn disp(b: &[u8; 32]) -> String { b.iter().rev().map(|x| format!("{x:02x}")).collect() }

impl HeadersReport {
    fn check(&mut self, ok: bool, what: impl Into<String>) -> bool { self.checks.push((ok, what.into())); ok }
}

/// Verify a chain of header-chain receipts (the first from genesis, each next one starting where the one before ended)
/// against the headers image id `image_hex`. `now` is the verifier's clock (unix seconds) for time-too-new;
/// `expect` = (height, hash as explorers print it) from a source the verifier trusts, for chain identity.
///
/// `include`: also check that a block (and optionally a transaction in it) is on the proven chain, against the last
/// run's MMR peaks; a failed inclusion is a failed check (verdict `Fail`).
/// The most chained runs [`verify_headers`] takes (as the WASM frame allows): a tip follower re-bases long before.
pub const MAX_RUNS: usize = 4096;

pub fn verify_headers(receipts: &[&[u8]], image_hex: &str, now: u64, expect: Option<(i64, &str)>,
                      include: Option<&Inclusion>) -> HeadersReport {
    verify_headers_with(receipts, image_hex, None, now, expect, include)
}

/// [`verify_headers`] with the headers image's engine key: every run is then an ENGINE (OpenVM) proof file of that image,
/// checked under the pinned key (engine/VERIFIER.md: the key, not the image id, authenticates it); without one, every
/// run is a RISC Zero receipt. Everything after the proof check (the runs' linking, time, identity, inclusion, work) is
/// the same code for both.
pub fn verify_headers_with(receipts: &[&[u8]], image_hex: &str, engine_key: Option<&swiftsync_verify::EngineKey>, now: u64,
                           expect: Option<(i64, &str)>, include: Option<&Inclusion>) -> HeadersReport {
    let mut rep = HeadersReport { checks: Vec::new(), state: None, verdict: Verdict::Fail, included: None, tip_checked: false };
    let Some(id) = hex32(image_hex) else {
        rep.check(false, format!("the headers image id is 32 bytes of hex ({image_hex})"));
        return rep;
    };
    let image = Digest::from_bytes(id);
    if !rep.check(!receipts.is_empty(), "at least one receipt") { return rep; }
    // a cap on the runs, checked before any is verified: each costs a STARK verification
    if !rep.check(receipts.len() <= MAX_RUNS, format!("at most {MAX_RUNS} chained runs ({} given)", receipts.len())) { return rep; }
    // risc0 built with disable-dev-mode panics in Receipt::verify under RISC0_DEV_MODE (see swiftsync_verify::verify)
    if !rep.check(!dev_mode_env(), "RISC0_DEV_MODE is not set in this verifier's environment") { return rep; }
    let mut want_in = HeaderState::empty().digest();
    let mut last: Option<HeaderState> = None;
    for (i, bytes) in receipts.iter().enumerate() {
        let n = i + 1;
        // an engine run: verified under the key, its journal read only once it has
        let journal = if let Some(k) = engine_key {
            match engine_run(bytes, k, &id) {
                Ok(j) => {
                    rep.check(true, format!("receipt {n} (an engine proof) verifies against the headers image {image_hex} under the pinned key {}", swiftsync_verify::hex(&k.pin)));
                    j
                }
                Err(e) => { rep.check(false, format!("receipt {n} (an engine proof) verifies against the headers image {image_hex}: {e}")); return rep; }
            }
        } else {
            if let Err(e) = receipt_kind(bytes) {
                rep.check(false, format!("receipt {n} is a single succinct or Groth16 proof ({e})"));
                return rep;
            }
            let r: risc0_zkvm::Receipt = match decode_receipt(bytes) {
                Ok(r) => r,
                Err(e) => { rep.check(false, format!("receipt {n} decodes ({e})")); return rep; }
            };
            if !rep.check(!matches!(r.inner, risc0_zkvm::InnerReceipt::Fake(_)), format!("receipt {n} is a real proof")) {
                return rep;
            }
            // An unverified journal is not read at all: nothing in it is known, and its numbers (a height of i64::MAX)
            // must not reach the arithmetic below.
            let v = r.verify(image);
            if !rep.check(v.is_ok(), format!("receipt {n} verifies against the headers image {image_hex}{}",
                                             v.err().map(|e| format!(": {e}")).unwrap_or_default())) {
                return rep;
            }
            r.journal
        };
        let (din, out): ([u8; 32], HeaderState) = match decode_journal(&journal) {
            Ok(j) => j,
            Err(e) => { rep.check(false, format!("receipt {n}'s journal is (input digest, header state) ({e})")); return rep; }
        };
        let from = last.as_ref().map_or(-1, |s| s.height);
        // a broken link ends the check: nothing after it is on this chain, and the runs after it are not verified
        // (N copies of one receipt would otherwise cost N verifications)
        if !rep.check(din == want_in, if i == 0 {
            "the first run starts from genesis (its input is the empty state)".to_string()
        } else {
            format!("run {n} starts where run {i} ended (height {from})")
        }) {
            return rep;
        }
        if !rep.check(out.height > from, format!("run {n} adds headers ({}..{})", from.saturating_add(1), out.height)) {
            return rep;
        }
        want_in = out.digest();
        last = Some(out);
    }
    let s = last.expect("at least one receipt decoded");
    rep.check(s.max_time as u64 <= now.saturating_add(MAX_FUTURE_BLOCK_TIME),
              format!("time-too-new: largest header time {} <= this clock {} + {} s", s.max_time, now, MAX_FUTURE_BLOCK_TIME));
    let mut tip_match = false;
    if let Some((h, hash)) = expect {
        tip_match = s.height == h && s.hash_hex() == hash.to_ascii_lowercase();
        rep.check(tip_match, format!("chain identity: the tip is block {h} {hash} (from a source this verifier trusts)"));
    }
    if let Some(inc) = include {
        rep.check(verify_inclusion(&s, inc.height, &inc.hash, &inc.path),
                  format!("block {} {} is on this chain: its MMR path ({} hashes) leads to a committed peak (MMR root {})",
                          inc.height, disp(&inc.hash), inc.path.len(), s.mmr_root_hex()));
        if let Some(tx) = &inc.tx {
            match verify_tx_in_header(&inc.hash, tx) {
                Ok(w) => rep.check(true, format!("transaction {} is in block {}: {w}", disp(&tx.txid), inc.height)),
                Err(e) => rep.check(false, format!("transaction {} is in block {}: {e}", disp(&tx.txid), inc.height)),
            };
        }
    }
    let failed = rep.checks.iter().any(|(ok, _)| !ok);
    if !failed {
        rep.included = include.map(|inc| Included { height: inc.height, hash: inc.hash, txid: inc.tx.as_ref().map(|t| t.txid) });
    }
    let mainnet = IMAGES.iter().find(|i| i.network == "mainnet").expect("IMAGES holds mainnet");
    let enough = work_at_least(&s.work, mainnet.min_work_hex);
    rep.checks.push((true, format!("chainwork {} {} Core's minimum {}", s.work_hex(),
                                   if enough { ">=" } else { "<" }, mainnet.min_work_hex)));
    rep.verdict = if failed { Verdict::Fail } else if enough { Verdict::Ok } else { Verdict::Demo };
    rep.tip_checked = tip_match && !failed;
    rep.state = Some(s);
    rep
}

/// A block to find on the proven chain, and optionally a transaction in it.
#[derive(Clone, Debug)]
pub struct Inclusion {
    pub height: i64,
    /// internal byte order
    pub hash: [u8; 32],
    /// MMR siblings from the leaf up to its peak (lowest first), as `zyncapp headers-mmr-path` prints them
    pub path: Vec<[u8; 32]>,
    pub tx: Option<TxInclusion>,
}

/// A transaction in the included block, with what it takes to show it is a LEAF of the block's merkle tree:
/// the block's 80-byte header; the transaction's raw bytes (as getrawtransaction prints them, with or without its
/// witness), its position and merkle branch; and the block's raw COINBASE transaction with its branch (position 0).
/// Branches are internal order, lowest level first.
///
/// `txid` is the transaction ASKED ABOUT, from the caller (internal order), never from the path service's answer:
/// the raw bytes must hash to it. An answer that is a valid proof for ANOTHER confirmed transaction is refused.
#[derive(Clone, Debug)]
pub struct TxInclusion {
    pub header: [u8; 80],
    /// the transaction asked about (internal order): the caller's, not the answer's
    pub txid: [u8; 32],
    pub raw: Vec<u8>,
    pub pos: u32,
    pub branch: Vec<[u8; 32]>,
    pub coinbase: Vec<u8>,
    pub coinbase_branch: Vec<[u8; 32]>,
}

/// The path service's /tx answer, as the verifier takes it (`tx` of the include JSON, `zapp-headers --tx`): the six
/// fields read, and the answer's other DOCUMENTED keys (coordinator/deploy/headers-api: `txid`, `height`, `hash`, `tip`,
/// `tip_hash`, `leaves`, `path`, `mmr_root`; `ntx` in the node fixtures) taken and NOT read. Any other key, or a key given
/// twice, is refused (review 11-18): a key spelt wrong is an error, never dropped unread.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TxAnswer {
    pos: u64,
    header: String,
    raw: String,
    branch: Vec<String>,
    coinbase: String,
    coinbase_branch: Vec<String>,
    // not read: which transaction the raw bytes are is computed from them (verify_tx_in_header), never taken from here
    #[serde(default, rename = "txid")]
    _txid: Option<serde::de::IgnoredAny>,
    #[serde(default, rename = "height")]
    _height: Option<serde::de::IgnoredAny>,
    #[serde(default, rename = "hash")]
    _hash: Option<serde::de::IgnoredAny>,
    #[serde(default, rename = "tip")]
    _tip: Option<serde::de::IgnoredAny>,
    #[serde(default, rename = "tip_hash")]
    _tip_hash: Option<serde::de::IgnoredAny>,
    #[serde(default, rename = "leaves")]
    _leaves: Option<serde::de::IgnoredAny>,
    #[serde(default, rename = "path")]
    _path: Option<serde::de::IgnoredAny>,
    #[serde(default, rename = "mmr_root")]
    _mmr_root: Option<serde::de::IgnoredAny>,
    #[serde(default, rename = "ntx")]
    _ntx: Option<serde::de::IgnoredAny>,
}

/// The include JSON of the WASM module: `{"height", "hash", "path", "txid"?, "tx"?}`, those keys only, txid and tx
/// both or neither (a null for either is refused in [`parse_include_json`]).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IncludeJson {
    height: i64,
    hash: String,
    path: Vec<String>,
    #[serde(default)]
    txid: Option<String>,
    #[serde(default)]
    tx: Option<TxAnswer>,
}

/// serde's derived structs also take a JSON ARRAY of their fields in order: refuse anything but an object where one is
/// meant.
fn need_object(v: &serde_json::Value, what: &str) -> Result<(), String> {
    if v.is_object() { Ok(()) } else { Err(format!("{what} is not a JSON object")) }
}

impl TxInclusion {
    /// `txid`: the transaction asked about, from the CALLER (as explorers print it). `v`: the JSON the path service's
    /// /tx answers: `{"pos", "header", "raw", "branch", "coinbase", "coinbase_branch"}` and its documented other keys
    /// ([`TxAnswer`]: taken, not read; any other key is refused). Branch entries are reversed hex (as explorers and
    /// Electrum's blockchain.transaction.get_merkle print them); header, raw and coinbase are plain hex of the bytes
    /// (getblockheader <hash> false, getrawtransaction <txid>). The answer's own "txid" is never read: which transaction
    /// the raw bytes are is computed from them (verify_tx_in_header). A `serde_json::Value` has already kept the last of
    /// two equal keys: from bytes, use [`TxInclusion::from_json_bytes`], which refuses them.
    pub fn from_json(txid: &str, v: &serde_json::Value) -> Result<TxInclusion, String> {
        need_object(v, "tx")?;
        let a: TxAnswer = serde_json::from_value(v.clone()).map_err(|e| format!("tx: {e}"))?;
        Self::from_answer(txid, a)
    }

    /// [`TxInclusion::from_json`] from the answer's bytes: a key given twice is refused too.
    pub fn from_json_bytes(txid: &str, b: &[u8]) -> Result<TxInclusion, String> {
        let v: serde_json::Value = serde_json::from_slice(b).map_err(|e| format!("tx: not JSON ({e})"))?;
        need_object(&v, "tx")?;
        let a: TxAnswer = serde_json::from_slice(b).map_err(|e| format!("tx: {e}"))?;
        Self::from_answer(txid, a)
    }

    fn from_answer(txid: &str, a: TxAnswer) -> Result<TxInclusion, String> {
        let list = |k: &str, l: &[String]| -> Result<Vec<[u8; 32]>, String> {
            l.iter().map(|x| hex32(x).map(|mut b| { b.reverse(); b }).ok_or(format!("tx: {k} holds a non-hash"))).collect()
        };
        let header: [u8; 80] = unhex(&a.header).filter(|h| h.len() == 80).and_then(|h| h.try_into().ok())
            .ok_or("tx: header is not 80 bytes of hex")?;
        let mut txid = hex32(txid).ok_or(format!("tx: the txid asked about is not 32 bytes of hex ({txid})"))?;
        txid.reverse();
        let pos = u32::try_from(a.pos).map_err(|_| "tx: pos is not a position")?;
        Ok(TxInclusion { header, txid, raw: unhex(&a.raw).ok_or("tx: raw is not hex")?, pos, branch: list("branch", &a.branch)?,
                         coinbase: unhex(&a.coinbase).ok_or("tx: coinbase is not hex")?,
                         coinbase_branch: list("coinbase_branch", &a.coinbase_branch)? })
    }
}

/// The include JSON of the WASM module (zapps/headers/wasm: `{"height", "hash" (as explorers print it), "path" (MMR
/// hashes as headers-mmr-path prints them), "txid"? and "tx"?, both or neither}`), parsed strictly (review 11-18): it was
/// read key by key with every other key ignored, so `txid` and `tx` both spelt wrong gave a block-only inclusion. Now
/// any other key, a key given twice, a top level or `tx` that is not an object, and txid without tx (or tx without
/// txid) are refused.
pub fn parse_include_json(j: &[u8]) -> Result<Inclusion, String> {
    let v: serde_json::Value = serde_json::from_slice(j).map_err(|e| format!("include: not JSON ({e})"))?;
    need_object(&v, "include")?;
    // an explicit null is refused (review 11-18 round 2): `{"txid": null, "tx": null}` read as absent gave a block-only
    // inclusion to a caller who meant to ask about a transaction. Absent is a block alone (JSON.stringify drops undefined).
    for k in ["txid", "tx"] {
        if v.get(k).is_some_and(|x| x.is_null()) {
            return Err(format!("include: {k} is null: leave both txid and tx out to ask about the block alone"));
        }
    }
    // both or neither, said before the answer's own fields are read
    match (v.get("txid").is_some(), v.get("tx")) {
        (false, Some(_)) => return Err("include: tx needs txid, the transaction asked about (never taken from the answer)".into()),
        (true, None) => return Err("include: txid needs tx, the path service's /tx answer".into()),
        (_, Some(t)) => need_object(t, "include: tx")?,
        _ => {}
    }
    let i: IncludeJson = serde_json::from_slice(j)
        .map_err(|e| format!("include (only height, hash, path, txid, tx; tx the /tx answer): {e}"))?;
    let tx = match (i.txid, i.tx) {
        (None, None) => None,
        (Some(txid), Some(a)) => Some(TxInclusion::from_answer(&txid, a)?),
        _ => return Err("include: txid and tx go together".into()),   // checked above; a key twice is refused before
    };
    let path: Vec<&str> = i.path.iter().map(|s| s.as_str()).collect();
    Inclusion::parse(i.height, &i.hash, &path, tx)
}

/// Plain hex, even length, digits only: u8::from_str_radix alone would also take a sign ("+f").
fn unhex(x: &str) -> Option<Vec<u8>> {
    if x.len() % 2 != 0 || !x.bytes().all(|c| c.is_ascii_hexdigit()) { return None; }
    (0..x.len()).step_by(2).map(|i| u8::from_str_radix(x.get(i..i + 2)?, 16).ok()).collect()
}

impl Inclusion {
    /// From text: `hash` as explorers print it (reversed hex); `path`: 64-hex MMR hashes as headers-mmr-path prints
    /// them (raw byte order, not reversed); `tx` from [`TxInclusion::from_json`].
    pub fn parse(height: i64, hash: &str, path: &[&str], tx: Option<TxInclusion>) -> Result<Inclusion, String> {
        let mut h = hex32(hash).ok_or(format!("block hash: not 32 bytes of hex ({hash})"))?;
        h.reverse();
        let path = path.iter().map(|x| hex32(x).ok_or(format!("MMR path: not 32 bytes of hex ({x})"))).collect::<Result<Vec<_>, _>>()?;
        Ok(Inclusion { height, hash: h, path, tx })
    }
}

/// Is `block_hash` (internal order) the block at `height` of the chain whose header state is `state`, by this MMR
/// `path`? The peak, and the path's length, follow from `height` and the leaf count (state.height + 1).
pub fn verify_inclusion(state: &HeaderState, height: i64, block_hash: &[u8; 32], path: &[[u8; 32]]) -> bool {
    height >= 0 && crate::mmr::verify(&state.mmr_peaks, state.leaves(), height as u64, block_hash, path)
}

/// A transaction parsed just far enough to know it IS one: its txid preimage (the serialisation without witness), the
/// number of inputs and the first input's previous output (36 bytes).
struct ParsedTx { stripped: Vec<u8>, inputs: u64, first_prevout: [u8; 36] }

/// Parse a raw transaction (Bitcoin's serialisation, legacy or BIP 144 segwit): version, inputs (at least one),
/// outputs (at least one), the witness when the segwit marker and flag (0x00 0x01) follow the version, lock time, and
/// nothing after it. The txid is over the serialisation WITHOUT the witness, so a segwit transaction is stripped to it
/// here; a caller may pass either form.
fn parse_tx(raw: &[u8]) -> Result<ParsedTx, String> {
    struct R<'a> { b: &'a [u8], i: usize }
    impl R<'_> {
        fn take(&mut self, n: usize) -> Result<&[u8], String> {
            let e = self.i.checked_add(n).filter(|e| *e <= self.b.len()).ok_or("the transaction ends early")?;
            let s = &self.b[self.i..e]; self.i = e; Ok(s)
        }
        /// A count, as Core reads a compact size (libzync::facts::varint: the shortest form, at most MAX_SIZE).
        fn var(&mut self) -> Result<u64, String> {
            if self.i >= self.b.len() { return Err("the transaction ends early".into()); }
            let n = libzync::facts::varint(self.b, &mut self.i).ok_or("a compact size not as Core reads it (shortest form, at most MAX_SIZE), or cut short")?;
            if n > self.b.len() as u64 { return Err("a count larger than the transaction".into()); }
            Ok(n)
        }
        /// A length of bytes still to come: a count that also fits in the bytes left.
        /// (`var` bounds it by the transaction's length, so it fits a usize on wasm32 too.)
        fn len(&mut self) -> Result<usize, String> {
            let n = self.var()? as usize;
            if n > self.b.len() - self.i { return Err("a length past the end of the transaction".into()); }
            Ok(n)
        }
    }
    let mut r = R { b: raw, i: 0 };
    let version = r.take(4)?.to_vec();
    let segwit = raw.len() > 6 && raw[4] == 0 && raw[5] == 1;
    if segwit { r.take(2)?; }
    let body_start = r.i;
    let inputs = r.var()?;
    if inputs == 0 { return Err("a transaction with no inputs".into()); }
    let mut first_prevout = [0u8; 36];
    for k in 0..inputs {
        let prev = r.take(36)?;
        if k == 0 { first_prevout.copy_from_slice(prev); }
        let n = r.len()?; r.take(n)?; r.take(4)?;
    }
    let outputs = r.var()?;
    if outputs == 0 { return Err("a transaction with no outputs".into()); }
    for _ in 0..outputs { r.take(8)?; let n = r.len()?; r.take(n)?; }
    let body_end = r.i;
    if segwit {
        let mut any = false;
        for _ in 0..inputs {
            let k = r.var()?;
            any |= k > 0;
            for _ in 0..k { let n = r.len()?; r.take(n)?; }
        }
        // Core's UnserializeTransaction: "Superfluous witness record" (the marker with every stack empty)
        if !any { return Err("a witness section with every stack empty".into()); }
    }
    let lock = r.take(4)?.to_vec();
    if r.i != raw.len() { return Err("bytes after the lock time".into()); }
    let mut stripped = version;
    stripped.extend_from_slice(&raw[body_start..body_end]);
    stripped.extend_from_slice(&lock);
    Ok(ParsedTx { stripped, inputs, first_prevout })
}

/// Bitcoin's merkle climb from `leaf` at `pos` by `branch` (internal order, lowest first): at level j the node is the
/// left child when bit j of `pos` is 0 (parent = dsha(node || sibling)), else the right child (dsha(sibling || node)).
/// Bitcoin's odd-level rule (the last node is paired with itself) shows as a sibling equal to the node on the LEFT
/// child side, which is accepted; an equal sibling on the RIGHT child side cannot occur in a real tree and is refused
/// (None), as is a position that does not fit the branch (pos >= 2^len).
fn merkle_climb(leaf: &[u8; 32], pos: u32, branch: &[[u8; 32]]) -> Option<[u8; 32]> {
    if branch.len() >= 32 || (pos as u64) >> branch.len() != 0 { return None; }
    let mut cur = *leaf;
    for (j, sib) in branch.iter().enumerate() {
        let mut b = [0u8; 64];
        if pos >> j & 1 == 0 {
            b[..32].copy_from_slice(&cur);
            b[32..].copy_from_slice(sib);
        } else {
            if *sib == cur { return None; }
            b[..32].copy_from_slice(sib);
            b[32..].copy_from_slice(&cur);
        }
        cur = dsha(&b);
    }
    Some(cur)
}

fn dsha(b: &[u8]) -> [u8; 32] { Sha256::digest(Sha256::digest(b)).into() }

/// Is `tx` a transaction of the block `block_hash`, and the one asked about (`tx.txid`, the caller's)? `Ok` with
/// what was shown, or `Err` with why not.
///
/// THE ATTACK THIS CLOSES (the merkle tree weakness, CVE-2012-2459's family). Bitcoin's merkle tree hashes a leaf
/// (a txid = dsha of a transaction) exactly as it hashes an inner node (dsha of the 64-byte concatenation of its two
/// children), and the header does not say how deep the tree is. So:
///   - SHORTER: an inner node's two children, 64 bytes, presented as a "transaction", hash to that node; with the
///     branch above the node it reaches the merkle root. A txid check alone does not help: the 64 bytes DO hash to the
///     claimed "txid".
///   - LONGER: a miner can put a real 64-byte transaction in a block whose bytes are two 32-byte values of their choice
///     (one being dsha of a fake transaction), so a branch one level deeper "proves" the fake one.
/// Both need the tree depth to be wrong for the claimed leaf. This check closes both with the standard SPV rules:
///   1. the RAW transaction is given, must parse as a transaction (parse_tx), must hash (dsha of its serialisation
///      without witness) to the txid, and must NOT be exactly 64 bytes long. 64 bytes is precisely the size of an inner
///      node; a real 64-byte transaction cannot be told apart from one, is non-standard, and BIP 54 (consensus cleanup)
///      proposes to forbid it, so it is refused rather than guessed at.
///   2. the DEPTH is proven by the coinbase: the block's raw coinbase (position 0) is given with its own branch, must
///      parse as a coinbase (one input, the null previous output: 32 zero bytes and index 0xffffffff), must not be 64
///      bytes, and its branch must reach the same merkle root; the transaction's branch must be exactly as long. The
///      coinbase pins the depth because it cannot be faked at another depth: one level SHORTER, the "coinbase" would be
///      a 64-byte inner node (refused); one level LONGER, the block's real coinbase would have to be the 64-byte
///      concatenation of two hashes, the first being dsha of the presented coinbase, but a real coinbase's bytes 5..36
///      are its null previous txid (zeros), so that first hash would need ~216 leading zero bits: infeasible.
/// With the depth pinned the transaction's branch ends on a real leaf, and rule 1 makes that leaf this transaction.
/// Also checked: the header hashes (dsha) to `block_hash`, and the odd-level and position rules of merkle_climb.
pub fn verify_tx_in_header(block_hash: &[u8; 32], tx: &TxInclusion) -> Result<String, String> {
    if dsha(&tx.header) != *block_hash { return Err("the header does not hash to the block".into()); }
    let root: [u8; 32] = tx.header[36..68].try_into().unwrap();
    const SIXTY_FOUR: &str = "is exactly 64 bytes long: it cannot be told apart from an inner merkle node (the \
                              CVE-2012-2459 merkle weakness), is non-standard, and BIP 54 proposes to forbid it";
    // 64 bytes is refused before parsing, whatever the bytes: an inner node need not parse to be one
    if tx.raw.len() == 64 { return Err(format!("the transaction {SIXTY_FOUR}")); }
    let t = parse_tx(&tx.raw).map_err(|e| format!("the raw transaction does not parse: {e}"))?;
    if t.stripped.len() == 64 { return Err(format!("the transaction {SIXTY_FOUR}")); }
    // which transaction this is comes from its bytes: it must be the one the caller asked about
    let txid = dsha(&t.stripped);
    if txid != tx.txid {
        return Err(format!("the raw transaction is {}, not the transaction asked about {}", disp(&txid), disp(&tx.txid)));
    }
    if tx.coinbase.len() == 64 { return Err(format!("the coinbase {SIXTY_FOUR}")); }
    let c = parse_tx(&tx.coinbase).map_err(|e| format!("the raw coinbase does not parse: {e}"))?;
    let mut null = [0u8; 36];
    null[32..].copy_from_slice(&[0xff; 4]);
    if c.inputs != 1 || c.first_prevout != null { return Err("the coinbase is not a coinbase (one input, the null previous output)".into()); }
    if c.stripped.len() == 64 { return Err(format!("the coinbase {SIXTY_FOUR}")); }
    if merkle_climb(&dsha(&c.stripped), 0, &tx.coinbase_branch) != Some(root) {
        return Err("the coinbase's branch does not lead to the header's merkle root (not this block's coinbase)".into());
    }
    if tx.branch.len() != tx.coinbase_branch.len() {
        return Err(format!("the transaction's branch is {} levels deep, the coinbase's (the tree's depth) {}",
                           tx.branch.len(), tx.coinbase_branch.len()));
    }
    if merkle_climb(&txid, tx.pos, &tx.branch) != Some(root) {
        return Err(format!("the merkle branch from position {} does not lead to the header's merkle root", tx.pos));
    }
    Ok(format!("at position {} of {} levels, the tree's depth by the block's coinbase", tx.pos, tx.branch.len()))
}

fn hex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 || !s.bytes().all(|c| c.is_ascii_hexdigit()) { return None; }
    let mut o = [0u8; 32];
    for i in 0..32 { o[i] = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?; }
    Some(o)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The header-chain zyncAPP's built-in id is the published one (reproduce/LIBZYNC_IDS, `zyncapp-headers` line).
    #[test]
    fn headers_image_matches_the_published_id() {
        let f = include_str!("../../../reproduce/LIBZYNC_IDS");
        let line = f.lines().find(|l| l.starts_with("zyncapp-headers mainnet ")).expect("reproduce/LIBZYNC_IDS has no zyncapp-headers line");
        let id = line.split_whitespace().skip_while(|w| *w != "METHOD_ID").nth(1).unwrap();
        assert_eq!(id, HEADERS_IMAGE_ID, "HEADERS_IMAGE_ID and reproduce/LIBZYNC_IDS disagree");
    }

    use crate::mmr;
    /// The empty state's digest must equal what the guest computes over the same serde words: pinned here so a
    /// change to HeaderState's layout (which would break every chain of runs) fails a test.
    #[test]
    fn empty_state_serialises_as_the_guest_reads_it() {
        let w = risc0_zkvm::serde::to_vec(&HeaderState::empty()).unwrap();
        // height i64 (2 words), hash (32 words, one per byte), work (32), bits, time, epoch_first_time, len 0, max_time,
        // mmr_peaks len 0
        assert_eq!(w.len(), 2 + 32 + 32 + 3 + 1 + 1 + 1);
        assert_eq!(&w[..2], &[0xffff_ffff, 0xffff_ffff]);
        assert_eq!(w[w.len() - 1], 0, "mmr_peaks is the LAST field: an empty list");
        // a state with peaks: each peak is 32 words (one per byte), after a length word
        let mut s = HeaderState::empty();
        s.height = 2;
        s.mmr_peaks = vec![[7; 32], [9; 32]];
        let w = risc0_zkvm::serde::to_vec(&s).unwrap();
        assert_eq!(w.len(), 72 + 64);
        assert_eq!(w[71], 2);
        assert_eq!(w[72], 7);
        assert_eq!(w[w.len() - 1], 9);
    }

    fn dsha(b: &[u8]) -> [u8; 32] { Sha256::digest(Sha256::digest(b)).into() }
    fn unhex(x: &str) -> Vec<u8> { (0..x.len()).step_by(2).map(|i| u8::from_str_radix(&x[i..i + 2], 16).unwrap()).collect() }
    /// Mainnet blocks 0..=200 (the fixture the guest's receipts were proved on).
    fn mainnet_hashes() -> Vec<[u8; 32]> {
        include_str!("../../../experimental/libzync-bench/headers/h0_200.txt").lines().filter(|l| !l.trim().is_empty())
            .map(|l| dsha(&unhex(l.split_whitespace().nth(1).unwrap()))).collect()
    }
    fn state_of(hashes: &[[u8; 32]]) -> HeaderState {
        let mut s = HeaderState::empty();
        s.height = hashes.len() as i64 - 1;
        s.mmr_peaks = mmr::peaks_of(hashes);
        s
    }

    /// Roots pinned from an independent implementation (a Python reference that builds each peak as a full tree, not
    /// by appending), over real mainnet block hashes: the guest's append, the verifier and the host agree with it.
    #[test]
    fn mmr_roots_match_the_independent_reference() {
        let hs = mainnet_hashes();
        assert_eq!(hs.len(), 201);
        for (n, peaks, root) in [
            (0, 0, "0000000000000000000000000000000000000000000000000000000000000000"),
            (1, 1, "14cbc70fe018b60dcc8885a59cd72c3f0ab99b26997ac29a48080b7ec77c8684"),
            (2, 1, "1148006e4876b7865b4a8ad64cacb392d145a1d4ff17ba8895c010a834e659ae"),
            (3, 2, "78844159c8e37dbcee93742db481ed94224fca6c9334547f3fb087959cf47cf0"),
            (7, 3, "7359b6912829c0a71949b8f4d249e539d7a550620cbd5f1cd0d64b46983c349a"),
            (8, 1, "cbe218829865cc81ebbc1359acfe7faff96215459460cea9527c00e9a9a19456"),
            (100, 3, "8bac5735cd5a685491fca158fcf613a90abafad6c8ec93b9ac1f9bbf43f7f80b"),
            (101, 4, "e6c523b11ea97d01240f42613f3809698b0ab757557c496032d8a3466b1a8b95"),
            (201, 4, "f0de733aec1e32c865566ca4e1e23fa40f56c599c2bdf2399dec502606bfe5e3"),
        ] {
            let s = state_of(&hs[..n]);
            assert_eq!(s.mmr_peaks.len(), peaks, "n = {n}");
            assert_eq!(s.mmr_root_hex(), root, "n = {n}");
        }
        // appending across a run boundary (a run chained to another) gives the same peaks as one run
        let mut p = mmr::peaks_of(&hs[..101]);
        for (i, h) in hs[101..].iter().enumerate() { mmr::append(&mut p, 101 + i as u64, h); }
        assert_eq!(p, mmr::peaks_of(&hs));
    }

    #[test]
    fn every_block_has_a_path_and_only_its_own() {
        let hs = mainnet_hashes();
        for n in [1usize, 2, 3, 5, 8, 13, 64, 100, 201] {
            let s = state_of(&hs[..n]);
            for i in 0..n {
                let p = mmr::path(&hs[..n], i as u64).unwrap();
                assert!(verify_inclusion(&s, i as i64, &hs[i], &p), "n {n} block {i}");
                // the same path for the neighbour's hash, or at the neighbour's height, fails
                let j = (i + 1) % n;
                if j != i {
                    assert!(!verify_inclusion(&s, i as i64, &hs[j], &p), "n {n}: block {j}'s hash at {i}");
                    assert!(!verify_inclusion(&s, j as i64, &hs[i], &p), "n {n}: block {i} at height {j}");
                }
            }
            assert!(mmr::path(&hs[..n], n as u64).is_none());
        }
    }

    #[test]
    fn inclusion_refusals() {
        let hs = mainnet_hashes();
        let s = state_of(&hs); // 201 leaves: peaks of 128, 64, 8, 1
        let (i, other) = (37usize, 150usize);
        let p = mmr::path(&hs, i as u64).unwrap();
        assert_eq!(p.len(), 7);
        assert!(verify_inclusion(&s, i as i64, &hs[i], &p));
        // wrong height (same peak, another position; another peak; out of range; negative)
        assert!(!verify_inclusion(&s, 36, &hs[i], &p));
        assert!(!verify_inclusion(&s, 130, &hs[i], &p));
        assert!(!verify_inclusion(&s, 201, &hs[i], &p));
        assert!(!verify_inclusion(&s, -1, &hs[i], &p));
        // wrong hash (one bit)
        let mut h = hs[i]; h[0] ^= 1;
        assert!(!verify_inclusion(&s, i as i64, &h, &p));
        // a path for another block
        let q = mmr::path(&hs, other as u64).unwrap();
        assert!(!verify_inclusion(&s, i as i64, &hs[i], &q));
        assert!(!verify_inclusion(&s, other as i64, &hs[i], &q));
        // truncated / extended / one sibling altered
        assert!(!verify_inclusion(&s, i as i64, &hs[i], &p[..6]));
        let mut e = p.clone(); e.push(s.mmr_peaks[1]);
        assert!(!verify_inclusion(&s, i as i64, &hs[i], &e));
        let mut m = p.clone(); m[3][31] ^= 0x80;
        assert!(!verify_inclusion(&s, i as i64, &hs[i], &m));
        // a node presented as a leaf: the level-1 node over blocks 36, 37 as a "block hash" with the rest of the path,
        // and padded back to the right length; and the raw leaf hash presented as a block hash
        let n1 = mmr::node(&mmr::leaf(&hs[36]), &mmr::leaf(&hs[37]));
        assert!(!verify_inclusion(&s, 18, &n1, &p[1..]));
        let mut pad = vec![[0u8; 32]]; pad.extend_from_slice(&p[1..]);
        assert!(!verify_inclusion(&s, 36, &n1, &pad));
        assert!(!verify_inclusion(&s, i as i64, &mmr::leaf(&hs[i]), &p));
        // a path that is valid in ANOTHER MMR (all 256 leaves would make 37's peak the 256-tree): the state's peaks
        // refuse it, as they refuse a path to a peak of a shorter chain
        let longer: Vec<[u8; 32]> = (0..256).map(|k| if k < hs.len() { hs[k] } else { [k as u8; 32] }).collect();
        let pl = mmr::path(&longer, i as u64).unwrap();
        assert!(verify_inclusion(&state_of(&longer), i as i64, &hs[i], &pl));
        assert!(!verify_inclusion(&s, i as i64, &hs[i], &pl));
        let ps = mmr::path(&hs[..100], i as u64).unwrap(); // in 100 leaves, 37 is in the 64-peak (6 siblings)
        assert!(!verify_inclusion(&s, i as i64, &hs[i], &ps));
        // a state whose peak list does not fit its height is refused outright
        let mut bad = s.clone(); bad.mmr_peaks.pop();
        assert!(!verify_inclusion(&bad, 200, &hs[200], &[]));
        assert!(verify_inclusion(&s, 200, &hs[200], &[]), "block 200 is a one-leaf peak: an empty path");
    }

    fn fixture(name: &str) -> serde_json::Value {
        let t = match name {
            "170_0" => include_str!("../../../experimental/libzync-bench/headers/tx/tx_170_0.json"),
            "170_1" => include_str!("../../../experimental/libzync-bench/headers/tx/tx_170_1.json"),
            "100003_0" => include_str!("../../../experimental/libzync-bench/headers/tx/tx_100003_0.json"),
            "seg" => include_str!("../../../experimental/libzync-bench/headers/tx/tx_seg.json"),
            _ => unreachable!(),
        };
        serde_json::from_str(t).unwrap()
    }
    fn block_100002() -> Vec<serde_json::Value> {
        include_str!("../../../experimental/libzync-bench/headers/tx/tx_100002.jsonl").lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }
    fn bhash(v: &serde_json::Value) -> [u8; 32] { let mut b = hex32(v["hash"].as_str().unwrap()).unwrap(); b.reverse(); b }
    /// The fixture's own transaction asked about (fixtures are a trusted node's answers; the asked txid is its "txid")
    fn tx_of(v: &serde_json::Value) -> Result<TxInclusion, String> { TxInclusion::from_json(v["txid"].as_str().unwrap_or(""), v) }
    fn check(v: &serde_json::Value) -> Result<String, String> { verify_tx_in_header(&bhash(v), &tx_of(v)?) }

    /// Real transactions (fixtures from a mainnet node, experimental/libzync-bench/headers/tx/): every one of block
    /// 100,002's 9 (the odd-level rule at three levels, so several branches carry a duplicated node), both of block 170
    /// (the coinbase itself, and Satoshi to Hal Finney), and a segwit transaction of block 800,000 (3,721 transactions,
    /// 12 levels), given with its witness and without it.
    #[test]
    fn real_transactions_are_accepted() {
        let b = block_100002();
        assert_eq!(b.len(), 9);
        for (k, v) in b.iter().enumerate() {
            assert_eq!(v["pos"], k as u64);
            assert_eq!(check(v), Ok(format!("at position {k} of 4 levels, the tree's depth by the block's coinbase")), "tx {k}");
        }
        // tx 8 is paired with itself at level 0: its first sibling is itself
        assert_eq!(b[8]["branch"][0], b[8]["txid"]);
        check(&fixture("170_0")).unwrap();
        check(&fixture("170_1")).unwrap();
        let seg = fixture("seg");
        assert_eq!(check(&seg).unwrap(), "at position 1 of 12 levels, the tree's depth by the block's coinbase");
        let mut tx = tx_of(&seg).unwrap();
        let p = parse_tx(&tx.raw).unwrap();
        assert!(p.stripped.len() < tx.raw.len(), "the fixture carries a witness");
        tx.raw = p.stripped;
        assert!(verify_tx_in_header(&bhash(&seg), &tx).is_ok(), "the same transaction without its witness");
    }

    #[test]
    fn transaction_refusals() {
        let b = block_100002();
        let v = &b[5];
        let base = tx_of(v).unwrap();
        let bh = bhash(v);
        let refuse = |t: &TxInclusion, want: &str| {
            let e = verify_tx_in_header(&bh, t).unwrap_err();
            assert!(e.contains(want), "{e:?} lacks {want:?}");
        };
        // the wrong position; a position that does not fit the branch
        let mut t = base.clone(); t.pos = 4; refuse(&t, "does not lead to the header's merkle root");
        let mut t = base.clone(); t.pos += 16; refuse(&t, "does not lead to the header's merkle root");
        // raw bytes that are not the txid's (another transaction of the block), and a txid that is not the raw's
        let mut t = base.clone(); t.raw = tx_of(&b[6]).unwrap().raw; refuse(&t, "not the transaction asked about");
        let mut t = base.clone(); t.txid[0] ^= 1; refuse(&t, "not the transaction asked about");
        // a raw transaction with a byte appended, or cut short
        let mut t = base.clone(); t.raw.push(0); refuse(&t, "does not parse");
        let mut t = base.clone(); t.raw.pop(); refuse(&t, "does not parse");
        // another block's header
        let mut t = base.clone(); t.header = tx_of(&fixture("100003_0")).unwrap().header; refuse(&t, "does not hash to the block");

        // THE 64-BYTE ATTACK, shorter: block 100,002's level-1 node over txs 0 and 1 is dsha(txid0 || txid1). Present
        // those 64 bytes as a "transaction" at position 0 with the rest of tx 0's branch: the txid check alone would pass
        // (the bytes DO hash to the node), and the branch DOES reach the merkle root.
        let t0 = tx_of(&b[0]).unwrap();
        let t1 = tx_of(&b[1]).unwrap();
        let mut inner = t0.txid.to_vec(); inner.extend_from_slice(&t1.txid);
        let node = dsha(&inner);
        assert_eq!(merkle_climb(&node, 0, &t0.branch[1..]), Some(base.header[36..68].try_into().unwrap()), "the forgery reaches the root");
        let mut t = t0.clone(); t.raw = inner.clone(); t.txid = node; t.branch = t0.branch[1..].to_vec();
        refuse(&t, "64 bytes");
        // the depth rule alone refuses it too: a real transaction (tx 0's raw) pretends to be at depth 3
        let mut t = t0.clone(); t.branch = t0.branch[1..].to_vec(); refuse(&t, "3 levels deep");
        let mut t = base.clone(); t.branch.truncate(3); refuse(&t, "3 levels deep, the coinbase's (the tree's depth) 4");
        let mut t = base.clone(); t.branch.push(t.branch[0]); refuse(&t, "5 levels deep");
        // ...and a coinbase "branch" shortened to match needs the coinbase to be the 64-byte inner node: refused
        let mut t = base.clone(); t.branch.truncate(3); t.coinbase = inner.clone(); t.coinbase_branch = t0.branch[1..].to_vec();
        refuse(&t, "the coinbase is exactly 64 bytes");
        // a coinbase from another block (100,003's, with its own branch), and a non-coinbase given as the coinbase
        let other = tx_of(&fixture("100003_0")).unwrap();
        let mut t = base.clone(); t.coinbase = other.coinbase.clone(); t.coinbase_branch = other.coinbase_branch.clone();
        refuse(&t, "not this block's coinbase");
        let mut t = base.clone(); t.coinbase = t1.raw.clone(); refuse(&t, "not a coinbase");
        // the duplicated node claimed on the right-child side (tx 8 at position 9)
        let mut t = tx_of(&b[8]).unwrap(); t.pos = 9; refuse(&t, "does not lead to the header's merkle root");
        // text that is not a transaction answer
        assert!(TxInclusion::from_json("00", &serde_json::json!({})).is_err());
        let mut j = v.clone(); j["raw"] = serde_json::json!("0g"); assert!(tx_of(&j).is_err());
        assert!(Inclusion::parse(1, "00", &[], None).is_err());
        assert!(Inclusion::parse(1, v["hash"].as_str().unwrap(), &["zz"], None).is_err());
    }

    const HDR_0_100: &[u8] = include_bytes!("../../../experimental/libzync-bench/headers/hdr_0_100.rcpt");
    const HDR_101_200: &[u8] = include_bytes!("../../../experimental/libzync-bench/headers/hdr_101_200.rcpt");

    /// #758 (fuzz round 11-17) carried here when it met #724's move: the header-chain reader uses the core's strict
    /// receipt decoder and journal guard, so it refuses the same inputs the core does, as a FAIL, never a panic.
    #[test]
    fn strict_receipt_and_journal_decoding() {
        let mut trailing = HDR_0_100.to_vec();
        trailing.push(0);
        let mut r: risc0_zkvm::Receipt = bincode::deserialize(HDR_0_100).unwrap();
        r.journal.bytes.push(0);
        let journal = bincode::serialize(&r).unwrap();
        let mut m: risc0_zkvm::Receipt = bincode::deserialize(HDR_0_100).unwrap();
        let risc0_zkvm::InnerReceipt::Succinct(s) = &mut m.inner else { panic!("hdr_0_100 is a succinct receipt") };
        let mut words: [u32; 8] = s.control_inclusion_proof.digests[0].as_words().try_into().unwrap();
        words[0] = 0xffff_ffff;
        s.control_inclusion_proof.digests[0] = words.into();
        let ctrl = bincode::serialize(&m).unwrap();
        for (b, want) in [(&ctrl, "malformed receipt"), (&trailing, "not a receipt")] {
            let rep = verify_headers(&[b.as_slice()], HEADERS_IMAGE_ID, now(), None, None);
            assert_eq!(rep.verdict, Verdict::Fail);
            assert!(rep.checks.iter().any(|(ok, w)| !ok && w.contains(want)), "{want}: {:?}", rep.checks);
        }
        // A journal that is not whole words: this reader never decodes an unverified journal, so it is refused at the
        // verify step (the claim no longer matches), before the guard — still a FAIL, never a panic.
        let rep = verify_headers(&[journal.as_slice()], HEADERS_IMAGE_ID, now(), None, None);
        assert_eq!(rep.verdict, Verdict::Fail);
    }
    const T170: &str = "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16"; // block 170, Satoshi to Hal Finney
    fn now() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() }
    fn lines(t: &str) -> Vec<&str> { t.lines().map(str::trim).filter(|l| !l.is_empty()).collect() }
    fn include_170(asked: &str, answer: &serde_json::Value) -> Result<Inclusion, String> {
        let p = lines(include_str!("../../../experimental/libzync-bench/headers/mmr_path_170_tip200.txt"));
        Inclusion::parse(170, answer["hash"].as_str().unwrap(), &p, Some(TxInclusion::from_json(asked, answer)?))
    }

    /// Review finding 1: a lying path service answers a question about T with a VALID proof for another confirmed
    /// transaction T' (here block 170's coinbase). Before, the txid came from the answer, so that proof passed as
    /// "a transaction is included"; now the txid is the caller's and the raw bytes must hash to it.
    #[test]
    fn an_answer_about_another_transaction_is_refused() {
        let other = fixture("170_0"); // block 170's coinbase: a real, valid answer, for another transaction
        assert_ne!(other["txid"].as_str().unwrap(), T170);
        let e = verify_tx_in_header(&bhash(&other), &TxInclusion::from_json(T170, &other).unwrap()).unwrap_err();
        assert!(e.contains("not the transaction asked about"), "{e}");
        // ...also when the answer's own "txid" field claims the asked one: that field is not read
        let mut lie = other.clone(); lie["txid"] = serde_json::json!(T170);
        assert!(verify_tx_in_header(&bhash(&lie), &TxInclusion::from_json(T170, &lie).unwrap()).is_err());
        // the right answer passes with its "txid" field gone or wrong: only the asked txid and the raw bytes count
        let mut right = fixture("170_1"); right["txid"] = serde_json::json!("not read");
        assert!(verify_tx_in_header(&bhash(&right), &TxInclusion::from_json(T170, &right).unwrap()).is_ok());
        right.as_object_mut().unwrap().remove("txid");
        assert!(verify_tx_in_header(&bhash(&right), &TxInclusion::from_json(T170, &right).unwrap()).is_ok());

        // through verify_headers, on the real chained receipts: the lie is a FAIL with nothing reported as included;
        // the right answer reports the block and the ASKED txid (the verdict is DEMO: 201 headers are below the
        // minimum work, and `included` must be read with the verdict)
        let rs = [HDR_0_100, HDR_101_200];
        let r = verify_headers(&rs, HEADERS_IMAGE_ID, now(), None, Some(&include_170(T170, &other).unwrap()));
        assert_eq!(r.verdict, Verdict::Fail);
        assert!(r.included.is_none());
        let r = verify_headers(&rs, HEADERS_IMAGE_ID, now(), None, Some(&include_170(T170, &fixture("170_1")).unwrap()));
        assert_eq!(r.verdict, Verdict::Demo);
        let inc = r.included.expect("included");
        assert_eq!((inc.height, inc.hash_hex().as_str(), inc.txid_hex().as_deref()),
                   (170, "00000000d1145790a8694403d4063f323d499e655c83426834d4ce2f8dd4a2ee", Some(T170)));
        // nothing included when nothing was asked, nor when another check failed (a wrong expected tip)
        assert!(verify_headers(&rs, HEADERS_IMAGE_ID, now(), None, None).included.is_none());
        let r = verify_headers(&rs, HEADERS_IMAGE_ID, now(), Some((199, "00")), Some(&include_170(T170, &fixture("170_1")).unwrap()));
        assert_eq!((r.verdict, r.included.is_none()), (Verdict::Fail, true));
    }

    /// Review finding 7: hex with a sign. u8::from_str_radix takes "+f", so "+f" passed as a byte.
    #[test]
    fn hex_refuses_a_sign() {
        let h = "+f".to_string() + &"0".repeat(62);
        assert_eq!(h.len(), 64);
        assert!(hex32(&h).is_none());
        assert!(hex32(&"0f".repeat(32)).is_some());
        assert!(super::unhex("+f").is_none());
        assert!(super::unhex("-f").is_none());
        assert_eq!(super::unhex("0fAB"), Some(vec![0x0f, 0xab]));
        assert!(Inclusion::parse(1, &h, &[], None).is_err());
        assert!(TxInclusion::from_json(&h, &fixture("170_1")).is_err());
        let mut j = fixture("170_1"); j["raw"] = serde_json::json!(format!("+{}", &j["raw"].as_str().unwrap()[1..]));
        assert!(TxInclusion::from_json(T170, &j).is_err());
    }

    /// Review finding 7: heights at i64::MAX. leaves() was (height + 1) in i64, and a run's "from + 1" was computed
    /// from the journal of a receipt that had FAILED to verify; both overflow (a panic in a debug build).
    #[test]
    fn heights_at_i64_max_do_not_overflow() {
        let mut s = HeaderState::empty();
        assert_eq!(s.leaves(), 0);
        s.height = i64::MAX;
        assert_eq!(s.leaves(), 1u64 << 63);
        // a real receipt with its journal's height set to i64::MAX: it no longer verifies, and the verifier must stop
        // there rather than read the journal (twice, so the second run's "from" would be i64::MAX)
        let mut r: risc0_zkvm::Receipt = bincode::deserialize(HDR_0_100).unwrap();
        let (din, mut out): ([u8; 32], HeaderState) = r.journal.decode().unwrap();
        out.height = i64::MAX;
        r.journal.bytes = risc0_zkvm::serde::to_vec(&(din, out)).unwrap().iter().flat_map(|w| w.to_le_bytes()).collect();
        let forged = bincode::serialize(&r).unwrap();
        let rep = verify_headers(&[&forged, &forged], HEADERS_IMAGE_ID, now(), None, None);
        assert_eq!(rep.verdict, Verdict::Fail);
        assert!(rep.state.is_none());
        assert!(!rep.checks.last().unwrap().0, "the last check is the failed verify: {:?}", rep.checks.last());
    }

    /// Review (11-15): the run-chaining checks did not end the check, so N copies of one receipt were all verified
    /// (N STARK verifications for one failed link). Now the first broken link returns, and the runs are capped.
    #[test]
    fn a_broken_link_stops_the_check_and_runs_are_capped() {
        let verified = |r: &HeadersReport| r.checks.iter().filter(|c| c.0 && c.1.contains("verifies against the headers image")).count();
        let rep = verify_headers(&[HDR_0_100, HDR_0_100, HDR_0_100, HDR_0_100], HEADERS_IMAGE_ID, now(), None, None);
        assert_eq!(rep.verdict, Verdict::Fail);
        assert_eq!(verified(&rep), 2, "copy 2's link fails, copies 3 and 4 are never verified: {:?}", rep.checks);
        assert!(!rep.checks.last().unwrap().0 && rep.checks.last().unwrap().1.contains("run 2 starts where run 1 ended"));
        assert!(rep.state.is_none());
        // the first run not from genesis: one verification, then the end
        let rep = verify_headers(&[HDR_101_200, HDR_0_100], HEADERS_IMAGE_ID, now(), None, None);
        assert_eq!((rep.verdict, verified(&rep)), (Verdict::Fail, 1));
        // more than MAX_RUNS: refused before any verification
        let many = vec![HDR_0_100; MAX_RUNS + 1];
        let rep = verify_headers(&many, HEADERS_IMAGE_ID, now(), None, None);
        assert_eq!((rep.verdict, verified(&rep)), (Verdict::Fail, 0));
        assert!(rep.checks.iter().any(|c| !c.0 && c.1.contains("at most 4096 chained runs")));
        // the good chain still verifies both runs
        let rep = verify_headers(&[HDR_0_100, HDR_101_200], HEADERS_IMAGE_ID, now(), None, None);
        assert_eq!((rep.verdict, verified(&rep)), (Verdict::Demo, 2));
    }

    /// Review 11-18: the include JSON and the /tx answer take only their documented keys, each once. Before, every
    /// other key was ignored: `txid` and `tx` both spelt wrong gave a block-only inclusion, and a key given twice kept
    /// its last value (wasm/test-headers.mjs has the same cases through the module).
    #[test]
    fn include_and_tx_json_take_only_their_documented_keys() {
        let tx = include_str!("../../../experimental/libzync-bench/headers/tx/tx_170_1.json").trim();
        let path: Vec<String> = include_str!("../../../experimental/libzync-bench/headers/mmr_path_170_tip200.txt")
            .lines().filter(|l| !l.trim().is_empty()).map(|l| format!("{:?}", l.trim())).collect();
        let (h, p) = ("\"00000000d1145790a8694403d4063f323d499e655c83426834d4ce2f8dd4a2ee\"", path.join(","));
        let t = "\"f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16\"";
        let inc = |rest: &str| format!(r#"{{"height": 170, "hash": {h}, "path": [{p}]{rest}}}"#);
        // the documented shapes: a block alone, a block and a transaction (the /tx answer with its other keys), null as absent
        assert!(parse_include_json(inc("").as_bytes()).unwrap().tx.is_none());
        assert!(parse_include_json(inc(&format!(r#", "txid": {t}, "tx": {tx}"#)).as_bytes()).unwrap().tx.is_some());
        let bad_tx = tx.replacen("\"pos\"", "\"posn\": 1, \"pos\"", 1);
        let twice_tx = tx.replacen("\"pos\"", "\"raw\": \"00\", \"pos\"", 1);
        for (j, why) in [
            (inc(&format!(r#", "tixd": {t}, "txx": {tx}"#)), "unknown field `tixd`"),
            (inc(&format!(r#", "txid": {t}, "tx": {tx}, "extra": 1"#)), "unknown field `extra`"),
            (inc(&format!(r#", "hash": {h}"#)), "duplicate field `hash`"),
            (inc(&format!(r#", "txid": {t}, "tx": {bad_tx}"#)), "unknown field `posn`"),
            (inc(&format!(r#", "txid": {t}, "tx": {twice_tx}"#)), "duplicate field `raw`"),
            (inc(&format!(r#", "txid": {t}, "tx": [1]"#)), "tx is not a JSON object"),
            (inc(&format!(r#", "txid": {t}"#)), "txid needs tx"),
            // an explicit null is not "absent": refused (review 11-18 round 2; it gave a block-only inclusion)
            (inc(r#", "txid": null, "tx": null"#), "txid is null"),
            (inc(r#", "txid": null"#), "txid is null"),
            (inc(r#", "tx": null"#), "tx is null"),
            (inc(&format!(r#", "txid": {t}, "tx": null"#)), "tx is null"),
            (inc(&format!(r#", "tx": {tx}"#)), "tx needs txid"),
            (format!("[170, {h}, [{p}]]"), "include is not a JSON object"),
            (r#"{"hash": "00", "path": []}"#.to_string(), "missing field `height`"),
        ] {
            let e = parse_include_json(j.as_bytes()).unwrap_err();
            assert!(e.contains(why), "{why}: {e}");
        }
        // the CLI's --tx file and from_json: the same answer rules
        assert!(TxInclusion::from_json_bytes(&t[1..65], tx.as_bytes()).is_ok());
        assert!(TxInclusion::from_json_bytes(&t[1..65], bad_tx.as_bytes()).unwrap_err().contains("unknown field `posn`"));
        assert!(TxInclusion::from_json_bytes(&t[1..65], twice_tx.as_bytes()).unwrap_err().contains("duplicate field `raw`"));
        let v: serde_json::Value = serde_json::from_str(&bad_tx).unwrap();
        assert!(TxInclusion::from_json(&t[1..65], &v).unwrap_err().contains("unknown field `posn`"));
    }
}

#[cfg(feature = "engine")]
fn engine_run(file: &[u8], key: &swiftsync_verify::EngineKey, id: &[u8; 32]) -> Result<risc0_zkvm::Journal, String> {
    if !swiftsync_verify::is_engine_proof(file) {
        return Err("not an engine proof file".into());
    }
    swiftsync_verify::engine_proof_journal(file, key, id)
}

#[cfg(not(feature = "engine"))]
fn engine_run(_: &[u8], _: &swiftsync_verify::EngineKey, _: &[u8; 32]) -> Result<risc0_zkvm::Journal, String> {
    Err("this build has no engine support (feature `engine`)".into())
}
