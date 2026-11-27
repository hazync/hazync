//! The Utreexo roots zAPP's fuzz properties (docs/libzync/ZAPP_TESTING.md §3.4, §4.8), one function per target, so the
//! fuzz targets and the stable corpus replay (tests/replay.rs) run the same code. A property that fails panics.
//! Engine proofs only since the cutover (27.11.2026, engine/CUTOVER.md).
//!
//! - `decode`: any bytes into `journal_kind` (the engine file's framing, unverified): no panic; a kind is read only from
//!   an engine proof file whose framing holds.
//! - `verify`: mutations of the engine's real base proof (tests/data/engine/base100.hze, roots after block 100) through
//!   `verify_utreexo` under its image id and pinned key: a verdict other than FAIL only for exactly the real file.

use swiftsync_verify::{hex32, EngineKey, Verdict};
use zapp_utreexo::{journal_kind, verify_utreexo};

pub const H100: &str = "000000007bc154e0fa7ea32218a72fe2c1bb9f86cf8c9ebf9a715ed27fdb229a";
pub const H200: &str = "000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320";
pub const BASE_ID: &str = "340f6675bafcdd893c12e0dfae0391043cc7fd8919b526ff756ec13d5d76cc95";
pub const CORE_ID: &str = "c7fe0eb85d2d4c10532742191b059a2adcc3e24e22aa81c1e632e2ee718d2fc9";
pub const BASE_PIN: &str = "a966e0ebe4e542192436c292a0db4b1c1b4ac2f0208a3d985fd1f5563ecd730a";
pub const NOW: u64 = 1_800_000_000;
pub const REAL: &[u8] = include_bytes!("../../tests/data/engine/base100.hze");
const VK: &[u8] = include_bytes!("../../tests/data/engine/base100.vk");

pub fn decode(data: &[u8]) {
    if journal_kind(data).is_some() {
        assert!(swiftsync_verify::is_engine_proof(data), "a journal kind read from bytes that are not an engine proof file");
    }
}

pub fn verify(data: &[u8]) {
    let key = EngineKey { bytes: VK, pin: hex32(BASE_PIN).unwrap() };
    let rep = verify_utreexo(data, BASE_ID, &key, CORE_ID, NOW, Some((100, H100)), Some((200, H200)));
    if rep.verdict != Verdict::Fail {
        assert!(data == REAL, "WRONG-OK: {:?} for a file that is not the real proof ({} bytes)", rep.verdict, data.len());
    }
}
