//! `swiftsync-verify`: the checks a SwiftSync user runs before trusting a hints file because of a libzync proof
//! (docs/libzync/SWIFTSYNC_PROOFS.md §5). One implementation, shared by the CLI and any later C or WASM build.
//!
//! Given a receipt R, a hints file F, the network, and optionally a trusted anchor state, [`verify`] runs
//! steps 1-6 and 8-11 of §5 and returns every check with its result, the proven chain's tip and work (for step
//! 7, which needs the user's own node), and one of three verdicts:
//!   - `Ok`:   a genesis-anchored proof; F is correct for the chain ending at the tip;
//!   - `Demo`: every check passed against a SUPPLIED anchor: never a proof of F;
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
        method_id_hex: "9f469930c612139342e4f2a403eccba7663b1231ecc53b396470c6fe93e72f55",
        profile: PROFILE_LIBZYNC,
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
        method_id_hex: "067822b958f7cc86d67ad9f37153b99c29664072a8e90a79a2d9af4dc71f5b36",
        profile: PROFILE_LIBZYNC | PROFILE_SIGNET,
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
    /// The handovers this image's source pins, oldest first (HANDOVER.md). A proof by this image carries either a
    /// zero lineage (one image since genesis) or the digest of exactly this list.
    /// The height of the checkpoint the image pins (the guest refuses any other block there). Below it a proof does not
    /// show that its chain contains the checkpoint, so the verdict is at best `Demo`, as for low work.
    pub checkpoint_height: u32,
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
    pub ss_set: u32,
    pub ss_acc: [u32; 16],
    pub ss_hints: [u8; 32],
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
    let Some((tree, mut pos)) = locate(cs.block_leaves, height as u64) else { return false };
    if path.len() != tree as usize {
        return false;
    }
    let mut node = block_leaf(hash, height, state);
    for sib in path {
        node = if pos & 1 == 0 { forest_parent(&node, sib) } else { forest_parent(sib, &node) };
        pos >>= 1;
    }
    cs.block_roots.get(tree as usize).copied().flatten() == Some(node)
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
    pub hints: &'a [u8],
    pub image: &'static Image,
    /// A trusted state to check `anchor_id` against instead of genesis: the verdict is then at best `Demo`.
    pub anchor: Option<ChainState>,
    pub now: u64,
}

pub fn verify(inp: &Inputs) -> Report {
    let mut rep = Report { checks: Vec::new(), state: None, verdict: Verdict::Fail, low_work: false };
    let m = inp.image.method_id();

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
    rep.check(cs.max_time as u64 <= inp.now + MAX_FUTURE_BLOCK_TIME, format!(
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

    // Step 10, then 4-6: the file. A coin-log image has none: its commitment is sound at every block.
    if inp.image.profile & PROFILE_COINLOG != 0 {
        rep.check(inp.hints.is_empty(), "a coin-log proof takes no hints file");
        rep.check(cs.ss_set == 0 && cs.ss_hints == [0; 32], "SwiftSync fields unused (ss_set == 0, ss_hints == 0)");
    } else {
    match hints_file(inp.hints) {
        Err(e) => {
            rep.check(false, format!("the hints file is canonically encoded and well formed ({e})"));
        }
        Ok((stop, fh)) => {
            rep.check(true, "the hints file is canonically encoded (re-encoding its lists gives the same bytes; no trailing data)");
            rep.check(cs.height == stop, format!("height {} == the file's stop height {}", cs.height, stop));
            rep.check(cs.ss_set == 0, "SwiftSync total is the identity (ss_set == 0)");
            rep.check(cs.ss_hints == fh, format!("ss_hints {} == the hash of the file", hex(&fh)));
        }
    }
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
        ss_set: 0,
        ss_acc: [0; 16],
        ss_hints: [0; 32],
        smt_root: empty_smt_root(),
        max_time: img.genesis_time,
        profile: 0,
        block_roots: vec![Some(block_leaf(&tip, 0, &utxo_state(0, 0, &[])))],
        block_leaves: 1,
        lineage: [0; 32],
    }
}

/// dsha256 of the state's journal encoding: what the guest commits as `anchor_id` after a base step on it.
pub fn anchor_id(s: &ChainState) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    let words = risc0_zkvm::serde::to_vec(s).expect("ChainState serialises");
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

/// Decode a hints file, require it to be canonically encoded, and return (stop height, ss_hints).
pub fn hints_file(raw: &[u8]) -> Result<(u32, [u8; 32]), String> {
    use hintsfile::{EliasFano, Hintsfile, HintsfileBuilder};
    let file = Hintsfile::from_reader(&mut &raw[..]).map_err(|e| format!("{e:?}"))?;
    let mut canon = Vec::new();
    let mut h = [0u8; 32];
    {
        let mut b = HintsfileBuilder::new(&mut canon).initialize(file.stop_height()).map_err(|e| format!("{e:?}"))?;
        for height in 1..=file.stop_height() {
            let idx = file.indices_at_height(height).ok_or_else(|| format!("height {height}: missing"))?;
            if !idx.windows(2).all(|p| p[0] < p[1]) {
                return Err(format!("height {height}: list is not strictly ascending"));
            }
            h = libzync::ss_hints::step(&h, height, &idx);
            b.append(EliasFano::compress(&idx)).map_err(|e| format!("{e:?}"))?;
        }
        b.finish().map_err(|e| format!("{e:?}"))?;
    }
    if canon != raw {
        return Err("not canonically encoded".into());
    }
    Ok((file.stop_height(), h))
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
    if s.len() != 64 {
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

#[cfg(test)]
mod tests {
    use super::*;

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

/// A proof bundle (`docs/libzync/PROOF_BUNDLE.md`): one file that travels with a hints file. Its header is a
/// convenience for people and indexes, NEVER trusted: [`check_bundle`] requires every field to equal what the
/// verified receipt and the actual file say.
#[derive(Clone, Debug, PartialEq)]
pub struct Bundle {
    pub network: u8,
    pub image: [u8; 32],
    pub stop_height: u32,
    pub tip_hash: [u8; 32],
    pub cum_work: [u8; 32],
    pub ss_hints: [u8; 32],
    pub hints_sha256: [u8; 32],
    pub receipt: Vec<u8>,
}

pub const BUNDLE_MAGIC: &[u8; 4] = b"SSPF";
pub const BUNDLE_VERSION: u8 = 1;
const BUNDLE_HEADER: usize = 4 + 1 + 1 + 32 + 4 + 32 + 32 + 32 + 32 + 4;

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
        out.extend_from_slice(&self.ss_hints);
        out.extend_from_slice(&self.hints_sha256);
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
            ss_hints: a32(106),
            hints_sha256: a32(138),
            receipt: b[BUNDLE_HEADER..].to_vec(),
        })
    }

    /// Build a bundle from a receipt and its hints file; fails unless the receipt verifies (OK or DEMO) for `img`.
    pub fn make(receipt: &[u8], hints: &[u8], img: &'static Image, anchor: Option<ChainState>, now: u64) -> Result<Bundle, String> {
        use sha2::{Digest as _, Sha256};
        let rep = verify(&Inputs { receipt, hints, image: img, anchor: anchor.clone(), now });
        if img.profile & PROFILE_COINLOG != 0 {
            return Err("a bundle carries a hints file; a coin-log proof has none".into());
        }
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
            ss_hints: cs.ss_hints,
            hints_sha256: Sha256::digest(hints).into(),
            receipt: receipt.to_vec(),
        })
    }
}

/// Verify a bundle: the receipt as [`verify`] does, then every header field against the result. The image and
/// network come from the VERIFIER (`img`), never from the bundle.
pub fn check_bundle(bundle: &Bundle, hints: &[u8], img: &'static Image, anchor: Option<ChainState>, now: u64) -> Report {
    use sha2::{Digest as _, Sha256};
    let mut rep = verify(&Inputs { receipt: &bundle.receipt, hints, image: img, anchor, now });
    rep.check(Some(bundle.network) == network_code(img.network), "bundle header: network is this verifier's");
    rep.check(hex32(img.method_id_hex) == Some(bundle.image), format!("bundle header: image {} is this verifier's", hex(&bundle.image)));
    let digest: [u8; 32] = Sha256::digest(hints).into();
    rep.check(digest == bundle.hints_sha256, "bundle header: sha256 of the hints file matches");
    if let Some(cs) = &rep.state {
        let same = cs.height == bundle.stop_height && cs.tip_hash == bundle.tip_hash && cs.cum_work == bundle.cum_work
            && cs.ss_hints == bundle.ss_hints;
        rep.check(same, "bundle header: height, tip, work and ss_hints equal the verified journal's");
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
                         ss_hints: [3; 32], hints_sha256: [4; 32], receipt: vec![9; 100] };
        let e = b.encode();
        assert_eq!(e.len(), BUNDLE_HEADER + 100);
        assert_eq!(Bundle::decode(&e).unwrap(), b);
        assert!(Bundle::decode(&e[..e.len() - 1]).is_err());
        let mut v = e.clone();
        v[4] = 2;
        assert!(Bundle::decode(&v).is_err());
    }
}
