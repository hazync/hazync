//! `zapp-statement-wasm`: the balance statement zyncAPP's checker in a browser (docs/libzync/STATEMENT_ZAPP.md). The core
//! receipt is verified by `swiftsync_verify::verify` (the core verifier as a library), the statement by
//! `zapp_statement::check_statement`. Local: nothing leaves the device.
//!
//! ```text
//!   alloc(len) -> ptr                    reserve len bytes for the input frame
//!   verify_statement(ptr, len) -> p      [u32 LE length][UTF-8 JSON]
//! ```
//!
//! Input frame (little-endian): `[u8 network: 0 mainnet, 1 signet][u64 now, unix seconds][u32 n][receipt]`
//! `[u32 n][image id hex, n = 0 for the network's built-in image][u32 n][statement JSON]`
//! `[u32 n][expect "<height>:<hash>"][u32 n][purpose][u32 n][nonce][u32 n][max age, blocks][u32 n][network tip height]`.
//! expect, purpose, nonce, the max age and the tip height come from the PAGE (the block it trusts, the challenge it gave
//! the holder, its recency policy and the tip it fetched), never from the statement's sender; each may be empty. Without
//! expect or the challenge the verdict is at most "demo"; without a max age (and a tip) `age_checked` is false and
//! `statement.what` says so. Purpose and nonce are passed as given (not trimmed): a statement with surrounding spaces is
//! refused, as by the CLI. The JSON has `verdict` ("ok", "demo", "fail"), `checks`, `statement` {ok, what, total,
//! scripts}, `age_checked` and `demo_because`.

use zapp_statement::{check_statement, Challenge, Statement};
use swiftsync_verify::{identity_matches, parse_expect, Verdict};

#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

fn ret(s: String) -> *const u8 {
    let b = s.into_bytes();
    let mut out = Vec::with_capacity(4 + b.len());
    out.extend_from_slice(&(b.len() as u32).to_le_bytes());
    out.extend_from_slice(&b);
    let p = out.as_ptr();
    std::mem::forget(out);
    p
}

struct Frame<'a> { b: &'a [u8], i: usize }
impl<'a> Frame<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.i.checked_add(n).ok_or("input frame truncated")?;
        let out = self.b.get(self.i..end).ok_or("input frame truncated")?;
        self.i = end;
        Ok(out)
    }
    fn chunk(&mut self) -> Result<&'a [u8], String> {
        let n = u32::from_le_bytes(self.take(4)?.try_into().unwrap()) as usize;
        self.take(n)
    }
    fn text(&mut self) -> Result<String, String> {
        Ok(self.raw()?.trim().to_string())
    }
    /// As given: for the purpose and nonce, which must be the signed bytes.
    fn raw(&mut self) -> Result<String, String> {
        Ok(std::str::from_utf8(self.chunk()?).map_err(|_| "not UTF-8".to_string())?.to_string())
    }
}

fn run(input: &[u8]) -> Result<serde_json::Value, String> {
    let mut f = Frame { b: input, i: 0 };
    let network = match f.take(1)?[0] { 0 => "mainnet", 1 => "signet", n => return Err(format!("unknown network code {n}")) };
    let now = u64::from_le_bytes(f.take(8)?.try_into().unwrap());
    let receipt = f.chunk()?;
    let image_hex = f.text()?.to_lowercase();
    let st: Statement = serde_json::from_slice(f.chunk()?).map_err(|e| format!("statement JSON: {e}"))?;
    let (expect, purpose, nonce) = (f.text()?, f.raw()?, f.raw()?);
    let num = |t: String, what: &str| -> Result<Option<u32>, String> {
        if t.is_empty() { Ok(None) } else { t.parse().map(Some).map_err(|_| format!("{what}: not a number")) }
    };
    let max_age = num(f.text()?, "max age")?;
    let tip_height = num(f.text()?, "tip height")?;
    if f.i != input.len() {
        return Err("input frame has trailing bytes".into());
    }
    // the image as every zAPP resolves it; an image id given is the only image the receipt may verify against (pinned)
    let image = zapp_corecheck::image_for(network, &image_hex, false, false)?;
    // the image reported is the one the receipt verified against (CoreCheck.image): asked for none, a receipt of an
    // earlier built-in image of the network verifies as that image, not the one asked for (review 11-18)
    let core = zapp_corecheck::verify_core(receipt, image, !image_hex.trim().is_empty(), None, now, None);
    let (mut rep, image) = (core.report, core.image);
    let mut why: Vec<&str> = Vec::new();
    let mut age_checked = false;
    let statement = match rep.state.as_ref().filter(|_| rep.verdict != Verdict::Fail) {
        None => Err("statement NOT checked: the receipt did not verify".to_string()),
        Some(cs) => {
            let tip = parse_expect(&expect).map_or(false, |(h, x)| identity_matches(h, &x, cs.height, &cs.tip_hash));
            if !expect.is_empty() && !tip {
                Err(format!("expect {expect:?}: not the proof's tip"))
            } else {
                let ch = Challenge { purpose: Some(purpose.as_str()).filter(|p| !p.is_empty()),
                                     nonce: Some(nonce.as_str()).filter(|n| !n.is_empty()) };
                let age = match (max_age, tip_height) {
                    (None, _) => Ok(None),
                    (Some(_), None) => Err("max age given without the network's tip height".to_string()),
                    (Some(m), Some(t)) => zapp_statement::check_age(cs.height, t, m).map(Some),
                };
                age.and_then(|age| check_statement(cs, rep.undo_from, &st, &ch).map(|c| (c, age))).map(|(mut c, age)| {
                    match age {
                        Some(l) => { c.line = format!("{}; {l}", c.line); age_checked = true; }
                        None => c.line = format!("{}; its age is NOT checked (the coins may have moved since)", c.line),
                    }
                    if rep.verdict == Verdict::Demo { why.push("the chain is below Core's minimum chainwork or the pinned checkpoint"); }
                    if !tip { why.push("the tip was not compared with a source you trust (expect)"); }
                    if !c.challenged { why.push("the statement was not compared with your challenge (purpose and nonce)"); }
                    c
                })
            }
        }
    };
    rep.verdict = match (&statement, rep.verdict) {
        (Err(_), _) | (_, Verdict::Fail) => Verdict::Fail,
        (Ok(_), _) if !why.is_empty() => Verdict::Demo,
        _ => Verdict::Ok,
    };
    Ok(serde_json::json!({
        "verdict": match rep.verdict { Verdict::Ok => "ok", Verdict::Demo => "demo", Verdict::Fail => "fail" },
        "image": image.method_id_hex,
        "network": image.network,
        "checks": rep.checks.iter().map(|(ok, what)| serde_json::json!({"ok": ok, "what": what})).collect::<Vec<_>>(),
        "statement": match &statement {
            Ok(c) => serde_json::json!({"ok": rep.verdict == Verdict::Ok, "what": c.line, "total": c.total, "scripts": c.scripts}),
            Err(e) => serde_json::json!({"ok": false, "what": e}),
        },
        "age_checked": age_checked,
        "demo_because": why,
    }))
}

#[no_mangle]
pub extern "C" fn verify_statement(ptr: *mut u8, len: usize) -> *const u8 {
    let input = unsafe { Vec::from_raw_parts(ptr, len, len) };
    ret(match run(&input) {
        Ok(v) => v.to_string(),
        Err(e) => serde_json::json!({"verdict": "fail", "checks": [{"ok": false, "what": e}], "statement": {"ok": false, "what": e}}).to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Review finding 7 (feat/headers-mmr): `self.i + n` overflowed (a panic in a debug build) on a length near usize::MAX.
    #[test]
    fn frame_take_does_not_overflow() {
        let b = [1u8, 2, 3];
        let mut f = Frame { b: &b, i: 1 };
        assert!(f.take(usize::MAX).is_err());
        assert_eq!(f.i, 1);
        assert_eq!(f.take(2).unwrap(), &[2, 3]);
        assert!(f.take(1).is_err());
    }
}
