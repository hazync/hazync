//! `zapp-liveness-wasm`: the coin-liveness zyncAPP (docs/libzync/LIVENESS_ZAPP.md) in a browser, a WASM module of its
//! own (docs/libzync/ZYNCAPPS.md §9), NOT part of the core verifier's hazync-verify.wasm (until 15.11.2026 it was the
//! `"liveness"` key of the core's verify_facts). Same raw ABI as the core's (no wasm-bindgen):
//!
//! ```text
//!   alloc(len) -> ptr              reserve len bytes for the caller to write the input frame into
//!   verify_liveness(ptr, len) -> p p points to [u32 LE length][UTF-8 JSON]
//! ```
//!
//! Input frame (little-endian): `[u8 network: 0 mainnet, 1 signet; | 0x10 coin-log image, | 0x20 product image]`
//! `[u64 now][u32 n][engine proof file or .sspf bundle][u32 n][anchor JSON, n = 0 for genesis][u32 n][image id hex]
//! [u32 n][JSON {"liveness": [{"proof": hex, "ask": question}]}][u32 n][expect "<height>:<hash>", n = 0 for
//! none][u32 n][the core image's verifying key file][u32 32][the key's pin: SHA-256 of that file]`. Each question must claim its answer (=unspent / =spent...): this returns whether the claims hold. `expect` is
//! the network's block at the proof's height from a source the PAGE trusts, never the fact server (an "expect" key
//! inside the JSON is refused). Glue: zapp-liveness.js.

use swiftsync_verify::{ChainState, Verdict};

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

#[no_mangle]
pub extern "C" fn verify_liveness(ptr: *mut u8, len: usize) -> *const u8 {
    let input = unsafe { Vec::from_raw_parts(ptr, len, len) };
    let parsed = (|| -> Result<_, String> {
        let mut f = Frame { b: &input, i: 0 };
        let code = f.take(1)?[0];
        let now = u64::from_le_bytes(f.take(8)?.try_into().unwrap());
        let (receipt, anchor_json, image_hex, facts, expect) = (f.chunk()?, f.chunk()?, f.chunk()?, f.chunk()?, f.chunk()?);
        let vk = f.chunk()?;
        let pin: [u8; 32] = f.chunk()?.try_into().map_err(|_| "the key's pin is 32 bytes".to_string())?;
        if f.i != input.len() {
            return Err("input frame has trailing bytes".to_string());
        }
        let network = match code & !0x30 {
            0 => "mainnet",
            1 => "signet",
            n => return Err(format!("unknown network code {n}")),
        };
        let image_hex = std::str::from_utf8(image_hex).map_err(|_| "image id is not text")?;
        if image_hex.trim().is_empty() || vk.is_empty() {
            return Err("no engine core image is published yet: give the image id, its verifying key and pin".to_string());
        }
        let image = zapp_corecheck::with_key(zapp_liveness::image_for(network, image_hex, code & 0x10 != 0, code & 0x20 != 0)?, vk, Some(&pin))?;
        let anchor: Option<ChainState> = if anchor_json.is_empty() { None } else {
            Some(serde_json::from_slice(anchor_json).map_err(|e| format!("anchor is not a ChainState: {e}"))?)
        };
        // {"liveness": [...]} and no other key; an "expect" inside it refused (zapp_liveness::facts_from_json)
        let lives = zapp_liveness::facts_from_json(facts)?;
        let expect = std::str::from_utf8(expect).map_err(|_| "expect: not UTF-8".to_string())?.trim().to_string();
        Ok((receipt, image, anchor, now, lives, expect))
    })();
    let (receipt, image, anchor, now, lives, expect) = match parsed { Ok(p) => p, Err(e) => return fail(e) };
    let o = zapp_liveness::verify_liveness(receipt, image, anchor, now, &lives, Some(expect.as_str()).filter(|e| !e.is_empty()), true);
    let rep = &o.report;
    // A fact is "ok" only under an OK verdict: under DEMO it holds on a chain not shown to be the network's.
    let demo = rep.verdict != Verdict::Ok;
    let facts: Vec<serde_json::Value> = o.facts.iter().map(|r| match r {
        Ok(w) if demo => serde_json::json!({"ok": false, "what": format!("holds on this chain, but the chain is not shown to be the network's: {w}")}),
        Ok(w) => serde_json::json!({"ok": true, "what": w}),
        Err(w) => serde_json::json!({"ok": false, "what": w}),
    }).collect();
    ret(serde_json::json!({
        "verdict": match rep.verdict { Verdict::Ok => "ok", Verdict::Demo => "demo", Verdict::Fail => "fail" },
        // the image the receipt verified against: the one given, or (none given) the earlier built-in image it names
        "image": o.image.method_id_hex,
        "network": image.network,
        "checks": rep.checks.iter().map(|(ok, what)| serde_json::json!({"ok": ok, "what": what})).collect::<Vec<_>>(),
        "height": rep.state.as_ref().filter(|_| rep.verdict != Verdict::Fail).map(|c| c.height),
        "tip_checked": o.tip_checked,
        "facts": facts,
    }).to_string())
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
