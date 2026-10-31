//! `swiftsync-verify-wasm`: the SwiftSync hints-proof verifier in a browser. Local verification, not a
//! service: the proof and the file are checked on the device, by the same `swiftsync_verify::verify` the CLI
//! uses, with no network. Raw ABI and no wasm-bindgen, for the reason ../verifier-wasm gives: the .wasm is a
//! plain cargo output anyone can rebuild and compare.
//!
//! ```text
//!   alloc(len) -> ptr        reserve len bytes for the caller to write the input frame into
//!   verify(ptr, len) -> p    returns a pointer to [u32 LE length][UTF-8 JSON]
//! ```
//!
//! Input frame (little-endian): `[u8 network: 0 mainnet, 1 signet][u64 now, unix seconds]`
//! `[u32 n][receipt or .sspf bundle][u32 n][hints file][u32 n][anchor JSON, n = 0 for genesis]`
//! `[u32 n][image id hex, n = 0 for the built-in id]`. The page supplies `now` because WebAssembly has no clock.
//!
//! The JSON always has `verdict` ("ok", "demo" or "fail", the CLI's 0 / 2 / 1) and `checks`. There is no step 7
//! here (a browser has no node); the page shows the tip and work for the reader to compare.

use swiftsync_verify::{check_bundle, hex, verify, Bundle, ChainState, Image, Inputs, Report, Verdict, BUNDLE_MAGIC};

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
        let out = self.b.get(self.i..self.i + n).ok_or("input frame truncated")?;
        self.i += n;
        Ok(out)
    }
    fn chunk(&mut self) -> Result<&'a [u8], String> {
        let n = u32::from_le_bytes(self.take(4)?.try_into().unwrap()) as usize;
        self.take(n)
    }
}

fn run(input: &[u8]) -> Result<(Report, &'static Image), String> {
    let mut f = Frame { b: input, i: 0 };
    let network = match f.take(1)?[0] {
        0 => "mainnet",
        1 => "signet",
        n => return Err(format!("unknown network code {n}")),
    };
    let now = u64::from_le_bytes(f.take(8)?.try_into().unwrap());
    let receipt = f.chunk()?;
    let hints = f.chunk()?;
    let anchor_json = f.chunk()?;
    let image_hex = f.chunk()?;
    if f.i != input.len() {
        return Err("input frame has trailing bytes".into());
    }
    let mut image = Image::by_network(network).ok_or("no built-in image")?;
    if !image_hex.is_empty() {
        let h = std::str::from_utf8(image_hex).map_err(|_| "image id is not text")?.trim().to_lowercase();
        image = image.with_method_id(&h).ok_or("image id is not 64 hex digits")?;
    }
    let anchor: Option<ChainState> = if anchor_json.is_empty() {
        None
    } else {
        Some(serde_json::from_slice(anchor_json).map_err(|e| format!("anchor is not a ChainState: {e}"))?)
    };
    let rep = if receipt.starts_with(BUNDLE_MAGIC) {
        check_bundle(&Bundle::decode(receipt)?, hints, image, anchor, now)
    } else {
        verify(&Inputs { receipt, hints, image, anchor, now })
    };
    Ok((rep, image))
}

#[no_mangle]
pub extern "C" fn verify_frame(ptr: *mut u8, len: usize) -> *const u8 {
    let input = unsafe { Vec::from_raw_parts(ptr, len, len) };
    match run(&input) {
        Err(e) => fail(e),
        Ok((rep, image)) => {
            let rev = |h: &[u8; 32]| {
                let mut r = *h;
                r.reverse();
                hex(&r)
            };
            let st = rep.state.as_ref();
            ret(serde_json::json!({
                "verdict": match rep.verdict { Verdict::Ok => "ok", Verdict::Demo => "demo", Verdict::Fail => "fail" },
                "image": image.method_id_hex,
                "network": image.network,
                "checks": rep.checks.iter().map(|(ok, what)| serde_json::json!({"ok": ok, "what": what})).collect::<Vec<_>>(),
                "height": st.map(|c| c.height),
                "tip": st.map(|c| rev(&c.tip_hash)),
                "chainwork": st.map(|c| rev(&c.cum_work)),
                "utxo_leaves": st.map(|c| c.utxo_leaves),
                "profile": st.map(|c| swiftsync_verify::describe_profile(c.profile)),
            }).to_string())
        }
    }
}

/// The built-in image id for a network code (0 mainnet, 1 signet), for the page to display.
#[no_mangle]
pub extern "C" fn image_id(network: u32) -> *const u8 {
    ret(match network {
        0 => Image::by_network("mainnet"),
        1 => Image::by_network("signet"),
        _ => None,
    }
    .map(|i| i.method_id_hex.to_string())
    .unwrap_or_default())
}
