//! `zapp-corecheck`: the core receipt check every zAPP resting on a core proof makes first (docs/libzync/ZYNCAPPS.md
//! §9): the image a caller names ([`image_for`], as the core verifier's WASM resolves it) and the receipt (or .sspf
//! bundle) verified by the core verifier with its tip compared with the caller's `expect` ([`verify_core`]). Not a
//! zAPP and not part of the core verifier; no CLI code (curl, bitcoin-cli: that is `zapps/common`, CLIs only), so a
//! zAPP's library, C ABI and WASM module may use it.

use swiftsync_verify::{check_bundle, identity_matches, parse_expect, verify, Bundle, ChainState, Image, Inputs, Report, Verdict, BUNDLE_MAGIC};

/// A hash in display order (as `getblockhash` prints it), from internal byte order.
fn display_hex(h: &[u8; 32]) -> String {
    h.iter().rev().map(|b| format!("{b:02x}")).collect()
}

/// The image a caller names, as the core verifier's WASM resolves it: `network` "mainnet" or "signet"; `image_hex`
/// empty for the network's built-in image, else a built-in image by id (with its own lineage) or the network's current
/// image re-pointed at that id; `coinlog` / `product` as the core's network bits 0x10 / 0x20. (No lineage text: an image
/// the built-in table does not list, with its own handovers, is checked by the core verifier.)
pub fn image_for(network: &str, image_hex: &str, coinlog: bool, product: bool) -> Result<&'static Image, String> {
    // Image's with_* helpers leak a new Image per call; a long-lived caller (a server, a page verifying many times)
    // would grow without bound. One leaked Image per distinct request, cached.
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<(String, String, bool, bool), &'static Image>>> = OnceLock::new();
    let key = (network.to_string(), image_hex.trim().to_lowercase(), coinlog, product);
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(i) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(i);
    }
    let i = image_for_uncached(network, image_hex, coinlog, product)?;
    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(key, i);
    Ok(i)
}

fn image_for_uncached(network: &str, image_hex: &str, coinlog: bool, product: bool) -> Result<&'static Image, String> {
    let h = image_hex.trim().to_lowercase();
    let mut image = match Image::by_id(network, &h) {
        Some(i) if !h.is_empty() => i,
        _ => {
            let i = Image::by_network(network).ok_or_else(|| format!("unknown network {network:?}"))?;
            if h.is_empty() { i } else { i.with_method_id(&h).ok_or("image id is not 64 hex characters")? }
        }
    };
    if coinlog {
        image = image.with_coinlog();
    }
    if product {
        image = image.with_product();
    }
    Ok(image)
}

/// A core receipt (or .sspf bundle) verified by the core verifier, and its tip compared with `expect` ("<height>:<hash>",
/// display order, from a source the CALLER trusts): what a zAPP resting on a core proof checks first. A wrong `expect`
/// is a FAIL check; `tip_checked` says whether the tip was compared and matched.
pub struct CoreCheck {
    pub report: Report,
    pub tip_checked: bool,
}

pub fn verify_core(receipt: &[u8], image: &'static Image, anchor: Option<ChainState>, now: u64, expect: Option<&str>) -> CoreCheck {
    let mut report = if receipt.starts_with(BUNDLE_MAGIC) {
        match Bundle::decode(receipt) {
            Ok(b) => check_bundle(&b, image, anchor, now),
            // the core's report for no receipt at all (a FAIL with no state), with the bundle's own error first
            Err(e) => {
                let mut r = verify(&Inputs { receipt: &[], image, anchor: None, now });
                r.checks.insert(0, (false, format!("bundle: {e}")));
                r.verdict = Verdict::Fail;
                r.state = None;
                r
            }
        }
    } else {
        verify(&Inputs { receipt, image, anchor, now })
    };
    let tip_checked = match (expect.map(str::trim).filter(|e| !e.is_empty()), report.state.as_ref()) {
        (Some(e), Some(cs)) => match parse_expect(e) {
            Some((h, x)) if identity_matches(h, &x, cs.height, &cs.tip_hash) => {
                report.checks.push((true, format!("block {h} is {x}: the proof's tip")));
                true
            }
            _ => {
                report.checks.push((false, format!("expect {e:?}: not the proof's tip (block {} {})", cs.height, display_hex(&cs.tip_hash))));
                report.verdict = Verdict::Fail;
                false
            }
        },
        _ => false,
    };
    CoreCheck { report, tip_checked }
}

#[cfg(test)]
mod tests {
    use super::*;
    const H200: &str = "000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320";

    /// The real mainnet 1..200 proof (8888d59c): DEMO (low work); its tip expected, and another block, refused.
    #[test]
    fn verify_core_and_expect() {
        let r = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../swiftsync-verify/vectors/mainnet_1_200_8888d59c.rcpt")).unwrap();
        let img = image_for("mainnet", "8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4", false, false).unwrap();
        let ok = verify_core(&r, img, None, 1_800_000_000, Some(&format!("200:{H200}")));
        assert_eq!((ok.report.verdict, ok.tip_checked), (Verdict::Demo, true), "{:?}", ok.report.checks);
        let bad = verify_core(&r, img, None, 1_800_000_000, Some(&format!("200:{}1", &H200[..63])));
        assert_eq!((bad.report.verdict, bad.tip_checked), (Verdict::Fail, false));
        assert_eq!(verify_core(&r, img, None, 1_800_000_000, None).tip_checked, false);
        assert!(image_for("mainnet", "xyz", false, false).is_err() && image_for("testnet", "", false, false).is_err());
        // cached: the same request gives the same Image, so repeated calls leak nothing new
        let a = image_for("mainnet", "", true, true).unwrap();
        assert!(std::ptr::eq(a, image_for("mainnet", " ", true, true).unwrap()));
        assert!(!std::ptr::eq(a, image_for("mainnet", "", true, false).unwrap()));
        // a bundle that does not decode: FAIL, no state
        let junk = [swiftsync_verify::BUNDLE_MAGIC, &[0u8; 4][..]].concat();
        let j = verify_core(&junk, img, None, 1_800_000_000, None);
        assert!(j.report.verdict == Verdict::Fail && j.report.state.is_none() && j.report.checks[0].1.starts_with("bundle:"));
    }
}
