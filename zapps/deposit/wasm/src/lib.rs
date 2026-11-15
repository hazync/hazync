//! `zapp-deposit-wasm`: the deposit-proof zyncAPP (docs/libzync/DEPOSIT_ZAPP.md) in a browser, a WASM module of its own
//! (docs/libzync/ZYNCAPPS.md §9), NOT part of the core verifier's hazync-verify.wasm (until 15.11.2026 it was the
//! core's `verify_deposit`). Same raw ABI as the core's (no wasm-bindgen): `alloc(len) -> ptr`,
//! `verify_deposit(ptr, len) -> p` with p pointing to [u32 LE length][UTF-8 JSON]. Glue: zapp-deposit.js.

use swiftsync_verify::{hex, Image, Verdict};

#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

fn ret(s: String) -> *const u8 {
    let bytes = s.into_bytes();
    let mut out = Vec::with_capacity(4 + bytes.len());
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&bytes);
    let ptr = out.as_ptr();
    std::mem::forget(out);
    ptr
}

fn fail(msg: String) -> *const u8 {
    ret(serde_json::json!({"verdict": "fail", "checks": [{"ok": false, "what": msg}]}).to_string())
}

struct Frame<'a> {
    b: &'a [u8],
    i: usize,
}

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
}

/// The deposit-proof zyncAPP (docs/libzync/DEPOSIT_ZAPP.md), by the same `zapp_deposit::verify_deposit`
/// the CLI `zapp-deposit` uses. Input frame (little-endian): `[u64 now][u32 n][bundle JSON, as zapp-deposit build
/// writes it][u32 n][ask JSON]`. The ask is the CALLER's, never the bundle's: `{"txid" (as explorers print it), "vout"?,
/// "amount"? (sat), "script"? (hex), "op_return"? (hex payload), "min_depth"?, "min_validated_depth"?,
/// "allow_pow_only_block"?, "expect"? ("height:hash" from a source the page trusts), "max_tip_age"? (s), "network"?}`.
///
/// Output: `verdict` ("ok", "demo", "fail"), `checks`, and `statement` (null unless no check failed: read it with the
/// verdict) with the block, the output checked, the tip, `validated_through` (the core tip), `confirmations`,
/// `validated_confirmations` and `block_fully_validated`.
#[no_mangle]
pub extern "C" fn verify_deposit(ptr: *const u8, len: usize) -> *const u8 {
    use zapp_deposit::{disp, DepositAsk, DepositBundle};
    let input = unsafe { std::slice::from_raw_parts(ptr, len) };
    let parsed = (|| -> Result<(u64, DepositBundle, DepositAsk, String), String> {
        let mut f = Frame { b: input, i: 0 };
        let now = u64::from_le_bytes(f.take(8)?.try_into().unwrap());
        let b: DepositBundle = serde_json::from_slice(f.chunk()?).map_err(|e| format!("bundle: not a deposit bundle ({e})"))?;
        let v: serde_json::Value = serde_json::from_slice(f.chunk()?).map_err(|e| format!("ask: not JSON ({e})"))?;
        if f.i != input.len() { return Err("input frame has trailing bytes".into()); }
        let hexv = |k: &str| -> Result<Option<Vec<u8>>, String> {
            match &v[k] { serde_json::Value::Null => Ok(None),
                          x => x.as_str().and_then(swiftsync_verify::unhex).map(Some).ok_or(format!("ask: {k} is not hex")) }
        };
        let num = |k: &str| -> Result<Option<u64>, String> {
            match &v[k] { serde_json::Value::Null => Ok(None), x => x.as_u64().map(Some).ok_or(format!("ask: {k} is not a number")) }
        };
        let mut txid = v["txid"].as_str().and_then(|t| swiftsync_verify::hex32(&t.to_lowercase()))
            .ok_or("ask: txid (the transaction YOU ask about, 64 hex digits) is required")?;
        txid.reverse();
        let expect_tip = match &v["expect"] {
            serde_json::Value::Null => None,
            x => Some(x.as_str().and_then(|e| e.split_once(':')).and_then(|(h, x)| h.parse::<i64>().ok().map(|h| (h, x.to_string())))
                .ok_or("ask: expect is height:hash")?),
        };
        let vout = num("vout")?.map(|x| u32::try_from(x).map_err(|_| "ask: vout too large")).transpose()?;
        let ask = DepositAsk { txid, vout, amount: num("amount")?, script: hexv("script")?, op_return: hexv("op_return")?,
                               min_depth: num("min_depth")?.unwrap_or(1), min_validated_depth: num("min_validated_depth")?.unwrap_or(0),
                               allow_pow_only_block: v["allow_pow_only_block"].as_bool().unwrap_or(false), expect_tip,
                               max_tip_age: num("max_tip_age")? };
        Ok((now, b, ask, v["network"].as_str().unwrap_or("mainnet").to_string()))
    })();
    let (now, b, ask, network) = match parsed { Ok(p) => p, Err(e) => return fail(e) };
    let Some(image) = Image::by_network(&network) else { return fail(format!("unknown network {network:?}")) };
    let rep = zapp_deposit::verify_deposit(&b, &ask, &network, image, zapp_headers::HEADERS_IMAGE_ID, now);
    ret(serde_json::json!({
        "verdict": match rep.verdict { Verdict::Ok => "ok", Verdict::Demo => "demo", Verdict::Fail => "fail" },
        "checks": rep.checks.iter().map(|(ok, what)| serde_json::json!({"ok": ok, "what": what})).collect::<Vec<_>>(),
        "identity_checked": rep.verdict != Verdict::Fail && ask.expect_tip.is_some(),
        "statement": rep.statement.as_ref().filter(|_| rep.verdict != Verdict::Fail).map(|s| serde_json::json!({
            "txid": disp(&s.txid), "height": s.height, "block_hash": disp(&s.block_hash),
            "output": s.output.as_ref().map(|(n, v, spk)| serde_json::json!({"vout": n, "amount": v, "script": hex(spk)})),
            "tip_height": s.tip_height, "tip_hash": disp(&s.tip_hash), "tip_chainwork": disp(&s.tip_work),
            "validated_through": s.validated_through, "confirmations": s.confirmations,
            "validated_confirmations": s.validated_confirmations, "block_fully_validated": s.block_fully_validated,
            "core_image": s.core_image, "headers_image": s.headers_image,
        })),
    }).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Review finding 7: `self.i + n` overflowed (a panic in a debug build) on a length near usize::MAX.
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
