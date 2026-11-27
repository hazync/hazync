//! C ABI for the coin-liveness zyncAPP (include/zapp_liveness.h), moved out of swiftsync-verify-ffi's facts JSON
//! (`"liveness"`) on 15.11.2026: a zAPP module of its own (docs/libzync/ZYNCAPPS.md §9). One implementation of the
//! check: this calls the same `zapp_liveness::verify_liveness` as the CLI and the WASM build. Bad input is an error,
//! never a panic; and since the crate is built with `panic = "abort"` (no unwinding, so no `catch_unwind`), a panic
//! from a bug aborts the process rather than unwinding into C.

use std::ffi::{c_char, CStr};
use swiftsync_verify::{ChainState, Verdict};

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

unsafe fn text<'a>(p: *const c_char, what: &str) -> Result<Option<&'a str>, String> {
    if p.is_null() { return Ok(None); }
    CStr::from_ptr(p).to_str().map(Some).map_err(|_| format!("{what} is not UTF-8"))
}

/// See include/zapp_liveness.h.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn zapp_liveness_verify(receipt: *const u8, receipt_len: usize, network: u8, now: u64,
                                              image_hex: *const c_char, vk: *const u8, vk_len: usize, vk_pin: *const u8,
                                              anchor_json: *const c_char,
                                              facts_json: *const c_char, expect: *const c_char,
                                              msg: *mut c_char, msg_len: usize) -> i32 {
    let fail = |m: &str| { write_msg(msg, msg_len, m); FAIL };
    let net = match network & !0x30 { 0 => "mainnet", 1 => "signet", _ => return fail("unknown network") };
    let (image_hex, anchor_json, facts_json, expect) =
        match (text(image_hex, "image id"), text(anchor_json, "anchor"), text(facts_json, "facts JSON"), text(expect, "expect")) {
            (Ok(a), Ok(b), Ok(c), Ok(d)) => (a, b, c, d),
            (Err(e), ..) | (_, Err(e), ..) | (_, _, Err(e), _) | (.., Err(e)) => return fail(&e),
        };
    // the core image's id and its verifying key (engine/VERIFIER.md): no engine image is published yet, so both are given
    if image_hex.is_none_or(|h| h.trim().is_empty()) || vk.is_null() || vk_len == 0 || vk_pin.is_null() {
        return fail("no engine core image is published yet: give the image id, its verifying key and the key's 32-byte pin");
    }
    let pin: [u8; 32] = slice(vk_pin, 32).try_into().unwrap();
    let image = match zapp_liveness::image_for(net, image_hex.unwrap_or(""), network & 0x10 != 0, network & 0x20 != 0)
        .and_then(|i| zapp_corecheck::with_key(i, slice(vk, vk_len), &pin)) {
        Ok(i) => i,
        Err(e) => return fail(&e),
    };
    let anchor: Option<ChainState> = match anchor_json.map(serde_json::from_str) {
        None => None,
        Some(Ok(a)) => Some(a),
        Some(Err(e)) => return fail(&format!("anchor is not a ChainState: {e}")),
    };
    // {"liveness": [{"proof": "<hex>", "ask": "<question>"}]} and no other key (zapp_liveness::facts_from_json); an
    // "expect" inside it is the fact server vouching for its own fork (cold review 11-06): refused, as in the core's.
    let facts = match facts_json {
        None => return fail("no facts JSON: give {\"liveness\": [...]}"),
        Some(j) => match zapp_liveness::facts_from_json(j.as_bytes()) { Ok(f) => f, Err(e) => return fail(&e) },
    };
    let o = zapp_liveness::verify_liveness(slice(receipt, receipt_len), image, anchor, now, &facts, expect, true);
    match o.report.verdict {
        Verdict::Ok => { write_msg(msg, msg_len, "every liveness claim holds on the proven chain, its tip the block expect names"); OK }
        Verdict::Demo => { write_msg(msg, msg_len, "the liveness claims hold, but the chain is not shown to be the network's (no matching expect, below minimum work, or from an anchor)"); DEMO }
        Verdict::Fail => fail(o.report.checks.iter().filter(|(ok, _)| !ok).map(|(_, w)| w.as_str())
            .chain(o.facts.iter().filter_map(|r| r.as_ref().err().map(|e| e.as_str()))).next().unwrap_or("failed")),
    }
}
