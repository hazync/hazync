//! C ABI for the SwiftSync hints zyncAPP (include/zapp_hints.h), moved out of swiftsync-verify-ffi on 15.11.2026: a
//! zAPP module of its own (docs/libzync/ZYNCAPPS.md §9). One implementation of the check: this calls the same
//! `zapp_hints::verify_hints` as the CLI and the WASM build. Bad input is an error, never a panic; and since the crate
//! is built with `panic = "abort"` (no unwinding, so no `catch_unwind`), a panic from a bug aborts the process rather
//! than unwinding into C.

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

/// See include/zapp_hints.h.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn swiftsync_verify_hints(receipt: *const u8, receipt_len: usize, file: *const u8, file_len: usize,
                                                undo: *const u8, undo_len: usize, image_hex: *const c_char,
                                                core_image_hex: *const c_char, vk: *const u8, vk_len: usize, vk_pin: *const u8,
                                                network: *const c_char, format: u32, now: u64, expect: *const c_char,
                                                out: *mut SwiftsyncHints, msg: *mut c_char, msg_len: usize) -> i32 {
    let fail = |m: &str| { write_msg(msg, msg_len, m); FAIL };
    let exp = if expect.is_null() { None } else {
        let Ok(e) = CStr::from_ptr(expect).to_str() else { return fail("expect is not UTF-8") };
        match e.split_once(':').and_then(|(h, x)| h.parse::<u32>().ok().map(|h| (h, x.to_string()))) {
            Some(x) => Some(x),
            None => return fail("expect: give height:hash"),
        }
    };
    // the hints image, its key and the core image (engine/VERIFIER.md): no engine image is published yet, so all are given
    let text = |p: *const c_char| -> Option<String> { if p.is_null() { None } else { CStr::from_ptr(p).to_str().ok().map(|s| s.trim().to_string()) } };
    let (Some(image), Some(core)) = (text(image_hex), text(core_image_hex)) else {
        return fail("no engine hints image is published yet: give the hints image id and the core image id (UTF-8)");
    };
    if vk.is_null() || vk_len == 0 || vk_pin.is_null() {
        return fail("an engine proof needs the hints image's verifying key and the key's 32-byte pin");
    }
    let (vkb, pin): (&[u8], [u8; 32]) = (slice(vk, vk_len), slice(vk_pin, 32).try_into().unwrap());
    {
        use sha2::Digest as _;
        if <[u8; 32]>::from(sha2::Sha256::digest(vkb)) != pin {
            return fail("the verifying key is not the image's pinned key (its SHA-256 is not vk_pin)");
        }
    }
    // review R3-4 L1, L2: the node's network (NULL: mainnet) and the format it reads (0: not compared, at most DEMO)
    let net = if network.is_null() { "mainnet".to_string() } else {
        match CStr::from_ptr(network).to_str() { Ok(n) => n.to_string(), Err(_) => return fail("network is not UTF-8") }
    };
    if net != "mainnet" && net != "signet" { return fail("network: mainnet or signet"); }
    let fmt = match format { 0 => None, 1 | 2 => Some(format), _ => return fail("format: 0 (not compared), 1 (bip) or 2 (crate)") };
    let u = slice(undo, undo_len);
    let rep = zapp_hints::verify_hints(slice(receipt, receipt_len), slice(file, file_len), (!u.is_empty()).then_some(u),
                                                    &image, &swiftsync_verify::EngineKey { bytes: vkb, pin }, &core,
                                                    &zapp_hints::HintsAsk { network: &net, format: fmt }, now,
                                                    exp.as_ref().map(|(h, x)| (*h, x.as_str())), None);
    if let (Some(z), false, false) = (&rep.statement, out.is_null(), rep.verdict == Verdict::Fail) {
        *out = SwiftsyncHints { height: z.height, block_hash: z.block_hash, format: z.format, n_unspent: z.n_unspent, unspent_value: z.unspent_value };
    }
    match rep.verdict {
        Verdict::Ok => { write_msg(msg, msg_len, &format!("the {net} hints file is proven for block H, and block H is the one expect names{} (under the images you named: trust it only if you built them)",
                                                       if u.is_empty() { "; undo hashes not checked" } else { "" })); OK }
        Verdict::Demo => { write_msg(msg, msg_len, "a proven hints file, but block H was not compared with your header chain (expect), the format was not compared (format 0), or the core chain is below minimum work or the checkpoint"); DEMO }
        Verdict::Fail => fail(rep.checks.iter().find(|(ok, _)| !ok).map(|(_, w)| w.as_str()).unwrap_or("failed")),
    }
}
