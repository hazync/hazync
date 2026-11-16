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
//! CLIs only: no zAPP library, C ABI or WASM module depends on this crate (each zAPP's CLI is its own crate, `cli/`).
//! The core receipt check a zAPP resting on a core proof makes is `zapps/corecheck`.

use std::process::Command;
use swiftsync_verify::{identity_matches, parse_expect, ChainState};

/// curl's arguments before the URL: `-q` first (no ~/.curlrc: a config file could add a proxy, -k or another URL),
/// `-g` (no {}[] globbing in the URL), fail on HTTP errors, quiet, 30 s, https only (redirects too), and `--` so the URL
/// is never read as an option.
const CURL: [&str; 10] = ["-q", "-g", "-sf", "--max-time", "30", "--proto", "=https", "--proto-redir", "=https", "--"];

/// How long one `--bitcoin-cli` call may take before it is a failed check (a node that hangs must not hang the zAPP).
const NODE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

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
            // each source once: a repeated flag would silently keep whichever came last
            "--expect" if self.expect.is_some() => return Err("--expect: give it once".into()),
            "--bitcoin-cli" if self.bitcoin_cli.is_some() => return Err("--bitcoin-cli: give it once".into()),
            "--expect" => self.expect = Some(val()?),
            // one explorer: --mempool and --mempool-url together would silently keep whichever came last
            "--mempool" | "--mempool-url" if self.mempool.is_some() => return Err("--mempool and --mempool-url: give one of them, once".into()),
            "--mempool" => self.mempool = Some(String::new()),
            // https only: the answer names the network's block, so it must come from the server asked, unaltered
            "--mempool-url" => {
                let u = val()?;
                if !u.starts_with("https://") { return Err(format!("--mempool-url {u:?}: an https:// URL")); }
                self.mempool = Some(u)
            }
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
            // `--` ends curl's options (a base starting with '-' is a URL, not a flag); https only, redirects included
            match Command::new("curl").args(CURL).arg(&url).output() {
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
                    r.checks.push((true, format!("your node's block {height} is {hash}")));
                    // the node's chainwork is compared with the proven one; a node that cannot say is a FAIL, not a
                    // silently skipped comparison. Any hex up to 64 digits, as getblockheader prints it, padded to 64.
                    match node(cmd, &["getblockheader", &hash]) {
                        Err(e) => r.checks.push((false, format!("your node's getblockheader {hash} failed ({e}): the proven chainwork is not compared"))),
                        Ok(j) => match serde_json::from_str::<serde_json::Value>(&j).ok().and_then(|v| v["chainwork"].as_str().map(str::to_string))
                            .filter(|w| !w.is_empty() && w.len() <= 64 && w.bytes().all(|b| b.is_ascii_hexdigit())) {
                            Some(w) => r.node_chainwork = Some(format!("{:0>64}", w.to_lowercase())),
                            None => r.checks.push((false, format!("your node's getblockheader {hash} has no chainwork: the proven chainwork is not compared"))),
                        },
                    }
                    answers.push(("your node".into(), height, hash));
                }
                Ok(other) => r.checks.push((false, format!("your node's chain: getblockhash {height} gave {other:?}"))),
                Err(e) => r.checks.push((false, format!("your node's chain ({e})"))),
            }
        }
        // Every answer about the same block must name the same hash; --expect at another height than the lookups'
        // would leave one of them unchecked, so it is refused rather than half used.
        // Sources that disagree give NO answer: returning the first one's would let a caller that uses only `expect`
        // (expect_for_core, expect_at) check against a hash another source contradicts.
        if let Some((s0, h0, x0)) = answers.first().cloned() {
            let mut agree = true;
            for (s, h, x) in answers.iter().skip(1) {
                if *h != h0 {
                    r.checks.push((false, format!("{s0} is about block {h0}, {s} about block {h}: give sources about the same block")));
                    agree = false;
                } else if *x != x0 {
                    r.checks.push((false, format!("{s} says block {h} is {x}, {s0} says {x0}: the sources disagree")));
                    agree = false;
                }
            }
            if agree {
                r.expect = Some(format!("{h0}:{x0}"));
            }
        }
        r
    }
}

/// The network's tip height from the sources that can give one (`--bitcoin-cli`: `getblockcount`; `--mempool`:
/// `/blocks/tip/height`), for a recency check (`--max-age`). Every source asked must answer and agree; `--expect` gives
/// no tip height. Err: none given, one failed, or they disagree.
pub fn tip_height(id: &Identity, network: &str) -> Result<(u32, Vec<String>), String> {
    let mut got: Vec<(String, u32)> = Vec::new();
    if let Some(base) = &id.mempool {
        let base = if !base.is_empty() { base.trim_end_matches('/').to_string() }
                   else if network == "signet" { "https://mempool.space/signet/api".into() } else { "https://mempool.space/api".into() };
        let url = format!("{base}/blocks/tip/height");
        let o = Command::new("curl").args(CURL).arg(&url).output().map_err(|e| format!("curl {url}: {e}"))?;
        let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
        let h = t.parse::<u32>().ok().filter(|_| o.status.success()).ok_or_else(|| format!("{url}: no tip height ({:?})", t.chars().take(40).collect::<String>()))?;
        got.push((url, h));
    }
    if let Some(cmd) = &id.bitcoin_cli {
        let t = node(cmd, &["getblockcount"])?;
        got.push(("your node".into(), t.parse().map_err(|_| format!("your node's getblockcount gave {t:?}"))?));
    }
    let (s0, h0) = got.first().cloned().ok_or("a recency check needs the network's tip: give --mempool, --mempool-url or --bitcoin-cli")?;
    if let Some((s, h)) = got.iter().find(|(_, h)| *h != h0) {
        // a tip one source has seen and another not yet is a real race, but the check cannot pick one
        return Err(format!("{s} says the tip is {h}, {s0} says {h0}: ask again, or use one source"));
    }
    Ok((h0, got.into_iter().map(|(s, h)| format!("{s}: the network's tip is block {h}")).collect()))
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
/// `work`: the proof's chainwork at that block (little-endian, as the journals hold it), when the proof gives one; with
/// `--bitcoin-cli` the node's chainwork there must equal it, as [`check_core_tip`] requires of a core tip.
pub fn expect_at(id: &Identity, network: &str, height: u32, work: Option<&[u8; 32]>) -> (Option<String>, Vec<(bool, String)>) {
    if !id.needs_lookup() {
        return (id.expect.clone(), Vec::new());
    }
    let r = id.resolve(network, height);
    let mut checks = r.checks;
    if let (Some(w), Some(work)) = (&r.node_chainwork, work) {
        let pw = display_hex(work);
        checks.push((*w == pw, format!("your node's chainwork at block {height} equals the proven chainwork {pw} (node: {w})")));
    }
    (r.expect, checks)
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
    node_within(cmd, extra, NODE_TIMEOUT)
}

/// [`node`] with its time limit: past it the call is killed and is an error. Its output is read on threads of their own,
/// which are left behind on a timeout (a grandchild the command started may keep the pipes open; the zAPP does not wait).
fn node_within(cmd: &str, extra: &[&str], limit: std::time::Duration) -> Result<String, String> {
    use std::io::Read;
    use std::process::Stdio;
    let mut parts = cmd.split_whitespace();
    let prog = parts.next().ok_or("empty --bitcoin-cli")?;
    let mut child = Command::new(prog).args(parts).args(extra).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|e| format!("{prog}: {e}"))?;
    let reader = |mut p: Box<dyn Read + Send>| std::thread::spawn(move || { let mut v = Vec::new(); let _ = p.read_to_end(&mut v); v });
    let (o, e) = (reader(Box::new(child.stdout.take().unwrap())), reader(Box::new(child.stderr.take().unwrap())));
    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait().map_err(|e| format!("{prog}: {e}"))? {
            Some(s) => break s,
            None if start.elapsed() >= limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{cmd} {}: no answer within {} s", extra.join(" "), limit.as_secs()));
            }
            None => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    };
    let out = std::process::Output { status, stdout: o.join().unwrap_or_default(), stderr: e.join().unwrap_or_default() };
    if !out.status.success() {
        // {:?}: the node's stderr escaped, so it cannot forge a line of this tool's own output
        return Err(format!("{cmd} {}: {:?}", extra.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    const H200: &str = "000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320";

    /// A node that does not answer is a failed call within the limit, not a hang (review 11-16).
    #[test]
    fn a_hanging_node_times_out() {
        let t = std::time::Instant::now();
        let e = node_within("sleep", &["30"], std::time::Duration::from_millis(300)).unwrap_err();
        assert!(e.contains("no answer within") && t.elapsed() < std::time::Duration::from_secs(5), "{e} after {:?}", t.elapsed());
        assert_eq!(node_within("echo", &["hi"], std::time::Duration::from_secs(5)).unwrap(), "hi");
        assert!(node_within("false", &[], std::time::Duration::from_secs(5)).is_err());
    }

    fn take_all(args: &[&str]) -> Result<Identity, String> {
        let mut id = Identity::default();
        let mut it = args.iter().map(|s| s.to_string());
        while let Some(f) = it.next() {
            if !id.take(&f, &mut || it.next())? { return Err(format!("not an identity flag: {f}")); }
        }
        Ok(id)
    }

    #[test]
    fn one_explorer_only() {
        assert!(take_all(&["--mempool", "--mempool-url", "https://x"]).unwrap_err().contains("give one"));
        assert!(take_all(&["--mempool-url", "https://x", "--mempool"]).is_err());
        assert!(take_all(&["--mempool", "--mempool"]).is_err());
        // review 11-16: --expect and --bitcoin-cli once each, as the explorer
        assert!(take_all(&["--expect", "1:a", "--expect", "2:b"]).unwrap_err().contains("once"));
        assert!(take_all(&["--bitcoin-cli", "a", "--bitcoin-cli", "b"]).unwrap_err().contains("once"));
        assert_eq!(take_all(&["--mempool-url", "https://x"]).unwrap().mempool.as_deref(), Some("https://x"));
    }

    #[test]
    fn flags_parse_as_the_core_cli_does() {
        let id = take_all(&["--expect", &format!("200:{H200}"), "--mempool", "--bitcoin-cli", "bitcoin-cli -signet"]).unwrap();
        assert_eq!(id.expect.as_deref(), Some(format!("200:{H200}").as_str()));
        assert_eq!(id.mempool.as_deref(), Some(""));
        assert_eq!(id.bitcoin_cli.as_deref(), Some("bitcoin-cli -signet"));
        assert_eq!(take_all(&["--mempool-url", "https://x/api"]).unwrap().mempool.as_deref(), Some("https://x/api"));
        assert!(take_all(&["--mempool-url", "http://x/api"]).is_err(), "not https");
        assert!(take_all(&["--mempool-url", "-K/etc/passwd"]).is_err(), "an option, not a URL");
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
        std::fs::write(&cli, format!("#!/bin/sh\ncase \"$1\" in getblockhash) echo {H200};; getblockheader) echo '{{\"chainwork\": \"00000000000000000000000000000000000000000000000000000000000000ab\"}}';; esac\n")).unwrap();
        #[cfg(unix)]
        { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap(); }
        // each fake node runs as `sh <file>`, never exec'd itself: tests run on parallel threads, and a file another
        // thread's fork still holds open for writing cannot be exec'd (ETXTBSY, "Text file busy": a flaky FAIL)
        let c = &format!("sh {}", cli.to_str().unwrap());
        let r = take_all(&["--bitcoin-cli", c]).unwrap().resolve("mainnet", 200);
        assert_eq!(r.expect.as_deref(), Some(format!("200:{H200}").as_str()));
        assert_eq!(r.node_chainwork.as_deref(), Some("00000000000000000000000000000000000000000000000000000000000000ab"));
        assert!(r.checks.iter().all(|c| c.0), "{:?}", r.checks);
        let other = format!("200:{}1", &H200[..63]);
        let r = take_all(&["--bitcoin-cli", c, "--expect", &other]).unwrap().resolve("mainnet", 200);
        assert!(r.checks.iter().any(|c| !c.0 && c.1.contains("disagree")), "{:?}", r.checks);
        assert!(r.expect.is_none(), "disagreeing sources give no answer");
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
            let r = take_all(&["--bitcoin-cli", &format!("sh {}", f.to_str().unwrap())]).unwrap().resolve("mainnet", 200);
            assert!(r.node_chainwork.is_none() && r.checks.iter().any(|c| !c.0 && c.1.contains(want)), "{:?}", r.checks);
        }
        let r = take_all(&["--bitcoin-cli", "/nonexistent/cli"]).unwrap().resolve("mainnet", 200);
        assert!(r.expect.is_none() && r.checks.iter().any(|c| !c.0));
        std::fs::remove_dir_all(&d).unwrap();
    }


    /// A short chainwork is padded as getblockheader's number; a malformed one is a FAIL; expect_at compares the
    /// node's chainwork with the proof's when the proof gives one, and not when it does not.
    #[test]
    fn node_chainwork_and_expect_at() {
        let d = std::env::temp_dir().join(format!("zapp-common-test-cw-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let mk = |name: &str, hdr: &str| -> String {
            let p = d.join(name);
            std::fs::write(&p, format!("#!/bin/sh\ncase \"$1\" in getblockhash) echo {H200};; getblockheader) {hdr};; esac\n")).unwrap();
            #[cfg(unix)]
            { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap(); }
            format!("sh {}", p.to_str().unwrap()) // run by sh, not exec'd: see node_answers_and_disagreement
        };
        let c = mk("short", "echo '{\"chainwork\": \"00ab\"}'");
        let r = take_all(&["--bitcoin-cli", &c]).unwrap().resolve("mainnet", 200);
        assert_eq!(r.node_chainwork.as_deref(), Some(format!("{:0>64}", "ab").as_str()));
        for (name, hdr) in [("junk", "echo 'not json'"), ("badwork", "echo '{\"chainwork\": \"zz\"}'")] {
            let r = take_all(&["--bitcoin-cli", &mk(name, hdr)]).unwrap().resolve("mainnet", 200);
            assert!(r.node_chainwork.is_none() && r.checks.iter().any(|c| !c.0 && c.1.contains("chainwork")), "{name}: {:?}", r.checks);
        }
        let id = take_all(&["--bitcoin-cli", &c]).unwrap();
        let mut w = [0u8; 32];
        w[0] = 0xab;
        let (x, ch) = expect_at(&id, "mainnet", 200, Some(&w));
        assert_eq!(x.as_deref(), Some(format!("200:{H200}").as_str()));
        assert!(ch.iter().all(|c| c.0) && ch.iter().any(|c| c.1.contains("proven chainwork")), "{ch:?}");
        w[0] = 0xac;
        assert!(expect_at(&id, "mainnet", 200, Some(&w)).1.iter().any(|c| !c.0));
        assert!(expect_at(&id, "mainnet", 200, None).1.iter().all(|c| c.0));
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// Against the real mainnet 1..200 proof's state (8888d59c, verified by the core verifier): its tip expected holds,
    /// another height does not, no source gives no check.
    #[test]
    fn core_tip_against_a_real_state() {
        use swiftsync_verify::{verify, Image, Inputs};
        let r = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../swiftsync-verify/vectors/mainnet_1_200_8888d59c.rcpt")).unwrap();
        let img = Image::by_id("mainnet", "8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4").unwrap();
        let cs = verify(&Inputs { receipt: &r, image: img, anchor: None, now: 1_800_000_000 }).state.unwrap();
        let id = Identity { expect: Some(format!("200:{H200}")), ..Default::default() };
        assert!(check_core_tip(&id, "mainnet", &cs).1);
        assert!(!check_core_tip(&Identity { expect: Some(format!("199:{H200}")), ..Default::default() }, "mainnet", &cs).1);
        assert_eq!(check_core_tip(&Identity::default(), "mainnet", &cs), (Vec::new(), false));
        assert_eq!(expect_for_core(&id, "mainnet", &cs), (id.expect.clone(), Vec::new()));
        assert_eq!(expect_at(&id, "mainnet", 7, None), (id.expect.clone(), Vec::new()));
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
