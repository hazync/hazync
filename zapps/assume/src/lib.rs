//! The assumevalid and assumeutxo checkers (docs/libzync/ASSUME_ZAPPS.md, issue #721). No new guest: both read a
//! verified core chain proof.
//!
//! - **assumevalid**: a core proof to H with scripts checked (profile bit 1 clear) says every script of every block
//!   0..H passed Core's rules. [`assume_valid`] names the block a node may give as `-assumevalid`: the proven tip, or
//!   an ancestor of it shown by a header chain that ends at the proven tip ([`HeaderChain::bind`]).
//! - **assumeutxo**: the proof commits the live-coin multiset hash (ECMH, profile bit 6) at its tip. [`check_snapshot`]
//!   hashes a Core `dumptxoutset` file into the same ECMH and compares. A Core coin lacks two fields of the coin leaf
//!   (UTREEXO_FORMAT.md §2): the creation MTP, computed from the header chain, and the BIP30 tracked bit, from a list
//!   of outpoints. Both are side inputs that can only make a true snapshot fail, never a false one pass: the check is
//!   equality of two multisets of leaves, and every snapshot field is in the leaf preimage.
//!
//! Every function here is total on hostile input: errors, never panics.
//!
//! A zAPP module of its own (docs/libzync/ZYNCAPPS.md §9), on the core verifier (`swiftsync_verify`) as a library and
//! not part of it. Until 15.11.2026 this was `swiftsync_verify::assume`, the core CLI's `assumevalid` / `assumeutxo`, the
//! C ABI's `swiftsync_verify_assume` and the WASM's `verifyAssume`; the checks are unchanged.

use swiftsync_verify::{hex, ChainState, PROFILE_ASSUMEVALID, PROFILE_LIBZYNC, PROFILE_SIGNET, PROFILE_UTXOHASH};
use ark_ff::fields::{Fp256, MontBackend, MontConfig};
use ark_ff::{AdditiveGroup, BigInt, BigInteger, Field, PrimeField, Zero};
use sha2::{Digest as _, Sha256};
use std::collections::HashSet;
use std::io::Read;

// ---------------------------------------------------------------------------------------------------------------
// secp256k1's base field and the ECMH map (prover/methods/guest/src/swiftsync.rs, libzync/src/host.rs ecmh_hint)

#[derive(MontConfig)]
#[modulus = "115792089237316195423570985008687907853269984665640564039457584007908834671663"]
#[generator = "3"]
pub struct FqConfig;
/// secp256k1's base field, p = 2^256 - 2^32 - 977.
pub type Fq = Fp256<MontBackend<FqConfig, 4>>;

fn fq_from_be(b: &[u8; 32]) -> Option<Fq> {
    let mut l = [0u64; 4];
    for k in 0..4 {
        l[k] = u64::from_be_bytes(b[24 - 8 * k..32 - 8 * k].try_into().unwrap());
    }
    Fq::from_bigint(BigInt::new(l))
}

fn fq_to_be(x: &Fq) -> [u8; 32] {
    let l = x.into_bigint().0;
    let mut b = [0u8; 32];
    for k in 0..4 {
        b[24 - 8 * k..32 - 8 * k].copy_from_slice(&l[k].to_be_bytes());
    }
    b
}

fn fq_is_odd(x: &Fq) -> bool {
    x.into_bigint().is_odd()
}

fn sq_n(mut a: Fq, n: u32) -> Fq {
    for _ in 0..n {
        a.square_in_place();
    }
    a
}

/// a^((p+1)/4), a square root of a whenever one exists (p = 3 mod 4), by libsecp256k1's addition chain
/// (`secp256k1_fe_sqrt`: 253 squarings, 13 multiplications).
fn sqrt_candidate(a: &Fq) -> Fq {
    let x2 = a.square() * a;
    let x3 = x2.square() * a;
    let x6 = sq_n(x3, 3) * x3;
    let x9 = sq_n(x6, 3) * x3;
    let x11 = sq_n(x9, 2) * x2;
    let x22 = sq_n(x11, 11) * x11;
    let x44 = sq_n(x22, 22) * x22;
    let x88 = sq_n(x44, 44) * x44;
    let x176 = sq_n(x88, 88) * x88;
    let x220 = sq_n(x176, 44) * x44;
    let x223 = sq_n(x220, 3) * x3;
    let t = sq_n(x223, 23) * x22;
    let t = sq_n(t, 6) * x2;
    sq_n(t, 2)
}

/// y with y^2 = x^3 + 7 and the given parity, if x is on the curve: the candidate root, then a check of it.
fn lift_x(x: &Fq, odd: bool) -> Option<Fq> {
    let rhs = x.square() * x + Fq::from(7u64);
    let y = sqrt_candidate(&rhs);
    if y.square() != rhs {
        return None;
    }
    Some(if fq_is_odd(&y) == odd { y } else { -y })
}

/// The ECMH point of a 32-byte coin leaf: the first ctr = 0, 1, ... with x = SHA-256(leaf ‖ ctr u32 LE) (big-endian)
/// on the curve, and its EVEN y. The guest refuses x >= p (probability ~2^-224); so does this.
pub fn ecmh_point(leaf: &[u8; 32]) -> Result<(Fq, Fq), String> {
    for ctr in 0u32.. {
        let mut m = [0u8; 36];
        m[..32].copy_from_slice(leaf);
        m[32..].copy_from_slice(&ctr.to_le_bytes());
        let h: [u8; 32] = Sha256::digest(m).into();
        let x = fq_from_be(&h).ok_or("ECMH: hash >= p")?;
        if let Some(y) = lift_x(&x, false) {
            return Ok((x, y));
        }
    }
    Err("ECMH: no counter maps to the curve".into())
}

/// A running sum of points, in Jacobian coordinates (a = 0). `z == 0` is the identity.
#[derive(Clone, Debug, PartialEq)]
pub struct Ecmh {
    x: Fq,
    y: Fq,
    z: Fq,
    pub count: u64,
}

impl Default for Ecmh {
    fn default() -> Self {
        Ecmh { x: Fq::zero(), y: Fq::from(1u64), z: Fq::zero(), count: 0 }
    }
}

impl Ecmh {
    fn double(&mut self) {
        if self.z.is_zero() || self.y.is_zero() {
            *self = Ecmh { count: self.count, ..Ecmh::default() };
            return;
        }
        let a = self.x.square();
        let b = self.y.square();
        let c = b.square();
        let d = ((self.x + b).square() - a - c).double();
        let e = a.double() + a;
        let x3 = e.square() - d.double();
        let y3 = e * (d - x3) - c.double().double().double();
        let z3 = (self.y * self.z).double();
        self.x = x3;
        self.y = y3;
        self.z = z3;
    }

    /// Adds an affine point (x2, y2) on the curve.
    pub fn add_affine(&mut self, x2: &Fq, y2: &Fq) {
        self.count += 1;
        if self.z.is_zero() {
            self.x = *x2;
            self.y = *y2;
            self.z = Fq::from(1u64);
            return;
        }
        let z1z1 = self.z.square();
        let u2 = *x2 * z1z1;
        let s2 = *y2 * self.z * z1z1;
        let h = u2 - self.x;
        let r = s2 - self.y;
        if h.is_zero() {
            if r.is_zero() {
                self.double();
            } else {
                let c = self.count;
                *self = Ecmh { count: c, ..Ecmh::default() };
            }
            return;
        }
        let hh = h.square();
        let hhh = h * hh;
        let v = self.x * hh;
        let x3 = r.square() - hhh - v.double();
        let y3 = r * (v - x3) - self.y * hhh;
        self.z *= h;
        self.x = x3;
        self.y = y3;
    }

    /// Adds another sum (through its affine form; one inversion).
    pub fn add_sum(&mut self, o: &Ecmh) {
        let c = self.count + o.count;
        if !o.z.is_zero() {
            let zi = o.z.inverse().expect("z != 0");
            let zi2 = zi.square();
            self.add_affine(&(o.x * zi2), &(o.y * zi2 * zi));
        }
        self.count = c;
    }

    /// Adds a coin leaf's point.
    pub fn add_leaf(&mut self, leaf: &[u8; 32]) -> Result<(), String> {
        let (x, y) = ecmh_point(leaf)?;
        self.add_affine(&x, &y);
        Ok(())
    }

    /// The sum as `ChainState` commits it: `(0, zeros)` for the identity, else `(1, x limbs ‖ y limbs)`, each
    /// coordinate as 8 u32 words, least significant first (risc0 BigInt, `swiftsync.rs` `point`).
    pub fn committed(&self) -> (u32, [u32; 16]) {
        if self.z.is_zero() {
            return (0, [0; 16]);
        }
        let zi = self.z.inverse().expect("z != 0");
        let zi2 = zi.square();
        let (x, y) = (self.x * zi2, self.y * zi2 * zi);
        let mut w = [0u32; 16];
        for (k, c) in [x, y].iter().enumerate() {
            let l = c.into_bigint().0;
            for j in 0..4 {
                w[8 * k + 2 * j] = l[j] as u32;
                w[8 * k + 2 * j + 1] = (l[j] >> 32) as u32;
            }
        }
        (1, w)
    }
}

/// A committed live hash as two big-endian hex coordinates (x, y), the way `scripts/libzync/assume_ref.py` prints them.
pub fn live_hash_hex(w: &[u32; 16]) -> (String, String) {
    let be = |l: &[u32]| l.iter().rev().map(|v| format!("{v:08x}")).collect::<String>();
    (be(&w[..8]), be(&w[8..]))
}

// ---------------------------------------------------------------------------------------------------------------
// Coin leaf (UTREEXO_FORMAT.md §2)

/// A coin as Core's snapshot holds it, with the script decompressed.
#[derive(Clone, Debug, PartialEq)]
pub struct SnapCoin {
    /// internal byte order
    pub txid: [u8; 32],
    pub vout: u32,
    pub height: u32,
    pub coinbase: bool,
    pub value: u64,
    pub script_pubkey: Vec<u8>,
}

/// The coin leaf: SHA-256(0x00 ‖ txid ‖ vout ‖ value ‖ len ‖ scriptPubKey ‖ height ‖ flags ‖ creation_mtp).
pub fn coin_leaf(c: &SnapCoin, tracked: bool, creation_mtp: u32) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update([0u8]);
    h.update(c.txid);
    h.update(c.vout.to_le_bytes());
    h.update(c.value.to_le_bytes());
    h.update((c.script_pubkey.len() as u32).to_le_bytes());
    h.update(&c.script_pubkey);
    h.update(c.height.to_le_bytes());
    h.update([c.coinbase as u8 | (tracked as u8) << 1]);
    h.update(creation_mtp.to_le_bytes());
    h.finalize().into()
}

// ---------------------------------------------------------------------------------------------------------------
// Headers

fn dsha(b: &[u8]) -> [u8; 32] {
    Sha256::digest(Sha256::digest(b)).into()
}

/// A run of consecutive headers that ends at the proven tip, so every one is the proven chain's block at its height
/// (each header's `prev` is the hash of the one before it; the last one hashes to `tip_hash`).
#[derive(Debug)]
pub struct HeaderChain {
    pub first_height: u32,
    /// block hashes, internal byte order, heights first_height..=tip
    pub hashes: Vec<[u8; 32]>,
    pub times: Vec<u32>,
}

impl HeaderChain {
    /// `bytes`: 80-byte headers, oldest first, the last one the proven tip at `tip_height`.
    pub fn bind(bytes: &[u8], tip_hash: &[u8; 32], tip_height: u32) -> Result<HeaderChain, String> {
        if bytes.is_empty() || bytes.len() % 80 != 0 {
            return Err(format!("headers: {} bytes is not a positive multiple of 80", bytes.len()));
        }
        let n = bytes.len() / 80;
        if n as u64 > tip_height as u64 + 1 {
            return Err(format!("headers: {n} headers, more than the {} blocks 0..{tip_height}", tip_height as u64 + 1));
        }
        let mut hashes = Vec::with_capacity(n);
        let mut times = Vec::with_capacity(n);
        for (i, hd) in bytes.chunks_exact(80).enumerate() {
            if i > 0 && hd[4..36] != hashes[i - 1] {
                return Err(format!("headers: header {} (height {}) does not link to the one before it",
                                   i, tip_height as usize + 1 - n + i));
            }
            hashes.push(dsha(hd));
            times.push(u32::from_le_bytes(hd[68..72].try_into().unwrap()));
        }
        if hashes[n - 1] != *tip_hash {
            return Err(format!("headers: the last header hashes to {}, not the proven tip {}",
                               display(&hashes[n - 1]), display(tip_hash)));
        }
        // n <= tip_height + 1 (checked above, in u64), so this cannot wrap even at tip_height = u32::MAX
        let first_height = u32::try_from(tip_height as u64 + 1 - n as u64).map_err(|_| "headers: height out of range".to_string())?;
        Ok(HeaderChain { first_height, hashes, times })
    }

    pub fn tip_height(&self) -> u32 {
        self.first_height + self.hashes.len() as u32 - 1
    }

    /// The height of block `hash` (internal order) in this run.
    pub fn height_of(&self, hash: &[u8; 32]) -> Option<u32> {
        self.hashes.iter().position(|h| h == hash).map(|i| self.first_height + i as u32)
    }

    /// The median-time-past before block `h`: the median of the timestamps of blocks max(0, h-11)..h-1 (Core's
    /// `GetMedianTimePast` of h's parent), the coin leaf's `creation_mtp`. Needs those headers in the run.
    pub fn mtp_before(&self, h: u32) -> Result<u32, String> {
        if h == 0 || h > self.tip_height() {
            return Err(format!("no MTP before height {h} (headers {}..{})", self.first_height, self.tip_height()));
        }
        let lo = h.saturating_sub(11);
        if lo < self.first_height {
            return Err(format!("the MTP before height {h} needs the header at {lo}; the headers start at {}", self.first_height));
        }
        let mut t: Vec<u32> = self.times[(lo - self.first_height) as usize..(h - self.first_height) as usize].to_vec();
        t.sort_unstable();
        Ok(t[t.len() / 2])
    }
}

/// As `getblockhash` prints it.
pub fn display(h: &[u8; 32]) -> String {
    let mut r = *h;
    r.reverse();
    hex(&r)
}

/// A block hash in display order (as `getblockhash` prints it) to internal order.
pub fn parse_display_hash(s: &str) -> Option<[u8; 32]> {
    let mut h = swiftsync_verify::hex32(&s.trim().to_lowercase())?;
    h.reverse();
    Some(h)
}

// ---------------------------------------------------------------------------------------------------------------
// assumevalid

/// What [`assume_valid`] establishes.
#[derive(Debug, PartialEq)]
pub struct AssumeValid {
    /// the block a node may give as `-assumevalid` (internal order)
    pub block: [u8; 32],
    pub height: u32,
    /// the proof's tip height: every block 0..=proven_to passed Core's script rules
    pub proven_to: u32,
}

/// A verified core state `cs` with scripts checked says every block 0..H passed Core's script and signature rules.
/// `block`: None = the tip. Otherwise it must be the tip, or a block of `headers`, which must end at the tip.
/// Call ONLY with the state of a receipt that verified.
pub fn assume_valid(cs: &ChainState, headers: Option<&HeaderChain>, block: Option<[u8; 32]>) -> Result<AssumeValid, String> {
    if cs.profile & PROFILE_LIBZYNC == 0 {
        return Err(format!("profile 0x{:x} is not a libzync image", cs.profile));
    }
    if cs.profile & PROFILE_ASSUMEVALID != 0 {
        return Err("the proving image SKIPPED scripts and signatures (profile bit 1): it proves nothing for -assumevalid".into());
    }
    let x = block.unwrap_or(cs.tip_hash);
    if x == cs.tip_hash {
        return Ok(AssumeValid { block: x, height: cs.height, proven_to: cs.height });
    }
    let Some(hc) = headers else {
        return Err(format!("block {} is not the proven tip {}: give the headers from it to the tip", display(&x), display(&cs.tip_hash)));
    };
    if hc.tip_height() != cs.height || hc.hashes.last() != Some(&cs.tip_hash) {
        return Err("the headers do not end at the proven tip".into());
    }
    match hc.height_of(&x) {
        Some(h) => Ok(AssumeValid { block: x, height: h, proven_to: cs.height }),
        None => Err(format!("block {} is not in the headers {}..{} that end at the proven tip", display(&x), hc.first_height, hc.tip_height())),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Core's snapshot format (src/node/utxo_snapshot.h SnapshotMetadata; src/rpc/blockchain.cpp WriteUTXOSnapshot;
// src/coins.h Coin; src/compressor.h TxOutCompression / ScriptCompression / AmountCompression; src/serialize.h)

pub const SNAPSHOT_MAGIC: [u8; 5] = *b"utxo\xff";
pub const SNAPSHOT_VERSION: u16 = 2;
/// Core's MAX_SCRIPT_SIZE.
const MAX_SCRIPT_SIZE: u64 = 10_000;
/// Core's MAX_SIZE for CompactSize (0x02000000).
const MAX_SIZE: u64 = 0x0200_0000;
/// Core's MAX_MONEY.
const MAX_MONEY: u64 = 21_000_000 * 100_000_000;

/// The network's message start, which the snapshot names.
pub fn network_magic(profile: u32) -> [u8; 4] {
    if profile & PROFILE_SIGNET != 0 { [0x0a, 0x03, 0xcf, 0x40] } else { magic_not_signet(profile) }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotMeta {
    pub version: u16,
    pub network_magic: [u8; 4],
    /// internal order
    pub base_blockhash: [u8; 32],
    pub coins_count: u64,
}

/// A reader that hashes nothing and counts bytes, with Core's integer encodings.
struct Src<R: Read> {
    r: R,
    pos: u64,
}

impl<R: Read> Src<R> {
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let mut b = [0u8; N];
        self.r.read_exact(&mut b).map_err(|e| format!("snapshot: truncated at byte {} ({e})", self.pos))?;
        self.pos += N as u64;
        Ok(b)
    }
    fn vec(&mut self, n: usize) -> Result<Vec<u8>, String> {
        let mut v = vec![0u8; n];
        self.r.read_exact(&mut v).map_err(|e| format!("snapshot: truncated at byte {} ({e})", self.pos))?;
        self.pos += n as u64;
        Ok(v)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.bytes::<1>()?[0])
    }
    /// Core's ReadCompactSize with range_check: canonical, at most MAX_SIZE.
    fn compact_size(&mut self) -> Result<u64, String> {
        let at = self.pos;
        let c = self.u8()?;
        let (v, min) = match c {
            0..=252 => (c as u64, 0),
            253 => (u16::from_le_bytes(self.bytes()?) as u64, 253),
            254 => (u32::from_le_bytes(self.bytes()?) as u64, 0x1_0000),
            255 => (u64::from_le_bytes(self.bytes()?), 0x1_0000_0000),
        };
        if v < min {
            return Err(format!("snapshot: non-canonical CompactSize at byte {at}"));
        }
        if v > MAX_SIZE {
            return Err(format!("snapshot: CompactSize {v} at byte {at} exceeds MAX_SIZE"));
        }
        Ok(v)
    }
    /// Core's VARINT (VarIntMode::DEFAULT, MSB base-128 with the +1 offset) into a u64 range of `max`.
    fn varint(&mut self, max: u64) -> Result<u64, String> {
        let at = self.pos;
        let mut n: u64 = 0;
        loop {
            let ch = self.u8()?;
            if n > (max >> 7) {
                return Err(format!("snapshot: VARINT at byte {at} too large"));
            }
            n = (n << 7) | (ch & 0x7f) as u64;
            if ch & 0x80 != 0 {
                if n == max {
                    return Err(format!("snapshot: VARINT at byte {at} too large"));
                }
                n += 1;
            } else {
                return Ok(n);
            }
        }
    }
}

/// Core's WriteVarInt (VarIntMode::DEFAULT) encoding of `n`: the bytes of the coins-database key's vout.
pub fn varint_bytes(mut n: u64) -> Vec<u8> {
    let mut t = Vec::with_capacity(10);
    loop {
        t.push((n & 0x7f) as u8 | if t.is_empty() { 0 } else { 0x80 });
        if n <= 0x7f { break }
        n = (n >> 7) - 1;
    }
    t.reverse();
    t
}

/// Core's DecompressAmount.
pub fn decompress_amount(mut x: u64) -> u64 {
    if x == 0 {
        return 0;
    }
    x -= 1;
    let mut e = x % 10;
    x /= 10;
    let mut n = if e < 9 {
        let d = (x % 9) + 1;
        x /= 9;
        x.wrapping_mul(10).wrapping_add(d)
    } else {
        x.wrapping_add(1)
    };
    while e > 0 {
        n = n.wrapping_mul(10);
        e -= 1;
    }
    n
}

/// Core's ScriptCompression::Unser + DecompressScript. A special type 4/5 whose x is not on the curve is refused
/// (Core's DecompressScript fails and leaves the script empty; no such coin can be in a real UTXO set: Core only
/// compresses a key that passes IsFullyValid).
fn read_script<R: Read>(s: &mut Src<R>) -> Result<Vec<u8>, String> {
    let at = s.pos;
    let n = s.varint(u32::MAX as u64)?;
    match n {
        0 => {
            let h = s.bytes::<20>()?;
            Ok([&[0x76, 0xa9, 20][..], &h, &[0x88, 0xac]].concat())
        }
        1 => {
            let h = s.bytes::<20>()?;
            Ok([&[0xa9, 20][..], &h, &[0x87]].concat())
        }
        2 | 3 => {
            let x = s.bytes::<32>()?;
            Ok([&[33, n as u8][..], &x, &[0xac]].concat())
        }
        4 | 5 => {
            let xb = s.bytes::<32>()?;
            let x = fq_from_be(&xb).ok_or_else(|| format!("snapshot: compressed key at byte {at}: x >= p"))?;
            let y = lift_x(&x, n == 5).ok_or_else(|| format!("snapshot: compressed key at byte {at} is not on the curve"))?;
            Ok([&[65, 4][..], &xb, &fq_to_be(&y), &[0xac]].concat())
        }
        _ => {
            let len = n - 6;
            if len > MAX_SCRIPT_SIZE {
                // Core reads this as OP_RETURN, an unspendable coin: never in a UTXO set
                return Err(format!("snapshot: script of {len} bytes at byte {at} (over MAX_SCRIPT_SIZE: unspendable)"));
            }
            s.vec(len as usize)
        }
    }
}

/// Reads a snapshot, calling `f` for every coin in file order. Refuses what Core's loadtxoutset refuses (magic,
/// version, a coin above the base height is the caller's check), a count that disagrees with the coins present,
/// and any byte after the last coin; and, stricter than Core's loader, coins out of Core's database key order (the
/// order `hash_serialized_3` is taken in), a special-type 4/5 key off the curve, and a raw script over 10,000 bytes.
pub fn read_snapshot<R: Read>(r: R, f: impl FnMut(&SnapCoin) -> Result<(), String>) -> Result<SnapshotMeta, String> {
    read_snapshot_with(r, |_| Ok(()), f)
}

/// [`read_snapshot`], with `meta` called on the header before any coin is read (to refuse a wrong base block or
/// network without reading the whole file).
pub fn read_snapshot_with<R: Read>(r: R, meta: impl FnOnce(&SnapshotMeta) -> Result<(), String>,
                                   mut f: impl FnMut(&SnapCoin) -> Result<(), String>) -> Result<SnapshotMeta, String> {
    let mut s = Src { r, pos: 0 };
    let magic = s.bytes::<5>()?;
    if magic != SNAPSHOT_MAGIC {
        return Err("snapshot: not a Core UTXO snapshot (magic)".into());
    }
    let version = u16::from_le_bytes(s.bytes()?);
    if version != SNAPSHOT_VERSION {
        return Err(format!("snapshot: version {version}; this reader knows version {SNAPSHOT_VERSION} (Core 28 to 31)"));
    }
    let network_magic = s.bytes::<4>()?;
    let base_blockhash = s.bytes::<32>()?;
    let coins_count = u64::from_le_bytes(s.bytes()?);
    let m = SnapshotMeta { version, network_magic, base_blockhash, coins_count };
    meta(&m)?;
    let meta = m;
    let mut left = coins_count;
    // Core writes the coins in its database's key order ('C' ‖ txid ‖ VARINT(vout), compared bytewise), one group per
    // txid, and hash_serialized_3 is taken in that order. The ECMH does not depend on it, but the printed hash does, so
    // the order is required: txids strictly ascending (which also makes each group the txid's only one), and within a
    // group the ENCODED vouts strictly ascending (not numerically: 16,511 = ff7f sorts after 16,512 = 808000).
    let mut last_txid: Option<[u8; 32]> = None;
    while left > 0 {
        let txid = s.bytes::<32>()?;
        if let Some(p) = last_txid {
            if txid <= p {
                return Err(format!("snapshot: txid {} after {}: not Core's key order (txids ascending, one group each)",
                                   display(&txid), display(&p)));
            }
        }
        last_txid = Some(txid);
        let k = s.compact_size()?;
        if k == 0 || k > left {
            return Err(format!("snapshot: {k} coins for txid {} with {left} coins left", display(&txid)));
        }
        let mut last_vout: Option<Vec<u8>> = None;
        for _ in 0..k {
            let vout = s.compact_size()?;
            if vout >= u32::MAX as u64 {
                return Err(format!("snapshot: vout {vout} out of range"));
            }
            let key = varint_bytes(vout);
            if last_vout.as_ref().is_some_and(|p| key <= *p) {
                return Err(format!("snapshot: txid {} vout {vout}: not Core's key order (encoded vouts ascending)", display(&txid)));
            }
            last_vout = Some(key);
            let code = s.varint(u32::MAX as u64)?;
            let value = decompress_amount(s.varint(u64::MAX)?);
            if value > MAX_MONEY {
                return Err(format!("snapshot: amount {value} outside MoneyRange"));
            }
            let script_pubkey = read_script(&mut s)?;
            f(&SnapCoin { txid, vout: vout as u32, height: (code >> 1) as u32, coinbase: code & 1 == 1, value, script_pubkey })?;
        }
        left -= k;
    }
    let mut extra = [0u8; 1];
    match s.r.read(&mut extra) {
        Ok(0) => Ok(meta),
        Ok(_) => Err(format!("snapshot: bytes after the {coins_count} coins the header names")),
        Err(e) => Err(format!("snapshot: read error after the last coin ({e})")),
    }
}

/// Core's `hash_serialized_3` over coins in the order given (TxOutSer: outpoint ‖ u32 (height << 1 | coinbase) ‖
/// CTxOut), SHA-256d. Core hashes in its database's order, which is the snapshot's.
#[derive(Default)]
pub struct HashSerialized3(Sha256);

impl HashSerialized3 {
    pub fn add(&mut self, c: &SnapCoin) {
        self.0.update(c.txid);
        self.0.update(c.vout.to_le_bytes());
        self.0.update(((c.height << 1) | c.coinbase as u32).to_le_bytes());
        self.0.update(c.value.to_le_bytes());
        let n = c.script_pubkey.len();
        match n {
            0..=252 => self.0.update([n as u8]),
            253..=0xffff => { self.0.update([253]); self.0.update((n as u16).to_le_bytes()) }
            _ => { self.0.update([254]); self.0.update((n as u32).to_le_bytes()) }
        }
        self.0.update(&c.script_pubkey);
    }
    /// display order, as `gettxoutsetinfo` / `dumptxoutset` print it
    pub fn finish(self) -> String {
        let d: [u8; 32] = Sha256::digest(self.0.finalize()).into();
        display(&d)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// assumeutxo

/// Outpoints whose coin leaf has flag bit 1 (tracked by the BIP30 tree, UTREEXO_FORMAT.md §2a).
pub type Tracked = HashSet<([u8; 32], u32)>;

/// `<txid in display order>:<vout>` per line; `#` comments and blank lines ignored.
pub fn parse_tracked(text: &str) -> Result<Tracked, String> {
    let mut t = Tracked::new();
    for (i, l) in text.lines().enumerate() {
        let l = l.split('#').next().unwrap_or("").trim();
        if l.is_empty() {
            continue;
        }
        let bad = || format!("tracked list line {}: want <txid>:<vout>", i + 1);
        let (a, b) = l.split_once(':').ok_or_else(bad)?;
        let txid = parse_display_hash(a).ok_or_else(bad)?;
        let vout: u32 = b.trim().parse().map_err(|_| bad())?;
        t.insert((txid, vout));
    }
    Ok(t)
}

#[derive(Debug)]
pub struct SnapshotCheck {
    pub meta: SnapshotMeta,
    pub coins: u64,
    pub value: u64,
    /// Core's hash_serialized_3 of the snapshot (display order): the value a node pins in `m_assumeutxo_data`
    pub hash_serialized_3: String,
    pub tracked_used: usize,
    pub live_set: u32,
    pub live_hash: [u32; 16],
}

/// Hashes a snapshot into the live-coin ECMH and Core's hash_serialized_3, with no proof: the work of
/// [`check_snapshot`] before its comparisons. `base_height` bounds the coins' heights.
pub fn hash_snapshot<R: Read>(snap: R, headers: &HeaderChain, tracked: &Tracked, base_height: u32) -> Result<SnapshotCheck, String> {
    hash_snapshot_with(snap, headers, tracked, base_height, |_| Ok(()))
}

fn hash_snapshot_with<R: Read>(snap: R, headers: &HeaderChain, tracked: &Tracked, base_height: u32,
                               on_meta: impl FnOnce(&SnapshotMeta) -> Result<(), String>) -> Result<SnapshotCheck, String> {
    let mut h3 = HashSerialized3::default();
    let (mut value, mut tracked_used) = (0u64, 0usize);
    let mut mtp_cache: Vec<Option<u32>> = Vec::new();
    let mut sum = PointSum::new();
    let meta = read_snapshot_with(snap, on_meta, |c| {
        if c.height == 0 || c.height > base_height {
            return Err(format!("snapshot: coin {}:{} at height {}, outside 1..{base_height}", display(&c.txid), c.vout, c.height));
        }
        // before the cache grows: a coin's height must be one the headers cover (no allocation driven by the file)
        if c.height > headers.tip_height() {
            return Err(format!("snapshot: coin {}:{} at height {}, above the headers' tip {}", display(&c.txid), c.vout, c.height,
                               headers.tip_height()));
        }
        let i = c.height as usize;
        if mtp_cache.len() <= i {
            mtp_cache.resize(i + 1, None);
        }
        let mtp = match mtp_cache[i] {
            Some(m) => m,
            None => {
                let m = headers.mtp_before(c.height)?;
                mtp_cache[i] = Some(m);
                m
            }
        };
        let t = tracked.contains(&(c.txid, c.vout));
        tracked_used += t as usize;
        sum.push(coin_leaf(c, t, mtp))?;
        h3.add(c);
        value = value.checked_add(c.value).ok_or("snapshot: total amount overflows")?;
        Ok(())
    });
    let acc = sum.finish()?; // joins the workers even when the read failed
    let meta = meta?;
    let (live_set, live_hash) = acc.committed();
    Ok(SnapshotCheck { meta, coins: acc.count, value, hash_serialized_3: h3.finish(), tracked_used, live_set, live_hash })
}

/// The ECMH sum of a stream of leaves. The map to the curve (a square root per try) is the cost, and the sum
/// commutes, so off WASM the leaves go in batches to one worker per core, each with its own partial sum.
struct PointSum {
    #[cfg(not(target_arch = "wasm32"))]
    par: Option<Par>,
    batch: Vec<[u8; 32]>,
    acc: Ecmh,
}

#[cfg(not(target_arch = "wasm32"))]
struct Par {
    tx: Option<std::sync::mpsc::SyncSender<Vec<[u8; 32]>>>,
    workers: Vec<std::thread::JoinHandle<Result<Ecmh, String>>>,
}

const BATCH: usize = 4096;

impl PointSum {
    fn new() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
            if n > 1 {
                let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<[u8; 32]>>(2 * n);
                let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
                // Builder::spawn returns an error where thread::spawn panics (no threads, a resource limit): through
                // the C ABI a panic would abort the host process. Any failure falls back to this thread, and the
                // workers already started stop when the channel closes.
                let mut workers = Vec::with_capacity(n);
                for _ in 0..n {
                    let rx = rx.clone();
                    let w = std::thread::Builder::new().name("ecmh".into()).spawn(move || -> Result<Ecmh, String> {
                        let mut a = Ecmh::default();
                        loop {
                            let b = match rx.lock() { Ok(r) => r.recv(), Err(_) => return Err("ECMH worker: lock poisoned".into()) };
                            let Ok(b) = b else { return Ok(a) };
                            for l in &b {
                                a.add_leaf(l)?;
                            }
                        }
                    });
                    match w { Ok(h) => workers.push(h), Err(_) => break }
                }
                if workers.len() == n {
                    return PointSum { par: Some(Par { tx: Some(tx), workers }), batch: Vec::with_capacity(BATCH), acc: Ecmh::default() };
                }
                drop(tx);
                for w in workers { let _ = w.join(); }
            }
        }
        PointSum {
            #[cfg(not(target_arch = "wasm32"))]
            par: None,
            batch: Vec::new(),
            acc: Ecmh::default(),
        }
    }

    fn push(&mut self, leaf: [u8; 32]) -> Result<(), String> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(p) = &self.par {
            self.batch.push(leaf);
            if self.batch.len() == BATCH {
                let b = std::mem::replace(&mut self.batch, Vec::with_capacity(BATCH));
                p.tx.as_ref().expect("open").send(b).map_err(|_| "ECMH worker stopped".to_string())?;
            }
            return Ok(());
        }
        self.acc.add_leaf(&leaf)
    }

    fn finish(mut self) -> Result<Ecmh, String> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(mut p) = self.par.take() {
            let tx = p.tx.take().expect("open");
            let last = std::mem::take(&mut self.batch);
            let sent = if last.is_empty() { Ok(()) } else { tx.send(last).map_err(|_| "ECMH worker stopped".to_string()) };
            drop(tx);
            let mut total = Ecmh::default();
            let mut err = sent.err();
            for w in p.workers {
                match w.join() {
                    Ok(Ok(a)) => total.add_sum(&a),
                    Ok(Err(e)) => { err.get_or_insert(e); }
                    Err(_) => { err.get_or_insert("ECMH worker panicked".into()); }
                }
            }
            return match err { Some(e) => Err(e), None => Ok(total) };
        }
        Ok(self.acc)
    }
}

/// Checks a Core `dumptxoutset` snapshot against a verified core state with the live-coin hash (profile bit 6): the
/// snapshot's base block is the proven tip, its network is the proof's, and its coins, made into coin leaves, have
/// exactly the proven multiset hash and the proven total amount. `headers` must run from genesis to the tip (the
/// creation MTP of every coin); `tracked` names the coins with the BIP30 tracked bit. Call ONLY with the state of a
/// receipt that verified.
pub fn check_snapshot<R: Read>(cs: &ChainState, snap: R, headers: &HeaderChain, tracked: &Tracked) -> Result<SnapshotCheck, String> {
    if cs.profile & PROFILE_LIBZYNC == 0 {
        return Err(format!("profile 0x{:x} is not a libzync image", cs.profile));
    }
    if cs.profile & PROFILE_ASSUMEVALID != 0 {
        return Err("the proving image SKIPPED scripts and signatures (profile bit 1): its coin set is not one a node may load".into());
    }
    if cs.profile & PROFILE_UTXOHASH == 0 {
        return Err(format!("profile 0x{:x}: the proving image does not commit the live-coin hash (bit 6)", cs.profile));
    }
    if headers.first_height != 0 || headers.tip_height() != cs.height || headers.hashes.last() != Some(&cs.tip_hash) {
        return Err(format!("the headers must run from genesis to the proven tip at {} (given {}..{})", cs.height,
                           headers.first_height, headers.tip_height()));
    }
    // the header first: a wrong network or base block is refused before the coins are read
    let r = hash_snapshot_with(snap, headers, tracked, cs.height, |m| {
        if m.network_magic != network_magic(cs.profile) {
            return Err(format!("snapshot: network magic {} is not the proof's network ({})", hex(&m.network_magic),
                               hex(&network_magic(cs.profile))));
        }
        if m.base_blockhash != cs.tip_hash {
            return Err(format!("snapshot: base block {} is not the proven tip {} (height {}): the proof commits the live-coin hash at its tip only",
                               display(&m.base_blockhash), display(&cs.tip_hash), cs.height));
        }
        Ok(())
    })?;
    if r.value != cs.utxo_value {
        return Err(format!("snapshot: {} sat in {} coins; the proven live amount is {} sat", r.value, r.coins, cs.utxo_value));
    }
    if (r.live_set, r.live_hash) != (cs.live_set, cs.live_hash) {
        return Err(format!("snapshot: its {} coins hash to live-coin ECMH x {:08x}.. ; the proof commits x {:08x}.. : NOT the proven UTXO set{}",
                           r.coins, r.live_hash[7], cs.live_hash[7],
                           if r.tracked_used == 0 { " (no coin was marked BIP30-tracked; a chain with tracked live coins needs --tracked)" } else { "" }));
    }
    Ok(r)
}

// ---------------------------------------------------------------------------------------------------------------
// One entry for the CLI, the C ABI and WASM

/// What to check against a verified state.
pub struct AssumeRequest<'a, R: Read> {
    /// None: no assumevalid check; Some(None): the tip; Some(Some(hash)): that block (internal order)
    pub assumevalid: Option<Option<[u8; 32]>>,
    /// raw 80-byte headers ending at the proven tip
    pub headers: Option<&'a [u8]>,
    /// a Core `dumptxoutset` file
    pub snapshot: Option<R>,
    /// the BIP30-tracked outpoints ([`parse_tracked`])
    pub tracked: Option<&'a str>,
}

#[derive(Debug, Default)]
pub struct AssumeOutcome {
    /// (ok, what), one line per check made; any false is a FAIL
    pub checks: Vec<(bool, String)>,
    pub valid: Option<AssumeValid>,
    pub utxo: Option<SnapshotCheck>,
}

/// The assumevalid and assumeutxo checks of `req` against `cs`, which must be the state of a receipt that VERIFIED
/// (its verdict OK or DEMO): against any other journal the answers are the prover's word.
pub fn run_assume<R: Read>(cs: &ChainState, req: AssumeRequest<R>) -> AssumeOutcome {
    let mut o = AssumeOutcome::default();
    let hc = match req.headers.map(|b| HeaderChain::bind(b, &cs.tip_hash, cs.height)) {
        None => None,
        Some(Ok(h)) => {
            o.checks.push((true, format!("headers: {} headers, heights {}..{}, linked, the last one the proven tip", h.hashes.len(),
                                         h.first_height, h.tip_height())));
            Some(h)
        }
        Some(Err(e)) => {
            o.checks.push((false, e));
            return o;
        }
    };
    if let Some(block) = req.assumevalid {
        match assume_valid(cs, hc.as_ref(), block) {
            Ok(r) => {
                o.checks.push((true, format!("assumevalid: block {} at height {} is in the proven chain (0..{}), whose every script and signature the proof checked",
                                             display(&r.block), r.height, r.proven_to)));
                o.valid = Some(r);
            }
            Err(e) => o.checks.push((false, format!("assumevalid: {e}"))),
        }
    }
    if let Some(snap) = req.snapshot {
        let Some(hc) = hc.as_ref() else {
            o.checks.push((false, "assumeutxo: the headers (genesis to the proven tip) are required: each coin's creation MTP comes from them".into()));
            return o;
        };
        let t = match req.tracked.map(parse_tracked) {
            None => Tracked::new(),
            Some(Ok(t)) => t,
            Some(Err(e)) => {
                o.checks.push((false, format!("assumeutxo: {e}")));
                return o;
            }
        };
        match check_snapshot(cs, snap, hc, &t) {
            Ok(r) => {
                o.checks.push((true, format!("assumeutxo: snapshot of {} coins, {} sat, base block {} = the proven tip; its live-coin ECMH equals the proof's ({} coin(s) BIP30-tracked, of {} listed)",
                                             r.coins, r.value, display(&r.meta.base_blockhash), r.tracked_used, t.len())));
                o.utxo = Some(r);
            }
            Err(e) => o.checks.push((false, format!("assumeutxo: {e}"))),
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_bigint::BigUint;

    fn p() -> BigUint {
        BigUint::parse_bytes(b"fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f", 16).unwrap()
    }

    /// the reference map, written as libzync/src/host.rs `ecmh_hint` computes it (num-bigint)
    fn ref_point(leaf: &[u8; 32]) -> (BigUint, BigUint) {
        let p = p();
        let e = (&p + 1u32) >> 2;
        for ctr in 0u32.. {
            let mut m = leaf.to_vec();
            m.extend_from_slice(&ctr.to_le_bytes());
            let x = BigUint::from_bytes_be(&Sha256::digest(&m));
            let rhs = (&x * &x * &x + 7u32) % &p;
            let r = rhs.modpow(&e, &p);
            if &r * &r % &p == rhs {
                let y = if r.bit(0) { &p - &r } else { r };
                return (x, y);
            }
        }
        unreachable!()
    }

    fn big(x: &Fq) -> BigUint {
        BigUint::from_bytes_be(&fq_to_be(x))
    }

    fn ref_add(a: Option<(BigUint, BigUint)>, b: &(BigUint, BigUint)) -> Option<(BigUint, BigUint)> {
        let p = p();
        let inv = |v: &BigUint| v.modpow(&(&p - 2u32), &p);
        let Some(a) = a else { return Some(b.clone()) };
        let l = if a.0 == b.0 {
            if (&a.1 + &b.1) % &p == BigUint::from(0u32) { return None }
            (3u32 * &a.0 * &a.0) % &p * inv(&(2u32 * &a.1 % &p)) % &p
        } else {
            ((&b.1 + &p - &a.1) % &p) * inv(&((&b.0 + &p - &a.0) % &p)) % &p
        };
        let x = (&l * &l + 2u32 * &p - &a.0 - &b.0) % &p;
        let y = (&l * ((&a.0 + &p - &x) % &p) + &p - &a.1) % &p;
        Some((x, y))
    }

    #[test]
    fn ecmh_map_and_sum_match_the_reference() {
        let mut acc = Ecmh::default();
        let mut r: Option<(BigUint, BigUint)> = None;
        let mut skipped = 0;
        for i in 0u32..64 {
            let leaf: [u8; 32] = Sha256::digest(i.to_le_bytes()).into();
            let (x, y) = ecmh_point(&leaf).unwrap();
            let rp = ref_point(&leaf);
            assert_eq!((big(&x), big(&y)), rp.clone());
            assert!(!fq_is_odd(&y));
            let mut m = leaf.to_vec();
            m.extend_from_slice(&0u32.to_le_bytes());
            skipped += (BigUint::from_bytes_be(&Sha256::digest(&m)) != rp.0) as u32;
            acc.add_affine(&x, &y);
            r = ref_add(r, &rp);
            // the same point twice exercises the doubling branch
            if i % 7 == 0 {
                acc.add_affine(&x, &y);
                r = ref_add(r, &rp);
            }
        }
        assert!(skipped > 0, "some leaf needed ctr > 0");
        let (set, w) = acc.committed();
        assert_eq!(set, 1);
        let (rx, ry) = r.unwrap();
        let limbs = |v: &BigUint| { let d = v.to_u32_digits(); let mut o = [0u32; 8]; o[..d.len()].copy_from_slice(&d); o };
        assert_eq!(w[..8], limbs(&rx));
        assert_eq!(w[8..], limbs(&ry));
    }

    #[test]
    fn partial_sums_add_up() {
        let pts: Vec<(Fq, Fq)> = (0u32..40).map(|i| ecmh_point(&Sha256::digest(i.to_le_bytes()).into()).unwrap()).collect();
        let mut one = Ecmh::default();
        let (mut a, mut b) = (Ecmh::default(), Ecmh::default());
        for (i, (x, y)) in pts.iter().enumerate() {
            one.add_affine(x, y);
            if i % 3 == 0 { a.add_affine(x, y) } else { b.add_affine(x, y) }
        }
        let mut c = Ecmh::default();
        c.add_sum(&a);
        c.add_sum(&b);
        c.add_sum(&Ecmh::default());
        assert_eq!((c.committed(), c.count), (one.committed(), 40));
        // a sum and its negation cancel through add_sum
        let mut n = Ecmh::default();
        for (x, y) in &pts { n.add_affine(x, &(-*y)); }
        c.add_sum(&n);
        assert_eq!(c.committed(), (0, [0; 16]));
    }

    #[test]
    fn ecmh_identity_and_cancellation() {
        assert_eq!(Ecmh::default().committed(), (0, [0; 16]));
        let (x, y) = ecmh_point(&[7u8; 32]).unwrap();
        let mut a = Ecmh::default();
        a.add_affine(&x, &y);
        a.add_affine(&x, &(-y));
        assert_eq!(a.committed(), (0, [0; 16]));
    }

    #[test]
    fn amounts_decompress_as_core() {
        // Core's compress_tests: CompressAmount(n) for these n; DecompressAmount inverts it
        fn compress(mut n: u64) -> u64 {
            if n == 0 { return 0 }
            let mut e = 0;
            while n % 10 == 0 && e < 9 { n /= 10; e += 1; }
            if e < 9 { let d = n % 10; n /= 10; 1 + (n * 9 + d - 1) * 10 + e } else { 1 + (n - 1) * 10 + 9 }
        }
        for n in [0u64, 1, 9, 10, 100, 1_000_000, 50 * 100_000_000, 21_000_000 * 100_000_000, 123_456_789, 999_999_999_999] {
            assert_eq!(decompress_amount(compress(n)), n);
        }
        // Core's own vectors (src/test/compress_tests.cpp): 0 -> 0x0, 1 -> 0x1, CENT -> 0x7, COIN -> 0x9, 50 BTC -> 0x32, 21M BTC -> 0x1406f40
        for (n, c) in [(0u64, 0u64), (1, 1), (1_000_000, 7), (100_000_000, 9), (5_000_000_000, 0x32), (2_100_000_000_000_000, 0x1406f40)] {
            assert_eq!(compress(n), c);
            assert_eq!(decompress_amount(c), n);
        }
    }

    #[test]
    fn varints_and_compact_sizes_are_cores() {
        // serialize_tests.cpp varint vectors: 0 -> 00, 127 -> 7f, 128 -> 8000, 255 -> 807f, 16511 -> ff7f, 16512 -> 808000
        for (v, b) in [(0u64, &[0x00u8][..]), (127, &[0x7f]), (128, &[0x80, 0x00]), (255, &[0x80, 0x7f]), (16511, &[0xff, 0x7f]),
                       (16512, &[0x80, 0x80, 0x00])] {
            assert_eq!(Src { r: b, pos: 0 }.varint(u64::MAX).unwrap(), v);
        }
        assert!(Src { r: &[0xfdu8, 0x10, 0x00][..], pos: 0 }.compact_size().is_err(), "non-canonical");
        assert_eq!(Src { r: &[0xfdu8, 0xfd, 0x00][..], pos: 0 }.compact_size().unwrap(), 253);
        assert!(Src { r: &[0xffu8; 12][..], pos: 0 }.varint(u32::MAX as u64).is_err(), "too large");
    }

    #[test]
    fn mtp_is_the_median_of_the_eleven_before() {
        // 13 synthetic headers linked to a tip: times 0, 10, 20, ... shuffled a little
        let times = [5u32, 1, 9, 3, 7, 11, 2, 8, 4, 10, 6, 12, 0];
        let mut v = Vec::new();
        let mut prev = [0u8; 32];
        for t in times {
            let mut h = [0u8; 80];
            h[4..36].copy_from_slice(&prev);
            h[68..72].copy_from_slice(&t.to_le_bytes());
            prev = dsha(&h);
            v.extend_from_slice(&h);
        }
        let hc = HeaderChain::bind(&v, &prev, 12).unwrap();
        assert_eq!(hc.mtp_before(1).unwrap(), 5);
        assert_eq!(hc.mtp_before(2).unwrap(), 5); // [1,5] -> index 1
        assert_eq!(hc.mtp_before(3).unwrap(), 5); // [1,5,9]
        assert_eq!(hc.mtp_before(11).unwrap(), 6); // times[0..11] sorted 1..11 -> index 5 = 6
        assert_eq!(hc.mtp_before(12).unwrap(), 7); // times[1..12] = 1..12 minus 5 -> sorted [1,2,3,4,6,7,8,9,10,11,12] -> 7
        assert!(hc.mtp_before(0).is_err() && hc.mtp_before(13).is_err());
        // a broken link, a wrong tip, too many headers
        let mut bad = v.clone();
        bad[80 * 5 + 4] ^= 1;
        assert!(HeaderChain::bind(&bad, &prev, 12).unwrap_err().contains("does not link"));
        assert!(HeaderChain::bind(&v, &[0; 32], 12).unwrap_err().contains("not the proven tip"));
        assert!(HeaderChain::bind(&v, &prev, 11).unwrap_err().contains("more than"));
        // a partial run: heights from 8; the MTP before 12 needs 1..11
        let part = HeaderChain::bind(&v[80 * 8..], &prev, 12).unwrap();
        assert_eq!(part.first_height, 8);
        assert!(part.mtp_before(12).is_err());
    }

    // ---- the real vectors: mainnet 1..200 proved under 8888d59c (GPU), Core v31.1.0's dumptxoutset at 199, 200, 1003

    const RCPT: &[u8] = include_bytes!("../../../swiftsync-verify/vectors/mainnet_1_200_8888d59c.rcpt");
    const SNAP_200: &[u8] = include_bytes!("../vectors/utxo_200.dat");
    const SNAP_199: &[u8] = include_bytes!("../vectors/utxo_199.dat");
    const SNAP_1003: &[u8] = include_bytes!("../vectors/utxo_1003.dat");
    const HDRS: &[u8] = include_bytes!("../vectors/headers_0_200.bin");
    /// `dumptxoutset … rollback=200` printed this txoutset_hash
    const CORE_HASH_200: &str = "a51f51aea7cf42fc9f1ab491532de766675e983fde7166fa6273a6d6a6439d67";

    fn state() -> ChainState {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let rep = swiftsync_verify::verify(&swiftsync_verify::Inputs { receipt: RCPT, image: swiftsync_verify::Image::by_network("mainnet").unwrap(), anchor: None, now });
        assert_eq!(rep.verdict, swiftsync_verify::Verdict::Demo, "{:?}", rep.checks); // low work: DEMO, every check passed
        rep.state.unwrap()
    }

    fn coins(b: &[u8]) -> (SnapshotMeta, Vec<SnapCoin>) {
        let mut v = Vec::new();
        let m = read_snapshot(b, |c| { v.push(c.clone()); Ok(()) }).unwrap();
        (m, v)
    }

    fn w_varint(v: &mut Vec<u8>, mut n: u64) {
        let mut t = Vec::new();
        loop {
            t.push((n & 0x7f) as u8 | if t.is_empty() { 0 } else { 0x80 });
            if n <= 0x7f { break }
            n = (n >> 7) - 1;
        }
        t.reverse();
        v.extend(t);
    }

    fn w_compact(v: &mut Vec<u8>, n: u64) {
        assert!(n < 253);
        v.push(n as u8);
    }

    fn compress_amount(mut n: u64) -> u64 {
        if n == 0 { return 0 }
        let mut e = 0;
        while n % 10 == 0 && e < 9 { n /= 10; e += 1; }
        if e < 9 { let d = n % 10; n /= 10; 1 + (n * 9 + d - 1) * 10 + e } else { 1 + (n - 1) * 10 + 9 }
    }

    /// A snapshot of `cs` (raw, uncompressed scripts; Core reads them the same), coins grouped by consecutive txid.
    fn write(m: &SnapshotMeta, cs: &[SnapCoin]) -> Vec<u8> {
        // in Core's key order, so a tampered coin is refused by its hash, not by its position
        let mut cs = cs.to_vec();
        cs.sort_by(|a, b| (a.txid, varint_bytes(a.vout as u64)).cmp(&(b.txid, varint_bytes(b.vout as u64))));
        write_as_is(m, &cs)
    }

    /// The coins exactly in the order given, grouped by consecutive txid.
    fn write_as_is(m: &SnapshotMeta, cs: &[SnapCoin]) -> Vec<u8> {
        let mut v = SNAPSHOT_MAGIC.to_vec();
        v.extend(m.version.to_le_bytes());
        v.extend(m.network_magic);
        v.extend(m.base_blockhash);
        v.extend((cs.len() as u64).to_le_bytes());
        let mut i = 0;
        while i < cs.len() {
            let mut j = i;
            while j < cs.len() && cs[j].txid == cs[i].txid { j += 1 }
            v.extend(cs[i].txid);
            w_compact(&mut v, (j - i) as u64);
            for c in &cs[i..j] {
                w_compact(&mut v, c.vout as u64);
                w_varint(&mut v, (c.height as u64) << 1 | c.coinbase as u64);
                w_varint(&mut v, compress_amount(c.value));
                w_varint(&mut v, c.script_pubkey.len() as u64 + 6);
                v.extend(&c.script_pubkey);
            }
            i = j;
        }
        v
    }

    fn check(cs: &ChainState, snap: &[u8], t: &Tracked) -> Result<SnapshotCheck, String> {
        let hc = HeaderChain::bind(HDRS, &cs.tip_hash, cs.height).unwrap();
        check_snapshot(cs, snap, &hc, t)
    }

    #[test]
    fn the_snapshot_must_be_in_cores_key_order() {
        let cs = state();
        let (m, v) = coins(SNAP_200);
        let none = Tracked::new();
        // the real files are in key order: re-sorting them changes nothing, and Core's own 1003 dump reads
        assert_eq!(write(&m, &v), write_as_is(&m, &v));
        let (m3, v3) = coins(SNAP_1003);
        assert_eq!(write(&m3, &v3), write_as_is(&m3, &v3));
        // permuted: two groups swapped. The same coins, so the same ECMH, but not Core's order (its hash_serialized_3
        // would not be Core's): refused, against the real proof
        let mut p = v.clone();
        p.swap(0, 2);
        let e = check(&cs, &write_as_is(&m, &p), &none).unwrap_err();
        assert!(e.contains("key order"), "{e}");
        // a txid with two coins (none is live at 200; Core's dump at 1003 has them)
        let i = (1..v3.len()).find(|&i| v3[i].txid == v3[i - 1].txid).expect("a txid with two coins at 1003");
        // split: the txid's two coins in two consecutive groups
        let mut b = SNAPSHOT_MAGIC.to_vec();
        b.extend(m3.version.to_le_bytes()); b.extend(m3.network_magic); b.extend(m3.base_blockhash); b.extend((v3.len() as u64).to_le_bytes());
        for c in &v3 {
            b.extend(c.txid); w_compact(&mut b, 1); w_compact(&mut b, c.vout as u64);
            w_varint(&mut b, (c.height as u64) << 1 | c.coinbase as u64); w_varint(&mut b, compress_amount(c.value));
            w_varint(&mut b, c.script_pubkey.len() as u64 + 6); b.extend(&c.script_pubkey);
        }
        let e = read_snapshot(&b[..], |_| Ok(())).unwrap_err();
        assert!(e.contains("key order") && e.contains(&display(&v3[i].txid)), "{e}");
        // within a group, vouts out of order
        let mut q = v3.clone();
        q.swap(i - 1, i);
        let e = read_snapshot(&write_as_is(&m3, &q)[..], |_| Ok(())).unwrap_err();
        assert!(e.contains("encoded vouts ascending"), "{e}");
        assert!(read_snapshot(&write_as_is(&m3, &v3)[..], |_| Ok(())).is_ok());
    }

    #[test]
    fn vout_order_is_the_encoded_bytes() {
        // 16,511 encodes as ff7f and 16,512 as 808000: bytewise, 16,512 comes FIRST (Core's database order)
        assert_eq!(varint_bytes(16_511), vec![0xff, 0x7f]);
        assert_eq!(varint_bytes(16_512), vec![0x80, 0x80, 0x00]);
        let snap = |vouts: &[u64]| {
            let mut b = SNAPSHOT_MAGIC.to_vec();
            b.extend(SNAPSHOT_VERSION.to_le_bytes()); b.extend([0xf9, 0xbe, 0xb4, 0xd9]); b.extend([0u8; 32]);
            b.extend((vouts.len() as u64).to_le_bytes()); b.extend([7u8; 32]); b.push(vouts.len() as u8);
            for &o in vouts {
                b.push(0xfd); b.extend((o as u16).to_le_bytes()); // CompactSize vout
                w_varint(&mut b, 2); w_varint(&mut b, 0); w_varint(&mut b, 7); b.push(0x51); // height 1, 0 sat, script OP_1
            }
            b
        };
        assert!(read_snapshot(&snap(&[16_512, 16_511])[..], |_| Ok(())).is_ok());
        assert!(read_snapshot(&snap(&[16_511, 16_512])[..], |_| Ok(())).unwrap_err().contains("encoded vouts ascending"));
        assert!(read_snapshot(&snap(&[300, 300])[..], |_| Ok(())).is_err(), "a vout twice");
    }

    #[test]
    fn scripts_skipped_images_and_heights_above_the_headers_are_refused() {
        let cs = state();
        let hc = HeaderChain::bind(HDRS, &cs.tip_hash, cs.height).unwrap();
        let mut c = cs.clone(); c.profile |= PROFILE_ASSUMEVALID;
        assert!(check_snapshot(&c, SNAP_200, &hc, &Tracked::new()).unwrap_err().contains("SKIPPED"));
        let mut c = cs.clone(); c.profile &= !PROFILE_LIBZYNC;
        assert!(check_snapshot(&c, SNAP_200, &hc, &Tracked::new()).unwrap_err().contains("not a libzync image"));
        // hash_snapshot with a base above the headers' tip: refused before anything is sized by the coin's height
        let part = HeaderChain::bind(&HDRS[..80 * 101], &hc.hashes[100], 100).unwrap();
        assert!(hash_snapshot(SNAP_200, &part, &Tracked::new(), u32::MAX).unwrap_err().contains("above the headers' tip"));
        // a tip height at u32::MAX does not wrap
        let top = HeaderChain::bind(&HDRS[..80], &hc.hashes[0], u32::MAX).unwrap();
        assert_eq!((top.first_height, top.tip_height()), (u32::MAX, u32::MAX));
    }

    #[test]
    fn the_real_snapshot_is_the_proven_utxo_set() {
        let cs = state();
        let r = check(&cs, SNAP_200, &Tracked::new()).unwrap();
        assert_eq!((r.coins, r.value, r.tracked_used), (204, 1_000_000_000_000, 0));
        assert_eq!(r.hash_serialized_3, CORE_HASH_200, "our parse gives Core's own hash_serialized_3");
        // re-serialised with every script raw: the same coins, so the same answer (Core reads both alike)
        let (m, v) = coins(SNAP_200);
        assert!(v.iter().any(|c| c.script_pubkey.len() == 67), "P2PK uncompressed keys decompressed (type 4/5)");
        let raw = write(&m, &v);
        assert_ne!(raw, SNAP_200);
        assert_eq!(check(&cs, &raw, &Tracked::new()).unwrap().hash_serialized_3, CORE_HASH_200);
    }

    #[test]
    fn tampered_snapshots_are_refused() {
        let cs = state();
        let (m, v) = coins(SNAP_200);
        let none = Tracked::new();
        let refuse = |v: &[SnapCoin], m: &SnapshotMeta, what: &str| {
            let e = check(&cs, &write(m, v), &none).unwrap_err();
            assert!(e.contains("NOT the proven UTXO set") || e.contains("proven live amount") || e.contains("outside") || e.contains("base block")
                    || e.contains("key order")
                    || e.contains("network magic"), "{what}: {e}");
        };
        let mut t = v.clone(); t[3].value += 1; t[4].value -= 1; refuse(&t, &m, "amounts moved, total kept");
        let mut t = v.clone(); t[5].value += 1; refuse(&t, &m, "amount");
        let mut t = v.clone(); t[5].height -= 1; refuse(&t, &m, "height");
        let mut t = v.clone(); t[5].coinbase = !t[5].coinbase; refuse(&t, &m, "coinbase flag");
        let mut t = v.clone(); t[5].vout = 1; refuse(&t, &m, "vout");
        let mut t = v.clone(); t[5].txid[0] ^= 1; refuse(&t, &m, "txid");
        let mut t = v.clone(); let k = t[5].script_pubkey.len() - 2; t[5].script_pubkey[k] ^= 1; refuse(&t, &m, "script");
        let mut t = v.clone(); t.remove(7); refuse(&t, &m, "a coin dropped");
        let mut t = v.clone(); let c = t[7].clone(); t.insert(8, c); refuse(&t, &m, "a coin twice");
        let mut t = v.clone(); let mut c = t[7].clone(); c.value = 0; c.vout = 9; t.insert(8, c); refuse(&t, &m, "an extra zero-value coin");
        let mut t = v.clone(); t[5].height = 201; refuse(&t, &m, "a coin above the base");
        let mut t = v.clone(); t[0].value -= 1; t.push(SnapCoin { value: 1, vout: 77, ..t[0].clone() }); refuse(&t, &m, "a coin split, total kept");
        // the same coins under another base or network
        let mut mm = m.clone(); mm.base_blockhash[0] ^= 1; refuse(&v, &mm, "base block");
        let mut mm = m.clone(); mm.network_magic = network_magic(PROFILE_SIGNET); refuse(&v, &mm, "network");
        // a wrong tracked bit only refuses
        let mut tr = Tracked::new(); tr.insert((v[9].txid, v[9].vout));
        assert!(check(&cs, SNAP_200, &tr).unwrap_err().contains("NOT the proven UTXO set"));
        // framing: count, trailing byte, truncated, version, magic
        let mut b = SNAP_200.to_vec(); b[43] += 1; assert!(check(&cs, &b, &none).is_err());
        let mut b = SNAP_200.to_vec(); b[43] -= 1; assert!(check(&cs, &b, &none).unwrap_err().contains("bytes after"));
        let mut b = SNAP_200.to_vec(); b.push(0); assert!(check(&cs, &b, &none).unwrap_err().contains("bytes after"));
        assert!(check(&cs, &SNAP_200[..SNAP_200.len() - 1], &none).unwrap_err().contains("truncated"));
        let mut b = SNAP_200.to_vec(); b[5] = 3; assert!(check(&cs, &b, &none).unwrap_err().contains("version"));
        let mut b = SNAP_200.to_vec(); b[0] = b'U'; assert!(check(&cs, &b, &none).unwrap_err().contains("magic"));
    }

    #[test]
    fn snapshots_at_other_heights_are_refused() {
        let cs = state();
        let e = check(&cs, SNAP_199, &Tracked::new()).unwrap_err();
        assert!(e.contains("base block") && e.contains("tip only"), "{e}");
        let e = check(&cs, SNAP_1003, &Tracked::new()).unwrap_err();
        assert!(e.contains("base block"), "{e}");
        // and with the base block relabelled, by its coins above 200
        let (mut m, v) = coins(SNAP_1003);
        m.base_blockhash = cs.tip_hash;
        assert!(check(&cs, &write(&m, &v), &Tracked::new()).unwrap_err().contains("outside 1..200"));
        // and their coins, relabelled as the base at 200, do not hash to the proof's
        let (mut m, v) = coins(SNAP_199);
        m.base_blockhash = cs.tip_hash;
        assert!(check(&cs, &write(&m, &v), &Tracked::new()).is_err());
    }

    #[test]
    fn headers_and_profiles_are_checked() {
        let cs = state();
        // the headers must reach from genesis to the proven tip
        let hc = HeaderChain::bind(&HDRS[80 * 100..], &cs.tip_hash, cs.height).unwrap();
        assert!(check_snapshot(&cs, SNAP_200, &hc, &Tracked::new()).unwrap_err().contains("from genesis"));
        assert!(HeaderChain::bind(&HDRS[..80 * 200], &cs.tip_hash, cs.height).is_err(), "ends below the tip");
        let mut b = HDRS.to_vec(); b[80 * 150 + 68] ^= 1; // a timestamp (so an MTP) changed
        assert!(HeaderChain::bind(&b, &cs.tip_hash, cs.height).unwrap_err().contains("does not link"));
        // a state without the live-coin hash
        let mut c = cs.clone(); c.profile &= !PROFILE_UTXOHASH;
        let hc = HeaderChain::bind(HDRS, &cs.tip_hash, cs.height).unwrap();
        assert!(check_snapshot(&c, SNAP_200, &hc, &Tracked::new()).unwrap_err().contains("bit 6"));
        // a different committed hash
        let mut c = cs.clone(); c.live_hash[0] ^= 1;
        assert!(check_snapshot(&c, SNAP_200, &hc, &Tracked::new()).unwrap_err().contains("NOT the proven"));
    }

    #[test]
    fn assumevalid_names_only_proven_blocks() {
        let cs = state();
        let hc = HeaderChain::bind(HDRS, &cs.tip_hash, cs.height).unwrap();
        assert_eq!(assume_valid(&cs, None, None).unwrap(), AssumeValid { block: cs.tip_hash, height: 200, proven_to: 200 });
        assert_eq!(assume_valid(&cs, Some(&hc), Some(hc.hashes[170])).unwrap().height, 170);
        assert_eq!(assume_valid(&cs, Some(&hc), Some(hc.hashes[0])).unwrap().height, 0);
        // a partial run 150..200 serves a block in it
        let part = HeaderChain::bind(&HDRS[80 * 150..], &cs.tip_hash, cs.height).unwrap();
        assert_eq!(assume_valid(&cs, Some(&part), Some(hc.hashes[170])).unwrap().height, 170);
        assert!(assume_valid(&cs, Some(&part), Some(hc.hashes[149])).is_err());
        // a block above the proof (1003, from the snapshot) or not in the chain; no headers
        let (m, _) = coins(SNAP_1003);
        assert!(assume_valid(&cs, Some(&hc), Some(m.base_blockhash)).unwrap_err().contains("not in the headers"));
        assert!(assume_valid(&cs, None, Some(hc.hashes[170])).unwrap_err().contains("give the headers"));
        // an image that skipped scripts proves nothing for it
        let mut c = cs.clone(); c.profile |= PROFILE_ASSUMEVALID;
        assert!(assume_valid(&c, None, None).unwrap_err().contains("SKIPPED"));
    }
}

// Kept at the end of the file: a line added above the code a WASM module is built from moves the panic locations it
// embeds, and so its bytes (the published sha). Without the tests-only `regtest-test` feature this is mainnet's magic.
#[cfg(all(feature = "regtest-test", not(debug_assertions)))]
compile_error!("zapp-assume's `regtest-test` feature is for tests only: it never builds without debug assertions (a release build).");

/// [`network_magic`] of a profile without the signet bit: mainnet's, or under the tests-only `regtest-test` feature
/// (zapps/corecheck's regtest image, profile bit 3) Core's regtest pchMessageStart.
#[inline(always)]
fn magic_not_signet(_profile: u32) -> [u8; 4] {
    #[cfg(feature = "regtest-test")]
    if _profile & 8 != 0 {
        return [0xfa, 0xbf, 0xb5, 0xda];
    }
    [0xf9, 0xbe, 0xb4, 0xd9]
}
