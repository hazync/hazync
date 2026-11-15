//! `zapp-statement-wasm`: the balance statement zyncAPP's checker in a browser (docs/libzync/STATEMENT_ZAPP.md). The core
//! receipt is verified by `swiftsync_verify::verify` (the core verifier as a library), the statement by
//! `balance_statement::check_statement`. Local: nothing leaves the device.
//!
//! ```text
//!   alloc(len) -> ptr                    reserve len bytes for the input frame
//!   verify_statement(ptr, len) -> p      [u32 LE length][UTF-8 JSON]
//! ```
//!
//! Input frame (little-endian): `[u8 network: 0 mainnet, 1 signet][u64 now, unix seconds][u32 n][receipt]`
//! `[u32 n][image id hex, n = 0 for the network's built-in image][u32 n][statement JSON]`
//! `[u32 n][expect "<height>:<hash>"][u32 n][purpose][u32 n][nonce]`. expect, purpose and nonce come from the PAGE (the
//! block it trusts, the challenge it gave the holder), never from the statement's sender; each may be empty, and then
//! the verdict is at most "demo". The JSON has `verdict` ("ok", "demo", "fail"), `checks`, `statement` {ok, what, total,
//! scripts} and `demo_because`.

use balance_statement::{check_statement, Challenge, Statement};
use swiftsync_verify::{identity_matches, parse_expect, verify, Image, Inputs, Verdict};

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
        let out = self.b.get(self.i..self.i + n).ok_or("input frame truncated")?;
        self.i += n;
        Ok(out)
    }
    fn chunk(&mut self) -> Result<&'a [u8], String> {
        let n = u32::from_le_bytes(self.take(4)?.try_into().unwrap()) as usize;
        self.take(n)
    }
    fn text(&mut self) -> Result<String, String> {
        Ok(std::str::from_utf8(self.chunk()?).map_err(|_| "not UTF-8".to_string())?.trim().to_string())
    }
}

fn run(input: &[u8]) -> Result<serde_json::Value, String> {
    let mut f = Frame { b: input, i: 0 };
    let network = match f.take(1)?[0] { 0 => "mainnet", 1 => "signet", n => return Err(format!("unknown network code {n}")) };
    let now = u64::from_le_bytes(f.take(8)?.try_into().unwrap());
    let receipt = f.chunk()?;
    let image_hex = f.text()?.to_lowercase();
    let st: Statement = serde_json::from_slice(f.chunk()?).map_err(|e| format!("statement JSON: {e}"))?;
    let (expect, purpose, nonce) = (f.text()?, f.text()?, f.text()?);
    if f.i != input.len() {
        return Err("input frame has trailing bytes".into());
    }
    let image = match Image::by_id(network, &image_hex) {
        Some(i) if !image_hex.is_empty() => i,
        _ => {
            let i = Image::by_network(network).ok_or("no built-in image")?;
            if image_hex.is_empty() { i } else { i.with_method_id(&image_hex).ok_or("image id is not 64 hex characters")? }
        }
    };
    let mut rep = verify(&Inputs { receipt, image, anchor: None, now });
    let mut why: Vec<&str> = Vec::new();
    let statement = match rep.state.as_ref().filter(|_| rep.verdict != Verdict::Fail) {
        None => Err("statement NOT checked: the receipt did not verify".to_string()),
        Some(cs) => {
            let tip = parse_expect(&expect).map_or(false, |(h, x)| identity_matches(h, &x, cs.height, &cs.tip_hash));
            if !expect.is_empty() && !tip {
                Err(format!("expect {expect:?}: not the proof's tip"))
            } else {
                let ch = Challenge { purpose: Some(purpose.as_str()).filter(|p| !p.is_empty()),
                                     nonce: Some(nonce.as_str()).filter(|n| !n.is_empty()) };
                check_statement(cs, rep.undo_from, &st, &ch).map(|c| {
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
