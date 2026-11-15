//! C ABI for the SwiftSync hints zyncAPP (include/zapp_hints.h), moved out of swiftsync-verify-ffi on 15.11.2026: a
//! zAPP module of its own (docs/libzync/ZYNCAPPS.md §9). One implementation of the check: this calls the same
//! `zapp_hints::verify_hints` as the CLI and the WASM build. No panics cross the boundary.

use std::ffi::{c_char, CStr};
use swiftsync_verify::Verdict;

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

/// The result of `swiftsync_verify_hints` (only filled when the verdict is not FAIL).
#[repr(C)]
pub struct SwiftsyncHints {
    pub height: u32,
    pub block_hash: [u8; 32],
    pub format: u32,
    pub n_unspent: u64,
    pub unspent_value: u64,
}

/// See include/swiftsync_verify.h.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn swiftsync_verify_hints(receipt: *const u8, receipt_len: usize, file: *const u8, file_len: usize,
                                                undo: *const u8, undo_len: usize, now: u64, expect: *const c_char,
                                                out: *mut SwiftsyncHints, msg: *mut c_char, msg_len: usize) -> i32 {
    let fail = |m: &str| { write_msg(msg, msg_len, m); FAIL };
    let exp = if expect.is_null() { None } else {
        let Ok(e) = CStr::from_ptr(expect).to_str() else { return fail("expect is not UTF-8") };
        match e.split_once(':').and_then(|(h, x)| h.parse::<u32>().ok().map(|h| (h, x.to_string()))) {
            Some(x) => Some(x),
            None => return fail("expect: give height:hash"),
        }
    };
    let u = slice(undo, undo_len);
    let rep = zapp_hints::verify_hints(slice(receipt, receipt_len), slice(file, file_len), (!u.is_empty()).then_some(u),
                                                    zapp_hints::HINTS_IMAGE_ID, None, now,
                                                    exp.as_ref().map(|(h, x)| (*h, x.as_str())), None);
    if let (Some(z), false, false) = (&rep.statement, out.is_null(), rep.verdict == Verdict::Fail) {
        *out = SwiftsyncHints { height: z.height, block_hash: z.block_hash, format: z.format, n_unspent: z.n_unspent, unspent_value: z.unspent_value };
    }
    match rep.verdict {
        Verdict::Ok => { write_msg(msg, msg_len, "the hints file is proven for block H, and block H is the one expect names"); OK }
        Verdict::Demo => { write_msg(msg, msg_len, "a proven hints file, but block H was not compared with your header chain (expect), or the core chain is below minimum work or the checkpoint"); DEMO }
        Verdict::Fail => fail(rep.checks.iter().find(|(ok, _)| !ok).map(|(_, w)| w.as_str()).unwrap_or("failed")),
    }
}
