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

/// The image the core verifier checks `receipt` (or a .sspf bundle's receipt) against when asked for `image`: `image`
/// itself, or the earlier built-in image of its chain that the receipt names, which `swiftsync_verify::verify`
/// substitutes when asked for the network's current built-in image (`earlier_image_named_by`, under the same
/// `receipt_kind` gate). Asked of the core library, never read back from a check string.
pub fn image_used(receipt: &[u8], image: &'static Image) -> &'static Image {
    let inner;
    let r = if receipt.starts_with(BUNDLE_MAGIC) {
        match Bundle::decode(receipt) { Ok(b) => { inner = b.receipt; &inner[..] } Err(_) => return image }
    } else {
        receipt
    };
    if swiftsync_verify::receipt_kind(r).is_err() {
        return image;
    }
    bincode::deserialize::<risc0_zkvm::Receipt>(r).ok()
        .and_then(|rc| swiftsync_verify::earlier_image_named_by(&rc, image))
        .unwrap_or(image)
}

/// A core receipt (or .sspf bundle) verified by the core verifier, and its tip compared with `expect` ("<height>:<hash>",
/// display order, from a source the CALLER trusts): what a zAPP resting on a core proof checks first. A wrong `expect`
/// is a FAIL check; `tip_checked` says whether the tip was compared and matched in a report that is not a FAIL.
pub struct CoreCheck {
    pub report: Report,
    pub tip_checked: bool,
    /// The image the receipt was verified against ([`image_used`]): the one asked for, or the earlier built-in image
    /// the receipt names. A zAPP that says which image proved the chain reports this one.
    pub image: &'static Image,
}

/// `pinned`: the caller named the image explicitly (`--image`, a non-empty image id): then it is the ONLY image the
/// receipt may verify against. The core verifier, asked for the network's current built-in image, would accept a
/// receipt of an earlier built-in image of the chain instead (right for "the network's image", wrong for "this image");
/// a pinned image refuses that receipt (review 11-16). The core verifier itself is not changed.
pub fn verify_core(receipt: &[u8], image: &'static Image, pinned: bool, anchor: Option<ChainState>, now: u64,
                   expect: Option<&str>) -> CoreCheck {
    let used = image_used(receipt, image);
    if pinned && used.method_id_hex != image.method_id_hex {
        let mut r = verify(&Inputs { receipt: &[], image, anchor: None, now });
        r.checks.insert(0, (false, format!("the receipt is of image {} ({}), not the image given, {}: a given image is the only one \
                                            it may verify against", used.method_id_hex, used.network, image.method_id_hex)));
        r.verdict = Verdict::Fail;
        r.state = None;
        return CoreCheck { report: r, tip_checked: false, image };
    }
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
    // a tip that matched in a report that FAILS (a bad receipt, a failed check) is not a checked tip (review 11-16)
    let tip_checked = tip_checked && report.verdict != Verdict::Fail;
    CoreCheck { report, tip_checked, image: used }
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
        let ok = verify_core(&r, img, false, None, 1_800_000_000, Some(&format!("200:{H200}")));
        assert_eq!((ok.report.verdict, ok.tip_checked), (Verdict::Demo, true), "{:?}", ok.report.checks);
        let bad = verify_core(&r, img, false, None, 1_800_000_000, Some(&format!("200:{}1", &H200[..63])));
        assert_eq!((bad.report.verdict, bad.tip_checked), (Verdict::Fail, false));
        assert_eq!(verify_core(&r, img, false, None, 1_800_000_000, None).tip_checked, false);
        assert!(image_for("mainnet", "xyz", false, false).is_err() && image_for("testnet", "", false, false).is_err());
        // cached: the same request gives the same Image, so repeated calls leak nothing new
        let a = image_for("mainnet", "", true, true).unwrap();
        assert!(std::ptr::eq(a, image_for("mainnet", " ", true, true).unwrap()));
        assert!(!std::ptr::eq(a, image_for("mainnet", "", true, false).unwrap()));
        // a bundle that does not decode: FAIL, no state
        let junk = [swiftsync_verify::BUNDLE_MAGIC, &[0u8; 4][..]].concat();
        let j = verify_core(&junk, img, false, None, 1_800_000_000, None);
        assert!(j.report.verdict == Verdict::Fail && j.report.state.is_none() && j.report.checks[0].1.starts_with("bundle:"));
    }

    /// Review 11-16: a given (pinned) image is the only one the receipt may verify against; the image that verified is
    /// reported; a tip that matches in a FAILED report is not a checked tip.
    #[test]
    fn pinned_image_reported_image_and_failed_tip() {
        let r = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../swiftsync-verify/vectors/mainnet_1_200_8888d59c.rcpt")).unwrap();
        let cur = image_for("mainnet", "", false, false).unwrap();
        assert_ne!(cur.method_id_hex, "8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4", "the current image is a later one");
        // the network's image, not pinned: the earlier built-in image the receipt names verifies it, and is reported
        let d = verify_core(&r, cur, false, None, 1_800_000_000, Some(&format!("200:{H200}")));
        assert_eq!((d.report.verdict, d.tip_checked), (Verdict::Demo, true), "{:?}", d.report.checks);
        assert_eq!(d.image.method_id_hex, "8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4");
        assert_eq!(image_used(&r, cur).method_id_hex, d.image.method_id_hex);
        // the same id given explicitly: refused, nothing verified
        let pinned = image_for("mainnet", cur.method_id_hex, false, false).unwrap();
        let p = verify_core(&r, pinned, true, None, 1_800_000_000, Some(&format!("200:{H200}")));
        assert!(p.report.verdict == Verdict::Fail && !p.tip_checked && p.report.state.is_none(), "{:?}", p.report.checks);
        assert!(p.report.checks[0].1.contains("not the image given"), "{:?}", p.report.checks);
        // the receipt's own image given: verifies, reported as itself
        let own = image_for("mainnet", "8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4", false, false).unwrap();
        let o = verify_core(&r, own, true, None, 1_800_000_000, None);
        assert!(o.report.verdict == Verdict::Demo && std::ptr::eq(o.image, own), "{:?}", o.report.checks);
        // a FAILED report (this clock is before the chain's blocks: time-too-new) with the tip expected: not tip_checked
        let f = verify_core(&r, own, false, None, 0, Some(&format!("200:{H200}")));
        assert_eq!(f.report.verdict, Verdict::Fail, "{:?}", f.report.checks);
        assert!(f.report.state.is_some() && f.report.checks.iter().any(|c| c.1.starts_with("block 200 is")), "the tip WAS compared: {:?}", f.report.checks);
        assert!(!f.tip_checked);
    }
}
