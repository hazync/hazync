//! `swiftsync-verify`: the checks a user runs before trusting a libzync chain proof, and the facts (coins, the
//! counting trie) checked against the state it proves. One implementation, shared by the CLI, the C ABI and WASM.
//! (The name is historical: the SwiftSync hints file, design B, was retired in E2. The proofs need none.)
//!
//! Given a receipt R, the network, and optionally a trusted anchor state, [`verify`] checks the receipt, its
//! journal, the profile, the anchor, the block forest and time-too-new, and returns every check with its result,
//! the proven chain's tip and work (whose identity with the network's chain a caller checks against a source it
//! trusts), and one of three verdicts:
//!   - `Ok`:   a genesis-anchored proof of a chain with enough work;
//!   - `Demo`: every check passed, but against a SUPPLIED anchor, or below the minimum work / checkpoint;
//!   - `Fail`: anything else, including unreadable inputs.
//!
//! It never panics on input: a malformed receipt, file or anchor is a failed check.

use risc0_zkvm::sha::Digest;
use serde::{Deserialize, Serialize};

/// The libzync images this verifier accepts, one per network (scripts and signatures checked). These are
/// the published image ids M of §5 step 1; a reader should rebuild them (§5 step 9) rather than take them
/// from this file.
pub const IMAGES: &[Image] = &[
    Image {
        network: "mainnet",
        method_id_hex: "863c9ce35104740d8b6fa2972f0dd1b29289e3e67e5679400f0e8ecdbef69af5",
        profile: PROFILE_LIBZYNC | PROFILE_COINLOG | PROFILE_TRIE | PROFILE_UTXOHASH,
        genesis_hash_hex: "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f",
        genesis_time: 1_231_006_505,
        genesis_bits: 0x1d00_ffff,
        genesis_work: 4_295_032_833,
        // Core v28 chainparams.cpp, consensus.nMinimumChainWork (mainnet).
        min_work_hex: "000000000000000000000000000000000000000088e186b70e0862c193ec44d6",
        genesis_anchor_hex: "852c5c7e009317c2dd232dcf9b1a9e2babd58df324add461a6ff61824f390167",
        // The guest's pinned checkpoint (prover/methods/guest/src/checkpoint.rs): block 970000 = 000000000000000000014e6a9bad5f954f266cfb3e8f63f8c2fb7967f29b5a07.
        checkpoint_height: 970_000,
        lineage: &[],
    },
    Image {
        network: "signet",
        method_id_hex: "73b62530ca89eea376e7753f47ab4f16aed3be3138d6052d84d30510aed0f38d",
        profile: PROFILE_LIBZYNC | PROFILE_SIGNET | PROFILE_COINLOG | PROFILE_TRIE | PROFILE_UTXOHASH,
        genesis_hash_hex: "00000008819873e925422c1ff0f99f7cc9bbb232af63a077a480a3633bee1ef6",
        genesis_time: 1_598_918_400,
        genesis_bits: 0x1e03_77ae,
        genesis_work: 4_838_420,
        // Core v28 chainparams.cpp, consensus.nMinimumChainWork (default signet).
        min_work_hex: "0000000000000000000000000000000000000000000000000000025dbd66e58f",
        genesis_anchor_hex: "3b67ead0618b7786b001525b68c7b088405d6a577292838361cc07e5bb938827",
        // The guest's pinned checkpoint (prover/methods/guest/src/checkpoint.rs): block 325000 = 000000057a363b7d1d713167191e272744532b507d919c81ec5a2c57e306fcd4.
        checkpoint_height: 325_000,
        lineage: &[],
    },
];

pub const KIND_CHAIN: u32 = 0xC4A1_0012;
/// The ChainState layout this crate reads (the guest's CHAIN_STATE_VERSION). Later versions only append fields.
pub const CHAIN_STATE_VERSION: u32 = 2;
pub const PROFILE_LIBZYNC: u32 = 1;
pub const PROFILE_ASSUMEVALID: u32 = 2;
pub const PROFILE_SIGNET: u32 = 4;
/// The coin log (docs/libzync/COINLOG.md): no hints file, sound at every block.
pub const PROFILE_COINLOG: u32 = 16;
/// The counting trie (libzync/src/ttree.rs): `trie_root` is its root; absence, history and balance facts ([`TrieFact`]).
pub const PROFILE_TRIE: u32 = 32;
/// The live-coin multiset hash (instant IBD): `live_set` / `live_hash` hash the unspent coins.
pub const PROFILE_UTXOHASH: u32 = 64;
/// Core's MAX_FUTURE_BLOCK_TIME.
pub const MAX_FUTURE_BLOCK_TIME: u64 = 2 * 60 * 60;

pub struct Image {
    pub network: &'static str,
    pub method_id_hex: &'static str,
    pub profile: u32,
    /// Display order, as `getblockhash 0` prints it.
    pub genesis_hash_hex: &'static str,
    pub genesis_time: u32,
    pub genesis_bits: u32,
    /// Core's GetBlockProof(genesis_bits) = 2^256 / (target + 1); checked by the unit test.
    pub genesis_work: u128,
    /// Core's `nMinimumChainWork` for the network, 64 hex digits big-endian as chainparams writes it. A proof below it is
    /// a valid chain from genesis but no evidence that it is THE chain (a low-work fork costs little to mine), so the
    /// verdict is at best `Demo`.
    pub min_work_hex: &'static str,
    /// The network's genesis `anchor_id`, PINNED: dsha256 of the genesis ChainState journal in the layout of the first
    /// image of the network's chain (version 2, kind 0xC4A1_0012). A chain keeps its anchor_id across every handover,
    /// including one that appends fields, so a later reader must not recompute it in its own layout (review 11-03).
    /// The unit test `genesis_anchors_are_pinned` checks it against [`genesis_anchor`].
    pub genesis_anchor_hex: &'static str,
    /// The height of the checkpoint the image pins (the guest refuses any other block there). Below it a proof does not
    /// show that its chain contains the checkpoint, so the verdict is at best `Demo`, as for low work.
    pub checkpoint_height: u32,
    /// The handovers this image's source pins, oldest first (HANDOVER.md). A proof by this image carries either a
    /// zero lineage (one image since genesis) or the digest of exactly this list.
    pub lineage: &'static [Handover],
}

impl Image {
    pub fn by_network(name: &str) -> Option<&'static Image> {
        IMAGES.iter().find(|i| i.network == name)
    }
    /// The network's parameters with another image id: for a reader who rebuilt M themselves (§5 step 9), or
    /// an older image. Leaked so it can live in `Inputs` like the built-in table.
    pub fn with_method_id(&'static self, hex: &str) -> Option<&'static Image> {
        hex32(hex)?;
        Some(Box::leak(Box::new(Image { method_id_hex: Box::leak(hex.to_string().into_boxed_str()), ..*self })))
    }
    /// The same image with a stated lineage: for a reader who rebuilt an image that pins one.
    pub fn with_lineage(&'static self, list: Vec<Handover>) -> &'static Image {
        Box::leak(Box::new(Image { lineage: Box::leak(list.into_boxed_slice()), ..*self }))
    }
    /// The same image built as the coin log (profile bit 4, docs/libzync/COINLOG.md): no hints file.
    pub fn with_coinlog(&'static self) -> &'static Image {
        Box::leak(Box::new(Image { profile: self.profile | PROFILE_COINLOG, ..*self }))
    }
    /// The product image (E2): coin log, counting trie and live-coin hash (profile bits 4, 5, 6). No hints file.
    pub fn with_product(&'static self) -> &'static Image {
        Box::leak(Box::new(Image { profile: self.profile | PROFILE_COINLOG | PROFILE_TRIE | PROFILE_UTXOHASH, ..*self }))
    }
    pub fn method_id(&self) -> Digest {
        Digest::from_bytes(hex32(self.method_id_hex).expect("IMAGES holds valid hex"))
    }
}

/// The libzync guest's committed state, field for field and in order (§3 of the proposal). The journal is
/// risc0's serde encoding of exactly this struct.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ChainState {
    pub kind: u32,
    /// [`CHAIN_STATE_VERSION`]. 0 in a JSON written before the field.
    #[serde(default)]
    pub version: u32,
    pub tip_hash: [u8; 32],
    pub utxo_roots: Vec<Option<[u8; 32]>>,
    pub utxo_leaves: u64,
    /// Satoshis held by the coin forest: at H, the UTXO set's total amount.
    #[serde(default)]
    pub utxo_value: u64,
    pub cum_work: [u8; 32],
    pub height: u32,
    pub prev_nbits: u32,
    pub prev_time: u32,
    pub epoch_start: u32,
    pub recent_times: Vec<u32>,
    pub anchor_id: [u8; 32],
    pub self_id: [u32; 8],
    /// The multiset hash of the LIVE coins (profile bit 6, instant IBD): 0 = the identity, 1 = `live_hash` is the point.
    /// The serde aliases read a JSON written before E2 renamed the retired SwiftSync design B's fields.
    #[serde(alias = "ss_set")]
    pub live_set: u32,
    #[serde(alias = "ss_acc")]
    pub live_hash: [u32; 16],
    /// The counting trie's root (profile bit 5): absence, history and balance facts ([`TrieFact`]).
    #[serde(alias = "ss_hints")]
    pub trie_root: [u8; 32],
    pub smt_root: [u8; 32],
    pub max_time: u32,
    pub profile: u32,
    /// One leaf per block, genesis first: see [`block_in_chain`] and docs/libzync/HANDOVER.md.
    #[serde(default)]
    pub block_roots: Vec<Option<[u8; 32]>>,
    #[serde(default)]
    pub block_leaves: u64,
    /// Zero for a chain that has been one image since genesis; see [`lineage_of`].
    #[serde(default)]
    pub lineage: [u8; 32],
}

/// The guest's own definitions of a handover, a block leaf and a lineage step.
pub mod headers;

pub mod pin {
    include!("../../prover/methods/guest/src/handover_pin.rs");
}
pub use pin::Handover;

fn sha256(b: &[u8]) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    Sha256::digest(b).into()
}

/// The lineage digest of a list of handovers, oldest first.
pub fn lineage_of(list: &[Handover]) -> [u8; 32] {
    list.iter().fold([0u8; 32], |l, h| sha256(&pin::lineage_preimage(&l, h)))
}

/// A block's coin-state digest: the coin commitment (roots, leaf count, total value) after that block.
pub fn utxo_state(leaves: u64, value: u64, roots: &[Option<[u8; 32]>]) -> [u8; 32] {
    let mut p: Vec<u8> = Vec::new();
    pin::utxo_state_preimage(leaves, value, roots, &mut |b: &[u8]| p.extend_from_slice(b));
    sha256(&p)
}

/// The coin-state digest of a proven state's own tip: what its last block leaf commits.
pub fn utxo_state_of(cs: &ChainState) -> [u8; 32] {
    utxo_state(cs.utxo_leaves, cs.utxo_value, &cs.utxo_roots)
}

/// One block as the block forest holds it: its hash (internal byte order) and its coin-state digest ([`utxo_state`]).
pub type BlockEntry = ([u8; 32], [u8; 32]);

/// A block's leaf in the block forest.
pub fn block_leaf(hash: &[u8; 32], height: u32, state: &[u8; 32]) -> [u8; 32] {
    sha256(&pin::block_leaf_preimage(hash, height, state))
}

fn forest_parent(l: &[u8; 32], r: &[u8; 32]) -> [u8; 32] {
    let mut p = [0u8; 65];
    p[0] = 0x01;
    p[1..33].copy_from_slice(l);
    p[33..65].copy_from_slice(r);
    sha256(&p)
}

/// Where leaf `pos` sits in a forest of `leaves`: (tree height, position inside that tree). Trees run largest first,
/// the order `Stump::add` builds them in. None if there is no such leaf.
fn locate(leaves: u64, pos: u64) -> Option<(u32, u64)> {
    let mut offset = 0u64;
    for h in (0..64).rev() {
        if (leaves >> h) & 1 == 1 {
            if pos < offset + (1 << h) {
                return Some((h, pos - offset));
            }
            offset += 1 << h;
        }
    }
    None
}

/// Does this proven state commit to block `hash` at `height`, with coin state `state` after it? `path` is the sibling
/// hashes, lowest first. A proof of the tip checks any earlier block, and its coin state, this way, with no node and
/// no per-block proving. `cs` must have passed [`verify`].
pub fn block_in_chain(cs: &ChainState, height: u32, hash: &[u8; 32], state: &[u8; 32], path: &[[u8; 32]]) -> bool {
    // libzync's check, which also requires the roots present to match the leaf count's bits (review round 2).
    libzync::facts::block_in_chain(&cs.block_roots, cs.block_leaves, hash, height, state, path)
}

/// The block forest over `blocks` (index = height): the roots a proof at the last height commits.
pub fn block_forest(hashes: &[BlockEntry]) -> (Vec<Option<[u8; 32]>>, u64) {
    let mut roots: Vec<Option<[u8; 32]>> = Vec::new();
    for (i, b) in hashes.iter().enumerate() {
        let (mut node, mut h) = (block_leaf(&b.0, i as u32, &b.1), 0);
        while roots.get(h).copied().flatten().is_some() {
            node = forest_parent(&roots[h].take().unwrap(), &node);
            h += 1;
        }
        if h >= roots.len() {
            roots.resize(h + 1, None);
        }
        roots[h] = Some(node);
    }
    while roots.last() == Some(&None) {
        roots.pop();
    }
    (roots, hashes.len() as u64)
}

/// The path for the block at `height` in the forest over `hashes`: what a block-proof server hands out.
pub fn block_path(hashes: &[BlockEntry], height: u32) -> Option<Vec<[u8; 32]>> {
    let (tree, mut pos) = locate(hashes.len() as u64, height as u64)?;
    let start = height as u64 - pos;
    let mut level: Vec<[u8; 32]> = hashes[start as usize..(start + (1u64 << tree)) as usize].iter().enumerate()
        .map(|(i, b)| block_leaf(&b.0, start as u32 + i as u32, &b.1)).collect();
    let mut path = Vec::with_capacity(tree as usize);
    for _ in 0..tree {
        path.push(level[(pos ^ 1) as usize]);
        level = level.chunks_exact(2).map(|c| forest_parent(&c[0], &c[1])).collect();
        pos >>= 1;
    }
    Some(path)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    Demo,
    Fail,
}

impl Verdict {
    /// The documented exit codes: 0, 2, 1.
    pub fn exit_code(self) -> i32 {
        match self {
            Verdict::Ok => 0,
            Verdict::Demo => 2,
            Verdict::Fail => 1,
        }
    }
}

pub struct Report {
    pub checks: Vec<(bool, String)>,
    /// The chain's work is below the image's `min_work_hex` (the reason for a `Demo` verdict without an anchor).
    pub low_work: bool,
    /// The decoded journal, when the receipt held one.
    pub state: Option<ChainState>,
    pub verdict: Verdict,
}

impl Report {
    fn check(&mut self, ok: bool, what: impl Into<String>) -> bool {
        self.checks.push((ok, what.into()));
        ok
    }
    fn failed(&self) -> bool {
        self.checks.iter().any(|(ok, _)| !ok)
    }
}

/// Everything the verifier is given. `now` is the verifier's own clock (unix seconds), for step 11.
pub struct Inputs<'a> {
    pub receipt: &'a [u8],
    pub image: &'static Image,
    /// A trusted state to check `anchor_id` against instead of genesis: the verdict is then at best `Demo`.
    pub anchor: Option<ChainState>,
    pub now: u64,
}

/// The receipt's kind, from its first bytes, BEFORE it is decoded: a chain proof is succinct or Groth16. A composite
/// receipt holds assumption receipts that may themselves be composite, without bound, so decoding a hostile one ~50k
/// levels deep (~1 MB) overflowed the stack (an abort in the C ABI, a trap in WASM, no verdict line in the CLI; cold
/// review 11-06). Succinct and Groth16 claims nest to a fixed depth. bincode writes the InnerReceipt variant first, as
/// a u32 LE: 0 Composite, 1 Succinct, 2 Groth16, 3 Fake (risc0-zkvm 3.0.5).
pub fn receipt_kind(receipt: &[u8]) -> Result<&'static str, String> {
    match receipt.get(..4).map(|b| u32::from_le_bytes(b.try_into().unwrap())) {
        Some(1) => Ok("succinct"),
        Some(2) => Ok("groth16"),
        Some(0) => Err("a composite receipt: a chain proof is succinct or Groth16 (composite receipts nest without bound, so they are refused before decoding)".into()),
        Some(3) => Err("a dev-mode FakeReceipt, not a proof".into()),
        Some(k) => Err(format!("not a receipt (kind tag {k})")),
        None => Err("not a receipt (too short)".into()),
    }
}

pub fn verify(inp: &Inputs) -> Report {
    let mut rep = Report { checks: Vec::new(), state: None, verdict: Verdict::Fail, low_work: false };
    let m = inp.image.method_id();
    if let Err(e) = receipt_kind(inp.receipt) {
        rep.check(false, format!("the receipt file is a single succinct or Groth16 proof ({e})"));
        return rep;
    }

    // Steps 1 and 8: a real proof of this image.
    let receipt: risc0_zkvm::Receipt = match bincode::deserialize(inp.receipt) {
        Ok(r) => r,
        Err(e) => {
            rep.check(false, format!("the receipt file decodes ({e})"));
            return rep;
        }
    };
    rep.check(!matches!(receipt.inner, risc0_zkvm::InnerReceipt::Fake(_)),
              "receipt is a real proof (not a dev-mode FakeReceipt)");
    rep.check(!dev_mode_env(), "RISC0_DEV_MODE is not set in this verifier's environment");
    let v = receipt.verify(m);
    rep.check(v.is_ok(), format!("receipt verifies against image {} ({}){}", inp.image.method_id_hex,
              inp.image.network, v.err().map(|e| format!(": {e}")).unwrap_or_default()));

    // Step 2: the journal is this image's ChainState.
    let cs: ChainState = match receipt.journal.decode() {
        Ok(c) => c,
        Err(e) => {
            rep.check(false, format!("journal is a ChainState ({e})"));
            return rep;
        }
    };
    // A later version only appends fields (HANDOVER.md §2), so this reader checks the prefix it knows.
    rep.check(cs.kind == KIND_CHAIN && cs.version >= CHAIN_STATE_VERSION && cs.self_id == words(&m),
              format!("journal is a ChainState v{} committed by this image (kind, version, self_id)", cs.version));

    // Step 9: what the image checked.
    rep.check(cs.profile == inp.image.profile, format!("profile 0x{:x} == 0x{:x} ({})", cs.profile,
              inp.image.profile, describe_profile(cs.profile)));

    // Step 11: time-too-new on this verifier's clock.
    rep.check(cs.max_time as u64 <= inp.now.saturating_add(MAX_FUTURE_BLOCK_TIME), format!(
        "time-too-new: largest block time {} <= this clock {} + {} s", cs.max_time, inp.now, MAX_FUTURE_BLOCK_TIME));

    // The block forest: one leaf per block, in the shape that leaf count forces.
    let shape = (0..64).all(|i| ((cs.block_leaves >> i) & 1 == 1) == cs.block_roots.get(i).copied().flatten().is_some());
    rep.check(cs.block_leaves == cs.height as u64 + 1 && shape,
              format!("block forest has one leaf per block ({} leaves at height {})", cs.block_leaves, cs.height));

    // The lineage: an image can only ever commit zero or its own pinned list. A list the reader did not state fails.
    let full = lineage_of(inp.image.lineage);
    rep.check(cs.lineage == [0; 32] || cs.lineage == full, format!("lineage {} is this image's ({} handover(s) stated)",
              hex(&cs.lineage), inp.image.lineage.len()));

    // Step 3: anchored at genesis (or at the supplied state, which can only ever give Demo).
    let (want, what) = match &inp.anchor {
        None => (hex32(inp.image.genesis_anchor_hex).unwrap_or([0xff; 32]), "genesis".to_string()),
        Some(a) => (anchor_id(a), format!("the GIVEN anchor at height {} -- NOT genesis", a.height)),
    };
    rep.check(cs.anchor_id == want, format!("anchor_id is {what}"));

    // The coin log is sound at every block: nothing to close at H. A design-B image (a SwiftSync hints file closed
    // the commitment at H) is retired: its proofs are not checked.
    rep.check(inp.image.profile & PROFILE_COINLOG != 0, "the image keeps its coins in the coin log (design B, the hints-file images, is retired)");
    // The live-coin hash (bit 6) and the trie root (bit 5), or zero without them.
    if inp.image.profile & PROFILE_UTXOHASH == 0 {
        rep.check(cs.live_set == 0 && cs.live_hash == [0; 16], "live-coin hash unused in this image (live_set == 0)");
    }
    if inp.image.profile & PROFILE_TRIE == 0 {
        rep.check(cs.trie_root == [0; 32], "trie root unused in this image (trie_root == 0)");
    }
    // Not a failed check: the chain may be valid, it just does not carry the work that makes it the network's chain.
    let enough_work = work_at_least(&cs.cum_work, inp.image.min_work_hex);
    rep.checks.push((true, format!("cum_work 0x{} {} Core's nMinimumChainWork 0x{}{}", work_be_hex(&cs.cum_work),
        if enough_work { ">=" } else { "<" }, inp.image.min_work_hex,
        if enough_work { "" } else { " -- a valid chain, but NOT shown to be the network's: verdict at best DEMO" })));
    let past_checkpoint = cs.height >= inp.image.checkpoint_height;
    rep.checks.push((true, format!("height {} {} the image's checkpoint height {}{}", cs.height,
        if past_checkpoint { ">=" } else { "<" }, inp.image.checkpoint_height,
        if past_checkpoint { ": the chain contains the pinned checkpoint block" } else { " -- the pinned checkpoint is not reached: verdict at best DEMO" })));
    let enough_work = enough_work && past_checkpoint;
    rep.low_work = !enough_work;
    rep.verdict = if rep.failed() {
        Verdict::Fail
    } else if inp.anchor.is_some() || !enough_work {
        Verdict::Demo
    } else {
        Verdict::Ok
    };
    rep.state = Some(cs);
    rep
}

/// RISC0_DEV_MODE as risc0 reads it: set and not empty / "0" / "false". A FakeReceipt "verifies" under it.
pub fn dev_mode_env() -> bool {
    std::env::var("RISC0_DEV_MODE").map(|v| !v.is_empty() && v != "0" && v != "false").unwrap_or(false)
}

pub fn describe_profile(p: u32) -> String {
    format!("{}, {}, {}{}",
            if p & PROFILE_LIBZYNC != 0 { "libzync" } else { "canonical" },
            if p & PROFILE_ASSUMEVALID != 0 { "ASSUMEVALID: scripts and signatures NOT checked" } else { "scripts checked" },
            if p & PROFILE_SIGNET != 0 { "signet" } else if p & 8 != 0 { "regtest" } else { "mainnet" },
            if p & PROFILE_COINLOG != 0 { ", coin log" } else { "" })
        + if p & PROFILE_TRIE != 0 { ", counting trie" } else { "" }
        + if p & PROFILE_UTXOHASH != 0 { ", live-coin hash" } else { "" }
}

/// cum_work as 64 hex digits big-endian (chainwork as Core prints it).
pub fn work_be_hex(w: &[u8; 32]) -> String {
    let mut r = *w;
    r.reverse();
    hex(&r)
}

/// cum_work (32 bytes little-endian) >= a big-endian hex threshold.
pub fn work_at_least(w: &[u8; 32], min_be_hex: &str) -> bool {
    let Some(mut m) = hex32(min_be_hex) else { return false };
    m.reverse();
    (0..32).rev().map(|i| w[i].cmp(&m[i])).find(|o| o.is_ne()).map_or(true, |o| o.is_gt())
}

/// The network's genesis ChainState for this image (§5 step 3).
pub fn genesis_anchor(img: &Image) -> ChainState {
    let mut tip = hex32(img.genesis_hash_hex).expect("IMAGES holds valid hex");
    tip.reverse();
    ChainState {
        kind: KIND_CHAIN,
        version: CHAIN_STATE_VERSION,
        tip_hash: tip,
        utxo_roots: vec![None],
        utxo_leaves: 0,
        utxo_value: 0,
        cum_work: {
            let mut w = [0u8; 32];
            w[..16].copy_from_slice(&img.genesis_work.to_le_bytes());
            w
        },
        height: 0,
        prev_nbits: img.genesis_bits,
        prev_time: img.genesis_time,
        epoch_start: img.genesis_time,
        recent_times: vec![img.genesis_time],
        anchor_id: [0; 32],
        // Zero, not this image: anchor_id must stay the same across a handover (HANDOVER.md).
        self_id: [0; 8],
        live_set: 0,
        live_hash: [0; 16],
        trie_root: [0; 32],
        smt_root: empty_smt_root(),
        max_time: img.genesis_time,
        profile: 0,
        block_roots: vec![Some(block_leaf(&tip, 0, &utxo_state(0, 0, &[])))],
        block_leaves: 1,
        lineage: [0; 32],
    }
}

/// The guest's link digest of a state: dsha256 of its journal encoding with `anchor_id` zeroed. What the guest commits
/// as `anchor_id` after a base step on it (ROADMAP H-B). The genesis state's anchor_id is zero: unchanged for it.
pub fn anchor_id(s: &ChainState) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    let mut z = s.clone();
    z.anchor_id = [0; 32];
    let words = risc0_zkvm::serde::to_vec(&z).expect("ChainState serialises");
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    Sha256::digest(Sha256::digest(bytes)).into()
}

/// The empty coinbase-SMT root: e_0 = 0^32, e_{d+1} = SHA-256(0x11 ‖ e_d ‖ e_d), 256 levels.
pub fn empty_smt_root() -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    let mut e = [0u8; 32];
    for _ in 0..256 {
        let mut h = Sha256::new();
        h.update([0x11]);
        h.update(e);
        h.update(e);
        e = h.finalize().into();
    }
    e
}

pub fn words(d: &Digest) -> [u32; 8] {
    let mut w = [0u32; 8];
    w.copy_from_slice(d.as_words());
    w
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn hex32(s: &str) -> Option<[u8; 32]> {
    // hex digits only: from_str_radix also takes a leading '+', so the printed string could differ from the bytes
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// `--expect <height>:<hash>`: the height and the hash, lowercased. None unless the hash is exactly 64 hex digits.
pub fn parse_expect(e: &str) -> Option<(u32, String)> {
    let (h, x) = e.split_once(':')?;
    let x = x.to_lowercase();
    (x.len() == 64 && x.bytes().all(|b| b.is_ascii_hexdigit())).then_some((h.parse().ok()?, x))
}

/// Chain identity (SWIFTSYNC_PROOFS.md step 7): `claimed` (display order, as `getblockhash` prints it) is the block at
/// `claimed_height`, and the proof's tip is `tip` (internal order) at `height`. Only the tip can be checked.
pub fn identity_matches(claimed_height: u32, claimed: &str, height: u32, tip: &[u8; 32]) -> bool {
    let mut r = *tip;
    r.reverse();
    let shown: String = r.iter().map(|b| format!("{b:02x}")).collect();
    claimed_height == height && claimed.len() == 64 && claimed == shown
}

/// A coin fact (docs/libzync/ZYNCAPPS.md §8): what `zync-path coin` writes and `swiftsync-verify --coin` checks against
/// a verified tip proof. Hashes are hex, internal byte order.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CoinFact {
    /// The height whose coin state the fact is about: the proof's tip, or an earlier block opened through the forest.
    pub at: u32,
    /// The coin's position in the coin log (birth order).
    pub pos: u64,
    /// The coin's unspent leaf (UTREEXO_FORMAT.md §2).
    pub coin: String,
    /// (height, tx position, input) of the spend, if the coin was spent by `at`.
    pub spent_by: Option<(u32, u32, u32)>,
    /// The coin's path in the log at `at`, bottom-up.
    pub path: Vec<String>,
    /// The coin log after `at`: leaf count, live value, roots (index = level). Unused when `at` is the tip.
    pub log_leaves: u64,
    pub log_value: u64,
    pub log_roots: Vec<Option<String>>,
    /// When `at` is below the tip: the block at `at` (hash) and its path in the tip's block forest.
    pub block: Option<(String, Vec<String>)>,
    /// The coin `coin` is the leaf OF: its fields, from the caller's own coin or from the server (either is sound: a
    /// flipped leaf can never equal a coin leaf, so a lying preimage simply fails). Required for an UNSPENT answer: the check proves only that a 32-byte value is the log's leaf at `pos`,
    /// and once a coin is spent that leaf is its spent-leaf hash, which an UNSPENT check cannot tell from a coin leaf
    /// (round 11). A SPENT answer is bound already (the spent leaf hashes `coin` with the spender). A journey
    /// (CoinJourney, from the creating transaction) binds the coin without this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preimage: Option<CoinPreimage>,
}

/// The fields a coin leaf commits (UTREEXO_FORMAT.md §2; libzync::facts::coin_leaf).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CoinPreimage {
    /// as block explorers print it
    pub txid: String,
    pub vout: u32,
    pub value: u64,
    /// hex
    pub script_pubkey: String,
    /// the block that created the coin
    pub height: u32,
    pub coinbase: bool,
    /// tracked by the BIP30 tree (UTREEXO_FORMAT.md §2a)
    pub tracked: bool,
    /// the median time past of the creating block's parent
    pub creation_mtp: u32,
}

fn hexes(v: &[String]) -> Option<Vec<[u8; 32]>> {
    v.iter().map(|x| hex32(&x.to_lowercase())).collect()
}

/// Checks a coin fact against a state that passed [`verify`] (and must be a coin-log chain). Ok: one line saying what
/// it proves.
pub fn check_coin_fact(cs: &ChainState, f: &CoinFact) -> Result<String, String> {
    // the coin the leaf is of (round 11): required for UNSPENT, checked whenever given
    let coin = hex32(&f.coin.to_lowercase()).ok_or("coin: not 64 hex digits")?;
    let bound = match &f.preimage {
        Some(p) => {
            let mut t = hex32(&p.txid.to_lowercase()).ok_or("preimage txid: not 64 hex digits")?;
            t.reverse();
            let spk = unhex(&p.script_pubkey).ok_or("preimage script_pubkey: not hex")?;
            let leaf = libzync::facts::coin_leaf(&t, p.vout, p.value, &spk, p.height, p.coinbase, p.tracked, p.creation_mtp);
            if leaf != coin { return Err("coin: not the leaf of the coin in `preimage`".into()); }
            true
        }
        None => false,
    };
    if f.spent_by.is_none() && !bound {
        return Err("an UNSPENT answer needs the coin it is about (`preimage`, from your own coin, or a journey): a spent coin's flipped leaf passes the position check too".into());
    }
    coin_fact_position(cs, f)
}

/// The position half of [`check_coin_fact`]: `coin` is the log's leaf at `pos` (unspent) or flipped by `spent_by`, in
/// the state at `at`. It does NOT say `coin` is a coin's leaf; the caller binds that (preimage or journey).
fn coin_fact_position(cs: &ChainState, f: &CoinFact) -> Result<String, String> {
    if cs.profile & PROFILE_COINLOG == 0 {
        return Err("the proof is not a coin-log chain: coin facts need the coin log".into());
    }
    let coin = hex32(&f.coin.to_lowercase()).ok_or("coin: not 64 hex digits")?;
    let path = hexes(&f.path).ok_or("path: not 64-hex-digit hashes")?;
    let (roots, n, block_line) = if f.at == cs.height {
        if f.block.is_some() {
            return Err("a fact at the tip carries no block opening".into());
        }
        (cs.utxo_roots.clone(), cs.utxo_leaves, String::new())
    } else if f.at < cs.height {
        let roots: Vec<Option<[u8; 32]>> = f.log_roots.iter()
            .map(|r| r.as_ref().map(|x| hex32(&x.to_lowercase()).ok_or("log_roots: not 64 hex digits")).transpose())
            .collect::<Result<_, _>>()?;
        let (bh, bp) = f.block.as_ref().ok_or("a fact below the tip needs the block opening")?;
        let bh = hex32(&bh.to_lowercase()).ok_or("block hash: not 64 hex digits")?;
        let bp = hexes(bp).ok_or("block path: not 64-hex-digit hashes")?;
        let state = utxo_state(f.log_leaves, f.log_value, &roots);
        if !block_in_chain(cs, f.at, &bh, &state, &bp) {
            return Err(format!("block {} with that coin state is not in the proof's block forest", f.at));
        }
        let mut d = bh;
        d.reverse();
        (roots, f.log_leaves, format!(" (block {} {}, in the proof's chain to {})", f.at, hex(&d), cs.height))
    } else {
        return Err(format!("at {} is above the proof's tip {}", f.at, cs.height));
    };
    let roots = normalize_roots(roots);
    match f.spent_by {
        None if libzync::facts::coin_unspent(&roots, n, f.pos, &coin, &path) =>
            Ok(format!("coin {}{} at log position {} is UNSPENT at height {}{block_line}", f.coin,
                       f.preimage.as_ref().map(|p| format!(" ({}:{})", p.txid, p.vout)).unwrap_or_default(), f.pos, f.at)),
        Some((h, t, v)) if h <= f.at && libzync::facts::coin_spent_by(&roots, n, f.pos, &coin, h, t, v, &path) =>
            Ok(format!("coin {} at log position {} was SPENT at height {h} by transaction {t} input {v} (state at {}){block_line}", f.coin, f.pos, f.at)),
        _ => Err(format!("coin {} is not at log position {} in the state it names", f.coin, f.pos)),
    }
}

/// A counting-trie fact (E2): a question about the proof's TIP and the trie proof that answers it (libzync::trie_fact).
/// `zync-path trie ...` writes one from a run's `state_<h>.trie`; `swiftsync-verify --trie` checks it.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TrieFact {
    /// "coin" (unspent / spent by / absent), "balance" (unspent count and value) or "history" (every coin ever paid)
    pub kind: String,
    /// The proof's tip the fact is about.
    pub at: u32,
    /// The output script, hex.
    pub script_pubkey: String,
    /// For "coin": the txid as block explorers print it, and the output index.
    #[serde(default)]
    pub txid: Option<String>,
    #[serde(default)]
    pub vout: Option<u32>,
    /// The trie pruned to the question, as little-endian u32 words, hex.
    pub proof: String,
    /// The CLAIMED answer, which the check must reproduce or refuse ([`trie_verdict`]): for "coin" `absent`,
    /// `unspent:<sat>` or `spent:<sat>:<height>:<tx>:<input>`; for "balance" / "history" `never` or `<count>:<sat>`.
    /// Without it the fact is a question: the check states the answer but cannot refuse it (the C ABI demands it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect: Option<String>,
    /// For "coin" answers UNSPENT or SPENT: the creating transaction ([`TxFact`]), required. The trie's coin key is the
    /// txid with the output index XORed into its first 4 bytes (the guest's `okey`), so (txid, vout) and
    /// (txid ^ d, vout ^ d) share a key: alone, a trie leaf does not show WHICH outpoint it is. The transaction, in a
    /// proven block, with this txid and an output at `vout` paying this script this value, does (an alias would need a
    /// real transaction whose txid is another's with its first 4 bytes changed). An ABSENT answer needs none: no key,
    /// no coin with that outpoint (cold review 11-06).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<TxFact>,
}

/// The canonical answer token of a coin state (see [`TrieFact::expect`]).
pub fn coin_verdict(s: &libzync::trie_fact::CoinState) -> String {
    use libzync::trie_fact::CoinState::*;
    match s {
        Absent => "absent".into(),
        Unspent { value } => format!("unspent:{value}"),
        Spent { value, by } => format!("spent:{value}:{}:{}:{}", by.height, by.tx, by.vin),
    }
}

/// The canonical answer token of a balance (see [`TrieFact::expect`]).
pub fn balance_verdict(b: Option<(u64, u64)>) -> String {
    match b { None => "never".into(), Some((c, v)) => format!("{c}:{v}") }
}

fn coin_state_line(s: &libzync::trie_fact::CoinState) -> String {
    use libzync::trie_fact::CoinState::*;
    match s {
        Absent => "ABSENT (no coin with this outpoint was ever paid to THIS script; the trie never files unspendable outputs or the genesis coinbase)".into(),
        Unspent { value } => format!("UNSPENT, {value} sat"),
        Spent { value, by } => format!("SPENT at height {} by transaction {} input {}, {value} sat", by.height, by.tx, by.vin),
    }
}

/// Checks a trie fact against a state that passed [`verify`] (and is a trie chain): its proof holds and, when it claims
/// an answer (`expect`), the proof gives that answer. Ok: what it proves.
pub fn check_trie_fact(cs: &ChainState, f: &TrieFact) -> Result<String, String> {
    let (got, line) = trie_verdict(cs, f)?;
    match &f.expect {
        Some(want) if *want != got => Err(format!("the fact claims {want:?}, its proof gives {got:?}: {line}")),
        _ => Ok(line),
    }
}

/// The answer a trie fact's proof gives, as its token, and the line saying what it proves. Err: the proof does not hold.
pub fn trie_verdict(cs: &ChainState, f: &TrieFact) -> Result<(String, String), String> {
    use libzync::trie_fact as tf;
    if cs.profile & PROFILE_TRIE == 0 {
        return Err("the proof is not a counting-trie chain".into());
    }
    if f.at != cs.height {
        return Err(format!("the fact is about height {}, the proof's tip is {} (trie facts are at the tip)", f.at, cs.height));
    }
    let spk = unhex(&f.script_pubkey).ok_or("script_pubkey: not hex")?;
    let pb = unhex(&f.proof).ok_or("proof: not hex")?;
    if pb.len() % 4 != 0 { return Err("proof: not whole words".into()); }
    let proof: Vec<u32> = pb.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    let root = libzync::coinlog::w_of(&cs.trie_root);
    let e = |x: tf::FactErr| format!("the trie proof does not hold against the proof's trie root ({x:?})");
    let sh = tf::script_hash(&spk);
    match f.kind.as_str() {
        "coin" => {
            let tx = f.txid.as_deref().and_then(|t| hex32(&t.to_lowercase())).ok_or("coin: txid (64 hex digits)")?;
            let mut txid = tx;
            txid.reverse();
            let vout = f.vout.ok_or("coin: vout")?;
            let s = tf::coin(&proof, &root, &tf::coin_key(&spk, &txid, vout)).map_err(e)?;
            // A coin the trie holds: bind the outpoint to a real transaction (see TrieFact::origin).
            let value = match &s { tf::CoinState::Absent => None,
                                    tf::CoinState::Unspent { value } | tf::CoinState::Spent { value, .. } => Some(*value) };
            if let Some(value) = value {
                let o = f.origin.as_ref().ok_or("coin: the trie holds a coin under this key, but which outpoint it is needs the creating transaction (`origin`, a tx fact)")?;
                let (otxid, view) = check_tx_fact(cs, o).map_err(|x| format!("origin: {x}"))?;
                if otxid != txid { return Err("origin: that transaction's txid is not this coin's".into()); }
                match view.outputs.get(vout as usize) {
                    Some((v, script)) if *v == value && *script == spk => {}
                    Some(_) => return Err(format!("origin: output {vout} does not pay this script {value} sat")),
                    None => return Err(format!("origin: the transaction has no output {vout}")),
                }
            }
            Ok((coin_verdict(&s), format!("coin {}:{vout} to script {} is {} at height {}", f.txid.as_deref().unwrap_or(""), f.script_pubkey, coin_state_line(&s), cs.height)))
        }
        "balance" => Ok(match tf::balance(&proof, &root, &sh).map_err(e)? {
            None => (balance_verdict(None), format!("script {} has NEVER been paid a spendable coin (height {})", f.script_pubkey, cs.height)),
            Some(b) => (balance_verdict(Some((b.count, b.value))),
                        format!("script {} holds {} unspent coin(s), {} sat, at height {}", f.script_pubkey, b.count, b.value, cs.height)),
        }),
        "history" => Ok(match tf::history(&proof, &root, &sh).map_err(e)? {
            None => (balance_verdict(None), format!("script {} has NEVER been paid a spendable coin (height {})", f.script_pubkey, cs.height)),
            Some((b, coins)) => {
                let spent = coins.iter().filter(|c| matches!(c.state, tf::CoinState::Spent { .. })).count();
                (balance_verdict(Some((b.count, b.value))),
                 format!("script {}: complete history at height {}: {} coin(s) ever paid, {spent} spent, {} unspent holding {} sat",
                         f.script_pubkey, cs.height, coins.len(), b.count, b.value))
            }
        }),
        k => Err(format!("kind {k:?}: want coin, balance or history")),
    }
}

/// The producer side (zync-path / zync-serve): a [`TrieFact`] from a run's trie snapshot at its tip. `txid` as explorers
/// print it. Holds no secret and proves nothing by itself: a wrong proof simply fails `check_trie_fact`.
pub fn make_trie_fact(st: &libzync::trie_store::TrieStore, kind: &str, script_pubkey: &[u8], txid: Option<&str>, vout: Option<u32>)
                      -> Result<TrieFact, String> {
    use libzync::trie_fact as tf;
    // the answer, from the proof itself against the store's root, goes into the fact as its claim (`expect`)
    let root = st.root();
    let fe = |x: tf::FactErr| format!("the store's own proof does not hold ({x:?})");
    let (proof, expect) = match kind {
        "coin" => {
            let mut t = txid.and_then(|t| hex32(&t.to_lowercase())).ok_or("coin: txid (64 hex digits)")?;
            t.reverse();
            let k = tf::coin_key(script_pubkey, &t, vout.ok_or("coin: vout")?);
            let p = st.prove_coins(&[k]);
            let v = coin_verdict(&tf::coin(&p, &root, &k).map_err(fe)?);
            (p, v)
        }
        "balance" => {
            let sh = tf::script_hash(script_pubkey);
            let p = st.prove_addr(&sh);
            let v = balance_verdict(tf::balance(&p, &root, &sh).map_err(fe)?.map(|b| (b.count, b.value)));
            (p, v)
        }
        "history" => {
            let sh = tf::script_hash(script_pubkey);
            let p = st.prove_history(&sh);
            let v = balance_verdict(tf::history(&p, &root, &sh).map_err(fe)?.map(|(b, _)| (b.count, b.value)));
            (p, v)
        }
        k => return Err(format!("kind {k:?}: want coin, balance or history")),
    };
    Ok(TrieFact {
        kind: kind.into(), at: st.height as u32, script_pubkey: hex(script_pubkey),
        txid: txid.map(|t| t.to_lowercase()), vout,
        proof: hex(&proof.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>()),
        expect: Some(expect),
        origin: None,
    })
}

/// Lower or upper case hex to bytes.
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// A transaction in a proven block (ZYNCAPPS.md §8): the block's header and coin state, its path in the proof's block
/// forest, and the transaction's Merkle branch.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TxFact {
    pub height: u32,
    /// The 80-byte header, hex.
    pub header: String,
    /// The coin-state digest after this block (`utxo_state`), hex.
    pub state: String,
    pub block_path: Vec<String>,
    /// The transaction, hex, with or without witness.
    pub tx: String,
    /// Its position in the block (coinbase 0) and its Merkle branch, bottom-up.
    pub index: u32,
    pub branch: Vec<String>,
    /// The block's coinbase (hex) and ITS branch: they bind the tree's depth (libzync::facts::tx_in_block).
    pub coinbase: String,
    pub coinbase_branch: Vec<String>,
}

/// Checks that the transaction is at `index` of the block at `height` of the proven chain. Ok: its txid and parse.
pub fn check_tx_fact(cs: &ChainState, f: &TxFact) -> Result<([u8; 32], libzync::facts::TxView), String> {
    let header: [u8; 80] = unhex(&f.header).and_then(|h| h.try_into().ok()).ok_or("header: not 80 bytes of hex")?;
    let state = hex32(&f.state.to_lowercase()).ok_or("state: not 64 hex digits")?;
    let bp = hexes(&f.block_path).ok_or("block_path: not 64-hex-digit hashes")?;
    let hash = libzync::facts::header_hash(&header);
    if !block_in_chain(cs, f.height, &hash, &state, &bp) {
        return Err(format!("block {} with that header and state is not in the proof's block forest", f.height));
    }
    let raw = unhex(&f.tx).ok_or("tx: not hex")?;
    let mut o = 0;
    let view = libzync::facts::parse_tx(&raw, &mut o).filter(|_| o == raw.len()).ok_or("tx: not one transaction")?;
    let br = hexes(&f.branch).ok_or("branch: not 64-hex-digit hashes")?;
    let cb = unhex(&f.coinbase).ok_or("coinbase: not hex")?;
    let cbb = hexes(&f.coinbase_branch).ok_or("coinbase_branch: not 64-hex-digit hashes")?;
    let txid = libzync::facts::tx_in_block(&header, &view.stripped, f.index, &br, &cb, &cbb)
        .ok_or_else(|| format!("tx is not at position {} of block {}", f.index, f.height))?;
    Ok((txid, view))
}

/// One hop of a coin's journey: the transaction that created it, the coin's state in the log, and (if spent) the
/// transaction that spent it. `tracked` and `creation_mtp` complete the coin leaf (UTREEXO_FORMAT.md §2); a wrong
/// value gives a leaf the log does not hold, so they are checked, not trusted.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CoinJourney {
    pub origin: TxFact,
    pub vout: u32,
    pub tracked: bool,
    pub creation_mtp: u32,
    pub fact: CoinFact,
    pub spend: Option<TxFact>,
}

pub fn check_coin_journey(cs: &ChainState, j: &CoinJourney) -> Result<String, String> {
    let (txid, view) = check_tx_fact(cs, &j.origin).map_err(|e| format!("origin: {e}"))?;
    let (value, spk) = view.outputs.get(j.vout as usize).ok_or("origin: no such output")?;
    let leaf = libzync::facts::coin_leaf(&txid, j.vout, *value, spk, j.origin.height, j.origin.index == 0, j.tracked, j.creation_mtp);
    if hex(&leaf) != j.fact.coin.to_lowercase() {
        return Err("the origin output's coin leaf is not the fact's coin".into());
    }
    // the journey has already bound `coin` to the origin's output (the leaf recomputed from the transaction), so only
    // the position half is needed here (round 12: check_coin_fact's preimage rule refused every unspent journey)
    let line = coin_fact_position(cs, &j.fact)?;
    let mut d = txid;
    d.reverse();
    let mut out = format!("{}:{} ({value} sat) created at height {} -- {line}", hex(&d), j.vout, j.origin.height);
    match (j.fact.spent_by, &j.spend) {
        (None, None) => {}
        (Some((h, t, v)), Some(sp)) => {
            let (sid, sview) = check_tx_fact(cs, sp).map_err(|e| format!("spend: {e}"))?;
            if sp.height != h || sp.index != t {
                return Err("spend: not the transaction the log names".into());
            }
            if sview.inputs.get(v as usize) != Some(&(txid, j.vout)) {
                return Err("spend: that input does not spend this coin".into());
            }
            let mut d = sid;
            d.reverse();
            out += &format!("; spent by {} input {v}", hex(&d));
        }
        (Some(_), None) => return Err("the coin was spent: the journey needs the spending transaction".into()),
        (None, Some(_)) => return Err("the coin is unspent: a spending transaction makes no sense".into()),
    }
    Ok(out)
}

fn normalize_roots(mut r: Vec<Option<[u8; 32]>>) -> Vec<Option<[u8; 32]>> {
    while r.last() == Some(&None) {
        r.pop();
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real-run check (no receipt): HAZYNC_TRIE_STATE=<state_h.json> HAZYNC_TRIE_FACTS=<fact.json>,... cargo test -- --ignored
    #[test]
    #[ignore]
    fn trie_facts_against_a_run_state() {
        let st: ChainState = serde_json::from_slice(&std::fs::read(std::env::var("HAZYNC_TRIE_STATE").unwrap()).unwrap()).unwrap();
        for p in std::env::var("HAZYNC_TRIE_FACTS").unwrap().split(',') {
            let f: TrieFact = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
            println!("{p}: {}", check_trie_fact(&st, &f).unwrap());
        }
    }

    #[test]
    fn trie_facts_round_trip_and_refuse() {
        use libzync::trie_fact::coin_key;
        let a = b"\x00\x14aaaaaaaaaaaaaaaaaaaa".to_vec();
        let txid_disp = "11".repeat(31) + "22";
        let mut txid = hex32(&txid_disp).unwrap();
        txid.reverse();
        let sp = |h: u32| -> [u32; 8] { [h, 1, 0, 0, 0, 0, 0, 0] };
        let mut st = libzync::trie_store::TrieStore::new(true);
        st.apply(&[], &[(coin_key(&a, &txid, 0), None, 500), (coin_key(&a, &txid, 1), None, 70)]).unwrap();
        st.apply(&[(coin_key(&a, &txid, 0), sp(2))], &[]).unwrap();
        st.height = 2;
        let mut cs = genesis_anchor(&IMAGES[0]);
        cs.profile |= PROFILE_COINLOG | PROFILE_TRIE;
        cs.height = 2;
        cs.trie_root = libzync::coinlog::h_of(&st.root());

        let f = make_trie_fact(&st, "coin", &a, Some(&txid_disp), Some(0)).unwrap();
        // a coin the trie HOLDS needs its creating transaction (TrieFact::origin; with one: coin_journey_end_to_end)
        assert!(check_trie_fact(&cs, &f).unwrap_err().contains("origin"), "spent coin without an origin");
        let f1 = make_trie_fact(&st, "coin", &a, Some(&txid_disp), Some(1)).unwrap();
        assert!(check_trie_fact(&cs, &f1).unwrap_err().contains("origin"), "unspent coin without an origin");
        let f9 = make_trie_fact(&st, "coin", &a, Some(&txid_disp), Some(9)).unwrap();
        assert!(check_trie_fact(&cs, &f9).unwrap().contains("ABSENT"));
        let b = make_trie_fact(&st, "balance", &a, None, None).unwrap();
        assert!(check_trie_fact(&cs, &b).unwrap().contains("1 unspent coin(s), 70 sat"));
        let hst = make_trie_fact(&st, "history", &a, None, None).unwrap();
        assert!(check_trie_fact(&cs, &hst).unwrap().contains("2 coin(s) ever paid, 1 spent"));
        let n = make_trie_fact(&st, "balance", b"\x6a", None, None).unwrap();
        assert!(check_trie_fact(&cs, &n).unwrap().contains("NEVER"));
        // the fact's own claim (txid / vout / script) is what is checked, not the proof's: a swapped question fails
        let mut sw = f.clone();
        sw.vout = Some(1);
        assert!(check_trie_fact(&cs, &sw).is_err(), "a coin proof answers only for its own coin");
        // another root, another height, a chain without the trie: refused
        let mut c2 = cs.clone();
        c2.trie_root[0] ^= 1;
        assert!(check_trie_fact(&c2, &f).is_err());
        let mut c3 = cs.clone();
        c3.height = 3;
        assert!(check_trie_fact(&c3, &f).is_err());
        let mut c4 = cs.clone();
        c4.profile &= !PROFILE_TRIE;
        assert!(check_trie_fact(&c4, &f).is_err());
        let mut bad = f.clone();
        bad.proof.replace_range(8..10, if &bad.proof[8..10] == "00" { "01" } else { "00" });
        assert!(check_trie_fact(&cs, &bad).is_err());

        // the producer writes the answer as the claim; a false claim is refused; no claim = a question (answered)
        assert_eq!(f.expect.as_deref(), Some("spent:500:2:1:0"));
        assert_eq!(f1.expect.as_deref(), Some("unspent:70"));
        assert_eq!(f9.expect.as_deref(), Some("absent"));
        assert_eq!(b.expect.as_deref(), Some("1:70"));
        assert_eq!(hst.expect.as_deref(), Some("1:70"));
        assert_eq!(n.expect.as_deref(), Some("never"));
        // lies about an absent coin are refused by the claim check (a held coin is refused before it without an origin)
        for lie in ["unspent:500", "spent:500:2:1:1", "spent:501:2:1:0"] {
            let mut l = f9.clone();
            l.expect = Some(lie.into());
            assert!(check_trie_fact(&cs, &l).unwrap_err().contains("claims"), "{lie}");
        }
        let mut l = f.clone();
        l.expect = Some("absent".into());
        assert!(check_trie_fact(&cs, &l).is_err(), "a held coin claimed absent");
        let mut l = b.clone();
        l.expect = Some("1:71".into());
        assert!(check_trie_fact(&cs, &l).is_err());
        let mut l = n.clone();
        l.expect = Some("0:0".into());
        assert!(check_trie_fact(&cs, &l).is_err(), "never-paid is not a zero balance claim");
        let mut q = f9.clone();
        q.expect = None;
        assert!(check_trie_fact(&cs, &q).unwrap().contains("ABSENT"), "a question (no claim) is answered");
        // the JSON without the field still reads (facts written before it)
        let mut j = serde_json::to_value(&q).unwrap();
        j.as_object_mut().unwrap().remove("expect");
        assert!(serde_json::from_value::<TrieFact>(j).unwrap().expect.is_none());
    }

    #[test]
    fn coin_facts_from_history() {
        use libzync::history::{block_path, replay, Rec};
        let h = |i: u64| -> [u8; 32] { sha256(&i.to_le_bytes()) };
        let recs = vec![
            Rec { height: 0, hash: h(1000), value: 0, flips: vec![], appends: vec![] },
            Rec { height: 1, hash: h(1001), value: 30, flips: vec![], appends: vec![h(0), h(1), h(2)] },
            Rec { height: 2, hash: h(1002), value: 40, flips: vec![(1, 1, 0)], appends: vec![h(3), h(4)] },
            Rec { height: 3, hash: h(1003), value: 35, flips: vec![(0, 1, 2)], appends: vec![h(5)] },
        ];
        let tip = replay(&recs, 3, 3).unwrap();
        let mut cs = genesis_anchor(&IMAGES[0]);
        let broots = block_path(&tip.block_leaves, 0).1;
        cs.profile |= PROFILE_COINLOG;
        cs.height = 3;
        cs.utxo_roots = tip.log.roots();
        cs.utxo_leaves = tip.log.len();
        cs.block_roots = broots;
        cs.block_leaves = 4;
        let fact = |at: u32, pos: u64| -> CoinFact {
            let r = replay(&recs, at, 3).unwrap();
            CoinFact {
                at, pos, coin: hex(&r.coins[pos as usize]), spent_by: r.spenders.get(&pos).copied(),
                path: r.log.path(pos).iter().map(|x| hex(x)).collect(), log_leaves: r.log.len(), log_value: r.value,
                log_roots: r.log.roots().iter().map(|x| x.map(|h| hex(&h))).collect(),
                block: (at < 3).then(|| (hex(&recs[at as usize].hash), block_path(&r.block_leaves, at as u64).0.iter().map(|x| hex(x)).collect())),
                preimage: None,
            }
        };
        assert!(check_coin_fact(&cs, &fact(3, 0)).unwrap().contains("SPENT at height 3 by transaction 1 input 2"));
        // an UNSPENT answer with no coin bound is refused (round 11); the position logic is checked below directly
        assert!(check_coin_fact(&cs, &fact(3, 5)).unwrap_err().contains("preimage"));
        let check_coin_fact = coin_fact_position; // these leaves are synthetic: no preimage exists for them
        assert!(check_coin_fact(&cs, &fact(3, 5)).unwrap().contains("UNSPENT at height 3"));
        // coin 0 was still unspent after block 2
        assert!(check_coin_fact(&cs, &fact(2, 0)).unwrap().contains("UNSPENT at height 2"));
        // lies: unspent at the tip, a wrong spender, a wrong block state, above the tip
        let mut f = fact(3, 0);
        f.spent_by = None;
        assert!(check_coin_fact(&cs, &f).is_err());
        let mut f = fact(3, 0);
        f.spent_by = Some((3, 1, 1));
        assert!(check_coin_fact(&cs, &f).is_err());
        let mut f = fact(2, 0);
        f.log_value += 1;
        assert!(check_coin_fact(&cs, &f).is_err());
        let mut f = fact(2, 0);
        f.at = 4;
        assert!(check_coin_fact(&cs, &f).is_err());
        let mut f = fact(3, 5);
        f.block = Some((hex(&h(1003)), vec![]));
        assert!(check_coin_fact(&cs, &f).is_err());
        cs.profile &= !PROFILE_COINLOG;
        assert!(check_coin_fact(&cs, &fact(3, 5)).is_err());
    }

    #[test]
    fn coin_journey_end_to_end() {
        use libzync::facts::{coin_leaf, header_hash, merkle_branch, parse_tx, txid};
        use libzync::history::{block_path, replay, Rec};
        // legacy txs: version, 1 input, outputs, locktime
        let mk = |prev: [u8; 32], vout: u32, outs: &[(u64, &[u8])]| -> Vec<u8> {
            let mut t = 1u32.to_le_bytes().to_vec();
            t.push(1);
            t.extend_from_slice(&prev);
            t.extend_from_slice(&vout.to_le_bytes());
            t.extend_from_slice(&[2, 0x51, 0x51]);
            t.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
            t.push(outs.len() as u8);
            for (v, spk) in outs {
                t.extend_from_slice(&v.to_le_bytes());
                t.push(spk.len() as u8);
                t.extend_from_slice(spk);
            }
            t.extend_from_slice(&[0; 4]);
            t
        };
        let cb1 = mk([0; 32], 0xffff_ffff, &[(50, b"\x51\x52\x60")]); // 65 bytes: a 64-byte tx is refused by design
        let id1 = txid(&parse_tx(&cb1, &mut 0).unwrap());
        let cb2 = mk([0; 32], 0xffff_ffff, &[(51, b"\x53")]);
        let sp = mk(id1, 0, &[(40, b"\x54\x55\x56")]);
        let id_cb2 = txid(&parse_tx(&cb2, &mut 0).unwrap());
        let id_sp = txid(&parse_tx(&sp, &mut 0).unwrap());
        let header = |prev: [u8; 32], root: [u8; 32]| -> [u8; 80] {
            let mut h = [0u8; 80];
            h[4..36].copy_from_slice(&prev);
            h[36..68].copy_from_slice(&root);
            h
        };
        let h1 = header([9; 32], id1);
        let mut p = [0u8; 64];
        p[..32].copy_from_slice(&id_cb2);
        p[32..].copy_from_slice(&id_sp);
        let root2: [u8; 32] = sha256(&sha256(&p));
        let h2 = header(header_hash(&h1), root2);
        let mtp1 = 1_231_006_505u32;
        let leaf_a = coin_leaf(&id1, 0, 50, b"\x51\x52\x60", 1, true, false, mtp1);
        let leaf_b = coin_leaf(&id_cb2, 0, 51, b"\x53", 2, true, false, 777);
        let leaf_c = coin_leaf(&id_sp, 0, 40, b"\x54\x55\x56", 2, false, false, 777);
        let recs = vec![
            Rec { height: 0, hash: [7; 32], value: 0, flips: vec![], appends: vec![] },
            Rec { height: 1, hash: header_hash(&h1), value: 50, flips: vec![], appends: vec![leaf_a] },
            Rec { height: 2, hash: header_hash(&h2), value: 91, flips: vec![(0, 1, 0)], appends: vec![leaf_b, leaf_c] },
        ];
        let r = replay(&recs, 2, 2).unwrap();
        let mut cs = genesis_anchor(&IMAGES[0]);
        cs.profile |= PROFILE_COINLOG;
        cs.height = 2;
        cs.utxo_roots = r.log.roots();
        cs.utxo_leaves = r.log.len();
        cs.block_roots = block_path(&r.block_leaves, 0).1;
        cs.block_leaves = 3;
        let txf = |h: u32, hdr: &[u8; 80], ids: &[[u8; 32]], raw: &[u8], index: usize, cb: &[u8]| TxFact {
            height: h, header: hex(hdr), state: hex(&r.states[h as usize]),
            block_path: block_path(&r.block_leaves, h as u64).0.iter().map(|x| hex(x)).collect(),
            tx: hex(raw), index: index as u32, branch: merkle_branch(ids, index).iter().map(|x| hex(x)).collect(),
            coinbase: hex(cb), coinbase_branch: merkle_branch(ids, 0).iter().map(|x| hex(x)).collect(),
        };
        let j = CoinJourney {
            origin: txf(1, &h1, &[id1], &cb1, 0, &cb1), vout: 0, tracked: false, creation_mtp: mtp1,
            fact: CoinFact {
                at: 2, pos: 0, coin: hex(&leaf_a), spent_by: r.spenders.get(&0).copied(),
                path: r.log.path(0).iter().map(|x| hex(x)).collect(), log_leaves: r.log.len(), log_value: r.value,
                log_roots: r.log.roots().iter().map(|x| x.map(|h| hex(&h))).collect(), block: None, preimage: None },
            spend: Some(txf(2, &h2, &[id_cb2, id_sp], &sp, 1, &cb2)),
        };
        let ok = check_coin_journey(&cs, &j).unwrap();
        assert!(ok.contains("(50 sat) created at height 1") && ok.contains("SPENT at height 2 by transaction 1 input 0"), "{ok}");
        // refusals
        let mut b = j.clone();
        b.creation_mtp += 1;
        assert!(check_coin_journey(&cs, &b).is_err(), "wrong mtp");
        let mut b = j.clone();
        b.spend = Some(txf(2, &h2, &[id_cb2, id_sp], &cb2, 0, &cb2));
        assert!(check_coin_journey(&cs, &b).is_err(), "a spend tx that is not the one the log names");
        let mut b = j.clone();
        b.spend = None;
        assert!(check_coin_journey(&cs, &b).is_err(), "spent but no spend tx");
        let mut b = j.clone();
        b.origin.index = 1;
        assert!(check_coin_journey(&cs, &b).is_err(), "wrong position in the block");
        let mut b = j.clone();
        b.spend.as_mut().unwrap().coinbase_branch.clear();
        assert!(check_coin_journey(&cs, &b).is_err(), "a coinbase branch of another depth");
        let mut b = j.clone();
        b.origin.state = hex(&r.states[2]);
        assert!(check_coin_journey(&cs, &b).is_err(), "wrong block state");

        // An UNSPENT journey (round 12: the preimage rule had refused every one; the journey binds the coin itself):
        // block 2's coinbase output, unspent at the tip
        let uj = CoinJourney {
            origin: txf(2, &h2, &[id_cb2, id_sp], &cb2, 0, &cb2), vout: 0, tracked: false, creation_mtp: 777,
            fact: CoinFact {
                at: 2, pos: 1, coin: hex(&leaf_b), spent_by: None,
                path: r.log.path(1).iter().map(|x| hex(x)).collect(), log_leaves: r.log.len(), log_value: r.value,
                log_roots: r.log.roots().iter().map(|x| x.map(|h| hex(&h))).collect(), block: None, preimage: None,
            },
            spend: None,
        };
        let ok = check_coin_journey(&cs, &uj).unwrap();
        assert!(ok.contains("UNSPENT at height 2"), "{ok}");
        // ... and an unspent journey for the SPENT coin (its flipped leaf at the tip) still fails
        let mut bad = j.clone();
        bad.spend = None;
        bad.fact.spent_by = None;
        assert!(check_coin_journey(&cs, &bad).is_err(), "a spent coin's journey claimed unspent");

        // A standalone coin fact binds the coin by its preimage (round 11): with the coin's fields it holds; the
        // SPENT coin's flipped leaf presented as an unspent coin is refused, with or without the real preimage.
        let pre = CoinPreimage { txid: { let mut d = id1; d.reverse(); hex(&d) }, vout: 0, value: 50,
                                 script_pubkey: hex(b"\x51\x52\x60"), height: 1, coinbase: true, tracked: false, creation_mtp: mtp1 };
        let r1 = replay(&recs, 1, 2).unwrap();
        let mut cf = CoinFact {
            at: 1, pos: 0, coin: hex(&leaf_a), spent_by: None, path: r1.log.path(0).iter().map(|x| hex(x)).collect(),
            log_leaves: r1.log.len(), log_value: r1.value, log_roots: r1.log.roots().iter().map(|x| x.map(|h| hex(&h))).collect(),
            block: Some((hex(&recs[1].hash), block_path(&r.block_leaves, 1).0.iter().map(|x| hex(x)).collect())),
            preimage: Some(pre.clone()),
        };
        assert!(check_coin_fact(&cs, &cf).unwrap().contains("UNSPENT at height 1"), "a bound unspent coin");
        cf.preimage = None;
        assert!(check_coin_fact(&cs, &cf).is_err(), "unbound unspent");
        // at the tip coin 0 is spent: its log leaf is the flipped one; offered as an "unspent coin" it is refused
        let tipf = |coin: [u8; 32], preimage: Option<CoinPreimage>| CoinFact {
            at: 2, pos: 0, coin: hex(&coin), spent_by: None, path: r.log.path(0).iter().map(|x| hex(x)).collect(),
            log_leaves: r.log.len(), log_value: r.value, log_roots: vec![], block: None, preimage };
        let flipped = libzync::coinlog::spent(&leaf_a, &libzync::coinlog::spender(2, 1, 0)); // spent at 2 by tx 1 input 0
        assert_ne!(flipped, leaf_a);
        assert!(check_coin_fact(&cs, &tipf(flipped, None)).is_err(), "the flipped leaf, unbound");
        assert!(check_coin_fact(&cs, &tipf(flipped, Some(pre.clone()))).is_err(), "the flipped leaf with the real preimage");
        assert!(coin_fact_position(&cs, &tipf(flipped, None)).is_ok(), "(the position check alone would have passed it)");

        // Trie coin facts (cold review 11-06): the trie key aliases (txid, vout) with (txid ^ d, vout ^ d); the
        // creating transaction, in a proven block, binds the outpoint.
        use libzync::trie_fact::coin_key;
        let spk_a: &[u8] = b"\x51\x52\x60";
        let mut st = libzync::trie_store::TrieStore::new(true);
        st.apply(&[], &[(coin_key(spk_a, &id1, 0), None, 50)]).unwrap();
        st.height = 2;
        cs.profile |= PROFILE_TRIE;
        cs.trie_root = libzync::coinlog::h_of(&st.root());
        let disp = |t: &[u8; 32]| { let mut d = *t; d.reverse(); hex(&d) };
        let mut tf = make_trie_fact(&st, "coin", spk_a, Some(&disp(&id1)), Some(0)).unwrap();
        assert!(check_trie_fact(&cs, &tf).unwrap_err().contains("origin"), "no origin");
        tf.origin = Some(txf(1, &h1, &[id1], &cb1, 0, &cb1));
        assert!(check_trie_fact(&cs, &tf).unwrap().contains("UNSPENT, 50 sat"), "with its origin");
        // the alias: same key, an outpoint that never existed; refused with or without the real coin's origin
        let mut alias = id1;
        alias[0] ^= 1;
        let mut al = make_trie_fact(&st, "coin", spk_a, Some(&disp(&alias)), Some(1)).unwrap();
        assert_eq!(coin_key(spk_a, &alias, 1), coin_key(spk_a, &id1, 0), "the key aliases");
        assert!(check_trie_fact(&cs, &al).is_err(), "alias, no origin");
        al.origin = Some(txf(1, &h1, &[id1], &cb1, 0, &cb1));
        assert!(check_trie_fact(&cs, &al).unwrap_err().contains("txid"), "alias with the real coin's origin");
        // an origin whose output does not pay this script or value
        let mut wrong = tf.clone();
        wrong.script_pubkey = hex(b"\x53");
        assert!(check_trie_fact(&cs, &wrong).is_err());
        // ABSENT needs no origin
        let ab = make_trie_fact(&st, "coin", spk_a, Some(&disp(&id1)), Some(7)).unwrap();
        assert!(check_trie_fact(&cs, &ab).unwrap().contains("ABSENT"));
    }

    /// The built-in table is the published one (reproduce/LIBZYNC_IDS, written with the table by
    /// reproduce/pin-libzync-ids.py): a hand edit to either fails here.
    #[test]
    fn images_match_the_published_ids() {
        let f = include_str!("../../reproduce/LIBZYNC_IDS");
        for img in IMAGES {
            let line = f.lines().find(|l| l.starts_with("libzync-product ") && l.split_whitespace().nth(1) == Some(img.network))
                .unwrap_or_else(|| panic!("reproduce/LIBZYNC_IDS has no {} line", img.network));
            let id = line.split_whitespace().skip_while(|w| *w != "METHOD_ID").nth(1).unwrap();
            assert_eq!(id, img.method_id_hex, "{}: IMAGES and reproduce/LIBZYNC_IDS disagree", img.network);
        }
    }

    /// The header-chain zyncAPP's built-in id is the published one (reproduce/LIBZYNC_IDS, `zyncapp-headers` line).
    #[test]
    fn headers_image_matches_the_published_id() {
        let f = include_str!("../../reproduce/LIBZYNC_IDS");
        let line = f.lines().find(|l| l.starts_with("zyncapp-headers mainnet ")).expect("reproduce/LIBZYNC_IDS has no zyncapp-headers line");
        let id = line.split_whitespace().skip_while(|w| *w != "METHOD_ID").nth(1).unwrap();
        assert_eq!(id, crate::headers::HEADERS_IMAGE_ID, "headers::HEADERS_IMAGE_ID and reproduce/LIBZYNC_IDS disagree");
    }

    #[test]
    fn checkpoint_heights_match_the_guest() {
        let g = include_str!("../../prover/methods/guest/src/checkpoint.rs");
        for img in IMAGES {
            let h = img.checkpoint_height.to_string();
            let h = format!("{}_{}", &h[..h.len() - 3], &h[h.len() - 3..]);
            assert!(g.contains(&format!("Some(({h}, [")), "{} checkpoint {h} not in the guest's checkpoint.rs", img.network);
        }
    }

    #[test]
    fn chain_identity() {
        // mainnet genesis: display order vs internal order
        let shown = "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f";
        let mut tip = [0u8; 32];
        for i in 0..32 { tip[31 - i] = u8::from_str_radix(&shown[2 * i..2 * i + 2], 16).unwrap(); }
        assert!(identity_matches(0, shown, 0, &tip));
        let (h, x) = parse_expect(&format!("0:{}", shown.to_uppercase())).unwrap();
        assert!(identity_matches(h, &x, 0, &tip));
        assert!(!identity_matches(1, shown, 0, &tip), "wrong height");
        let mut rev: Vec<u8> = shown.bytes().collect();
        rev.reverse();
        assert!(!identity_matches(0, &String::from_utf8(rev).unwrap(), 0, &tip), "internal order is not display order");
        let other = format!("{}0", &shown[..63]);
        assert!(!identity_matches(0, &other, 0, &tip), "one digit differs");
        for bad in ["", "<html>", &shown[..63], &format!("0x{shown}")] {
            assert!(!identity_matches(0, bad, 0, &tip), "{bad:?}");
        }
        for bad in [format!("{shown}"), format!("x:{shown}"), format!("0: {shown}"), format!("0:{}", &shown[..63]), format!("0:{shown}0")] {
            assert!(parse_expect(&bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn genesis_work_matches_core() {
        // GetBlockProof = 2^256 / (target + 1), with the target expanded from nBits, in 2^256 / x by long
        // division on bytes (no bignum crate), for each image's genesis.
        for img in IMAGES {
            let exp = img.genesis_bits >> 24;
            let mut target = [0u8; 33]; // big-endian, 264 bits
            let m = (img.genesis_bits & 0x007f_ffff).to_be_bytes();
            let pos = 33 - exp as usize;
            target[pos..pos + 3].copy_from_slice(&m[1..]);
            // target + 1
            for k in (0..33).rev() { target[k] = target[k].wrapping_add(1); if target[k] != 0 { break; } }
            // 2^256 / (target + 1), by binary long division over 257 bits.
            let mut rem = [0u8; 33];
            let mut q: u128 = 0;
            for bit in 0..=256 {
                // rem = rem * 2 + (bit == 0 ? 1 : 0)  (the dividend 2^256 is a 1 followed by 256 zeros)
                let mut carry = if bit == 0 { 1 } else { 0 };
                for k in (0..33).rev() { let v = (rem[k] as u16) << 1 | carry; rem[k] = v as u8; carry = v >> 8; }
                q <<= 1;
                if rem >= target {
                    let mut borrow = 0i16;
                    for k in (0..33).rev() { let v = rem[k] as i16 - target[k] as i16 - borrow; rem[k] = v as u8; borrow = (v < 0) as i16; }
                    q |= 1;
                }
            }
            assert_eq!(q, img.genesis_work, "{}", img.network);
        }
    }

    #[test]
    fn empty_smt_root_vector() {
        assert_eq!(hex(&empty_smt_root()), "507d86c51845c467583449a973898eff681f18e74aa6d62b206da023cbcaec32");
    }

    #[test]
    fn genesis_anchor_vectors() {
        // SWIFTSYNC_PROOFS.md §5 step 3. Independent of the image; computed by scripts/libzync/genesis_anchor_id.py.
        let a = |n| hex(&anchor_id(&genesis_anchor(Image::by_network(n).unwrap())));
        assert_eq!(a("mainnet"), "852c5c7e009317c2dd232dcf9b1a9e2babd58df324add461a6ff61824f390167");
        assert_eq!(a("signet"), "3b67ead0618b7786b001525b68c7b088405d6a577292838361cc07e5bb938827");
    }

    #[test]
    fn genesis_anchors_are_pinned() {
        // The pinned constants are what this layout computes (they must never be recomputed in a later layout).
        for img in IMAGES {
            assert_eq!(hex(&anchor_id(&genesis_anchor(img))), img.genesis_anchor_hex, "{}", img.network);
        }
    }

    #[test]
    fn minimum_work() {
        let mut w = [0u8; 32];
        w[..8].copy_from_slice(&0x93ec44d6u64.to_le_bytes());
        assert!(work_at_least(&w, "00000000000000000000000000000000000000000000000000000000093ec44d6"[1..].into()));
        assert!(!work_at_least(&w, "00000000000000000000000000000000000000000000000000000000093ec44d7"[1..].into()));
        w[20] = 1;   // a high byte outweighs every lower one
        assert!(work_at_least(&w, IMAGES[0].min_work_hex));
        assert!(!work_at_least(&[0u8; 32], IMAGES[1].min_work_hex));
    }

    #[test]
    fn genesis_anchor_does_not_depend_on_the_image() {
        // What makes a handed-over chain still genesis-anchored.
        let img = Image::by_network("mainnet").unwrap();
        let other = img.with_method_id(&"ab".repeat(32)).unwrap();
        assert_eq!(anchor_id(&genesis_anchor(img)), anchor_id(&genesis_anchor(other)));
    }

    /// Made-up blocks: hash b_i = SHA-256(i as u32 LE), coin state s_i = utxo_state(i, 1000·i, [None, Some(SHA-256(b_i))]
    /// for odd i, else []): the same in scripts/libzync/block_forest_ref.py.
    fn sample_hashes(n: u32) -> Vec<BlockEntry> {
        use sha2::{Digest as _, Sha256};
        (0..n).map(|i| {
            let b: [u8; 32] = Sha256::digest(i.to_le_bytes()).into();
            let roots: Vec<Option<[u8; 32]>> = if i % 2 == 1 { vec![None, Some(Sha256::digest(b).into())] } else { vec![] };
            (b, utxo_state(i as u64, 1000 * i as u64, &roots))
        }).collect()
    }

    fn state_over(hashes: &[BlockEntry]) -> ChainState {
        let mut cs = genesis_anchor(Image::by_network("mainnet").unwrap());
        (cs.block_roots, cs.block_leaves) = block_forest(hashes);
        cs
    }

    #[test]
    fn block_forest_vectors() {
        // From scripts/libzync/block_forest_ref.py, which builds the forest from its definition and shares no code here.
        let hs = sample_hashes(11);
        assert_eq!(hex(&block_leaf(&hs[0].0, 0, &hs[0].1)), "228f6506359bf248e6ebbd79fe8ebf607a6ff3090c63c5905dcc1950dad1cc8a");
        assert_eq!(hex(&utxo_state(0, 0, &[])), "48b9b34b851a03302c02d0519a1b741babdbf7e4dbabf93c7a7f1ea7db057e7a");
        let (r, n) = block_forest(&hs);
        assert_eq!(n, 11);
        let want = [Some("c08e5924be0c590b0b0f283f897a72745fbcaca9f7a0ba0e42cc9fa214cbf460"),
                    Some("875737e7f3597222d0168ffd0fbcf2ba54f9c649f78a58655b2c0a20dfe3494f"), None,
                    Some("d93f2ed6c5919b2766eec5412eaafa771404765662b84a91f4b2d26daa5b1eba")];
        assert_eq!(r.iter().map(|x| x.map(|h| hex(&h))).collect::<Vec<_>>(), want.map(|x| x.map(String::from)).to_vec());
        let p = block_path(&hs, 5).unwrap();
        let want = ["5348a332bd8c07149cd91c22da7a3ced2cf7b1bf2a68c1fb82856067d779f344",
                    "2033a6d6ca4dca8c9bd58baae86d98c8be56eb792f2b830f45656343b1f52fb2",
                    "a913976775b5350f893699512397084fab510b5bd3c19b81e26d6ad97b16cae2"];
        assert_eq!(p.iter().map(|h| hex(h)).collect::<Vec<_>>(), want);
    }

    #[test]
    fn lineage_vectors() {
        // scripts/libzync/block_forest_ref.py again.
        let a = Handover { image: [1, 2, 3, 4, 5, 6, 7, 8], first_height: 974_000 };
        let b = Handover { image: [9; 8], first_height: 980_000 };
        assert_eq!(lineage_of(&[]), [0; 32]);
        let one = lineage_of(&[Handover { image: a.image, first_height: a.first_height }]);
        assert_eq!(hex(&one), "de40f074a41c9244272b66f8ce15759e646186968f348bc0cd930898c6948458");
        assert_eq!(hex(&lineage_of(&[a, b])), "fd23e4cf05d16147bcf677535f1c8bc64d8608369c55cc2fd07839dcd12a178c");
    }

    #[test]
    fn every_block_checks_and_nothing_else_does() {
        for n in 1..70u32 {
            let hs = sample_hashes(n);
            let cs = state_over(&hs);
            for h in 0..n {
                let p = block_path(&hs, h).unwrap();
                let (b, st) = &hs[h as usize];
                assert!(block_in_chain(&cs, h, b, st, &p), "n {n} height {h}");
                // A different block, a different coin state, or the right block at the wrong height.
                assert!(!block_in_chain(&cs, h, &[0xee; 32], st, &p));
                assert!(!block_in_chain(&cs, h, b, &[0xee; 32], &p));
                assert!(!block_in_chain(&cs, (h + 1) % n, b, st, &p) || n == 1);
                // One flipped bit anywhere in the path.
                for i in 0..p.len() {
                    let mut q = p.clone();
                    q[i][0] ^= 1;
                    assert!(!block_in_chain(&cs, h, b, st, &q));
                }
                // A short path (an interior node posing as a leaf) and a long one.
                if !p.is_empty() {
                    assert!(!block_in_chain(&cs, h, b, st, &p[..p.len() - 1]));
                }
                let mut long = p.clone();
                long.push([0; 32]);
                assert!(!block_in_chain(&cs, h, b, st, &long));
            }
            assert!(!block_in_chain(&cs, n, &[0; 32], &[0; 32], &[]), "a height past the tip");
        }
    }
}

/// A proof bundle (`docs/libzync/PROOF_BUNDLE.md`): a receipt behind a short header naming what it proves. The header
/// is a convenience for people and indexes, NEVER trusted: [`check_bundle`] requires every field to equal what the
/// verified receipt says. Version 2 (E2): version 1 also carried the sha256 of a SwiftSync hints file.
#[derive(Clone, Debug, PartialEq)]
pub struct Bundle {
    pub network: u8,
    pub image: [u8; 32],
    pub stop_height: u32,
    pub tip_hash: [u8; 32],
    pub cum_work: [u8; 32],
    pub trie_root: [u8; 32],
    pub receipt: Vec<u8>,
}

pub const BUNDLE_MAGIC: &[u8; 4] = b"SSPF";
pub const BUNDLE_VERSION: u8 = 2;
const BUNDLE_HEADER: usize = 4 + 1 + 1 + 32 + 4 + 32 + 32 + 32 + 4;

pub fn network_code(name: &str) -> Option<u8> {
    match name {
        "mainnet" => Some(0),
        "signet" => Some(1),
        _ => None,
    }
}

impl Bundle {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(BUNDLE_HEADER + self.receipt.len());
        out.extend_from_slice(BUNDLE_MAGIC);
        out.push(BUNDLE_VERSION);
        out.push(self.network);
        out.extend_from_slice(&self.image);
        out.extend_from_slice(&self.stop_height.to_le_bytes());
        out.extend_from_slice(&self.tip_hash);
        out.extend_from_slice(&self.cum_work);
        out.extend_from_slice(&self.trie_root);
        out.extend_from_slice(&(self.receipt.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.receipt);
        out
    }

    pub fn decode(b: &[u8]) -> Result<Bundle, String> {
        if b.len() < BUNDLE_HEADER || &b[..4] != BUNDLE_MAGIC {
            return Err("not a proof bundle".into());
        }
        if b[4] != BUNDLE_VERSION {
            return Err(format!("unsupported bundle version {}", b[4]));
        }
        let a32 = |o: usize| -> [u8; 32] { b[o..o + 32].try_into().unwrap() };
        let u32le = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let len = u32le(BUNDLE_HEADER - 4) as usize;
        if b.len() != BUNDLE_HEADER + len {
            return Err(format!("bundle is {} bytes, header says {}", b.len(), BUNDLE_HEADER + len));
        }
        Ok(Bundle {
            network: b[5],
            image: a32(6),
            stop_height: u32le(38),
            tip_hash: a32(42),
            cum_work: a32(74),
            trie_root: a32(106),
            receipt: b[BUNDLE_HEADER..].to_vec(),
        })
    }

    /// Build a bundle from a receipt; fails unless the receipt verifies (OK or DEMO) for `img`.
    pub fn make(receipt: &[u8], img: &'static Image, anchor: Option<ChainState>, now: u64) -> Result<Bundle, String> {
        let rep = verify(&Inputs { receipt, image: img, anchor: anchor.clone(), now });
        // Ok, or Demo only for low work on a chain from genesis (a valid chain, e.g. a signet test range). Never an
        // anchored proof: the bundle's reader would have to trust the same anchor.
        let anchored = anchor.is_some();
        if rep.verdict == Verdict::Fail || (rep.verdict == Verdict::Demo && (anchored || !rep.low_work)) {
            return Err(format!("the receipt's verdict is {:?}{}; refusing to bundle it", rep.verdict,
                               if anchored { " from a given anchor" } else { "" }));
        }
        let cs = rep.state.expect("verified");
        Ok(Bundle {
            network: network_code(img.network).expect("known network"),
            image: hex32(img.method_id_hex).expect("hex"),
            stop_height: cs.height,
            tip_hash: cs.tip_hash,
            cum_work: cs.cum_work,
            trie_root: cs.trie_root,
            receipt: receipt.to_vec(),
        })
    }
}

/// Verify a bundle: the receipt as [`verify`] does, then every header field against the result. The image and
/// network come from the VERIFIER (`img`), never from the bundle.
pub fn check_bundle(bundle: &Bundle, img: &'static Image, anchor: Option<ChainState>, now: u64) -> Report {
    let mut rep = verify(&Inputs { receipt: &bundle.receipt, image: img, anchor, now });
    rep.check(Some(bundle.network) == network_code(img.network), "bundle header: network is this verifier's");
    rep.check(hex32(img.method_id_hex) == Some(bundle.image), format!("bundle header: image {} is this verifier's", hex(&bundle.image)));
    if let Some(cs) = &rep.state {
        let same = cs.height == bundle.stop_height && cs.tip_hash == bundle.tip_hash && cs.cum_work == bundle.cum_work
            && cs.trie_root == bundle.trie_root;
        rep.check(same, "bundle header: height, tip, work and trie_root equal the verified journal's");
    }
    if rep.failed() {
        rep.verdict = Verdict::Fail;
    }
    rep
}

#[cfg(test)]
mod bundle_tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let b = Bundle { network: 1, image: [7; 32], stop_height: 2600, tip_hash: [1; 32], cum_work: [2; 32],
                         trie_root: [3; 32], receipt: vec![9; 100] };
        let e = b.encode();
        assert_eq!(e.len(), BUNDLE_HEADER + 100);
        assert_eq!(Bundle::decode(&e).unwrap(), b);
        assert!(Bundle::decode(&e[..e.len() - 1]).is_err());
        let mut v = e.clone();
        v[4] = 1; // a version-1 bundle (with a hints-file hash) is refused
        assert!(Bundle::decode(&v).is_err());
    }
}
