//! zapps/common's fuzz property (docs/libzync/ZAPP_TESTING.md §3.4, §5: "fuzz the --expect parser"); the stable corpus
//! replay (tests/replay.rs) runs the same code. A property that fails panics.
//!
//! `identity`: NUL-separated CLI arguments taken by `Identity::take` as a zAPP CLI takes them; each identity source is
//! given at most once and `--mempool-url` is https. With `--expect` as the only source (the others ask the network or
//! run a command, so they are never resolved here), `check_core_tip` against the real 1..200 proof is OK only for an
//! `--expect` naming block 200 by its hash (any case), and `expect_for_core` returns the `--expect` given, unchanged.

use std::sync::OnceLock;
use swiftsync_verify::{ChainState, Image, Inputs, Verdict};
use zapp_common::Identity;

pub const RCPT: &[u8] = include_bytes!("../../../../swiftsync-verify/vectors/mainnet_1_200_8888d59c.rcpt");
pub const TIP_200: &str = "000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320";

pub fn state() -> &'static ChainState {
    static S: OnceLock<ChainState> = OnceLock::new();
    S.get_or_init(|| {
        let image = Image::by_id("mainnet", "8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4").unwrap();
        let r = swiftsync_verify::verify(&Inputs { receipt: RCPT, image, anchor: None, now: 1_800_000_000 });
        assert_eq!(r.verdict, Verdict::Demo);
        r.state.unwrap()
    })
}

/// Whether `e` names block 200 by its hash: the height is the number 200 (an integer, however written), the hash the
/// tip's in any case. Anything else naming another block, or not a block at all, must not match.
fn names_block_200(e: &str) -> bool {
    let Some((h, x)) = e.split_once(':') else { return false };
    let digits = h.strip_prefix('+').unwrap_or(h);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) && digits.trim_start_matches('0') == "200"
        && x.to_lowercase() == TIP_200
}

pub fn identity(data: &[u8]) {
    let Ok(s) = std::str::from_utf8(data) else { return };
    let args: Vec<&str> = s.split('\0').collect();
    let mut id = Identity::default();
    let mut i = 0;
    let mut seen: Vec<String> = Vec::new();
    while i < args.len() {
        let flag = args[i];
        i += 1;
        let mut next = || { let v = args.get(i).map(|x| x.to_string()); if v.is_some() { i += 1; } v };
        match id.take(flag, &mut next) {
            Err(_) => return,
            Ok(true) => seen.push(if flag == "--mempool-url" { "--mempool".into() } else { flag.into() }),
            Ok(false) => {}
        }
    }
    for f in ["--expect", "--mempool", "--bitcoin-cli"] {
        assert!(seen.iter().filter(|x| *x == f).count() <= 1, "TAKE: {f} taken twice: {args:?}");
    }
    if let Some(u) = id.mempool.as_ref().filter(|u| !u.is_empty()) { assert!(u.starts_with("https://"), "TAKE: --mempool-url {u:?}"); }
    if id.needs_lookup() { return; }
    let cs = state();
    let (checks, ok) = zapp_common::check_core_tip(&id, "mainnet", cs);
    match &id.expect {
        None => assert!(!ok && checks.is_empty(), "no source given, yet {checks:?}"),
        Some(e) => assert_eq!(ok, names_block_200(e), "WRONG: --expect {e:?} gives ok={ok}: {checks:?}"),
    }
    let (e2, extra) = zapp_common::expect_for_core(&id, "mainnet", cs);
    assert!(e2 == id.expect && extra.is_empty(), "expect_for_core changed --expect with no lookup source");
}
