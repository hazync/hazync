//! `zapp-zeroconf-wasm`: the zero-confirmation zyncAPP (zapps/zeroconf/README.md) in a browser, a WASM module of its
//! own (docs/libzync/ZYNCAPPS.md §9), NOT part of the core verifier's hazync-verify.wasm. Same raw ABI as the core's
//! (no wasm-bindgen):
//!
//! ```text
//!   alloc(len) -> ptr              reserve len bytes for the caller to write the input frame into
//!   verify_zeroconf(ptr, len) -> p p points to [u32 LE length][UTF-8 JSON]
//!   zeroconf_image_id() -> p       the published zeroconf image id ("" until the Stage 4 freeze publishes one)
//! ```
//!
//! Input frame (little-endian): `[u8 network: 0 mainnet, 1 signet][u64 now]` then nine chunks `[u32 n][bytes]`: the
//! engine proof file (HZE1), the anchor ChainState JSON (n = 0: genesis), the zeroconf image id hex, the core image id
//! hex (an engine build: node format 2), the txid as explorers print it, the wtxid of the caller's bytes (n = 0: not
//! checked), "<tip height>:<hash>" from the page's own header chain (n = 0: none, then at most DEMO), the zeroconf
//! image's verifying key file, and its 32-byte pin (SHA-256 of that file, a constant the page is published with).
//! The checks are zapp_zeroconf::verify_zeroconf_asked's, the CLI zapp-zeroconf's: one implementation. No engine image is
//! published yet, so both ids and the key are required. Glue: zapp-zeroconf.js.

use swiftsync_verify::{ChainState, EngineKey, Verdict};

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

fn text(b: &[u8], what: &str) -> Result<String, String> {
    std::str::from_utf8(b).map(|s| s.trim().to_string()).map_err(|_| format!("{what}: not UTF-8"))
}

/// The zero-confirmation zyncAPP, by the same `zapp_zeroconf::verify_zeroconf_asked` the CLI uses (frame: the module
/// doc above).
#[no_mangle]
pub extern "C" fn verify_zeroconf(ptr: *const u8, len: usize) -> *const u8 {
    let input = unsafe { std::slice::from_raw_parts(ptr, len) };
    type In<'a> = (&'static str, u64, [&'a [u8]; 9]);
    let parsed = (|| -> Result<In, String> {
        let mut f = Frame { b: input, i: 0 };
        let code = f.take(1)?[0];
        let now = u64::from_le_bytes(f.take(8)?.try_into().unwrap());
        let c = [f.chunk()?, f.chunk()?, f.chunk()?, f.chunk()?, f.chunk()?, f.chunk()?, f.chunk()?, f.chunk()?, f.chunk()?];
        if f.i != input.len() {
            return Err("input frame has trailing bytes".into());
        }
        let network = match code {
            0 => "mainnet",
            1 => "signet",
            n => return Err(format!("unknown network code {n}")),
        };
        Ok((network, now, c))
    })();
    let (network, now, c) = match parsed { Ok(p) => p, Err(e) => return fail(e) };
    let fields = (|| -> Result<_, String> {
        let (image, core, txid, wtxid, expect) =
            (text(c[2], "image id")?, text(c[3], "core image id")?, text(c[4], "txid")?, text(c[5], "wtxid")?, text(c[6], "expect")?);
        if image.is_empty() || core.is_empty() || c[7].is_empty() {
            return Err("no engine zeroconf image is published yet: give the zeroconf image id, the core image id and the key".into());
        }
        if txid.is_empty() {
            return Err("give the txid you are asking about".into());
        }
        let pin = <[u8; 32]>::try_from(c[8]).map_err(|_| "the key's pin is 32 bytes".to_string())?;
        let anchor: Option<ChainState> = if c[1].is_empty() { None } else {
            Some(serde_json::from_slice(c[1]).map_err(|e| format!("anchor is not a ChainState: {e}"))?)
        };
        let tip = if expect.is_empty() { None } else {
            Some(swiftsync_verify::parse_expect(&expect).ok_or("expect: want <height>:<64 hex digits>")?)
        };
        // the trusted core image: the network's record re-pointed at the core image id (an engine build), as the CLI
        let img = zapp_corecheck::image_for(network, &core, true, true)?;
        Ok((image, txid, wtxid, anchor, tip, pin, img))
    })();
    let (image, txid, wtxid, anchor, tip, pin, img) = match fields { Ok(x) => x, Err(e) => return fail(e) };
    let key = EngineKey { bytes: c[7], pin };
    let rep = zapp_zeroconf::verify_zeroconf_asked(c[0], &image, &key, img, anchor.as_ref(), now,
                                                   zapp_zeroconf::Asked { txid: &txid, wtxid: (!wtxid.is_empty()).then_some(wtxid.as_str()) },
                                                   tip.as_ref().map(|(h, x)| (*h, x.as_str())));
    let z = rep.statement.as_ref().filter(|_| rep.verdict != Verdict::Fail);
    ret(serde_json::json!({
        "verdict": match (rep.verdict, z) { (Verdict::Ok, Some(_)) => "ok", (Verdict::Demo, Some(_)) => "demo", _ => "fail" },
        "network": network,
        "image": image,
        "checks": rep.checks.iter().map(|(ok, what)| serde_json::json!({"ok": ok, "what": what})).collect::<Vec<_>>(),
        "txid": z.map(|z| z.txid_hex()),
        "wtxid": z.map(|z| z.wtxid_hex()),
        "next_block": z.map(|z| z.tip_height + 1),
        "tip": z.map(|z| z.tip_hex()),
        "fee": z.map(|z| z.fee),
        "weight": z.map(|z| z.weight),
        "sigops": z.map(|z| z.sigops),
        "tip_checked": rep.tip_checked,
    }).to_string())
}

/// The published zeroconf image id, for the page to display: empty until the Stage 4 freeze publishes one.
#[no_mangle]
pub extern "C" fn zeroconf_image_id() -> *const u8 {
    ret(String::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    /// As every zAPP module's frame (review finding 7, feat/headers-mmr): a length near usize::MAX must not overflow.
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
