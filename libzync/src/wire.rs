//! libzync hint streams: the ONE definition of what the host writes and the guest reads.
//!
//! One length-prefixed block of u32 words per guest run, read with a single `read_slice`:
//!
//! ```text
//! chunk run:      CHUNK_MAGIC mtp block_time liftx_bytes sig_collect sig_bytes
//!                 liftx[liftx_bytes] sig[sig_bytes]                       (each padded to a word)
//! aggregate run:  AGG_MAGIC ecmh_collect ecmh_bytes ss_mode ss_nbits
//!                 ecmh[ecmh_bytes] ss_words[ceil(ss_nbits / 32)]          (ecmh padded to a word)
//! ```
//!
//! - `liftx`: 64-byte (x, y) pairs, big-endian, any order -- public-key Y hints, each checked
//!   y^2 == x^3 + 7 by libsecp before use (canonical since v0.21, `liftx_hint.rs`).
//! - `sig`: SIG_HINT_BYTES per signature verify, in call order: valid u32 LE | R.x BE | R.y BE.
//!   `sig_collect = 1` sends none and the guest logs every verify (BATCHREC) instead.
//! - `ecmh`: per ECMH operation, in operation order: ctr u32 LE | y BE | one w BE per skipped ctr.
//!   `ecmh_collect = 1` sends none and the guest logs every leaf (ECMHREC) instead.
//! - `ss`: design B's SwiftSync hint bits, one per indexed output, LSB-first in LE words. Mode 0 =
//!   no hints file (every created coin goes to the ECMH total).
//!
//! Everything here is UNTRUSTED input to the guest: each hint is re-checked where it is used, so a
//! wrong one makes a run slower or makes it fail; it never makes a bad block pass. The decoders only
//! reject what is malformed.
//!
//! Included by the guest via #[path]: `alloc` only, no dependencies.

use alloc::vec::Vec;

/// "LZC1" as little-endian bytes.
pub const CHUNK_MAGIC: u32 = u32::from_le_bytes(*b"LZC1");
/// "LZA1" as little-endian bytes.
pub const AGG_MAGIC: u32 = u32::from_le_bytes(*b"LZA1");
/// One batch-signature hint: valid u32 LE, R.x, R.y.
pub const SIG_HINT_BYTES: usize = 68;
/// One pubkey-Y hint: x, y.
pub const LIFTX_PAIR_BYTES: usize = 64;
/// Collect-mode log tags (guest prints, host parses).
pub const BATCHREC_TAG: &str = "BATCHREC";
pub const ECMHREC_TAG: &str = "ECMHREC ";

/// What one chunk run is given.
pub struct ChunkHints<'a> {
    pub mtp: u32,
    pub block_time: u32,
    pub liftx: &'a [u8],
    pub sig_collect: bool,
    pub sig: &'a [u8],
}

/// What one aggregate run is given.
pub struct AggHints<'a> {
    pub ecmh_collect: bool,
    pub ecmh: &'a [u8],
    /// Some((nbits, words)) = design B hint bits for this block; None = no hints file.
    pub ss: Option<(u32, &'a [u32])>,
}

fn words_for(bytes: usize) -> usize {
    bytes.div_ceil(4)
}

/// The words as their little-endian bytes. The guest is rv32 (little-endian) and the host packs LE,
/// so this is the byte sequence the host wrote.
fn as_bytes(w: &[u32]) -> &[u8] {
    // SAFETY: u32 has no padding and any byte pattern is a valid u8; the length is exact.
    #[cfg(target_endian = "little")]
    unsafe {
        core::slice::from_raw_parts(w.as_ptr() as *const u8, w.len() * 4)
    }
}

/// The bytes of `w` past `len` (word padding) are all zero: one encoding per value.
fn padding_zero(w: &[u32], len: usize) -> bool {
    as_bytes(w)[len..].iter().all(|b| *b == 0)
}

fn push_bytes(out: &mut Vec<u32>, b: &[u8]) {
    for c in b.chunks(4) {
        let mut x = [0u8; 4];
        x[..c.len()].copy_from_slice(c);
        out.push(u32::from_le_bytes(x));
    }
}

impl<'a> ChunkHints<'a> {
    pub fn encode(&self) -> Vec<u32> {
        let mut v = Vec::with_capacity(6 + words_for(self.liftx.len()) + words_for(self.sig.len()));
        v.extend_from_slice(&[CHUNK_MAGIC, self.mtp, self.block_time, self.liftx.len() as u32,
                              self.sig_collect as u32, self.sig.len() as u32]);
        push_bytes(&mut v, self.liftx);
        push_bytes(&mut v, self.sig);
        v
    }

    pub fn decode(w: &'a [u32]) -> Result<Self, &'static str> {
        if w.len() < 6 || w[0] != CHUNK_MAGIC { return Err("chunk hints: bad header"); }
        let (lb, sc, sb) = (w[3] as usize, w[4], w[5] as usize);
        if lb % LIFTX_PAIR_BYTES != 0 { return Err("chunk hints: liftx not whole pairs"); }
        if sb % SIG_HINT_BYTES != 0 { return Err("chunk hints: sig not whole records"); }
        if sc > 1 || (sc == 1 && sb != 0) { return Err("chunk hints: bad sig mode"); }
        let (lw, sw) = (words_for(lb), words_for(sb));
        if w.len() != 6 + lw + sw { return Err("chunk hints: length mismatch"); }
        if !padding_zero(&w[6..6 + lw], lb) || !padding_zero(&w[6 + lw..], sb) { return Err("chunk hints: nonzero padding"); }
        Ok(ChunkHints {
            mtp: w[1],
            block_time: w[2],
            liftx: &as_bytes(&w[6..6 + lw])[..lb],
            sig_collect: sc == 1,
            sig: &as_bytes(&w[6 + lw..])[..sb],
        })
    }
}

impl<'a> AggHints<'a> {
    pub fn encode(&self) -> Vec<u32> {
        let (mode, nbits, sw): (u32, u32, &[u32]) = match self.ss { Some((n, w)) => (1, n, w), None => (0, 0, &[]) };
        let mut v = Vec::with_capacity(5 + words_for(self.ecmh.len()) + sw.len());
        v.extend_from_slice(&[AGG_MAGIC, self.ecmh_collect as u32, self.ecmh.len() as u32, mode, nbits]);
        push_bytes(&mut v, self.ecmh);
        v.extend_from_slice(sw);
        v
    }

    pub fn decode(w: &'a [u32]) -> Result<Self, &'static str> {
        if w.len() < 5 || w[0] != AGG_MAGIC { return Err("agg hints: bad header"); }
        let (ec, eb, mode, nbits) = (w[1], w[2] as usize, w[3], w[4]);
        if ec > 1 || (ec == 1 && eb != 0) { return Err("agg hints: bad ecmh mode"); }
        if mode > 1 || (mode == 0 && nbits != 0) { return Err("agg hints: bad ss mode"); }
        let ew = words_for(eb);
        let sw = (nbits as usize).div_ceil(32);
        if w.len() != 5 + ew + sw { return Err("agg hints: length mismatch"); }
        let ss_words = &w[5 + ew..];
        if !padding_zero(&w[5..5 + ew], eb) { return Err("agg hints: nonzero padding"); }
        if mode == 1 && !super::ss_hints::bits_canonical(nbits, ss_words) {
            return Err("agg hints: a hint bit is set past the block's indexed outputs");
        }
        Ok(AggHints {
            ecmh_collect: ec == 1,
            ecmh: &as_bytes(&w[5..5 + ew])[..eb],
            ss: if mode == 1 { Some((nbits, ss_words)) } else { None },
        })
    }
}
