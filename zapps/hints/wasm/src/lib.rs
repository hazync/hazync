//! `zapp-hints-wasm`: the SwiftSync hints zyncAPP (docs/libzync/HINTS_ZAPP.md) in a browser, a WASM module of its own
//! (docs/libzync/ZYNCAPPS.md §9), NOT part of the core verifier's hazync-verify.wasm. Same raw ABI as the core's
//! (no wasm-bindgen): `alloc(len) -> ptr`, `verify_hints(ptr, len) -> p` with p pointing to [u32 LE length][UTF-8 JSON],
//! `hints_image_id() -> p`. Glue: zapp-hints.js.

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

/// The SwiftSync hints zyncAPP (docs/libzync/HINTS_ZAPP.md), by the same `zapp_hints::verify_hints` the CLI
/// `zapp-hints` uses. Input frame (little-endian): `[u64 now]` then eleven chunks `[u32 n][bytes]`: the engine proof
/// file, the hints file, the undo-hash file (n = 0: not checked), the hints image id hex, the core image id hex, "H:hash"
/// from the node's header chain (n = 0: none), "T:hash" for the core tip when it is above H (n = 0: none), the hints
/// image's verifying key file, its 32-byte pin (SHA-256 of that file, a constant the page is published with), the
/// node's network ("mainnet" or "signet"; n = 0: mainnet) and the format it reads ("bip" or "crate"; n = 0: not
/// compared, so at most DEMO; review R3-4 L1, L2). No
/// engine image is published yet, so both ids and the key are required. The file is hashed here: a page streams
/// nothing to a server.
#[no_mangle]
pub extern "C" fn verify_hints(ptr: *const u8, len: usize) -> *const u8 {
    let input = unsafe { std::slice::from_raw_parts(ptr, len) };
    type In<'a> = (u64, [&'a [u8]; 11]);
    let parsed = (|| -> Result<In, String> {
        let mut f = Frame { b: input, i: 0 };
        let now = u64::from_le_bytes(f.take(8)?.try_into().unwrap());
        let c = [f.chunk()?, f.chunk()?, f.chunk()?, f.chunk()?, f.chunk()?, f.chunk()?, f.chunk()?, f.chunk()?, f.chunk()?,
                 f.chunk()?, f.chunk()?];
        if f.i != input.len() { return Err("input frame has trailing bytes".into()); }
        Ok((now, c))
    })();
    let (now, c) = match parsed { Ok(p) => p, Err(e) => return fail(e) };
    let s = |b: &[u8]| -> Result<String, String> { std::str::from_utf8(b).map(|x| x.to_string()).map_err(|_| "not UTF-8".to_string()) };
    let (image, core, exp, tip, net, fmt) = match (s(c[3]), s(c[4]), s(c[5]), s(c[6]), s(c[9]), s(c[10])) {
        (Ok(a), Ok(b), Ok(d), Ok(e), Ok(n), Ok(m)) => (a, b, d, e, n, m),
        _ => return fail("a text field is not UTF-8".into()),
    };
    let network = if net.is_empty() { "mainnet".to_string() } else { net };
    if network != "mainnet" && network != "signet" { return fail(format!("{network:?}: the network is mainnet or signet")); }
    let format = if fmt.is_empty() { None } else {
        match zapp_hints::parse_format(&fmt) { Some(f) => Some(f), None => return fail(format!("{fmt:?}: the format is bip or crate")) }
    };
    if image.is_empty() || core.is_empty() || c[7].is_empty() {
        return fail("no engine hints image is published yet: give the hints image id, the core image id and the key".into());
    }
    let Ok(pin) = <[u8; 32]>::try_from(c[8]) else { return fail("the key's pin is 32 bytes".into()) };
    let pe = |e: &str| -> Result<Option<(u32, String)>, String> {
        if e.is_empty() { return Ok(None); }
        e.split_once(':').and_then(|(h, x)| h.parse::<u32>().ok().map(|h| Some((h, x.to_string())))).ok_or(format!("{e:?}: give height:hash"))
    };
    let (e1, e2) = match (pe(&exp), pe(&tip)) { (Ok(a), Ok(b)) => (a, b), (Err(e), _) | (_, Err(e)) => return fail(e) };
    let rep = zapp_hints::verify_hints(c[0], c[1], (!c[2].is_empty()).then_some(c[2]), &image,
                                                    &swiftsync_verify::EngineKey { bytes: c[7], pin }, &core,
                                                    &zapp_hints::HintsAsk { network: &network, format }, now,
                                                    e1.as_ref().map(|(h, x)| (*h, x.as_str())), e2.as_ref().map(|(h, x)| (*h, x.as_str())));
    let z = rep.statement.as_ref().filter(|_| rep.verdict != Verdict::Fail);
    let d = |b: &[u8; 32]| -> String { b.iter().rev().map(|x| format!("{x:02x}")).collect() };
    ret(serde_json::json!({
        "verdict": match rep.verdict { Verdict::Ok => "ok", Verdict::Demo => "demo", Verdict::Fail => "fail" },
        "image": image,
        "network": network,
        "undo_checked": !c[2].is_empty(),
        "checks": rep.checks.iter().map(|(ok, what)| serde_json::json!({"ok": ok, "what": what})).collect::<Vec<_>>(),
        "height": z.map(|z| z.height),
        "hash": z.map(|z| d(&z.block_hash)),
        "format": z.map(|z| z.format),
        "unspent": z.map(|z| z.n_unspent),
        "value": z.map(|z| z.unspent_value),
        "identity_checked": rep.identity_checked,
    }).to_string())
}

/// The published hints image id, for the page to display: empty until the Stage 4 freeze publishes one.
#[no_mangle]
pub extern "C" fn hints_image_id() -> *const u8 {
    ret(String::new())
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
