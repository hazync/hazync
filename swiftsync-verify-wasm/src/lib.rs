//! `swiftsync-verify-wasm`: the libzync chain-proof verifier in a browser. Local verification, not a
//! service: the proof and its facts are checked on the device, by the same `swiftsync_verify::verify` the CLI
//! uses, with no network. Raw ABI and no wasm-bindgen, for the reason ../history/verifier-wasm gives: the .wasm is a
//! plain cargo output anyone can rebuild and compare.
//!
//! ```text
//!   alloc(len) -> ptr        reserve len bytes for the caller to write the input frame into
//!   verify(ptr, len) -> p    returns a pointer to [u32 LE length][UTF-8 JSON]
//! ```
//!
//! Input frame (little-endian): `[u8 network: 0 mainnet, 1 signet; | 0x10 coin-log image, | 0x20 product image]`
//! `[u64 now, unix seconds][u32 n][receipt or .sspf bundle][u32 n][anchor JSON, n = 0 for genesis]`
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
        // checked: on wasm32 a crafted chunk length near u32::MAX would wrap `i + n` (the wrapped range was refused by
        // `get` only by accident, and a debug build panicked); review of #724, finding 7
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

fn run(input: &[u8]) -> Result<(Report, &'static Image), String> {
    let mut f = Frame { b: input, i: 0 };
    let base = read_base(&mut f)?;
    let lineage = lineage_chunk(&mut f)?;
    if f.i != input.len() {
        return Err("input frame has trailing bytes".into());
    }
    run_base(base, lineage)
}

/// The frame's fixed head: network code, now, receipt, anchor JSON, image id.
struct Base<'a> { code: u8, now: u64, receipt: &'a [u8], anchor_json: &'a [u8], image_hex: &'a [u8] }

fn read_base<'a>(f: &mut Frame<'a>) -> Result<Base<'a>, String> {
    let code = f.take(1)?[0];
    let now = u64::from_le_bytes(f.take(8)?.try_into().unwrap());
    Ok(Base { code, now, receipt: f.chunk()?, anchor_json: f.chunk()?, image_hex: f.chunk()? })
}

/// Optional, LAST in either frame: the image's lineage (docs/HANDOVER.md). In verify_facts' frame it follows the facts
/// and expect chunks. Before 15.11.2026 it was read right after the image id there too, so the facts JSON was taken
/// for a lineage and every verifyFacts call (and every verify with an expected tip) failed.
fn lineage_chunk<'a>(f: &mut Frame<'a>) -> Result<&'a [u8], String> {
    if f.i < f.b.len() { f.chunk() } else { Ok(&[][..]) }
}

fn run_base(b: Base, lineage_txt: &[u8]) -> Result<(Report, &'static Image), String> {
    let Base { code, now, receipt, anchor_json, image_hex } = b;
    // Network code: 0 mainnet, 1 signet; bit 0x10 = a coin-log image (COINLOG.md), whose proofs carry coin facts.
    // bit 0x20 = the product image (coin log, counting trie, live-coin hash), whose proofs carry trie facts.
    let coinlog = code & 0x10 != 0;
    let product = code & 0x20 != 0;
    let network = match code & !0x30 {
        0 => "mainnet",
        1 => "signet",
        n => return Err(format!("unknown network code {n}")),
    };
    // Optional, last: the image's lineage (docs/HANDOVER.md), for an image the built-in table does not list. Text,
    // one handover per comma, oldest first: `<image hex>:<first height>[:<predecessor RISC Zero verifier digest hex>]`.
    let h = std::str::from_utf8(image_hex).map_err(|_| "image id is not text")?.trim().to_lowercase();
    // A built-in image by id (an earlier or the current image of the chain, with its own lineage), else the
    // network's current image, re-pointed at the stated id.
    let mut image = match Image::by_id(network, &h) {
        Some(i) if !h.is_empty() => i,
        _ => {
            let i = Image::by_network(network).ok_or("no built-in image")?;
            if h.is_empty() { i } else { i.with_method_id(&h).ok_or("image id is not 64 hex characters")? }
        }
    };
    if coinlog {
        image = image.with_coinlog();
    }
    if product {
        image = image.with_product();
    }
    if !lineage_txt.is_empty() {
        let t = std::str::from_utf8(lineage_txt).map_err(|_| "lineage is not text")?;
        let mut list = Vec::new();
        for e in t.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            let bad = || format!("lineage entry {e:?}: want <image hex>:<height>[:<verifier hex>]");
            let mut it = e.split(':');
            let id = swiftsync_verify::hex32(&it.next().ok_or_else(bad)?.to_lowercase()).ok_or_else(bad)?;
            let first_height: u32 = it.next().ok_or_else(bad)?.parse().map_err(|_| bad())?;
            let verifier = match it.next() {
                None => swiftsync_verify::pin::SAME_ZKVM,
                Some(x) => swiftsync_verify::hex32(&x.to_lowercase()).ok_or_else(bad)?,
            };
            if it.next().is_some() { return Err(bad()); }
            list.push(swiftsync_verify::Handover {
                image: swiftsync_verify::image_words(id), first_height, verifier,
            });
        }
        image = image.with_lineage(list);
    }
    let anchor: Option<ChainState> = if anchor_json.is_empty() {
        None
    } else {
        Some(serde_json::from_slice(anchor_json).map_err(|e| format!("anchor is not a ChainState: {e}"))?)
    };
    let rep = if receipt.starts_with(BUNDLE_MAGIC) {
        check_bundle(&Bundle::decode(receipt)?, image, anchor, now)
    } else {
        verify(&Inputs { receipt, image, anchor, now })
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
            // Only a journal that passed: under FAIL its fields are the prover's word (review round 3).
            let st = rep.state.as_ref().filter(|_| rep.verdict != Verdict::Fail);
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

/// `verify_facts(ptr, len)`: the `verify_frame` input (network code | 0x10 for a coin-log image) followed by
/// `[u32 n][JSON {"coins": [CoinFact], "journeys": [CoinJourney], "tries": [TrieFact]}][u32 n][expect]`; expect
/// ("<height>:<hash>", empty for none) is the network's block at the proof's height from a source the PAGE trusts, kept
/// out of the facts JSON so a fact server cannot vouch for its own fork (an "expect" key inside it is refused; cold
/// review 11-06). Without it the verdict is at most DEMO (ZYNCAPPS.md §8). Returns `verify_frame`'s JSON plus `facts`: one {ok, what} per fact, checked
/// against the verified state. A failed fact makes the verdict "fail", as `swiftsync-verify --coin` does.
#[no_mangle]
pub extern "C" fn verify_facts(ptr: *mut u8, len: usize) -> *const u8 {
    let input = unsafe { Vec::from_raw_parts(ptr, len, len) };
    let mut f = Frame { b: &input, i: 0 };
    let res = read_base(&mut f).and_then(|base| {
        let facts = f.chunk()?;
        let expect = std::str::from_utf8(f.chunk()?).map_err(|_| "expect: not UTF-8".to_string())?.trim().to_string();
        let lineage = lineage_chunk(&mut f)?;
        if f.i != input.len() {
            return Err("input frame has trailing bytes".to_string());
        }
        let r = run_base(base, lineage)?;
        let v: serde_json::Value = serde_json::from_slice(facts).map_err(|e| format!("facts JSON: {e}"))?;
        swiftsync_verify::check_facts_wrapper(&v)?;
        let coins: Vec<swiftsync_verify::CoinFact> = serde_json::from_value(v.get("coins").cloned().unwrap_or_else(|| serde_json::json!([])))
            .map_err(|e| format!("facts JSON coins: {e}"))?;
        let journeys: Vec<swiftsync_verify::CoinJourney> = serde_json::from_value(v.get("journeys").cloned().unwrap_or_else(|| serde_json::json!([])))
            .map_err(|e| format!("facts JSON journeys: {e}"))?;
        let tries: Vec<swiftsync_verify::TrieFact> = serde_json::from_value(v.get("tries").cloned().unwrap_or_else(|| serde_json::json!([])))
            .map_err(|e| format!("facts JSON tries: {e}"))?;
        if v.get("expect").is_some() {
            return Err("\"expect\" inside the facts JSON is refused: pass the expected tip separately, from a source the page trusts".to_string());
        }
        let expect = Some(expect).filter(|e| !e.is_empty());
        Ok((r, coins, journeys, tries, expect))
    });
    match res {
        Err(e) => fail(e),
        Ok(((mut rep, image), coins, journeys, tries, expect)) => {
            let any = !coins.is_empty() || !journeys.is_empty() || !tries.is_empty();
            let mut out = Vec::new();
            // Facts need the tip compared with the network (step 7): the page passes "expect": "h:hash" from a source
            // it trusts. Without a match the verdict is at most DEMO (adversarial review, 11-03).
            let tip_checked = match (&expect, rep.state.as_ref()) {
                (Some(e), Some(cs)) => match swiftsync_verify::parse_expect(e) {
                    Some((h, x)) if swiftsync_verify::identity_matches(h, &x, cs.height, &cs.tip_hash) => true,
                    _ => { out.push(Err(format!("expect {e:?}: not the proof's tip"))); false }
                },
                _ => false,
            };
            if rep.verdict == Verdict::Fail {
                // the journal of a receipt that did not verify is the prover's word: nothing is checked against it
                if any {
                    out.push(Err("facts NOT checked: the receipt did not verify".to_string()));
                }
            } else if let Some(cs) = rep.state.as_ref() {
                let uf = rep.undo_from; // the block-leaf format by height (docs/libzync/UNDO.md)
                for c in &coins {
                    out.push(swiftsync_verify::check_coin_fact(cs, uf, c));
                }
                for j in &journeys {
                    out.push(swiftsync_verify::check_coin_journey(cs, uf, j));
                }
                for t in &tries {
                    out.push(swiftsync_verify::check_trie_fact(cs, uf, t));
                }
            } else if any {
                out.push(Err("no verified state to check facts against".to_string()));
            }
            if out.iter().any(|r| r.is_err()) {
                rep.verdict = Verdict::Fail;
            }
            if rep.verdict == Verdict::Ok && !tip_checked && any {
                rep.verdict = Verdict::Demo;
            }
            // A fact is "ok" only under an OK verdict: under DEMO it holds on a chain not shown to be the network's.
            let demo = rep.verdict != Verdict::Ok;
            let facts: Vec<serde_json::Value> = out.into_iter().map(|r| match r {
                Ok(w) if demo => serde_json::json!({"ok": false, "what": format!("holds on this chain, but the chain is not shown to be the network's: {w}")}),
                Ok(w) => serde_json::json!({"ok": true, "what": w}),
                Err(w) => serde_json::json!({"ok": false, "what": w}),
            }).collect();
            ret(serde_json::json!({
                "verdict": match rep.verdict { Verdict::Ok => "ok", Verdict::Demo => "demo", Verdict::Fail => "fail" },
                "image": image.method_id_hex,
                "network": image.network,
                "checks": rep.checks.iter().map(|(ok, what)| serde_json::json!({"ok": ok, "what": what})).collect::<Vec<_>>(),
                "height": rep.state.as_ref().filter(|_| rep.verdict != Verdict::Fail).map(|c| c.height),
                "facts": facts,
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

#[cfg(test)]
mod frame_tests {
    use super::Frame;

    #[test]
    fn take_refuses_a_length_that_would_wrap() {
        let b = [0u8; 8];
        let mut f = Frame { b: &b, i: usize::MAX - 2 };
        assert!(f.take(4).is_err());
        let mut f = Frame { b: &b, i: 4 };
        assert!(f.take(usize::MAX).is_err());
        let mut f = Frame { b: &b, i: 0 };
        assert_eq!(f.take(8).unwrap().len(), 8);
        assert!(f.take(1).is_err());
    }
}
