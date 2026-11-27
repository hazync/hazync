//! `zapp-corecheck`: the core proof check every zAPP resting on a core proof makes first (docs/libzync/ZYNCAPPS.md
//! §9): the image a caller names ([`image_for`], as the core verifier's WASM resolves it) with its pinned key
//! ([`with_key`]), and the engine proof (or .sspf bundle) verified by the core verifier with its tip compared with the
//! caller's `expect` ([`verify_core`]). Engine (OpenVM) proofs only since the cutover (27.11.2026, engine/CUTOVER.md). Not a
//! zAPP and not part of the core verifier; no CLI code (curl, bitcoin-cli: that is `zapps/common`, CLIs only), so a
//! zAPP's library, C ABI and WASM module may use it.

use swiftsync_verify::{check_bundle, identity_matches, parse_expect, verify, Bundle, ChainState, Image, Inputs, Report, Verdict, BUNDLE_MAGIC};

#[cfg(all(feature = "regtest-test", not(debug_assertions)))]
compile_error!("zapp-corecheck's `regtest-test` feature is for tests only: it never builds without debug assertions (a release \
                build). Build the test CLIs with the dev profile (scripts/zapp-regtest/README.md).");

/// The zAPP regtest harness's core image (scripts/zapp-regtest/README.md, "What the regtest-test feature must do"), TESTS
/// ONLY: the engine build keys-rtch (node format 2), which proved the harness's block 110 for the zeroconf zAPP
/// (engine/zapps/zeroconf/real_proofs.sh, zapps/zeroconf/tests/data/engine/core_image_id). No key here: a test gives the
/// key file of that build ([`with_key`]). Core's regtest has no minimum chain work and the guest pins no checkpoint, so a
/// genesis-anchored proof reaches OK: that is the point (an `--anchor` caps it at DEMO). Not in swiftsync-verify's
/// IMAGES, so the core verifier, its CLI and its .wasm do not change.
#[cfg(feature = "regtest-test")]
pub static REGTEST_IMAGE: Image = Image {
    network: "regtest",
    method_id_hex: "2d7401595eee8ae196702fafee49057eaf1ce8627bf79dd2b04d320e2533dc4f",
    // LIBZYNC 1 | regtest 8 | COINLOG 16 | TRIE 32 | UTXOHASH 64 | NODE2 128, as the engine chunk and prep states carry it
    profile: 0xf9,
    genesis_hash_hex: "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206",
    genesis_time: 1_296_688_602,
    genesis_bits: 0x207f_ffff,
    // 2^256 / (target + 1)
    genesis_work: 2,
    // Core's regtest nMinimumChainWork is 0
    min_work_hex: "0000000000000000000000000000000000000000000000000000000000000000",
    // scripts/libzync/genesis_anchor_id.py regtest, = the executed chain's anchor_id (scripts/zapp-regtest/execute.sh)
    genesis_anchor_hex: "99509fc506d022f06dfd72bb146fc894718ce8047e69e10b201e1a78d4bccf63",
    // the regtest guest pins no checkpoint (checkpoint.rs CHECKPOINT = None) and no handover
    checkpoint_height: 0,
    lineage: &[],
    engine_key: None,
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

/// `image` with its engine verifying key `vk` (engine/VERIFIER.md): accepted only as exactly the bytes `pin` names (the
/// published pin, or the reader's own build's vk.sha256; review R1-6 #1, R2-5 #1: no unpinned form). Cached, as
/// [`image_for`]: one Image per (image, key).
pub fn with_key(image: &'static Image, vk: &[u8], pin: &[u8; 32]) -> Result<&'static Image, String> {
    use sha2::Digest as _;
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    let got: [u8; 32] = sha2::Sha256::digest(vk).into();
    if *pin != got {
        return Err("the verifying key is not the image's pinned key (its SHA-256 is not the pin)".into());
    }
    static CACHE: OnceLock<Mutex<HashMap<(usize, [u8; 32]), &'static Image>>> = OnceLock::new();
    let key = (image as *const Image as usize, got);
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
    Ok(*c.entry(key).or_insert_with(|| image.with_engine_key_unchecked(vk.to_vec())))
}

/// A core proof (or .sspf bundle) verified by the core verifier, and its tip compared with `expect` ("<height>:<hash>",
/// display order, from a source the CALLER trusts): what a zAPP resting on a core proof checks first. A wrong `expect`
/// is a FAIL check; `tip_checked` says whether the tip was compared and matched in a report that is not a FAIL.
pub struct CoreCheck {
    pub report: Report,
    pub tip_checked: bool,
    /// The image the proof was verified against: the one asked for (an engine proof verifies against one image only).
    pub image: &'static Image,
}

/// `image` carries the image id and its key ([`image_for`], [`with_key`]); without them the core verifier refuses the
/// proof, with the reason.
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
    // a tip that matched in a report that FAILS (a bad receipt, a failed check) is not a checked tip (review 11-16)
    let tip_checked = tip_checked && report.verdict != Verdict::Fail;
    test_image_line(&mut report, image);
    CoreCheck { report, tip_checked, image }
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
    const V: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../swiftsync-verify/vectors/");

    fn rd(f: &str) -> Vec<u8> {
        std::fs::read(format!("{V}{f}")).unwrap()
    }

    /// The engine chain's mainnet 1..200 image, with its key.
    fn img() -> &'static Image {
        let id = String::from_utf8(rd("mainnet_1_200_engine.image_id")).unwrap();
        with_key(image_for("mainnet", id.trim(), false, false).unwrap(), &rd("mainnet_1_200_engine.vk"), &{ use sha2::Digest as _; <[u8; 32]>::from(sha2::Sha256::digest(rd("mainnet_1_200_engine.vk"))) }).unwrap()
    }

    /// The engine chain's real mainnet 1..200 proof: DEMO (low work); its tip expected, and another block, refused.
    #[test]
    fn verify_core_and_expect() {
        let r = rd("mainnet_1_200_engine.hze");
        let ok = verify_core(&r, img(), None, 1_800_000_000, Some(&format!("200:{H200}")));
        assert_eq!((ok.report.verdict, ok.tip_checked), (Verdict::Demo, true), "{:?}", ok.report.checks);
        assert!(std::ptr::eq(ok.image, img()), "the image asked for, with its key (cached: the same Image)");
        let bad = verify_core(&r, img(), None, 1_800_000_000, Some(&format!("200:{}1", &H200[..63])));
        assert_eq!((bad.report.verdict, bad.tip_checked), (Verdict::Fail, false));
        assert_eq!(verify_core(&r, img(), None, 1_800_000_000, None).tip_checked, false);
        assert!(image_for("mainnet", "xyz", false, false).is_err() && image_for("testnet", "", false, false).is_err());
        // cached: the same request gives the same Image, so repeated calls leak nothing new
        let a = image_for("mainnet", "", true, true).unwrap();
        assert!(std::ptr::eq(a, image_for("mainnet", " ", true, true).unwrap()));
        assert!(!std::ptr::eq(a, image_for("mainnet", "", true, false).unwrap()));
        // a bundle that does not decode: FAIL, no state
        let junk = [swiftsync_verify::BUNDLE_MAGIC, &[0u8; 4][..]].concat();
        let j = verify_core(&junk, img(), None, 1_800_000_000, None);
        assert!(j.report.verdict == Verdict::Fail && j.report.state.is_none() && j.report.checks[0].1.starts_with("bundle:"));
        // the network's record alone (no engine image published yet): FAIL with the reason
        let n = verify_core(&r, image_for("mainnet", "", false, false).unwrap(), None, 1_800_000_000, None);
        assert!(n.report.checks.iter().any(|(ok, l)| !ok && l.contains("no engine image is published")), "{:?}", n.report.checks);
    }

    /// with_key: a pin that is not the key's SHA-256 is refused; the key's own pin and no pin both give the key.
    #[test]
    fn with_key_checks_the_pin() {
        let vk = rd("mainnet_1_200_engine.vk");
        let i = image_for("mainnet", &"ab".repeat(32), false, false).unwrap();
        use sha2::Digest as _;
        let pin: [u8; 32] = sha2::Sha256::digest(&vk).into();
        assert!(std::ptr::eq(with_key(i, &vk, &pin).unwrap(), with_key(i, &vk, &pin).unwrap()));
        let mut p2 = pin;
        p2[0] ^= 1;
        assert!(with_key(i, &vk, &p2).err().unwrap().contains("pinned key"));
    }

    /// A tip that matches in a FAILED report (this clock is before the chain's blocks: time-too-new) is not a checked tip.
    #[test]
    fn failed_report_tip() {
        let f = verify_core(&rd("mainnet_1_200_engine.hze"), img(), None, 0, Some(&format!("200:{H200}")));
        assert_eq!(f.report.verdict, Verdict::Fail, "{:?}", f.report.checks);
        assert!(f.report.state.is_some() && f.report.checks.iter().any(|c| c.1.starts_with("block 200 is")), "the tip WAS compared: {:?}", f.report.checks);
        assert!(!f.tip_checked);
    }

    /// The zAPP regtest harness's block hashes Bitcoin Core gave (scripts/zapp-regtest/oracle): the independent answer.
    const REGTEST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/zapp-regtest");

    /// Shipped build (no feature): "regtest" is an unknown network.
    #[cfg(not(feature = "regtest-test"))]
    #[test]
    fn shipped_build_has_no_regtest_image() {
        assert!(image_for("regtest", "", false, false).err().unwrap().contains("unknown network"));
        assert!(std::path::Path::new(REGTEST).is_dir());
    }

    /// With the feature (dev profile only): the regtest image is the harness's engine image, and every report on it starts
    /// "TEST IMAGE" (here a FAIL: no key is given, so nothing verifies).
    #[cfg(feature = "regtest-test")]
    #[test]
    fn regtest_image_is_the_engine_harness_image() {
        use swiftsync_verify::{anchor_id, genesis_anchor, hex};
        let img = image_for("regtest", "", false, false).unwrap();
        assert!(std::ptr::eq(img, &REGTEST_IMAGE));
        let zc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../zeroconf/tests/data/engine/core_image_id")).unwrap();
        assert_eq!(img.method_id_hex, zc.trim(), "the core image that proved the zeroconf fixture's block 110");
        // the pinned anchor is what the core library computes for these parameters (as genesis_anchors_are_pinned)
        assert_eq!(hex(&anchor_id(&genesis_anchor(img))), img.genesis_anchor_hex);
        let c = verify_core(b"HZE1", img, None, 1_800_000_000, None);
        assert_eq!(c.report.checks[0].1, TEST_IMAGE_LINE);
        assert_eq!(c.report.verdict, Verdict::Fail);
    }
}
