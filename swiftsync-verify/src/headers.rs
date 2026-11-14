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
//! with the block's 80-byte header and a transaction merkle branch, "is transaction T in a block on this chain"
//! ([`verify_tx_in_header`]). The MMR root ([`HeaderState::mmr_root`]) is derived from the committed peaks, not
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
            rep.check(verify_tx_in_header(&tx.header, &inc.hash, &tx.txid, tx.pos, &tx.branch),
                      format!("transaction {} is at position {} of block {}: the header hashes to the block and the merkle \
                               branch ({} hashes) leads to its merkle root", disp(&tx.txid), tx.pos, inc.height, tx.branch.len()));
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

/// A transaction in the included block: the block's 80-byte header, the txid (internal order), its position in the
/// block, and the merkle branch from it to the header's merkle root (internal order, lowest level first).
#[derive(Clone, Debug)]
pub struct TxInclusion {
    pub header: [u8; 80],
    pub txid: [u8; 32],
    pub pos: u32,
    pub branch: Vec<[u8; 32]>,
}

impl Inclusion {
    /// From text: `hash` as explorers print it (reversed hex); `path`: 64-hex MMR hashes as headers-mmr-path prints
    /// them (raw byte order, not reversed). `tx` = (txid as explorers print it, position, the 80-byte header as hex
    /// (getblockheader <hash> false), the branch as Electrum's blockchain.transaction.get_merkle prints it: reversed hex).
    pub fn parse(height: i64, hash: &str, path: &[&str], tx: Option<(&str, u32, &str, &[&str])>) -> Result<Inclusion, String> {
        let rev = |x: &str, what: &str| hex32(x).map(|mut b| { b.reverse(); b }).ok_or(format!("{what}: not 32 bytes of hex ({x})"));
        let hash = rev(hash, "block hash")?;
        let path = path.iter().map(|x| hex32(x).ok_or(format!("MMR path: not 32 bytes of hex ({x})"))).collect::<Result<Vec<_>, _>>()?;
        let tx = match tx {
            None => None,
            Some((txid, pos, header, branch)) => {
                if header.len() != 160 { return Err(format!("header: not 80 bytes of hex ({} digits)", header.len())); }
                let mut h = [0u8; 80];
                for i in 0..80 { h[i] = u8::from_str_radix(header.get(2 * i..2 * i + 2).unwrap_or("zz"), 16).map_err(|_| "header: not hex")?; }
                let branch = branch.iter().map(|x| rev(x, "merkle branch")).collect::<Result<Vec<_>, _>>()?;
                Some(TxInclusion { header: h, txid: rev(txid, "txid")?, pos, branch })
            }
        };
        Ok(Inclusion { height, hash, path, tx })
    }
}

/// Is `block_hash` (internal order) the block at `height` of the chain whose header state is `state`, by this MMR
/// `path`? The peak, and the path's length, follow from `height` and the leaf count (state.height + 1).
pub fn verify_inclusion(state: &HeaderState, height: i64, block_hash: &[u8; 32], path: &[[u8; 32]]) -> bool {
    height >= 0 && crate::mmr::verify(&state.mmr_peaks, state.leaves(), height as u64, block_hash, path)
}

/// Is transaction `txid` (internal order) at position `pos` of the block `block_hash` whose 80-byte header is
/// `header`, by this Bitcoin merkle `branch` (internal order, lowest level first)? The header must hash (dsha) to
/// `block_hash`, and the branch must lead from the txid to the header's merkle root: at level j the node is the left
/// child when bit j of `pos` is 0 (parent = dsha(node || sibling)), else the right child (dsha(sibling || node)).
/// Bitcoin's odd-level rule (the last node is paired with itself) shows as a sibling equal to the node on the LEFT
/// child side, which is accepted; an equal sibling on the right child side cannot occur in a real tree and is refused.
/// `pos` must fit the branch (pos < 2^len).
///
/// ⚠ NOT checked: that the branch's length is the block's real tree depth. The header does not carry the
/// transaction count, and a 64-byte transaction is indistinguishable from an inner node (the merkle tree weakness of
/// CVE-2012-2459's family), so a branch one level short could present an inner node as a "txid". A caller that needs
/// certainty checks the depth independently (every position of a Bitcoin tree has the same branch length, so e.g. the
/// coinbase's branch, position 0, fixes it).
pub fn verify_tx_in_header(header: &[u8; 80], block_hash: &[u8; 32], txid: &[u8; 32], pos: u32, branch: &[[u8; 32]]) -> bool {
    let dsha = |b: &[u8]| -> [u8; 32] { Sha256::digest(Sha256::digest(b)).into() };
    if dsha(header) != *block_hash || branch.len() >= 32 || (pos as u64) >> branch.len() != 0 { return false; }
    let mut cur = *txid;
    for (j, sib) in branch.iter().enumerate() {
        let mut b = [0u8; 64];
        if pos >> j & 1 == 0 {
            b[..32].copy_from_slice(&cur);
            b[32..].copy_from_slice(sib);
        } else {
            if *sib == cur { return false; }
            b[..32].copy_from_slice(sib);
            b[32..].copy_from_slice(&cur);
        }
        cur = dsha(&b);
    }
    cur[..] == header[36..68]
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
    fn hx(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }
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

    /// The Bitcoin merkle branch of position `pos` among `txids` (internal order), built the way Core builds the tree.
    fn branch(txids: &[[u8; 32]], mut pos: usize) -> Vec<[u8; 32]> {
        let mut lv = txids.to_vec();
        let mut out = Vec::new();
        while lv.len() > 1 {
            if lv.len() % 2 == 1 { lv.push(*lv.last().unwrap()); }
            out.push(lv[pos ^ 1]);
            lv = lv.chunks_exact(2).map(|c| { let mut b = [0u8; 64]; b[..32].copy_from_slice(&c[0]); b[32..].copy_from_slice(&c[1]); dsha(&b) }).collect();
            pos >>= 1;
        }
        out
    }

    /// Mainnet block 100,002 (9 transactions, so the odd-level rule applies at three levels): every transaction is in
    /// it by its branch; the wrong position, txid, header or a short branch is refused.
    #[test]
    fn transactions_in_a_real_block() {
        let header_hex = "0100000090f0a9f110702f808219ebea1173056042a714bad51b916cb6800000000000005275289558f51c9966699404ae2294730c3c9f9bda53523ce50e9b95e558da2fdb261b4d4c86041b1ab1bf93";
        let block = "0000000000013b8ab2cd513b0261a14096412195a72a0c4827d229dcc7e0f7af";
        let shown = ["ef1d870d24c85b89d92ad50f4631026f585d6a34e972eaf427475e5d60acf3a3", "f9fc751cb7dc372406a9f8d738d5e6f8f63bab71986a39cf36ee70ee17036d07",
                     "db60fb93d736894ed0b86cb92548920a3fe8310dd19b0da7ad97e48725e1e12e", "220ebc64e21abece964927322cba69180ed853bb187fbc6923bac7d010b9d87a",
                     "71b3dbaca67e9f9189dad3617138c19725ab541ef0b49c05a94913e9f28e3f4e", "fe305e1ed08212d76161d853222048eea1f34af42ea0e197896a269fbf8dc2e0",
                     "21d2eb195736af2a40d42107e6abd59c97eb6cffd4a5a7a7709e86590ae61987", "dd1fd2a6fc16404faf339881a90adbde7f4f728691ac62e8f168809cdfae1053",
                     "74d681e0e03bafa802c8aa084379aa98d9fcd632ddc2ed9782b586ec87451f20"];
        let txids: Vec<[u8; 32]> = shown.iter().map(|x| { let mut b: [u8; 32] = unhex(x).try_into().unwrap(); b.reverse(); b }).collect();
        let header: [u8; 80] = unhex(header_hex).try_into().unwrap();
        let mut bh: [u8; 32] = unhex(block).try_into().unwrap(); bh.reverse();
        for (k, t) in txids.iter().enumerate() {
            let b = branch(&txids, k);
            assert_eq!(b.len(), 4);
            assert!(verify_tx_in_header(&header, &bh, t, k as u32, &b), "tx {k}");
            assert!(!verify_tx_in_header(&header, &bh, t, (k as u32 + 1) % 9, &b), "tx {k} at the wrong position");
            assert!(!verify_tx_in_header(&header, &bh, t, k as u32 + 16, &b), "a position that does not fit the branch");
            assert!(!verify_tx_in_header(&header, &bh, &txids[(k + 1) % 9], k as u32, &b), "another txid");
            assert!(!verify_tx_in_header(&header, &bh, t, k as u32 & 7, &b[..3]), "a short branch");
            let mut h2 = header; h2[0] ^= 1;
            assert!(!verify_tx_in_header(&h2, &bh, t, k as u32, &b), "a header that is not the block's");
        }
        // tx 8 is paired with itself at level 0: its branch's first sibling is itself, on the left-child side (accepted);
        // the same equal sibling claimed on the right-child side is refused
        assert_eq!(branch(&txids, 8)[0], txids[8]);
        let mut fake = branch(&txids, 8); fake[0] = txids[8];
        assert!(!verify_tx_in_header(&header, &bh, &txids[8], 9, &fake));
        // through the text parser, as the CLI and WASM give it
        let b = branch(&txids, 5);
        let bs: Vec<String> = b.iter().map(|x| { let mut y = *x; y.reverse(); hx(&y) }).collect();
        let br: Vec<&str> = bs.iter().map(|x| x.as_str()).collect();
        let inc = Inclusion::parse(100_002, block, &[], Some((shown[5], 5, header_hex, &br))).unwrap();
        let t = inc.tx.unwrap();
        assert!(verify_tx_in_header(&t.header, &inc.hash, &t.txid, t.pos, &t.branch));
        assert!(Inclusion::parse(1, "00", &[], None).is_err());
        assert!(Inclusion::parse(1, block, &["zz"], None).is_err());
    }
}
