//! The chain-identity checks' verdict paths (docs/libzync/ZAPP_TESTING.md §3.5, the cargo-mutants pass of 17.11.2026):
//! what decides whether a zAPP's tip counts as compared with a source the caller trusts. Fake nodes are shell scripts
//! (run as `sh <file>`, never exec'd: see the crate's own node tests), the state is the engine chain's real proof of
//! mainnet 1..200 (swiftsync-verify/vectors/mainnet_1_200_engine.*).

use swiftsync_verify::{verify, ChainState, Image, Inputs};
use zapp_common::*;

const H200: &str = "000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320";

fn state() -> ChainState {
    let rd = |f: &str| std::fs::read(format!("{}/../../swiftsync-verify/vectors/{f}", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let id = String::from_utf8(rd("mainnet_1_200_engine.image_id")).unwrap();
    let img = Image::by_network("mainnet").unwrap().with_method_id(id.trim()).unwrap().with_engine_key_unchecked(rd("mainnet_1_200_engine.vk"));
    verify(&Inputs { receipt: &rd("mainnet_1_200_engine.hze"), image: img, anchor: None, now: 1_800_000_000 }).state.unwrap()
}

/// Block 200's chainwork as getblockheader prints it (201 blocks of difficulty 1, 0x100010001 each).
fn work_200() -> String {
    format!("{:064x}", 201u128 * 0x1_0001_0001)
}

/// A fake `bitcoin-cli` answering getblockhash with `hash`, getblockheader with `chainwork`, getblockcount with `count`.
struct Node(std::path::PathBuf);

impl Node {
    fn new(tag: &str, hash: &str, chainwork: &str, count: &str) -> Node {
        let d = std::env::temp_dir().join(format!("zapp-common-verdict-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("cli"), format!("#!/bin/sh\ncase \"$1\" in getblockhash) echo '{hash}';; \
            getblockheader) echo '{{\"chainwork\": \"{chainwork}\"}}';; getblockcount) echo '{count}';; *) exit 1;; esac\n")).unwrap();
        Node(d)
    }
    fn cmd(&self) -> String {
        format!("sh {}", self.0.join("cli").to_str().unwrap())
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn node_id(n: &Node, expect: Option<String>) -> Identity {
    Identity { expect, bitcoin_cli: Some(n.cmd()), ..Default::default() }
}

/// check_core_tip with a node: the tip AND the node's chainwork must both match the proof's for `ok`.
#[test]
fn core_tip_needs_the_nodes_chainwork_too() {
    let cs = state();
    let good = Node::new("good", H200, &work_200(), "200");
    let (checks, ok) = check_core_tip(&node_id(&good, None), "mainnet", &cs);
    assert!(ok && checks.iter().all(|c| c.0), "{checks:?}");
    // the right block, another chainwork: not ok, and the chainwork line is the failed one
    let other = Node::new("work", H200, &format!("{:064x}", 202u128 * 0x1_0001_0001), "200");
    let (checks, ok) = check_core_tip(&node_id(&other, None), "mainnet", &cs);
    assert!(!ok, "{checks:?}");
    assert!(checks.iter().any(|c| !c.0 && c.1.contains("chainwork")), "{checks:?}");
}

/// expect_for_core with a node: the node is asked (not the given --expect passed through), so a node that disagrees
/// with --expect gives no expect and a failed line, and its chainwork is compared with the proof's.
#[test]
fn expect_for_core_asks_the_node() {
    let cs = state();
    let good = Node::new("efc", H200, &work_200(), "200");
    let (x, ch) = expect_for_core(&node_id(&good, None), "mainnet", &cs);
    assert_eq!(x.as_deref(), Some(format!("200:{H200}").as_str()));
    assert!(ch.iter().all(|c| c.0) && ch.iter().any(|c| c.1.contains("chainwork")), "{ch:?}");
    let other = format!("200:{}1", &H200[..63]);
    let (x, ch) = expect_for_core(&node_id(&good, Some(other)), "mainnet", &cs);
    assert!(x.is_none() && ch.iter().any(|c| !c.0 && c.1.contains("disagree")), "{x:?} {ch:?}");
    let bad_work = Node::new("efcw", H200, "ff", "200");
    let (_, ch) = expect_for_core(&node_id(&bad_work, None), "mainnet", &cs);
    assert!(ch.iter().any(|c| !c.0 && c.1.contains("chainwork")), "{ch:?}");
}

/// A node's getblockhash answer must be exactly 64 hex digits: 63 digits, 64 non-hex characters, or text are refused
/// (no expect, a failed line).
#[test]
fn a_node_answer_that_is_not_a_hash_is_refused() {
    let zs = "z".repeat(64);
    for (tag, ans) in [("short", &H200[..63]), ("nonhex", zs.as_str()), ("text", "error: not ready")] {
        let n = Node::new(tag, ans, &work_200(), "200");
        let r = node_id(&n, None).resolve("mainnet", 200);
        assert!(r.expect.is_none() && r.checks.iter().any(|c| !c.0 && c.1.contains("gave")), "{tag}: {:?}", r.checks);
    }
}

/// tip_height from one node: its getblockcount; a non-number is an error.
#[test]
fn tip_height_from_one_node() {
    let n = Node::new("tip", H200, &work_200(), "975278");
    let (h, lines) = tip_height(&node_id(&n, None), "mainnet").unwrap();
    assert_eq!(h, 975_278);
    assert!(lines.len() == 1 && lines[0].contains("975278"), "{lines:?}");
    let bad = Node::new("tipbad", H200, &work_200(), "soon");
    assert!(tip_height(&node_id(&bad, None), "mainnet").is_err());
    assert!(tip_height(&Identity::default(), "mainnet").is_err(), "no source");
}

/// An explorer that cannot be reached is a failed request (curl's own exit status), not a parsed answer.
#[test]
fn an_unreachable_explorer_is_a_failed_request() {
    let id = Identity { mempool: Some("https://127.0.0.1:1/api".into()), ..Default::default() };
    let r = id.resolve("mainnet", 200);
    assert!(r.expect.is_none(), "{:?}", r.checks);
    assert!(r.checks.len() == 1 && !r.checks[0].0 && r.checks[0].1.contains("request failed"), "{:?}", r.checks);
}
