//! `zapp-corecheck`: the core receipt check every zAPP resting on a core proof makes first (docs/libzync/ZYNCAPPS.md
//! §9): the image a caller names ([`image_for`], as the core verifier's WASM resolves it) and the receipt (or .sspf
//! bundle) verified by the core verifier with its tip compared with the caller's `expect` ([`verify_core`]). Not a
//! zAPP and not part of the core verifier; no CLI code (curl, bitcoin-cli: that is `zapps/common`, CLIs only), so a
//! zAPP's library, C ABI and WASM module may use it.

use swiftsync_verify::{check_bundle, identity_matches, parse_expect, verify, Bundle, ChainState, Image, Inputs, Report, Verdict, BUNDLE_MAGIC};

#[cfg(all(feature = "regtest-test", not(debug_assertions)))]
compile_error!("zapp-corecheck's `regtest-test` feature is for tests only: it never builds without debug assertions (a release \
                build). Build the test CLIs with the dev profile (scripts/zapp-regtest/README.md).");

/// The zAPP regtest harness's image (scripts/zapp-regtest/README.md, "What the regtest-test feature must do"), TESTS
/// ONLY: CI's `host_reg` (`build-hosts.sh`, run 43835411815 on main 797b777e; re-pin from the `libzync-hosts`
/// artefact's ids.txt on any guest change, never from a laptop build). Core's regtest has no minimum chain work and the
/// guest pins no checkpoint, so a genesis-anchored proof reaches OK: that is the point (an `--anchor` caps it at DEMO).
/// Not in swiftsync-verify's IMAGES, so the core verifier, its CLI and its .wasm do not change.
#[cfg(feature = "regtest-test")]
pub static REGTEST_IMAGE: Image = Image {
    network: "regtest",
    method_id_hex: "eb2cd424b94a4aa74f3458efc0d5edb1d3c35d46615eab8363696f1fa4b47ccd",
    // LIBZYNC 1 | regtest 8 | COINLOG 16 | TRIE 32 | UTXOHASH 64, as the executed state_*.json carry it
    profile: 0x79,
    genesis_hash_hex: "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206",
    genesis_time: 1_296_688_602,
    genesis_bits: 0x207f_ffff,
    // 2^256 / (target + 1)
    genesis_work: 2,
    // Core's regtest nMinimumChainWork is 0
    min_work_hex: "0000000000000000000000000000000000000000000000000000000000000000",
    // scripts/libzync/genesis_anchor_id.py regtest, = the executed chain's anchor_id (scripts/zapp-regtest/execute.sh)
    genesis_anchor_hex: "99509fc506d022f06dfd72bb146fc894718ce8047e69e10b201e1a78d4bccf63",
    // the regtest guest pins no checkpoint (checkpoint.rs CHECKPOINT = None) and no handover (lineage/regtest_coinlog.rs)
    checkpoint_height: 0,
    lineage: &[],
};

/// The first check of every report on the regtest image, so its verdict cannot be read as a mainnet one.
#[cfg(feature = "regtest-test")]
pub const TEST_IMAGE_LINE: &str = "TEST IMAGE (regtest-test): never a release";

/// The network's built-in image: the core verifier's, or under `regtest-test` the harness's for "regtest".
fn network_image(network: &str) -> Option<&'static Image> {
    #[cfg(feature = "regtest-test")]
    if network == "regtest" {
        return Some(&REGTEST_IMAGE);
    }
    Image::by_network(network)
}

/// A hash in display order (as `getblockhash` prints it), from internal byte order.
fn display_hex(h: &[u8; 32]) -> String {
    h.iter().rev().map(|b| format!("{b:02x}")).collect()
}

/// The image a caller names, as the core verifier's WASM resolves it: `network` "mainnet" or "signet" (and "regtest" under
/// the tests-only `regtest-test` feature: [`REGTEST_IMAGE`]); `image_hex`
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
            let i = network_image(network).ok_or_else(|| format!("unknown network {network:?}"))?;
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
    // the core's strict decoder (#758: the kind gate, no trailing bytes, no word risc0 asserts on), as verify() decodes
    swiftsync_verify::decode_receipt(r).ok()
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
        test_image_line(&mut r, image);
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
    test_image_line(&mut report, used);
    CoreCheck { report, tip_checked, image: used }
}

/// Under `regtest-test`, a report on the regtest image starts with [`TEST_IMAGE_LINE`] (a passing check: it says what the
/// verdict is about, it does not change it). Without the feature there is no regtest image and nothing to mark.
#[allow(unused_variables)]
fn test_image_line(report: &mut Report, image: &Image) {
    #[cfg(feature = "regtest-test")]
    if image.network == "regtest" {
        report.checks.insert(0, (true, TEST_IMAGE_LINE.to_string()));
    }
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

    /// The zAPP regtest harness's real receipts (scripts/zapp-regtest/receipts, GPU-proved, image eb2cd424) and the
    /// block hashes Bitcoin Core gave for that chain (scripts/zapp-regtest/oracle): the independent answer.
    const REGTEST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/zapp-regtest");
    const A112: &str = "67c1d2e718067bba4ffd07a10f2ce27f5e6b55702eb2c766fafbf36db6160574";
    const B113: &str = "1b5db3dac39b651ab1186418ddefa6746d524e9601666ad9826ab969476bf3d7";
    /// b's block 112 (oracle/blocks_b.json): the same height as a 112, the other branch
    const B112: &str = "4395baa44c0364ae2f6f415e439b4ce82a5d9e77cb9ad80bdf51eebb2406ce16";
    const REGTEST_ID: &str = "eb2cd424b94a4aa74f3458efc0d5edb1d3c35d46615eab8363696f1fa4b47ccd";

    fn regtest_file(rel: &str) -> Vec<u8> {
        std::fs::read(format!("{REGTEST}/{rel}")).unwrap_or_else(|e| panic!("{REGTEST}/{rel}: {e}"))
    }

    /// The hashes above are Core's (the oracle), not this crate's.
    #[test]
    fn regtest_hashes_are_the_oracles() {
        let a = String::from_utf8(regtest_file("oracle/snap_a112.json")).unwrap();
        let b = String::from_utf8(regtest_file("oracle/snap_b113.json")).unwrap();
        let bb = String::from_utf8(regtest_file("oracle/blocks_b.json")).unwrap();
        assert!(a.contains(&format!("\"hash\": \"{A112}\"")) && a.contains("\"height\": 112"));
        assert!(b.contains(&format!("\"hash\": \"{B113}\"")) && b.contains("\"height\": 113"));
        assert!(bb.contains(&format!("\"hash\": \"{B112}\"")));
    }

    /// Positive control, SHIPPED build (no feature): the regtest receipt is refused: "regtest" is an unknown network, and
    /// against the mainnet / signet image it does not verify (another image).
    #[cfg(not(feature = "regtest-test"))]
    #[test]
    fn shipped_build_refuses_the_regtest_receipt() {
        let r = regtest_file("receipts/a/block_112.rcpt");
        assert!(image_for("regtest", "", false, false).err().unwrap().contains("unknown network"));
        assert!(image_for("regtest", REGTEST_ID, false, false).is_err());
        for net in ["mainnet", "signet"] {
            let img = image_for(net, "", false, false).unwrap();
            let c = verify_core(&r, img, false, None, 1_800_000_000, Some(&format!("112:{A112}")));
            assert!(c.report.verdict == Verdict::Fail && !c.tip_checked, "{net}: {:?}", c.report.checks);
            assert!(c.report.checks.iter().any(|(ok, l)| !ok && l.starts_with("receipt verifies against image")), "{net}: {:?}", c.report.checks);
            assert!(!c.report.checks.iter().any(|(_, l)| l.contains("TEST IMAGE")));
        }
    }

    /// With the feature (dev profile only): the regtest image is the harness's, a real a 112 receipt is OK with Core's
    /// block 112 expected, and FAIL with b's 112 or 113; b 113 is OK with b's 113; every report starts "TEST IMAGE".
    #[cfg(feature = "regtest-test")]
    #[test]
    fn regtest_receipts_ok_and_reorg() {
        use swiftsync_verify::{anchor_id, genesis_anchor, hex};
        let img = image_for("regtest", "", false, false).unwrap();
        assert!(std::ptr::eq(img, &REGTEST_IMAGE) && img.method_id_hex == REGTEST_ID);
        // the pinned anchor is what the core library computes for these parameters (as genesis_anchors_are_pinned)
        assert_eq!(hex(&anchor_id(&genesis_anchor(img))), img.genesis_anchor_hex);
        // coinlog / product change nothing: the profile already carries those bits
        assert_eq!(image_for("regtest", "", true, true).unwrap().profile, 0x79);
        let a = regtest_file("receipts/a/block_112.rcpt");
        let b = regtest_file("receipts/b/block_113.rcpt");
        let now = 1_800_000_000;
        let ok = verify_core(&a, img, false, None, now, Some(&format!("112:{A112}")));
        assert_eq!((ok.report.verdict, ok.tip_checked), (Verdict::Ok, true), "{:?}", ok.report.checks);
        assert_eq!(ok.report.checks[0], (true, TEST_IMAGE_LINE.to_string()));
        assert!(ok.report.checks.iter().all(|c| c.0), "{:?}", ok.report.checks);
        let cs = ok.report.state.as_ref().unwrap();
        assert_eq!((cs.height, display_hex(&cs.tip_hash)), (112, A112.to_string()));
        // Core's chainwork at a 112 (oracle snap_a112.json header): 0xe2
        assert_eq!(display_hex(&cs.cum_work), format!("{:064x}", 0xe2));
        // the image given explicitly (pinned) is the same image: OK
        let pinned = image_for("regtest", REGTEST_ID, false, false).unwrap();
        let p = verify_core(&a, pinned, true, None, now, Some(&format!("112:{A112}")));
        assert_eq!((p.report.verdict, p.tip_checked), (Verdict::Ok, true), "{:?}", p.report.checks);
        // the reorg: a 112 is not b's 112, nor b's tip
        for e in [format!("112:{B112}"), format!("113:{B113}")] {
            let f = verify_core(&a, img, false, None, now, Some(&e));
            assert_eq!((f.report.verdict, f.tip_checked), (Verdict::Fail, false), "{e}: {:?}", f.report.checks);
            assert_eq!(f.report.checks[0].1, TEST_IMAGE_LINE);
        }
        let okb = verify_core(&b, img, false, None, now, Some(&format!("113:{B113}")));
        assert_eq!((okb.report.verdict, okb.tip_checked), (Verdict::Ok, true), "{:?}", okb.report.checks);
        assert_eq!(display_hex(&okb.report.state.as_ref().unwrap().cum_work), format!("{:064x}", 0xe4));
        let fb = verify_core(&b, img, false, None, now, Some(&format!("112:{A112}")));
        assert_eq!(fb.report.verdict, Verdict::Fail);
        // no expect: still OK from the core's side (the zAPP decides what an unchecked tip is worth)
        assert_eq!(verify_core(&a, img, false, None, now, None).report.verdict, Verdict::Ok);
        // the earlier receipts of branch a: OK at their own heights
        for (h, rel) in [(104, "receipts/a/block_104.rcpt"), (105, "receipts/a/block_105.rcpt"), (110, "receipts/a/block_110.rcpt")] {
            let c = verify_core(&regtest_file(rel), img, false, None, now, None);
            assert!(c.report.verdict == Verdict::Ok && c.report.state.as_ref().unwrap().height == h, "{rel}: {:?}", c.report.checks);
        }
        // a mainnet image given for the regtest receipt: FAIL (and no TEST IMAGE line: the report is not on that image)
        let m = verify_core(&a, image_for("mainnet", "", false, false).unwrap(), false, None, now, None);
        assert!(m.report.verdict == Verdict::Fail && !m.report.checks.iter().any(|c| c.1 == TEST_IMAGE_LINE), "{:?}", m.report.checks);
    }

    /// Positive control: the feature's image with a non-zero minimum work turns the OK into DEMO (low work), at exactly
    /// the boundary: the chain's work 0xe2 is enough for 0xe2 and not for 0xe3.
    #[cfg(feature = "regtest-test")]
    #[test]
    fn regtest_min_work_mutant_is_demo() {
        let a = regtest_file("receipts/a/block_112.rcpt");
        let leak = |w: &'static str| -> &'static Image { Box::leak(Box::new(Image { min_work_hex: w, ..REGTEST_IMAGE })) };
        let at = leak("00000000000000000000000000000000000000000000000000000000000000e2");
        let above = leak("00000000000000000000000000000000000000000000000000000000000000e3");
        let ok = verify_core(&a, at, false, None, 1_800_000_000, Some(&format!("112:{A112}")));
        assert_eq!(ok.report.verdict, Verdict::Ok, "{:?}", ok.report.checks);
        let d = verify_core(&a, above, false, None, 1_800_000_000, Some(&format!("112:{A112}")));
        assert_eq!((d.report.verdict, d.report.low_work, d.tip_checked), (Verdict::Demo, true, true), "{:?}", d.report.checks);
        // so does a checkpoint above the tip, and an anchor (a given state is never OK)
        let cp: &'static Image = Box::leak(Box::new(Image { checkpoint_height: 113, ..REGTEST_IMAGE }));
        assert_eq!(verify_core(&a, cp, false, None, 1_800_000_000, None).report.verdict, Verdict::Demo);
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
