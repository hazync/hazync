//! `zapp-headers-wasm`: the header-chain zyncAPP (ROADMAP K1, N3) in a browser, a WASM module of its own
//! (docs/libzync/ZYNCAPPS.md §9), NOT part of the core verifier's hazync-verify.wasm (until 15.11.2026 it was the
//! core's `verify_headers`). Same raw ABI as the core's (no wasm-bindgen): `alloc(len) -> ptr`,
//! `verify_headers(ptr, len) -> p` with p pointing to [u32 LE length][UTF-8 JSON], `headers_image_id() -> p`.
//! Glue: zapp-headers.js.

use swiftsync_verify::Verdict;

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

/// The header-chain zyncAPP (ROADMAP K1, N3), by the same `zapp_headers::verify_headers` the CLI
/// `zapp-headers` uses. Input frame (little-endian): `[u64 now][u32 count]` then count times
/// `[u32 n][receipt]` (the first from genesis, each next one starting where the one before ended), then
/// `[u32 n][image id hex, n = 0 for the built-in id][u32 n]["height:hash" from a source the page trusts, n = 0 for none]`,
/// optionally followed by `[u32 n][include JSON, n = 0 for none]`: `{"height", "hash" (as explorers print it), "path"
/// (MMR hashes as zyncapp headers-mmr-path prints them), "txid"? and "tx"?, both or neither: the transaction asked
/// about (as explorers print it; the CALLER's, never the service's) and the path service's /tx answer (pos, header,
/// raw, branch, coinbase, coinbase_branch; zapp_headers::TxInclusion::from_json, which does not read the
/// answer's own txid: the raw transaction must hash to the asked one)}`.
///
/// Output beside verdict, checks, height, hash, chainwork, max_time, mmr_root, identity_checked:
///   `included`: null when nothing was asked; true ONLY when the verdict is "ok" and the block (and transaction) is on
///   the chain. "ok" needs Core's minimum work AND `expect` matched (identity_checked); a DEMO chain (below the work, or
///   its tip not compared with a source the caller trusts) never gives true.
///   `included_demo`: null when nothing was asked; true when the verdict is "demo" and the inclusion checks passed.
///   `inclusion`: `{height, hash, txid}` of what was shown (txid null for a block alone), non-null only when no check
///   failed (verdict "ok" or "demo": read it with the verdict, as `included` / `included_demo` do).
#[no_mangle]
pub extern "C" fn verify_headers(ptr: *const u8, len: usize) -> *const u8 {
    let input = unsafe { std::slice::from_raw_parts(ptr, len) };
    let parsed = (|| -> Result<(u64, Vec<&[u8]>, String, String, Option<zapp_headers::Inclusion>), String> {
        let mut f = Frame { b: input, i: 0 };
        let now = u64::from_le_bytes(f.take(8)?.try_into().unwrap());
        let n = u32::from_le_bytes(f.take(4)?.try_into().unwrap()) as usize;
        if n == 0 || n > 4096 { return Err(format!("{n} receipts: give 1 to 4096")); }
        let mut rs = Vec::with_capacity(n);
        for _ in 0..n { rs.push(f.chunk()?); }
        let image = std::str::from_utf8(f.chunk()?).map_err(|_| "image id: not UTF-8")?.to_string();
        let expect = std::str::from_utf8(f.chunk()?).map_err(|_| "expect: not UTF-8")?.to_string();
        let include = if f.i == input.len() { None } else {
            let j = f.chunk()?;
            if j.is_empty() { None } else { Some(parse_include(j)?) }
        };
        if f.i != input.len() { return Err("input frame has trailing bytes".into()); }
        Ok((now, rs, image, expect, include))
    })();
    let (now, rs, image, expect, include) = match parsed { Ok(p) => p, Err(e) => return fail(e) };
    let image = if image.is_empty() { zapp_headers::HEADERS_IMAGE_ID.to_string() } else { image };
    let exp = if expect.is_empty() { None } else {
        match expect.split_once(':').and_then(|(h, x)| h.parse::<i64>().ok().map(|h| (h, x))) {
            Some(e) => Some(e),
            None => return fail(format!("expect {expect:?}: give height:hash")),
        }
    };
    let rep = zapp_headers::verify_headers(&rs, &image, now, exp, include.as_ref());
    // "ok" needs the tip compared with `expect` (a source the caller trusts), as every zAPP (review 11-16): without it a
    // chain with enough work is "demo", and `included` is never true
    let verdict = rep.gated();
    let ok = verdict != Verdict::Fail;
    let s = rep.state.as_ref().filter(|_| ok);
    ret(serde_json::json!({
        "verdict": match verdict { Verdict::Ok => "ok", Verdict::Demo => "demo", Verdict::Fail => "fail" },
        "image": image,
        "checks": rep.checks.iter().map(|(ok, what)| serde_json::json!({"ok": ok, "what": what})).collect::<Vec<_>>(),
        "height": s.map(|s| s.height),
        "hash": s.map(|s| s.hash_hex()),
        "chainwork": s.map(|s| s.work_hex()),
        "max_time": s.map(|s| s.max_time),
        "mmr_root": s.map(|s| s.mmr_root_hex()),
        "identity_checked": rep.tip_checked,
        // the block (and transaction) asked for is on a chain with Core's minimum work; null when none was asked
        "included": include.as_ref().map(|_| verdict == Verdict::Ok && rep.included.is_some()),
        // ...on a valid chain BELOW it (a demo proof): shown, but not a claim about the network's chain
        "included_demo": include.as_ref().map(|_| verdict == Verdict::Demo && rep.included.is_some()),
        "inclusion": rep.included.as_ref().filter(|_| ok).map(|i| serde_json::json!({
            "height": i.height, "hash": i.hash_hex(), "txid": i.txid_hex(),
        })),
    }).to_string())
}

fn parse_include(j: &[u8]) -> Result<zapp_headers::Inclusion, String> {
    let v: serde_json::Value = serde_json::from_slice(j).map_err(|e| format!("include: not JSON ({e})"))?;
    let strs = |x: &serde_json::Value, what: &str| -> Result<Vec<String>, String> {
        x.as_array().ok_or(format!("include: {what} is not a list"))?.iter()
            .map(|y| y.as_str().map(str::to_string).ok_or(format!("include: {what} holds a non-string"))).collect()
    };
    let height = v["height"].as_i64().ok_or("include: height is not an integer")?;
    let hash = v["hash"].as_str().ok_or("include: hash is not a string")?;
    let path = strs(&v["path"], "path")?;
    let path: Vec<&str> = path.iter().map(|s| s.as_str()).collect();
    let tx = match (&v["txid"], &v["tx"]) {
        (serde_json::Value::Null, serde_json::Value::Null) => None,
        (serde_json::Value::String(txid), tx) if tx.is_object() => Some(zapp_headers::TxInclusion::from_json(txid, tx)?),
        (serde_json::Value::Null, _) => return Err("include: tx needs txid, the transaction asked about (never taken from the answer)".into()),
        (_, serde_json::Value::Null) => return Err("include: txid needs tx, the path service's /tx answer".into()),
        _ => return Err("include: txid is a string and tx an object".into()),
    };
    zapp_headers::Inclusion::parse(height, hash, &path, tx)
}

/// The built-in header-chain image id, for the page to display.
#[no_mangle]
pub extern "C" fn headers_image_id() -> *const u8 {
    ret(zapp_headers::HEADERS_IMAGE_ID.to_string())
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
    #[test]
    fn include_txid_and_tx_go_together() {
        let base = r#"{"height": 1, "hash": "00000000839a8e6886ab5951d76f411475428afc90947ee320161bbf18eb6048", "path": []"#;
        assert!(parse_include(format!("{base}}}").as_bytes()).is_ok());
        let e = parse_include(format!(r#"{base}, "tx": {{}}}}"#).as_bytes()).unwrap_err();
        assert!(e.contains("tx needs txid"), "{e}");
        let e = parse_include(format!(r#"{base}, "txid": "00"}}"#).as_bytes()).unwrap_err();
        assert!(e.contains("txid needs tx"), "{e}");
    }
}
