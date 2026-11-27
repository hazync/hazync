//! `zapp-assume-wasm`: the assumevalid and assumeutxo zyncAPPs (docs/libzync/ASSUME_ZAPPS.md) in a browser, a WASM
//! module of its own (docs/libzync/ZYNCAPPS.md §9), NOT part of the core verifier's hazync-verify.wasm (until 15.11.2026
//! it was the core's `verify_assume`). Same raw ABI as the core's (no wasm-bindgen): `alloc(len) -> ptr`,
//! `verify_assume(ptr, len) -> p` with p pointing to [u32 LE length][UTF-8 JSON]. Glue: zapp-assume.js. (No lineage text:
//! an image the built-in table does not list, with its own handovers, is checked by the core verifier.)

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

/// `verify_assume(ptr, len)`: the assumevalid and assumeutxo checks (docs/libzync/ASSUME_ZAPPS.md). Input frame
/// (little-endian): `[u8 network: 0 mainnet, 1 signet; | 0x10 coin-log image, | 0x20 product image][u64 now]`, then
/// `[u32 n][receipt]`, `[u32 n][image id hex]`, `[u32 n][its verifying key file]`, `[u32 32][the key's pin: SHA-256 of that file]`, `[u32 n][expect "<height>:<hash>": OK needs it]`,
/// `[u32 n][assumevalid: empty = no check, "tip", or a block hash in display order]`, `[u32 n][headers: raw 80-byte
/// headers ending at the proven tip]`, `[u32 n][a Core dumptxoutset file, n = 0 for no assumeutxo check]`, `[u32 n][tracked
/// list text]`. From genesis only (no anchor); a receipt, not a bundle. Returns {verdict, image, network, checks,
/// height, assume: {checks, assumevalid: {block, height, proven_to}, assumeutxo: {coins, value, hash_serialized_3}}};
/// any failed check makes the verdict "fail", and without a matching expect it is at most "demo".
#[no_mangle]
pub extern "C" fn verify_assume(ptr: *mut u8, len: usize) -> *const u8 {
    let input = unsafe { Vec::from_raw_parts(ptr, len, len) };
    let res = (|| -> Result<_, String> {
        let mut f = Frame { b: &input, i: 0 };
        let code = f.take(1)?[0];
        let now = u64::from_le_bytes(f.take(8)?.try_into().unwrap());
        let text = |b: &[u8], what: &str| std::str::from_utf8(b).map(|s| s.trim().to_string()).map_err(|_| format!("{what}: not UTF-8"));
        let receipt = f.chunk()?;
        let image_hex = text(f.chunk()?, "image id")?;
        let vk = f.chunk()?;
        let pin: [u8; 32] = f.chunk()?.try_into().map_err(|_| "the key's pin is 32 bytes".to_string())?;
        let expect = text(f.chunk()?, "expect")?;
        let av = text(f.chunk()?, "assumevalid")?;
        let headers = f.chunk()?;
        let snapshot = f.chunk()?;
        let tracked = text(f.chunk()?, "tracked")?;
        if f.i != input.len() {
            return Err("input frame has trailing bytes".to_string());
        }
        let network = match code & !0x30 { 0 => "mainnet", 1 => "signet", n => return Err(format!("unknown network code {n}")) };
        if image_hex.is_empty() || vk.is_empty() {
            return Err("no engine core image is published yet: give the image id, its verifying key and pin".to_string());
        }
        let image = zapp_corecheck::with_key(zapp_corecheck::image_for(network, &image_hex, code & 0x10 != 0, code & 0x20 != 0)?, vk, &pin)?;
        if receipt.starts_with(swiftsync_verify::BUNDLE_MAGIC) {
            return Err("assume: a proof bundle is not accepted here: give the receipt".to_string());
        }
        let av = match av.as_str() {
            "" => None,
            "tip" => Some(None),
            x => Some(Some(zapp_assume::parse_display_hash(x).ok_or(format!("assumevalid {x:?}: not 64 hex digits"))?)),
        };
        if av.is_none() && snapshot.is_empty() {
            return Err("neither an assumevalid block nor a snapshot was given".to_string());
        }
        Ok((receipt, image, now, expect, av, headers, snapshot, tracked))
    })();
    let (receipt, image, now, expect, av, headers, snapshot, tracked) = match res {
        Err(e) => return fail(e),
        Ok(x) => x,
    };
    let core = zapp_corecheck::verify_core(receipt, image, None, now, Some(expect.as_str()).filter(|e| !e.is_empty()));
    let (mut rep, tip_checked, used) = (core.report, core.tip_checked, core.image);
    if rep.checks.iter().any(|(ok, _)| !ok) { rep.verdict = Verdict::Fail; }
    let mut assume = serde_json::json!(null);
    if rep.verdict == Verdict::Fail {
        rep.checks.push((false, "assumevalid / assumeutxo NOT checked: the receipt did not verify".into()));
    } else if let Some(cs) = rep.state.as_ref() {
        let o = zapp_assume::run_assume(cs, zapp_assume::AssumeRequest {
            assumevalid: av,
            headers: Some(headers).filter(|h| !h.is_empty()),
            snapshot: Some(snapshot).filter(|s| !s.is_empty()),
            tracked: Some(tracked.as_str()).filter(|t| !t.is_empty()),
        });
        if o.checks.iter().any(|(ok, _)| !ok) { rep.verdict = Verdict::Fail; }
        if rep.verdict == Verdict::Ok && !tip_checked { rep.verdict = Verdict::Demo; }
        assume = serde_json::json!({
            "checks": o.checks.iter().map(|(ok, what)| serde_json::json!({"ok": ok, "what": what})).collect::<Vec<_>>(),
            "assumevalid": o.valid.as_ref().map(|v| serde_json::json!({"block": zapp_assume::display(&v.block), "height": v.height, "proven_to": v.proven_to})),
            "assumeutxo": o.utxo.as_ref().map(|u| serde_json::json!({"coins": u.coins, "value": u.value, "hash_serialized_3": u.hash_serialized_3})),
        });
    }
    ret(serde_json::json!({
        "verdict": match rep.verdict { Verdict::Ok => "ok", Verdict::Demo => "demo", Verdict::Fail => "fail" },
        // the image the receipt verified against: the one given, or (none given) the earlier built-in image it names
        "image": used.method_id_hex,
        "network": image.network,
        "checks": rep.checks.iter().map(|(ok, what)| serde_json::json!({"ok": ok, "what": what})).collect::<Vec<_>>(),
        "height": rep.state.as_ref().filter(|_| rep.verdict != Verdict::Fail).map(|c| c.height),
        "assume": assume,
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

/// No randomness in verification: OpenVM's graph (engine/verify-core, through swiftsync-verify) names getrandom; built
/// with --cfg getrandom_backend="custom" (engine/scripts/wasm_ovm.sh), every call is refused.
mod no_rng {
    fn refuse(_: &mut [u8]) -> Result<(), getrandom02::Error> {
        Err(getrandom02::Error::UNSUPPORTED)
    }
    getrandom02::register_custom_getrandom!(refuse);
    #[no_mangle]
    unsafe extern "Rust" fn __getrandom_v03_custom(_dest: *mut u8, _len: usize) -> Result<(), getrandom03::Error> {
        Err(getrandom03::Error::UNSUPPORTED)
    }
}
