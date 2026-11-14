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
use crate::{receipt_kind, work_at_least, Verdict, IMAGES, MAX_FUTURE_BLOCK_TIME};
use risc0_zkvm::sha::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// The header-chain image id this verifier trusts by default: the `zyncapp-headers` line of reproduce/LIBZYNC_IDS (the
/// reproducible build; reproduce/pin-libzync-ids.py sets it). `--image <hex>` replaces it for a reader who rebuilt it.
pub const HEADERS_IMAGE_ID: &str = "2db34e12cc993dd1edb003f00f85c93d02e0e61208589b57fc5c969d39b3e632";

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
    /// The MMR's leaf count: blocks 0..=height.
    pub fn leaves(&self) -> u64 { (self.height + 1).max(0) as u64 }
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
}

impl HeadersReport {
    fn check(&mut self, ok: bool, what: impl Into<String>) -> bool { self.checks.push((ok, what.into())); ok }
}

/// Verify a chain of header-chain receipts (the first from genesis, each next one starting where the one before ended)
/// against the headers image id `image_hex`. `now` is the verifier's clock (unix seconds) for time-too-new;
/// `expect` = (height, hash as explorers print it) from a source the verifier trusts, for chain identity.
///
/// `include`: also check that a block (and optionally a transaction in it) is on the proven chain, against the last
/// run's MMR peaks; a failed inclusion is a failed check (verdict `Fail`).
pub fn verify_headers(receipts: &[&[u8]], image_hex: &str, now: u64, expect: Option<(i64, &str)>,
                      include: Option<&Inclusion>) -> HeadersReport {
    let mut rep = HeadersReport { checks: Vec::new(), state: None, verdict: Verdict::Fail };
    let Some(id) = hex32(image_hex) else {
        rep.check(false, format!("the headers image id is 32 bytes of hex ({image_hex})"));
        return rep;
    };
    let image = Digest::from_bytes(id);
    if !rep.check(!receipts.is_empty(), "at least one receipt") { return rep; }
    let mut want_in = HeaderState::empty().digest();
    let mut last: Option<HeaderState> = None;
    for (i, bytes) in receipts.iter().enumerate() {
        let n = i + 1;
        if let Err(e) = receipt_kind(bytes) {
            rep.check(false, format!("receipt {n} is a single succinct or Groth16 proof ({e})"));
            return rep;
        }
        let r: risc0_zkvm::Receipt = match bincode::deserialize(bytes) {
            Ok(r) => r,
            Err(e) => { rep.check(false, format!("receipt {n} decodes ({e})")); return rep; }
        };
        rep.check(!matches!(r.inner, risc0_zkvm::InnerReceipt::Fake(_)), format!("receipt {n} is a real proof"));
        let v = r.verify(image);
        rep.check(v.is_ok(), format!("receipt {n} verifies against the headers image {image_hex}{}",
                                     v.err().map(|e| format!(": {e}")).unwrap_or_default()));
        let (din, out): ([u8; 32], HeaderState) = match r.journal.decode() {
            Ok(j) => j,
            Err(e) => { rep.check(false, format!("receipt {n}'s journal is (input digest, header state) ({e})")); return rep; }
        };
        let from = last.as_ref().map_or(-1, |s| s.height);
        rep.check(din == want_in, if i == 0 {
            "the first run starts from genesis (its input is the empty state)".to_string()
        } else {
            format!("run {n} starts where run {i} ended (height {from})")
        });
        rep.check(out.height > from, format!("run {n} adds headers ({}..{})", from + 1, out.height));
        want_in = out.digest();
        last = Some(out);
    }
    let s = last.expect("at least one receipt decoded");
    rep.check(s.max_time as u64 <= now.saturating_add(MAX_FUTURE_BLOCK_TIME),
              format!("time-too-new: largest header time {} <= this clock {} + {} s", s.max_time, now, MAX_FUTURE_BLOCK_TIME));
    if let Some((h, hash)) = expect {
        rep.check(s.height == h && s.hash_hex() == hash.to_ascii_lowercase(),
                  format!("chain identity: the tip is block {h} {hash} (from a source this verifier trusts)"));
    }
    if let Some(inc) = include {
        let disp = |b: &[u8; 32]| -> String { b.iter().rev().map(|x| format!("{x:02x}")).collect() };
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
    let mainnet = IMAGES.iter().find(|i| i.network == "mainnet").expect("IMAGES holds mainnet");
    let enough = work_at_least(&s.work, mainnet.min_work_hex);
    rep.checks.push((true, format!("chainwork {} {} Core's minimum {}", s.work_hex(),
                                   if enough { ">=" } else { "<" }, mainnet.min_work_hex)));
    rep.verdict = if failed { Verdict::Fail } else if enough { Verdict::Ok } else { Verdict::Demo };
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
/// witness), the txid it must hash to (internal order), its position and merkle branch; and the block's raw COINBASE
/// transaction with its branch (position 0). Branches are internal order, lowest level first.
#[derive(Clone, Debug)]
pub struct TxInclusion {
    pub header: [u8; 80],
    pub txid: [u8; 32],
    pub raw: Vec<u8>,
    pub pos: u32,
    pub branch: Vec<[u8; 32]>,
    pub coinbase: Vec<u8>,
    pub coinbase_branch: Vec<[u8; 32]>,
}

impl TxInclusion {
    /// From the JSON the path service's /tx answers (and `swiftsync-verify headers --tx <file>` reads):
    /// `{"txid", "pos", "header", "raw", "branch", "coinbase", "coinbase_branch", ...}`. txid and branch entries are
    /// reversed hex (as explorers and Electrum's blockchain.transaction.get_merkle print them); header, raw and
    /// coinbase are plain hex of the bytes (getblockheader <hash> false, getrawtransaction <txid>). Other keys are ignored.
    pub fn from_json(v: &serde_json::Value) -> Result<TxInclusion, String> {
        let s = |k: &str| v[k].as_str().ok_or(format!("tx: {k} is not a string"));
        let list = |k: &str| -> Result<Vec<[u8; 32]>, String> {
            v[k].as_array().ok_or(format!("tx: {k} is not a list"))?.iter()
                .map(|x| x.as_str().and_then(hex32).map(|mut b| { b.reverse(); b }).ok_or(format!("tx: {k} holds a non-hash")))
                .collect()
        };
        let header: [u8; 80] = unhex(s("header")?).filter(|h| h.len() == 80).and_then(|h| h.try_into().ok())
            .ok_or("tx: header is not 80 bytes of hex")?;
        let mut txid = hex32(s("txid")?).ok_or("tx: txid is not 32 bytes of hex")?;
        txid.reverse();
        let pos = v["pos"].as_u64().filter(|p| *p <= u32::MAX as u64).ok_or("tx: pos is not a position")? as u32;
        Ok(TxInclusion { header, txid, raw: unhex(s("raw")?).ok_or("tx: raw is not hex")?, pos, branch: list("branch")?,
                         coinbase: unhex(s("coinbase")?).ok_or("tx: coinbase is not hex")?, coinbase_branch: list("coinbase_branch")? })
    }
}

fn unhex(x: &str) -> Option<Vec<u8>> {
    if x.len() % 2 != 0 { return None; }
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
        fn var(&mut self) -> Result<u64, String> {
            let n = match self.take(1)?[0] {
                0xfd => u16::from_le_bytes(self.take(2)?.try_into().unwrap()) as u64,
                0xfe => u32::from_le_bytes(self.take(4)?.try_into().unwrap()) as u64,
                0xff => u64::from_le_bytes(self.take(8)?.try_into().unwrap()),
                x => x as u64,
            };
            if n > self.b.len() as u64 { return Err("a count larger than the transaction".into()); }
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
        let n = r.var()?; r.take(n as usize)?; r.take(4)?;
    }
    let outputs = r.var()?;
    if outputs == 0 { return Err("a transaction with no outputs".into()); }
    for _ in 0..outputs { r.take(8)?; let n = r.var()?; r.take(n as usize)?; }
    let body_end = r.i;
    if segwit {
        for _ in 0..inputs { for _ in 0..r.var()? { let n = r.var()?; r.take(n as usize)?; } }
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

/// Is `tx` a transaction of the block `block_hash`? `Ok` with what was shown, or `Err` with why not.
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
    if dsha(&t.stripped) != tx.txid { return Err("the raw transaction does not hash to the txid".into()); }
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
    if merkle_climb(&tx.txid, tx.pos, &tx.branch) != Some(root) {
        return Err(format!("the merkle branch from position {} does not lead to the header's merkle root", tx.pos));
    }
    Ok(format!("at position {} of {} levels, the tree's depth by the block's coinbase", tx.pos, tx.branch.len()))
}

fn hex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 { return None; }
    let mut o = [0u8; 32];
    for i in 0..32 { o[i] = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?; }
    Some(o)
}

#[cfg(test)]
mod tests {
    use super::*;
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
        include_str!("../../experimental/libzync-bench/headers/h0_200.txt").lines().filter(|l| !l.trim().is_empty())
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
            "170_0" => include_str!("../../experimental/libzync-bench/headers/tx/tx_170_0.json"),
            "170_1" => include_str!("../../experimental/libzync-bench/headers/tx/tx_170_1.json"),
            "100003_0" => include_str!("../../experimental/libzync-bench/headers/tx/tx_100003_0.json"),
            "seg" => include_str!("../../experimental/libzync-bench/headers/tx/tx_seg.json"),
            _ => unreachable!(),
        };
        serde_json::from_str(t).unwrap()
    }
    fn block_100002() -> Vec<serde_json::Value> {
        include_str!("../../experimental/libzync-bench/headers/tx/tx_100002.jsonl").lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }
    fn bhash(v: &serde_json::Value) -> [u8; 32] { let mut b = hex32(v["hash"].as_str().unwrap()).unwrap(); b.reverse(); b }
    fn check(v: &serde_json::Value) -> Result<String, String> { verify_tx_in_header(&bhash(v), &TxInclusion::from_json(v)?) }

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
        let mut tx = TxInclusion::from_json(&seg).unwrap();
        let p = parse_tx(&tx.raw).unwrap();
        assert!(p.stripped.len() < tx.raw.len(), "the fixture carries a witness");
        tx.raw = p.stripped;
        assert!(verify_tx_in_header(&bhash(&seg), &tx).is_ok(), "the same transaction without its witness");
    }

    #[test]
    fn transaction_refusals() {
        let b = block_100002();
        let v = &b[5];
        let base = TxInclusion::from_json(v).unwrap();
        let bh = bhash(v);
        let refuse = |t: &TxInclusion, want: &str| {
            let e = verify_tx_in_header(&bh, t).unwrap_err();
            assert!(e.contains(want), "{e:?} lacks {want:?}");
        };
        // the wrong position; a position that does not fit the branch
        let mut t = base.clone(); t.pos = 4; refuse(&t, "does not lead to the header's merkle root");
        let mut t = base.clone(); t.pos += 16; refuse(&t, "does not lead to the header's merkle root");
        // raw bytes that are not the txid's (another transaction of the block), and a txid that is not the raw's
        let mut t = base.clone(); t.raw = TxInclusion::from_json(&b[6]).unwrap().raw; refuse(&t, "does not hash to the txid");
        let mut t = base.clone(); t.txid[0] ^= 1; refuse(&t, "does not hash to the txid");
        // a raw transaction with a byte appended, or cut short
        let mut t = base.clone(); t.raw.push(0); refuse(&t, "does not parse");
        let mut t = base.clone(); t.raw.pop(); refuse(&t, "does not parse");
        // another block's header
        let mut t = base.clone(); t.header = TxInclusion::from_json(&fixture("100003_0")).unwrap().header; refuse(&t, "does not hash to the block");

        // THE 64-BYTE ATTACK, shorter: block 100,002's level-1 node over txs 0 and 1 is dsha(txid0 || txid1). Present
        // those 64 bytes as a "transaction" at position 0 with the rest of tx 0's branch: the txid check alone would pass
        // (the bytes DO hash to the node), and the branch DOES reach the merkle root.
        let t0 = TxInclusion::from_json(&b[0]).unwrap();
        let t1 = TxInclusion::from_json(&b[1]).unwrap();
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
        let other = TxInclusion::from_json(&fixture("100003_0")).unwrap();
        let mut t = base.clone(); t.coinbase = other.coinbase.clone(); t.coinbase_branch = other.coinbase_branch.clone();
        refuse(&t, "not this block's coinbase");
        let mut t = base.clone(); t.coinbase = t1.raw.clone(); refuse(&t, "not a coinbase");
        // the duplicated node claimed on the right-child side (tx 8 at position 9)
        let mut t = TxInclusion::from_json(&b[8]).unwrap(); t.pos = 9; refuse(&t, "does not lead to the header's merkle root");
        // text that is not a transaction answer
        assert!(TxInclusion::from_json(&serde_json::json!({"txid": "00"})).is_err());
        let mut j = v.clone(); j["raw"] = serde_json::json!("0g"); assert!(TxInclusion::from_json(&j).is_err());
        assert!(Inclusion::parse(1, "00", &[], None).is_err());
        assert!(Inclusion::parse(1, v["hash"].as_str().unwrap(), &["zz"], None).is_err());
    }
}
