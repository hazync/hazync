//! C ABI for `swiftsync-verify` (include/swiftsync_verify.h). One implementation of the checks: this calls the same
//! `swiftsync_verify::verify` / `check_bundle` as the CLI and the WASM build. No panics cross the boundary: input is
//! validated, and the library itself does not panic on input (see swiftsync-verify/src/lib.rs).

use std::ffi::{c_char, CStr};
use swiftsync_verify::{check_bundle, verify, Bundle, ChainState, Image, Inputs, Verdict, BUNDLE_MAGIC};

#[repr(C)]
pub struct SwiftsyncResult {
    pub height: u32,
    pub tip_hash: [u8; 32],
    pub chainwork: [u8; 32],
    pub utxo_leaves: u64,
    pub profile: u32,
}

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

/// See include/swiftsync_verify.h.
#[no_mangle]
pub unsafe extern "C" fn swiftsync_verify(receipt: *const u8, receipt_len: usize, hints: *const u8, hints_len: usize,
                                          network: u8, now: u64, image_hex: *const c_char, anchor_json: *const c_char,
                                          out: *mut SwiftsyncResult, msg: *mut c_char, msg_len: usize) -> i32 {
    let fail = |m: &str| { write_msg(msg, msg_len, m); FAIL };
    let Some(mut image) = Image::by_network(match network { 0 => "mainnet", 1 => "signet", _ => "" }) else {
        return fail("unknown network");
    };
    if !image_hex.is_null() {
        let Ok(h) = CStr::from_ptr(image_hex).to_str() else { return fail("image id is not UTF-8") };
        let Some(i) = image.with_method_id(&h.trim().to_lowercase()) else { return fail("image id is not 64 hex digits") };
        image = i;
    }
    let anchor: Option<ChainState> = if anchor_json.is_null() {
        None
    } else {
        match serde_json::from_slice(CStr::from_ptr(anchor_json).to_bytes()) {
            Ok(a) => Some(a),
            Err(e) => return fail(&format!("anchor is not a ChainState: {e}")),
        }
    };
    let (receipt, hints) = (slice(receipt, receipt_len), slice(hints, hints_len));
    let rep = if receipt.starts_with(BUNDLE_MAGIC) {
        match Bundle::decode(receipt) {
            Ok(b) => check_bundle(&b, hints, image, anchor, now),
            Err(e) => return fail(&e),
        }
    } else {
        verify(&Inputs { receipt, hints, image, anchor, now })
    };
    if let (Some(cs), false) = (&rep.state, out.is_null()) {
        *out = SwiftsyncResult { height: cs.height, tip_hash: cs.tip_hash, chainwork: cs.cum_work,
                                 utxo_leaves: cs.utxo_leaves, profile: cs.profile };
    }
    match rep.verdict {
        Verdict::Ok => { write_msg(msg, msg_len, ""); OK }
        Verdict::Demo => { write_msg(msg, msg_len, "verified against the given anchor only"); DEMO }
        Verdict::Fail => fail(rep.checks.iter().find(|(ok, _)| !ok).map(|(_, w)| w.as_str()).unwrap_or("failed")),
    }
}

static IDS: [&str; 2] = [
    "9f469930c612139342e4f2a403eccba7663b1231ecc53b396470c6fe93e72f55\0",
    "067822b958f7cc86d67ad9f37153b99c29664072a8e90a79a2d9af4dc71f5b36\0",
];

/// See include/swiftsync_verify.h.
#[no_mangle]
pub extern "C" fn swiftsync_image_id(network: u8) -> *const c_char {
    match IDS.get(network as usize) {
        Some(s) => s.as_ptr() as *const c_char,
        None => std::ptr::null(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ids_match_the_library() {
        for (k, n) in ["mainnet", "signet"].iter().enumerate() {
            assert_eq!(super::IDS[k].trim_end_matches('\0'), swiftsync_verify::Image::by_network(n).unwrap().method_id_hex);
        }
    }
}
