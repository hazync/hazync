//! zyncAPP "headers" (ROADMAP K1): a run of Bitcoin mainnet block headers is a valid header chain, as Core's header
//! checks: each links to the one before (prev hash), its hash is at or under its target (CheckProofOfWork), its nBits is
//! the previous one's or, every 2,016 blocks, Core's retarget (CalculateNextWorkRequired, no min-difficulty rule on
//! mainnet), its time is above the median of the previous 11 (time-too-old), its version meets BIP34 / BIP66 / BIP65
//! (bad-version). Chainwork is summed (GetBlockProof). SPV-level: no transactions, coins or scripts.
//!
//! Input: the state before the run (height of the last header, its hash, chainwork LE, its nBits and time, the first
//!        time of its retarget epoch, the last <= 11 times; a run from genesis starts from the empty state and its
//!        first header must be Core's genesis header), then the headers (80 bytes each, as words).
//! Commits: sha256 of the input state (a run joins the one that ends there; from genesis it is the empty state's), and
//!          the state after the run, in the same form. The time-too-new rule is the verifier's (max_time).
#![no_main]
extern crate alloc;
use alloc::vec::Vec;
use crypto_bigint::{Encoding, NonZero, U256};
use risc0_zkvm::guest::env;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
risc0_zkvm::guest::entry!(main);

#[derive(Serialize, Deserialize, Clone)]
struct HState {
    height: i64,          // of the last header; -1 = before genesis
    hash: [u8; 32],       // internal byte order
    work: [u8; 32],       // chainwork, little-endian
    bits: u32,
    time: u32,
    epoch_first_time: u32,
    recent_times: Vec<u32>,
    max_time: u32,
}

const GENESIS: [u8; 32] = {
    // 000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f, internal order
    let d = *b"\x6f\xe2\x8c\x0a\xb6\xf1\xb3\x72\xc1\xa6\xa2\x46\xae\x63\xf7\x4f\x93\x1e\x83\x65\xe1\x5a\x08\x9c\x68\xd6\x19\x00\x00\x00\x00\x00";
    d
};
const POW_LIMIT_BITS: u32 = 0x1d00ffff;
const INTERVAL: i64 = 2016;
const TARGET_TIMESPAN: u64 = 14 * 24 * 60 * 60;

/// Core's SetCompact: (target, negative, overflow).
fn set_compact(bits: u32) -> (U256, bool, bool) {
    let size = bits >> 24;
    let mut word = bits & 0x007f_ffff;
    let t = if size <= 3 {
        word >>= 8 * (3 - size);
        U256::from_u32(word)
    } else {
        U256::from_u32(word).shl_vartime(8 * (size as usize - 3))
    };
    let neg = word != 0 && bits & 0x0080_0000 != 0;
    let over = word != 0 && (size > 34 || (word > 0xff && size > 33) || (word > 0xffff && size > 32));
    (t, neg, over)
}

/// Core's GetCompact.
fn get_compact(t: &U256) -> u32 {
    let mut size = (t.bits_vartime() as u32 + 7) / 8;
    let mut compact: u32 = if size <= 3 {
        (t.as_words()[0] as u64 as u32) << (8 * (3 - size))
    } else {
        t.shr_vartime(8 * (size as usize - 3)).as_words()[0] as u64 as u32
    };
    if compact & 0x0080_0000 != 0 { compact >>= 8; size += 1; }
    compact | (size << 24)
}

fn dsha(b: &[u8]) -> [u8; 32] { Sha256::digest(Sha256::digest(b)).into() }

fn median(v: &[u32]) -> u32 { let mut s = v.to_vec(); s.sort_unstable(); s[s.len() / 2] }

fn main() {
    let st: HState = env::read();
    let n: u32 = env::read();
    let mut words = alloc::vec![0u32; n as usize * 20];
    if n > 0 { env::read_slice(&mut words); }
    let input_digest: [u8; 32] = Sha256::digest(risc0_zkvm::serde::to_vec(&st).unwrap().iter()
        .flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>()).into();
    let pow_limit = set_compact(POW_LIMIT_BITS).0;
    let mut s = st;
    // nBits changes only at a retarget: its target and work are computed once per value, not once per header.
    let mut cached: Option<(u32, U256, U256)> = None;
    for k in 0..n as usize {
        let mut hb = [0u8; 80];
        for (i, w) in words[20 * k..20 * k + 20].iter().enumerate() { hb[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes()); }
        let version = i32::from_le_bytes(hb[0..4].try_into().unwrap());
        let time = u32::from_le_bytes(hb[68..72].try_into().unwrap());
        let bits = u32::from_le_bytes(hb[72..76].try_into().unwrap());
        let hash = dsha(&hb);
        let h = s.height + 1;
        if h == 0 {
            assert!(hash == GENESIS, "headers: the first header of a run from genesis is not Core's genesis");
        } else {
            assert!(hb[4..36] == s.hash, "headers: {h} does not link to the previous header");
            let want_bits = if h % INTERVAL != 0 { s.bits } else {
                // Core's CalculateNextWorkRequired: the epoch's actual timespan, clamped to [T/4, 4T]
                let actual = (s.time as i64 - s.epoch_first_time as i64).clamp(TARGET_TIMESPAN as i64 / 4, TARGET_TIMESPAN as i64 * 4) as u64;
                let mut t = set_compact(s.bits).0.wrapping_mul(&U256::from_u64(actual));
                t = t.wrapping_div(&NonZero::new(U256::from_u64(TARGET_TIMESPAN)).unwrap());
                if t > pow_limit { t = pow_limit; }
                get_compact(&t)
            };
            assert!(bits == want_bits, "headers: {h} bad-diffbits");
            assert!(time > median(&s.recent_times), "headers: {h} time-too-old");
            assert!(!((version < 2 && h >= 227_931) || (version < 3 && h >= 363_725) || (version < 4 && h >= 388_381)),
                    "headers: {h} bad-version");
        }
        // Core's CheckProofOfWork, and GetBlockProof: (~t / (t + 1)) + 1
        let (target, w) = match cached {
            Some((b, t, w)) if b == bits => (t, w),
            _ => {
                let (target, neg, over) = set_compact(bits);
                assert!(!neg && !over && target != U256::ZERO && target <= pow_limit, "headers: {h} bad nBits");
                let w = target.not().wrapping_div(&NonZero::new(target.wrapping_add(&U256::ONE)).unwrap()).wrapping_add(&U256::ONE);
                cached = Some((bits, target, w));
                (target, w)
            }
        };
        assert!(U256::from_le_slice(&hash) <= target, "headers: {h} high-hash");
        let work = U256::from_le_slice(&s.work).wrapping_add(&w);
        if h % INTERVAL == 0 { s.epoch_first_time = time; }
        s.recent_times.push(time);
        if s.recent_times.len() > 11 { s.recent_times.remove(0); }
        s.height = h;
        s.hash = hash;
        s.work.copy_from_slice(&work.to_le_bytes());
        s.bits = bits;
        s.time = time;
        s.max_time = s.max_time.max(time);
    }
    env::commit(&(input_digest, s));
}
