//! The Utreexo roots zAPP's fuzz properties (docs/libzync/ZAPP_TESTING.md §3.4, §4.8), one function per target, so the
//! fuzz targets and the stable corpus replay (tests/replay.rs) run the same code. A property that fails panics.
//!
//! - `decode`: any bytes into `decode_receipt` (the 16 MiB limit): no panic, no allocation beyond the limit; a receipt
//!   it decodes re-encodes (bincode, fixed-width integers) to a prefix of the input (trailing bytes are allowed by
//!   design, as `bincode::deserialize`).
//! - `verify`: mutations of the real receipts (vectors/, CPU proofs at H = 2 and 3) through `verify_utreexo` with the
//!   questions the zAPP's tests ask: a verdict other than FAIL only for a receipt whose decoding is one of the real
//!   receipts (the input's own bytes up to any trailing ones).

use std::sync::OnceLock;
use swiftsync_verify::Verdict;
use zapp_utreexo::{decode_receipt, verify_utreexo, UTREEXO_IMAGE_ID};

pub const H3: &str = "0000000082b5015589a3fdf2d4baff403e6f0be035a5d9742c1cae6295464449";
pub const H2: &str = "000000006a625f06636b8bb6ac7b960a8d03705d1ace08b1a19da3fdcc99ddbd";
pub const NOW: u64 = 1_800_000_000;
pub const REAL: [&[u8]; 3] = [include_bytes!("../../vectors/mainnet-2.receipt"), include_bytes!("../../vectors/mainnet-3.receipt"),
                              include_bytes!("../../vectors/mainnet-3-split.receipt")];

fn canon(b: &[u8]) -> Option<Vec<u8>> {
    decode_receipt(b).ok().map(|r| bincode::serialize(&r).unwrap())
}

fn real_canon() -> &'static Vec<Vec<u8>> {
    static C: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    C.get_or_init(|| REAL.iter().map(|b| { let c = canon(b).unwrap(); assert_eq!(&c[..], *b, "a real receipt is not canonical"); c }).collect())
}

pub fn decode(data: &[u8]) {
    if let Ok(r) = decode_receipt(data) {
        let c = bincode::serialize(&r).unwrap();
        assert!(data.starts_with(&c), "NONCANONICAL: decode_receipt accepted bytes whose re-encoding is not a prefix of them");
    }
}

pub fn verify(data: &[u8]) {
    for (at, tip) in [((3, H3), None), ((2, H2), Some((3, H3)))] {
        let rep = verify_utreexo(data, UTREEXO_IMAGE_ID, None, NOW, Some(at), tip);
        if rep.verdict != Verdict::Fail {
            let c = canon(data);
            assert!(c.as_ref().is_some_and(|c| real_canon().contains(c)),
                    "WRONG-OK: {:?} for a receipt that is not one of the real ones ({} bytes)", rep.verdict, data.len());
        }
    }
}
