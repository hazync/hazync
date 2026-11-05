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
pub unsafe extern "C" fn swiftsync_verify(receipt: *const u8, receipt_len: usize,
                                          network: u8, now: u64, image_hex: *const c_char, anchor_json: *const c_char,
                                          out: *mut SwiftsyncResult, msg: *mut c_char, msg_len: usize) -> i32 {
    swiftsync_verify_facts(receipt, receipt_len, network, now, image_hex, anchor_json, std::ptr::null(),
                           out, msg, msg_len)
}

/// As `swiftsync_verify`, plus zyncAPP facts (ZYNCAPPS.md §8): `facts_json` (NUL-terminated, may be NULL) is
/// `{"coins": [CoinFact], "journeys": [CoinJourney], "tries": [TrieFact], "expect": "h:hash"}`, each checked against the verified state. Any fact that does not
/// hold is FAIL, with its reason in `msg`.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn swiftsync_verify_facts(receipt: *const u8, receipt_len: usize,
                                                network: u8, now: u64, image_hex: *const c_char, anchor_json: *const c_char,
                                                facts_json: *const c_char,
                                                out: *mut SwiftsyncResult, msg: *mut c_char, msg_len: usize) -> i32 {
    let fail = |m: &str| { write_msg(msg, msg_len, m); FAIL };
    // network: 0 mainnet, 1 signet; bit 0x10 = a coin-log image (the one whose proofs carry coin facts); bit 0x20 = the
    // product image (coin log, counting trie, live-coin hash: the one whose proofs carry trie facts).
    let Some(mut image) = Image::by_network(match network & !0x30 { 0 => "mainnet", 1 => "signet", _ => "" }) else {
        return fail("unknown network");
    };
    if network & 0x10 != 0 {
        image = image.with_coinlog();
    }
    if network & 0x20 != 0 {
        image = image.with_product();
    }
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
    let receipt = slice(receipt, receipt_len);
    let rep = if receipt.starts_with(BUNDLE_MAGIC) {
        match Bundle::decode(receipt) {
            Ok(b) => check_bundle(&b, image, anchor, now),
            Err(e) => return fail(&e),
        }
    } else {
        verify(&Inputs { receipt, image, anchor, now })
    };
    let mut rep = rep;
    if !facts_json.is_null() {
        let v: serde_json::Value = match serde_json::from_slice(CStr::from_ptr(facts_json).to_bytes()) {
            Ok(v) => v,
            Err(e) => return fail(&format!("facts JSON: {e}")),
        };
        let coins: Vec<swiftsync_verify::CoinFact> = match serde_json::from_value(v.get("coins").cloned().unwrap_or_else(|| serde_json::json!([]))) {
            Ok(c) => c,
            Err(e) => return fail(&format!("facts JSON coins: {e}")),
        };
        let journeys: Vec<swiftsync_verify::CoinJourney> = match serde_json::from_value(v.get("journeys").cloned().unwrap_or_else(|| serde_json::json!([]))) {
            Ok(j) => j,
            Err(e) => return fail(&format!("facts JSON journeys: {e}")),
        };
        let tries: Vec<swiftsync_verify::TrieFact> = match serde_json::from_value(v.get("tries").cloned().unwrap_or_else(|| serde_json::json!([]))) {
            Ok(t) => t,
            Err(e) => return fail(&format!("facts JSON tries: {e}")),
        };
        // Facts need the tip compared with the network (step 7): "expect": "h:hash" in the facts JSON. Without a
        // match the verdict is at most DEMO (adversarial review, 11-03).
        let tip_checked = match (v.get("expect").and_then(|x| x.as_str()), rep.state.as_ref()) {
            (Some(e), Some(cs)) => match swiftsync_verify::parse_expect(e) {
                Some((h, x)) if swiftsync_verify::identity_matches(h, &x, cs.height, &cs.tip_hash) => true,
                _ => { rep.checks.push((false, format!("expect {e:?}: not the proof's tip"))); false }
            },
            _ => false,
        };
        let any = !coins.is_empty() || !journeys.is_empty() || !tries.is_empty();
        match rep.state.as_ref().filter(|_| rep.verdict != Verdict::Fail) {
            Some(cs) => {
                for c in &coins {
                    let r = swiftsync_verify::check_coin_fact(cs, c);
                    rep.checks.push((r.is_ok(), r.unwrap_or_else(|e| e)));
                }
                for j in &journeys {
                    let r = swiftsync_verify::check_coin_journey(cs, j);
                    rep.checks.push((r.is_ok(), r.unwrap_or_else(|e| e)));
                }
                for t in &tries {
                    let r = trie_check(cs, t);
                    rep.checks.push((r.is_ok(), r.unwrap_or_else(|e| e)));
                }
            }
            None if any => rep.checks.push((false, "facts NOT checked: the receipt did not verify".into())),
            None => {}
        }
        if rep.checks.iter().any(|(ok, _)| !ok) {
            rep.verdict = Verdict::Fail;
        }
        if rep.verdict == Verdict::Ok && any && !tip_checked {
            rep.verdict = Verdict::Demo;
        }
    }
    if let (Some(cs), false) = (&rep.state, out.is_null()) {
        *out = SwiftsyncResult { height: cs.height, tip_hash: cs.tip_hash, chainwork: cs.cum_work,
                                 utxo_leaves: cs.utxo_leaves, profile: cs.profile };
    }
    match rep.verdict {
        Verdict::Ok => { write_msg(msg, msg_len, ""); OK }
        Verdict::Demo => { write_msg(msg, msg_len, "a valid chain, but from a given anchor, below minimum work, or below the checkpoint height, or facts without a matching \"expect\": not shown to be the network's"); DEMO }
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

/// The C result carries no answer, so a trie fact must CLAIM one (its "expect") for OK to mean anything.
fn trie_check(cs: &swiftsync_verify::ChainState, t: &swiftsync_verify::TrieFact) -> Result<String, String> {
    match &t.expect {
        None => Err(format!("trie fact ({}) without \"expect\": the C ABI returns only whether a claim holds", t.kind)),
        Some(_) => swiftsync_verify::check_trie_fact(cs, t),
    }
}

#[cfg(test)]
mod trie_tests {
    use super::*;

    #[test]
    fn trie_facts_need_a_claim_and_a_false_claim_fails() {
        use libzync::trie_fact::coin_key;
        let a = b"\x00\x14aaaaaaaaaaaaaaaaaaaa".to_vec();
        let disp = "11".repeat(31) + "22";
        let mut txid = [0x11u8; 32];
        txid[31] = 0x22;
        txid.reverse();
        let mut st = libzync::trie_store::TrieStore::new(true);
        st.apply(&[], &[(coin_key(&a, &txid, 0), None, 500)]).unwrap();
        st.height = 1;
        let mut cs = swiftsync_verify::genesis_anchor(&swiftsync_verify::IMAGES[0]);
        cs.profile |= swiftsync_verify::PROFILE_COINLOG | swiftsync_verify::PROFILE_TRIE;
        cs.height = 1;
        cs.trie_root = libzync::coinlog::h_of(&st.root());
        let f = swiftsync_verify::make_trie_fact(&st, "coin", &a, Some(&disp), Some(0)).unwrap();
        assert!(trie_check(&cs, &f).is_ok());
        let mut q = f.clone();
        q.expect = None;
        assert!(trie_check(&cs, &q).unwrap_err().contains("without"));
        let mut l = f.clone();
        l.expect = Some("absent".into());
        assert!(trie_check(&cs, &l).is_err());
    }
}
