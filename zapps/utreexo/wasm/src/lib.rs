//! `zapp-utreexo-wasm`: the Utreexo roots zyncAPP's checks in a browser, by the same `zapp_utreexo::verify_utreexo_on` the
//! CLI uses. Raw ABI, no wasm-bindgen (as swiftsync-verify-wasm): the .wasm is a plain cargo output anyone can rebuild.
//!
//! ```text
//!   alloc(len) -> ptr              reserve len bytes for the caller to write the input frame into
//!   verify_utreexo(ptr, len) -> p  returns a pointer to [u32 LE length][UTF-8 JSON]
//!   image_id() -> p                the built-in utreexo image id, same encoding
//! ```
//! Input frame (little-endian): `[u64 now][u32 n][receipt][u32 n][image id hex, n = 0 for the built-in]`
//! `[u32 n]["H:hash" from the page's own source, n = 0 for none][u32 n]["T:hash" for the core tip, n = 0 for none]`
//! `[u32 n][network: "mainnet" or "signet", n = 0 for mainnet]`. The network is the caller's: a proof of another
//! network is FAIL. (Until 18.11.2026 the frame had no network and the proof's own profile picked it; such a frame is
//! now refused as truncated.)
//! The JSON's `network` is the one checked; `image_source` is "builtin-unpinned" (with `image_label`: the verdict is at
//! most `demo`), "builtin-pinned", or "caller" (an image id the page named: this checker does not know its code).
//! The JSON's `roots` write an Empty root as the string "empty" (rustreexo's Display); the journal holds 32 zero bytes
//! for it. Either way a consumer must build `BitcoinNodeHash::Empty`, never the hash 00…00.
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
        let out = self.b.get(self.i..self.i.checked_add(n).ok_or("input frame truncated")?).ok_or("input frame truncated")?;
        self.i += n;
        Ok(out)
    }
    fn chunk(&mut self) -> Result<&'a [u8], String> {
        let n = u32::from_le_bytes(self.take(4)?.try_into().unwrap()) as usize;
        self.take(n)
    }
    fn text(&mut self, what: &str) -> Result<String, String> {
        std::str::from_utf8(self.chunk()?).map(|s| s.to_string()).map_err(|_| format!("{what}: not UTF-8"))
    }
}

fn expect(s: &str, what: &str) -> Result<Option<(u32, String)>, String> {
    if s.is_empty() { return Ok(None); }
    swiftsync_verify::parse_expect(s).map(Some).ok_or(format!("{what} {s:?}: give height:hash"))
}

#[no_mangle]
pub extern "C" fn verify_utreexo(ptr: *const u8, len: usize) -> *const u8 {
    let input = unsafe { std::slice::from_raw_parts(ptr, len) };
    let parsed = (|| -> Result<(u64, &[u8], String, Option<(u32, String)>, Option<(u32, String)>, String), String> {
        let mut f = Frame { b: input, i: 0 };
        let now = u64::from_le_bytes(f.take(8)?.try_into().unwrap());
        let r = f.chunk()?;
        let image = f.text("image id")?;
        let at = expect(&f.text("expect-at")?, "expect-at")?;
        let tip = expect(&f.text("expect-tip")?, "expect-tip")?;
        let network = f.text("network")?;
        if f.i != input.len() { return Err("input frame has trailing bytes".into()); }
        Ok((now, r, image, at, tip, network))
    })();
    let (now, r, image, at, tip, network) = match parsed { Ok(p) => p, Err(e) => return fail(e) };
    let network = if network.is_empty() { "mainnet".to_string() } else { network };
    let image = if image.is_empty() { zapp_utreexo::UTREEXO_IMAGE_ID.to_string() } else { image };
    let rep = zapp_utreexo::verify_utreexo_on(&network, r, &image, None, now, at.as_ref().map(|(h, x)| (*h, x.as_str())), tip.as_ref().map(|(h, x)| (*h, x.as_str())));
    let ok = rep.verdict != Verdict::Fail;
    let z = rep.statement.as_ref().filter(|_| ok);
    ret(serde_json::json!({
        "verdict": match rep.verdict { Verdict::Ok => "ok", Verdict::Demo => "demo", Verdict::Fail => "fail" },
        "image": image.to_lowercase(),
        "image_source": rep.image_source,
        "image_label": (rep.image_source == Some(zapp_utreexo::IMAGE_BUILTIN_UNPINNED)).then_some(zapp_utreexo::UTREEXO_IMAGE_LABEL),
        "checks": rep.checks.iter().map(|(ok, what)| serde_json::json!({"ok": ok, "what": what})).collect::<Vec<_>>(),
        "height": z.map(|z| z.height),
        "block_hash": z.map(|z| z.block_hash_hex()),
        "leaves": z.map(|z| z.leaves),
        "roots": z.map(|z| z.roots_hex()),
        "network": rep.network,
        "runs": z.map(|z| z.runs),
        "core_height": z.map(|z| z.core_height),
        "identity_checked": ok && rep.identity_checked,
    }).to_string())
}

#[no_mangle]
pub extern "C" fn image_id() -> *const u8 {
    ret(zapp_utreexo::UTREEXO_IMAGE_ID.to_string())
}
