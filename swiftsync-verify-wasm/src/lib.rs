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
use swiftsync_verify::headers;

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

fn run(input: &[u8]) -> Result<(Report, &'static Image), String> {
    let mut f = Frame { b: input, i: 0 };
    let out = run_frame(&mut f)?;
    if f.i != input.len() {
        return Err("input frame has trailing bytes".into());
    }
    Ok(out)
}

fn run_frame(f: &mut Frame) -> Result<(Report, &'static Image), String> {
    // Network code: 0 mainnet, 1 signet; bit 0x10 = a coin-log image (COINLOG.md), whose proofs carry coin facts.
    let code = f.take(1)?[0];
    // bit 0x20 = the product image (coin log, counting trie, live-coin hash), whose proofs carry trie facts.
    let coinlog = code & 0x10 != 0;
    let product = code & 0x20 != 0;
    let network = match code & !0x30 {
        0 => "mainnet",
        1 => "signet",
        n => return Err(format!("unknown network code {n}")),
    };
    let now = u64::from_le_bytes(f.take(8)?.try_into().unwrap());
    let receipt = f.chunk()?;
    let anchor_json = f.chunk()?;
    let image_hex = f.chunk()?;
    // Optional, last: the image's lineage (docs/HANDOVER.md), for an image the built-in table does not list. Text,
    // one handover per comma, oldest first: `<image hex>:<first height>[:<predecessor RISC Zero verifier digest hex>]`.
    let lineage_txt = if f.i < f.b.len() { f.chunk()? } else { &[][..] };
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
    let res = run_frame(&mut f).and_then(|r| {
        let facts = f.chunk()?;
        let expect = std::str::from_utf8(f.chunk()?).map_err(|_| "expect: not UTF-8".to_string())?.trim().to_string();
        if f.i != input.len() {
            return Err("input frame has trailing bytes".to_string());
        }
        let v: serde_json::Value = serde_json::from_slice(facts).map_err(|e| format!("facts JSON: {e}"))?;
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

/// The header-chain zyncAPP (ROADMAP K1, N3), by the same `swiftsync_verify::headers::verify_headers` the CLI's
/// `swiftsync-verify headers` uses. Input frame (little-endian): `[u64 now][u32 count]` then count times
/// `[u32 n][receipt]` (the first from genesis, each next one starting where the one before ended), then
/// `[u32 n][image id hex, n = 0 for the built-in id][u32 n]["height:hash" from a source the page trusts, n = 0 for none]`,
/// optionally followed by `[u32 n][include JSON, n = 0 for none]`: `{"height", "hash" (as explorers print it), "path"
/// (MMR hashes as zyncapp headers-mmr-path prints them), "txid"? and "tx"?, both or neither: the transaction asked
/// about (as explorers print it; the CALLER's, never the service's) and the path service's /tx answer (pos, header,
/// raw, branch, coinbase, coinbase_branch; swiftsync_verify::headers::TxInclusion::from_json, which does not read the
/// answer's own txid: the raw transaction must hash to the asked one)}`.
///
/// Output beside verdict, checks, height, hash, chainwork, max_time, mmr_root, identity_checked:
///   `included`: null when nothing was asked; true ONLY when the verdict is "ok" and the block (and transaction) is on
///   the chain. A DEMO chain (below Core's minimum work: anyone can mine a difficulty-1 fork) never gives true.
///   `included_demo`: null when nothing was asked; true when the verdict is "demo" and the inclusion checks passed.
///   `inclusion`: `{height, hash, txid}` of what was shown (txid null for a block alone), non-null only when no check
///   failed (verdict "ok" or "demo": read it with the verdict, as `included` / `included_demo` do).
#[no_mangle]
pub extern "C" fn verify_headers(ptr: *const u8, len: usize) -> *const u8 {
    let input = unsafe { std::slice::from_raw_parts(ptr, len) };
    let parsed = (|| -> Result<(u64, Vec<&[u8]>, String, String, Option<headers::Inclusion>), String> {
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
    let image = if image.is_empty() { swiftsync_verify::headers::HEADERS_IMAGE_ID.to_string() } else { image };
    let exp = if expect.is_empty() { None } else {
        match expect.split_once(':').and_then(|(h, x)| h.parse::<i64>().ok().map(|h| (h, x))) {
            Some(e) => Some(e),
            None => return fail(format!("expect {expect:?}: give height:hash")),
        }
    };
    let rep = swiftsync_verify::headers::verify_headers(&rs, &image, now, exp, include.as_ref());
    let ok = rep.verdict != Verdict::Fail;
    let s = rep.state.as_ref().filter(|_| ok);
    ret(serde_json::json!({
        "verdict": match rep.verdict { Verdict::Ok => "ok", Verdict::Demo => "demo", Verdict::Fail => "fail" },
        "image": image,
        "checks": rep.checks.iter().map(|(ok, what)| serde_json::json!({"ok": ok, "what": what})).collect::<Vec<_>>(),
        "height": s.map(|s| s.height),
        "hash": s.map(|s| s.hash_hex()),
        "chainwork": s.map(|s| s.work_hex()),
        "max_time": s.map(|s| s.max_time),
        "mmr_root": s.map(|s| s.mmr_root_hex()),
        "identity_checked": ok && exp.is_some(),
        // the block (and transaction) asked for is on a chain with Core's minimum work; null when none was asked
        "included": include.as_ref().map(|_| rep.verdict == Verdict::Ok && rep.included.is_some()),
        // ...on a valid chain BELOW it (a demo proof): shown, but not a claim about the network's chain
        "included_demo": include.as_ref().map(|_| rep.verdict == Verdict::Demo && rep.included.is_some()),
        "inclusion": rep.included.as_ref().filter(|_| ok).map(|i| serde_json::json!({
            "height": i.height, "hash": i.hash_hex(), "txid": i.txid_hex(),
        })),
    }).to_string())
}

fn parse_include(j: &[u8]) -> Result<headers::Inclusion, String> {
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
        (serde_json::Value::String(txid), tx) if tx.is_object() => Some(headers::TxInclusion::from_json(txid, tx)?),
        (serde_json::Value::Null, _) => return Err("include: tx needs txid, the transaction asked about (never taken from the answer)".into()),
        (_, serde_json::Value::Null) => return Err("include: txid needs tx, the path service's /tx answer".into()),
        _ => return Err("include: txid is a string and tx an object".into()),
    };
    headers::Inclusion::parse(height, hash, &path, tx)
}

/// The built-in header-chain image id, for the page to display.
#[no_mangle]
pub extern "C" fn headers_image_id() -> *const u8 {
    ret(swiftsync_verify::headers::HEADERS_IMAGE_ID.to_string())
}

/// The deposit-proof zyncAPP (docs/libzync/DEPOSIT_ZAPP.md), by the same `swiftsync_verify::deposit::verify_deposit`
/// the CLI's `swiftsync-verify deposit` uses. Input frame (little-endian): `[u64 now][u32 n][bundle JSON, as zync-deposit
/// writes it][u32 n][ask JSON]`. The ask is the CALLER's, never the bundle's: `{"txid" (as explorers print it), "vout"?,
/// "amount"? (sat), "script"? (hex), "op_return"? (hex payload), "min_depth"?, "min_validated_depth"?,
/// "allow_pow_only_block"?, "expect"? ("height:hash" from a source the page trusts), "max_tip_age"? (s), "network"?}`.
///
/// Output: `verdict` ("ok", "demo", "fail"), `checks`, and `statement` (null unless no check failed: read it with the
/// verdict) with the block, the output checked, the tip, `validated_through` (the core tip), `confirmations`,
/// `validated_confirmations` and `block_fully_validated`.
#[no_mangle]
pub extern "C" fn verify_deposit(ptr: *const u8, len: usize) -> *const u8 {
    use swiftsync_verify::deposit::{disp, DepositAsk, DepositBundle};
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
    let rep = swiftsync_verify::deposit::verify_deposit(&b, &ask, &network, image, swiftsync_verify::headers::HEADERS_IMAGE_ID, now);
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
