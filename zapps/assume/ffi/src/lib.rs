//! C ABI for the assumevalid and assumeutxo zyncAPPs (include/zapp_assume.h), moved out of swiftsync-verify-ffi on
//! 15.11.2026: a zAPP module of its own (docs/libzync/ZYNCAPPS.md §9). One implementation of the checks: this calls the
//! same `zapp_assume::run_assume` as the CLI and the WASM build, after the core verifier's receipt checks. Bad input
//! is an error, never a panic; and since the crate is built with `panic = "abort"` (no unwinding, so no `catch_unwind`),
//! a panic from a bug aborts the process rather than unwinding into C.

use std::ffi::{c_char, CStr};
use swiftsync_verify::{Image, Verdict};

const OK: i32 = 0;
const FAIL: i32 = 1;
const DEMO: i32 = 2;

unsafe fn slice<'a>(p: *const u8, n: usize) -> &'a [u8] {
    if p.is_null() || n == 0 { &[] } else { std::slice::from_raw_parts(p, n) }
}

unsafe fn write_msg(msg: *mut c_char, len: usize, s: &str) {
    if msg.is_null() || len == 0 {
        return;
    }
    let b = s.as_bytes();
    let n = b.len().min(len - 1);
    std::ptr::copy_nonoverlapping(b.as_ptr(), msg as *mut u8, n);
    *msg.add(n) = 0;
}

/// The verified chain (only filled when the verdict is not FAIL): the core's `swiftsync_result`, field for field.
#[repr(C)]
pub struct ZappAssumeChain {
    pub height: u32,
    pub tip_hash: [u8; 32],
    pub chainwork: [u8; 32],
    pub utxo_leaves: u64,
    pub profile: u32,
}

/// The answers of `swiftsync_verify_assume` (include/swiftsync_verify.h).
#[repr(C)]
pub struct SwiftsyncAssumeResult {
    pub assumevalid_block: [u8; 32],
    pub assumevalid_height: u32,
    pub snapshot_coins: u64,
    pub snapshot_value: u64,
    /// Core's hash_serialized_3 of the snapshot, 64 hex digits (display order) and a NUL
    pub hash_serialized_3: [u8; 65],
}

/// See include/zapp_assume.h.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn swiftsync_verify_assume(receipt: *const u8, receipt_len: usize, network: u8, now: u64,
                                                 image_hex: *const c_char, vk: *const u8, vk_len: usize, vk_pin: *const u8,
                                                 expect: *const c_char, assumevalid_block: *const u8, headers: *const u8, headers_len: usize,
                                                 snapshot: *const u8, snapshot_len: usize, tracked: *const c_char,
                                                 out: *mut ZappAssumeChain, aout: *mut SwiftsyncAssumeResult,
                                                 msg: *mut c_char, msg_len: usize) -> i32 {
    let fail = |m: &str| { write_msg(msg, msg_len, m); FAIL };
    // the receipt and the chain identity as the core's swiftsync_verify_facts checks them (zapp_corecheck::verify_core: the
    // core verifier's checks; no anchor: a node's shortcut must rest on a chain from genesis; a .rcpt, not a bundle)
    let Some(base) = Image::by_network(match network & !0x30 { 0 => "mainnet", 1 => "signet", _ => "" }) else { return fail("unknown network") };
    let image_hex = if image_hex.is_null() { String::new() } else {
        match CStr::from_ptr(image_hex).to_str() { Ok(h) => h.to_string(), Err(_) => return fail("image id is not UTF-8") }
    };
    // the core image's id and its verifying key (engine/VERIFIER.md): no engine image is published yet, so both are given
    if image_hex.trim().is_empty() || vk.is_null() || vk_len == 0 || vk_pin.is_null() {
        return fail("no engine core image is published yet: give the image id, its verifying key and the key's 32-byte pin");
    }
    let pin: [u8; 32] = slice(vk_pin, 32).try_into().unwrap();
    let image = match zapp_corecheck::image_for(base.network, &image_hex, network & 0x10 != 0, network & 0x20 != 0)
        .and_then(|i| zapp_corecheck::with_key(i, slice(vk, vk_len), Some(&pin))) {
        Ok(i) => i,
        Err(e) => return fail(&e),
    };
    let expect_s = if expect.is_null() { None } else {
        match CStr::from_ptr(expect).to_str() { Ok(e) => Some(e.trim().to_string()).filter(|e| !e.is_empty()), Err(_) => return fail("expect is not UTF-8") }
    };
    let rb = slice(receipt, receipt_len);
    if rb.starts_with(swiftsync_verify::BUNDLE_MAGIC) {
        return fail("assume: a proof bundle is not accepted here: give the receipt");
    }
    let core = zapp_corecheck::verify_core(rb, image, None, now, expect_s.as_deref());
    let rep = core.report;
    let tip_checked = core.tip_checked;
    if rep.verdict == Verdict::Fail || rep.checks.iter().any(|(ok, _)| !ok) {
        return fail(rep.checks.iter().find(|(ok, _)| !ok).map(|(_, w)| w.as_str()).unwrap_or("failed"));
    }
    let Some(cs) = rep.state.clone() else { return fail("no verified state") };
    let rc = if rep.verdict == Verdict::Ok { OK } else { DEMO };
    let tracked_s = if tracked.is_null() { None } else {
        match CStr::from_ptr(tracked).to_str() { Ok(t) => Some(t), Err(_) => return fail("tracked list is not UTF-8") }
    };
    let av = if assumevalid_block.is_null() { None } else {
        let b: [u8; 32] = slice(assumevalid_block, 32).try_into().unwrap();
        Some(if b == [0; 32] { None } else { Some(b) })
    };
    if av.is_none() && snapshot.is_null() {
        return fail("assume: neither an assumevalid block nor a snapshot was given");
    }
    let o = zapp_assume::run_assume(&cs, zapp_assume::AssumeRequest {
        assumevalid: av,
        headers: if headers.is_null() { None } else { Some(slice(headers, headers_len)) },
        snapshot: if snapshot.is_null() { None } else { Some(slice(snapshot, snapshot_len)) },
        tracked: tracked_s,
    });
    if let Some((_, w)) = o.checks.iter().find(|(ok, _)| !ok) {
        return fail(w);
    }
    if !out.is_null() {
        *out = ZappAssumeChain { height: cs.height, tip_hash: cs.tip_hash, chainwork: cs.cum_work, utxo_leaves: cs.utxo_leaves, profile: cs.profile };
    }
    if !aout.is_null() {
        let mut a = SwiftsyncAssumeResult { assumevalid_block: [0; 32], assumevalid_height: 0, snapshot_coins: 0, snapshot_value: 0,
                                            hash_serialized_3: [0; 65] };
        if let Some(v) = &o.valid {
            a.assumevalid_block = v.block;
            a.assumevalid_height = v.height;
        }
        if let Some(u) = &o.utxo {
            a.snapshot_coins = u.coins;
            a.snapshot_value = u.value;
            a.hash_serialized_3[..64].copy_from_slice(u.hash_serialized_3.as_bytes());
        }
        *aout = a;
    }
    // as facts: OK only for a proof with enough work whose tip the caller matched with `expect`
    if rc == OK && tip_checked {
        write_msg(msg, msg_len, "proven from genesis, chain identity checked; the assumevalid block / snapshot is proven");
        OK
    } else {
        write_msg(msg, msg_len, "the assumevalid block / snapshot matches the proof, but the chain is below minimum work or the checkpoint, or its tip was not matched with expect: not shown to be the network's");
        DEMO
    }
}

#[cfg(test)]
mod assume_tests {
    use super::*;
    const RCPT: &[u8] = include_bytes!("../../../../swiftsync-verify/vectors/mainnet_1_200_engine.hze");
    const FLIPPED: &[u8] = include_bytes!("../../../../swiftsync-verify/vectors/mainnet_1_200_engine_proof_flipped.hze");
    const VK: &[u8] = include_bytes!("../../../../swiftsync-verify/vectors/mainnet_1_200_engine.vk");
    const ID: &str = include_str!("../../../../swiftsync-verify/vectors/mainnet_1_200_engine.image_id");
    const SNAP: &[u8] = include_bytes!("../../vectors/utxo_200.dat");
    const SNAP199: &[u8] = include_bytes!("../../vectors/utxo_199.dat");
    const HDRS: &[u8] = include_bytes!("../../vectors/headers_0_200.bin");
    const TIP: &str = "200:000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320\0";

    fn call(rcpt: &[u8], av: Option<[u8; 32]>, snap: Option<&[u8]>, expect: Option<&str>) -> (i32, String, SwiftsyncAssumeResult) {
        let mut out = std::mem::MaybeUninit::<ZappAssumeChain>::zeroed();
        let mut a = std::mem::MaybeUninit::<SwiftsyncAssumeResult>::zeroed();
        let mut msg = [0 as c_char; 512];
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let avb = av.map(|b| b.to_vec());
        let id = std::ffi::CString::new(ID.trim()).unwrap();
        use sha2::Digest as _;
        let pin: [u8; 32] = sha2::Sha256::digest(VK).into();
        let rc = unsafe { swiftsync_verify_assume(rcpt.as_ptr(), rcpt.len(), 0, now, id.as_ptr(), VK.as_ptr(), VK.len(), pin.as_ptr(),
            expect.map_or(std::ptr::null(), |e| e.as_ptr() as *const c_char), avb.as_ref().map_or(std::ptr::null(), |b| b.as_ptr()),
            HDRS.as_ptr(), HDRS.len(), snap.map_or(std::ptr::null(), |s| s.as_ptr()), snap.map_or(0, |s| s.len()), std::ptr::null(),
            out.as_mut_ptr(), a.as_mut_ptr(), msg.as_mut_ptr(), msg.len()) };
        (rc, unsafe { CStr::from_ptr(msg.as_ptr()) }.to_string_lossy().to_string(), unsafe { a.assume_init() })
    }

    #[test]
    fn the_real_snapshot_and_assumevalid() {
        let (rc, m, a) = call(RCPT, Some([0; 32]), Some(SNAP), Some(TIP));
        assert_eq!(rc, DEMO, "{m}"); // 200 blocks: below minimum work
        assert_eq!((a.snapshot_coins, a.snapshot_value, a.assumevalid_height), (204, 1_000_000_000_000, 200));
        assert_eq!(&a.hash_serialized_3[..64], b"a51f51aea7cf42fc9f1ab491532de766675e983fde7166fa6273a6d6a6439d67");
        let mut b170 = swiftsync_verify::hex32("00000000d1145790a8694403d4063f323d499e655c83426834d4ce2f8dd4a2ee").unwrap();
        b170.reverse();
        let (rc, m, a) = call(RCPT, Some(b170), None, None);
        assert_eq!((rc, a.assumevalid_height), (DEMO, 170), "{m}");
    }

    #[test]
    fn refusals() {
        let mut bad = SNAP.to_vec();
        bad[51] ^= 1;
        assert_eq!(call(RCPT, None, Some(&bad), None).0, FAIL);
        assert!(call(RCPT, None, Some(SNAP199), None).1.contains("tip only"));
        assert_eq!(call(FLIPPED, None, Some(SNAP), None).0, FAIL);
        assert_eq!(call(RCPT, None, Some(SNAP), Some("200:00000000d1145790a8694403d4063f323d499e655c83426834d4ce2f8dd4a2ee\0")).0, FAIL);
        assert!(call(RCPT, Some([7; 32]), None, None).1.contains("not in the headers"));
        assert!(call(RCPT, None, None, None).1.contains("neither"));
    }
}
