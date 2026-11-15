//! `zapp-common`: what every zAPP's CLI shares (docs/libzync/ZYNCAPPS.md §9), so each takes the same flags and means the
//! same by them. Not a zAPP and not part of the core verifier.
//!
//! **Chain identity.** A proof shows a valid chain; WHICH chain it is, is a block hash at a height, compared with a
//! source the user trusts. Every zAPP CLI takes the core verifier's three sources, with the core's meaning:
//!
//! - `--expect <height>:<hash>`: the block at that height (display order, as `getblockhash` prints it);
//! - `--mempool` (mempool.space: `/api/block-height/<height>`, signet `/signet/api`) or `--mempool-url <base>` (another
//!   esplora-compatible API): one GET for the block at the height the proof is about;
//! - `--bitcoin-cli "<command>"`: the user's own node (`getblockhash`, and `getblockheader` for its chainwork).
//!
//! [`Identity::resolve`] asks each source given for the block at one height, requires them to agree, and returns the
//! answer as an `--expect` string, which the zAPP then checks against its proof exactly as it checks a given
//! `--expect` (the core library's `parse_expect` / `identity_matches`). A source that fails or disagrees is a FAIL check.
//! [`check_core_tip`] does that against a verified core state (and compares the node's chainwork, as the core CLI);
//! [`expect_for_core`] / [`expect_at`] turn the sources into the `--expect` string a zAPP's own check takes.
//!
//! **The core check.** [`image_for`] and [`verify_core`]: the image a caller names and the core receipt verified by the
//! core verifier, for a zAPP (CLI, C ABI or WASM) resting on a core proof.

use std::process::Command;
use swiftsync_verify::{check_bundle, identity_matches, parse_expect, verify, Bundle, ChainState, Image, Inputs, Report, Verdict, BUNDLE_MAGIC};

/// The identity flags, for a zAPP's usage line.
pub const IDENTITY_USAGE: &str = "[--expect <height>:<hash>] [--mempool | --mempool-url <base>] [--bitcoin-cli \"<command>\"]";

/// The chain-identity sources a CLI was given.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identity {
    pub expect: Option<String>,
    /// `Some("")`: mempool.space for the network; `Some(base)`: that esplora-compatible API.
    pub mempool: Option<String>,
    pub bitcoin_cli: Option<String>,
}

/// What [`Identity::resolve`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    /// `<height>:<hash>` for the zAPP to check against its proof: the given `--expect`, else the sources' answer.
    pub expect: Option<String>,
    /// One line per source asked; a source that failed or disagrees with another is `false`.
    pub checks: Vec<(bool, String)>,
    /// `--bitcoin-cli`: the node's chainwork at that block (display order hex, as `getblockheader` prints it).
    pub node_chainwork: Option<String>,
}

impl Identity {
    /// Takes `flag` if it is an identity flag, its value (if it has one) from `next`. `Ok(false)`: not one of them.
    pub fn take(&mut self, flag: &str, next: &mut dyn FnMut() -> Option<String>) -> Result<bool, String> {
        let mut val = || next().ok_or_else(|| format!("{flag} needs a value"));
        match flag {
            "--expect" => self.expect = Some(val()?),
            "--mempool" => self.mempool = Some(String::new()),
            "--mempool-url" => self.mempool = Some(val()?),
            "--bitcoin-cli" => self.bitcoin_cli = Some(val()?),
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// No source given: the verdict says the chain identity was NOT checked.
    pub fn is_empty(&self) -> bool {
        self.expect.is_none() && self.mempool.is_none() && self.bitcoin_cli.is_none()
    }

    /// A source that needs the proof's height before it can be asked (all but `--expect`).
    pub fn needs_lookup(&self) -> bool {
        self.mempool.is_some() || self.bitcoin_cli.is_some()
    }

    /// Asks every source given for the block at `height` of `network` ("mainnet" / "signet").
    pub fn resolve(&self, network: &str, height: u32) -> Resolved {
        let mut r = Resolved::default();
        // (what said it, the height it is about, the hash)
        let mut answers: Vec<(String, u32, String)> = Vec::new();
        if let Some(e) = &self.expect {
            match parse_expect(e) {
                Some((h, x)) => answers.push(("--expect".into(), h, x)),
                None => r.checks.push((false, format!("--expect {e:?}: want <height>:<64 hex digits>"))),
            }
        }
        if let Some(base) = &self.mempool {
            let base = if !base.is_empty() { base.trim_end_matches('/').to_string() }
                       else if network == "signet" { "https://mempool.space/signet/api".into() } else { "https://mempool.space/api".into() };
            let url = format!("{base}/block-height/{height}");
            match Command::new("curl").args(["-sf", "--max-time", "30", &url]).output() {
                Ok(o) if o.status.success() => {
                    let got = String::from_utf8_lossy(&o.stdout).trim().to_lowercase();
                    if is_hash(&got) {
                        r.checks.push((true, format!("{url} says block {height} is {got}")));
                        answers.push((url, height, got));
                    } else {
                        r.checks.push((false, format!("{url}: not a block hash ({:?}): chain identity not checked", got.chars().take(80).collect::<String>())));
                    }
                }
                Ok(o) => r.checks.push((false, format!("{url}: request failed ({}): chain identity not checked", o.status))),
                Err(e) => r.checks.push((false, format!("curl {url}: {e}"))),
            }
        }
        if let Some(cmd) = &self.bitcoin_cli {
            match node(cmd, &["getblockhash", &height.to_string()]) {
                Ok(hash) if is_hash(&hash.to_lowercase()) => {
                    let hash = hash.to_lowercase();
                    // the node's chainwork is compared with the proven one; a node that cannot say is a FAIL, not a
                    // silently skipped comparison
                    match node(cmd, &["getblockheader", &hash]) {
                        Ok(j) => match serde_json::from_str::<serde_json::Value>(&j).ok().and_then(|v| v["chainwork"].as_str().map(|w| w.to_lowercase())) {
                            Some(w) if is_hash(&w) => r.node_chainwork = Some(w),
                            _ => r.checks.push((false, format!("your node's getblockheader {hash} has no chainwork: the proven chainwork is not compared"))),
                        },
                        Err(e) => r.checks.push((false, format!("your node's getblockheader {hash} failed ({e}): the proven chainwork is not compared"))),
                    }
                    r.checks.push((true, format!("your node's block {height} is {hash}")));
                    answers.push(("your node".into(), height, hash));
                }
                Ok(other) => r.checks.push((false, format!("your node's chain: getblockhash {height} gave {other:?}"))),
                Err(e) => r.checks.push((false, format!("your node's chain ({e})"))),
            }
        }
        // Every answer about the same block must name the same hash; --expect at another height than the lookups'
        // would leave one of them unchecked, so it is refused rather than half used.
        if let Some((s0, h0, x0)) = answers.first().cloned() {
            for (s, h, x) in answers.iter().skip(1) {
                if *h != h0 {
                    r.checks.push((false, format!("{s0} is about block {h0}, {s} about block {h}: give sources about the same block")));
                } else if *x != x0 {
                    r.checks.push((false, format!("{s} says block {h} is {x}, {s0} says {x0}: the sources disagree")));
                }
            }
            r.expect = Some(format!("{h0}:{x0}"));
        }
        r
    }
}

/// Chain identity for a zAPP resting on a verified CORE state `cs`, as the core CLI checks it: every source asked for
/// the block at the proof's height and compared with the proven tip; `--bitcoin-cli` also compares the node's chainwork
/// with the proven one. Returns the check lines and whether the tip was compared and every check held (no source
/// given: no lines, `false`, and the zAPP says the identity was NOT checked).
pub fn check_core_tip(id: &Identity, network: &str, cs: &ChainState) -> (Vec<(bool, String)>, bool) {
    if id.is_empty() {
        return (Vec::new(), false);
    }
    let r = id.resolve(network, cs.height);
    let mut checks = r.checks;
    let tip = display_hex(&cs.tip_hash);
    let mut matched = false;
    if let Some(e) = &r.expect {
        match parse_expect(e) {
            Some((h, x)) => {
                matched = identity_matches(h, &x, cs.height, &cs.tip_hash);
                checks.push((matched, format!("block {h} is {x}; the proof's tip is block {} {tip}{}", cs.height,
                    if h != cs.height { " (only the tip height can be checked from the receipt alone)" } else { "" })));
            }
            None => checks.push((false, format!("--expect {e:?}: want <height>:<64 hex digits>"))),
        }
    }
    if let Some(w) = &r.node_chainwork {
        let pw = display_hex(&cs.cum_work);
        checks.push((*w == pw, format!("your node's chainwork at it equals the proven cum_work {pw} (node: {w})")));
    }
    let ok = matched && checks.iter().all(|c| c.0);
    (checks, ok)
}

/// For a zAPP whose own check compares a core tip with an `expect` string (as its C ABI and WASM take it): the sources'
/// answer for block `cs.height` as that string (the given `--expect` unchanged when there is no other source), and the
/// source lines to add to its checks (with `--bitcoin-cli`, the node's chainwork compared with the proven one).
pub fn expect_for_core(id: &Identity, network: &str, cs: &ChainState) -> (Option<String>, Vec<(bool, String)>) {
    if !id.needs_lookup() {
        return (id.expect.clone(), Vec::new());
    }
    let r = id.resolve(network, cs.height);
    let mut checks = r.checks;
    if let Some(w) = &r.node_chainwork {
        let pw = display_hex(&cs.cum_work);
        checks.push((*w == pw, format!("your node's chainwork at it equals the proven cum_work {pw} (node: {w})")));
    }
    (r.expect, checks)
}

/// For a zAPP that compares a block at `height` (not a core tip) with an `expect` string: the sources' answer as that
/// string (the given `--expect` unchanged when there is no other source), and the source lines to add to its checks.
pub fn expect_at(id: &Identity, network: &str, height: u32) -> (Option<String>, Vec<(bool, String)>) {
    if !id.needs_lookup() {
        return (id.expect.clone(), Vec::new());
    }
    let r = id.resolve(network, height);
    (r.expect, r.checks)
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
            Err(e) => Report { checks: vec![(false, format!("bundle: {e}"))], low_work: false, state: None, verdict: Verdict::Fail,
                               undo_from: swiftsync_verify::pin::NO_UNDO },
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

/// A hash in display order (as `getblockhash` prints it), from internal byte order.
pub fn display_hex(h: &[u8; 32]) -> String {
    h.iter().rev().map(|b| format!("{b:02x}")).collect()
}

fn is_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// One `bitcoin-cli` call: `cmd` is the command line (program and its own flags), `extra` the RPC and its arguments.
fn node(cmd: &str, extra: &[&str]) -> Result<String, String> {
    let mut parts = cmd.split_whitespace();
    let prog = parts.next().ok_or("empty --bitcoin-cli")?;
    let out = Command::new(prog).args(parts).args(extra).output().map_err(|e| format!("{prog}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{cmd} {}: {}", extra.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    const H200: &str = "000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320";

    fn take_all(args: &[&str]) -> Result<Identity, String> {
        let mut id = Identity::default();
        let mut it = args.iter().map(|s| s.to_string());
        while let Some(f) = it.next() {
            if !id.take(&f, &mut || it.next())? { return Err(format!("not an identity flag: {f}")); }
        }
        Ok(id)
    }

    #[test]
    fn flags_parse_as_the_core_cli_does() {
        let id = take_all(&["--expect", &format!("200:{H200}"), "--mempool", "--bitcoin-cli", "bitcoin-cli -signet"]).unwrap();
        assert_eq!(id.expect.as_deref(), Some(format!("200:{H200}").as_str()));
        assert_eq!(id.mempool.as_deref(), Some(""));
        assert_eq!(id.bitcoin_cli.as_deref(), Some("bitcoin-cli -signet"));
        assert_eq!(take_all(&["--mempool-url", "http://x/api"]).unwrap().mempool.as_deref(), Some("http://x/api"));
        assert!(take_all(&["--expect"]).is_err(), "a flag without its value");
        assert!(Identity::default().is_empty() && !Identity::default().needs_lookup());
        let mut id = Identity::default();
        assert_eq!(id.take("--image", &mut || None), Ok(false));
    }

    #[test]
    fn expect_alone_is_passed_through() {
        let id = take_all(&["--expect", &format!("200:{H200}")]).unwrap();
        let r = id.resolve("mainnet", 200);
        assert_eq!(r.expect.as_deref(), Some(format!("200:{}", H200).as_str()));
        assert!(r.checks.is_empty());
        let r = take_all(&["--expect", "200:xyz"]).unwrap().resolve("mainnet", 200);
        assert_eq!((r.expect, r.checks.len(), r.checks[0].0), (None, 1, false));
    }

    /// A fake node (a shell script) answers getblockhash / getblockheader; a disagreeing --expect is refused.
    #[test]
    fn node_answers_and_disagreement() {
        let d = std::env::temp_dir().join(format!("zapp-common-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let cli = d.join("cli");
        std::fs::write(&cli, format!("#!/bin/sh\ncase \"$1\" in getblockhash) echo {H200};; getblockheader) echo '{{\"chainwork\": \"00ab\"}}';; esac\n")).unwrap();
        #[cfg(unix)]
        { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap(); }
        let c = cli.to_str().unwrap();
        let r = take_all(&["--bitcoin-cli", c]).unwrap().resolve("mainnet", 200);
        assert_eq!(r.expect.as_deref(), Some(format!("200:{H200}").as_str()));
        assert_eq!(r.node_chainwork.as_deref(), Some("00ab"));
        assert!(r.checks.iter().all(|c| c.0), "{:?}", r.checks);
        let other = format!("200:{}1", &H200[..63]);
        let r = take_all(&["--bitcoin-cli", c, "--expect", &other]).unwrap().resolve("mainnet", 200);
        assert!(r.checks.iter().any(|c| !c.0 && c.1.contains("disagree")), "{:?}", r.checks);
        let r = take_all(&["--bitcoin-cli", c, "--expect", &format!("199:{H200}")]).unwrap().resolve("mainnet", 200);
        assert!(r.checks.iter().any(|c| !c.0 && c.1.contains("same block")), "{:?}", r.checks);
        // a node whose getblockheader has no chainwork, or fails: a FAIL line, never a silent skip
        let cli2 = d.join("cli2");
        std::fs::write(&cli2, format!("#!/bin/sh\ncase \"$1\" in getblockhash) echo {H200};; getblockheader) echo '{{}}';; esac\n")).unwrap();
        let cli3 = d.join("cli3");
        std::fs::write(&cli3, format!("#!/bin/sh\ncase \"$1\" in getblockhash) echo {H200};; *) exit 1;; esac\n")).unwrap();
        #[cfg(unix)]
        for f in [&cli2, &cli3] { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o755)).unwrap(); }
        for (f, want) in [(&cli2, "no chainwork"), (&cli3, "failed")] {
            let r = take_all(&["--bitcoin-cli", f.to_str().unwrap()]).unwrap().resolve("mainnet", 200);
            assert!(r.node_chainwork.is_none() && r.checks.iter().any(|c| !c.0 && c.1.contains(want)), "{:?}", r.checks);
        }
        let r = take_all(&["--bitcoin-cli", "/nonexistent/cli"]).unwrap().resolve("mainnet", 200);
        assert!(r.expect.is_none() && r.checks.iter().any(|c| !c.0));
        std::fs::remove_dir_all(&d).unwrap();
    }

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
        let cs = ok.report.state.unwrap();
        let id = Identity { expect: Some(format!("200:{H200}")), ..Default::default() };
        assert!(check_core_tip(&id, "mainnet", &cs).1);
        assert!(!check_core_tip(&Identity { expect: Some(format!("199:{H200}")), ..Default::default() }, "mainnet", &cs).1);
        assert_eq!(check_core_tip(&Identity::default(), "mainnet", &cs), (Vec::new(), false));
        assert_eq!(expect_for_core(&id, "mainnet", &cs), (id.expect.clone(), Vec::new()));
        assert_eq!(expect_at(&id, "mainnet", 7), (id.expect.clone(), Vec::new()));
        // block 200's chainwork: 201 blocks of difficulty 1, 0x100010001 each
        assert_eq!(display_hex(&cs.cum_work), format!("{:064x}", 201u128 * 0x1_0001_0001));
    }

    #[test]
    fn display_order() {
        let mut h = [0u8; 32];
        h[0] = 0xab;
        assert!(display_hex(&h).ends_with("ab") && display_hex(&h).starts_with("00"));
    }
}
