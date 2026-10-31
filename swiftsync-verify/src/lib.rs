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
    },
    Image {
        network: "signet",
        method_id_hex: "067822b958f7cc86d67ad9f37153b99c29664072a8e90a79a2d9af4dc71f5b36",
        profile: PROFILE_LIBZYNC | PROFILE_SIGNET,
        genesis_hash_hex: "00000008819873e925422c1ff0f99f7cc9bbb232af63a077a480a3633bee1ef6",
        genesis_time: 1_598_918_400,
        genesis_bits: 0x1e03_77ae,
        genesis_work: 4_838_420,
    },
];

pub const KIND_CHAIN: u32 = 0xC4A1_0002;
pub const PROFILE_LIBZYNC: u32 = 1;
pub const PROFILE_ASSUMEVALID: u32 = 2;
pub const PROFILE_SIGNET: u32 = 4;
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
    pub fn method_id(&self) -> Digest {
        Digest::from_bytes(hex32(self.method_id_hex).expect("IMAGES holds valid hex"))
    }
}

/// The libzync guest's committed state, field for field and in order (§3 of the proposal). The journal is
/// risc0's serde encoding of exactly this struct.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ChainState {
    pub kind: u32,
    pub tip_hash: [u8; 32],
    pub utxo_roots: Vec<Option<[u8; 32]>>,
    pub utxo_leaves: u64,
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
    let mut rep = Report { checks: Vec::new(), state: None, verdict: Verdict::Fail };
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
    rep.check(cs.kind == KIND_CHAIN && cs.self_id == words(&m),
              "journal is a ChainState committed by this image (kind, self_id)");

    // Step 9: what the image checked.
    rep.check(cs.profile == inp.image.profile, format!("profile 0x{:x} == 0x{:x} ({})", cs.profile,
              inp.image.profile, describe_profile(cs.profile)));

    // Step 11: time-too-new on this verifier's clock.
    rep.check(cs.max_time as u64 <= inp.now + MAX_FUTURE_BLOCK_TIME, format!(
        "time-too-new: largest block time {} <= this clock {} + {} s", cs.max_time, inp.now, MAX_FUTURE_BLOCK_TIME));

    // Step 3: anchored at genesis (or at the supplied state, which can only ever give Demo).
    let (want, what) = match &inp.anchor {
        None => (genesis_anchor(inp.image), "genesis".to_string()),
        Some(a) => (a.clone(), format!("the GIVEN anchor at height {} -- NOT genesis", a.height)),
    };
    rep.check(cs.anchor_id == anchor_id(&want), format!("anchor_id is {what}"));

    // Step 10, then 4-6: the file.
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
    rep.verdict = if rep.failed() {
        Verdict::Fail
    } else if inp.anchor.is_some() {
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
    format!("{}, {}, {}",
            if p & PROFILE_LIBZYNC != 0 { "libzync" } else { "canonical" },
            if p & PROFILE_ASSUMEVALID != 0 { "ASSUMEVALID: scripts and signatures NOT checked" } else { "scripts checked" },
            if p & PROFILE_SIGNET != 0 { "signet" } else { "mainnet" })
}

/// The network's genesis ChainState for this image (§5 step 3).
pub fn genesis_anchor(img: &Image) -> ChainState {
    let mut tip = hex32(img.genesis_hash_hex).expect("IMAGES holds valid hex");
    tip.reverse();
    ChainState {
        kind: KIND_CHAIN,
        tip_hash: tip,
        utxo_roots: vec![None],
        utxo_leaves: 0,
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
        self_id: words(&img.method_id()),
        ss_set: 0,
        ss_acc: [0; 16],
        ss_hints: [0; 32],
        smt_root: empty_smt_root(),
        max_time: img.genesis_time,
        profile: 0,
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

#[cfg(test)]
mod tests {
    use super::*;

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
        // SWIFTSYNC_PROOFS.md §5 step 3, round-8 images; also checked by scripts/libzync/genesis_anchor_id.py.
        let a = |n| hex(&anchor_id(&genesis_anchor(Image::by_network(n).unwrap())));
        assert_eq!(a("mainnet"), "5b2020ae48fdf2d180837b612b13c2b36192208f4afb94dee350553fa7ea1d44");
        assert_eq!(a("signet"), "636271d3f46ea120f22da161648247bdd004afba1cb92378ba3af407c9e4e2b2");
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
        let rep = verify(&Inputs { receipt, hints, image: img, anchor, now });
        if rep.verdict == Verdict::Fail {
            return Err("the receipt does not verify; refusing to bundle it".into());
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
