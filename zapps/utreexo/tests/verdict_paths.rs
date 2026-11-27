//! zapp-utreexo's verdict paths (docs/libzync/ZAPP_TESTING.md §3.5, the cargo-mutants pass of 17.11.2026).

use zapp_utreexo::*;

// ---- The verdict as a pure function of the statement (judge_verified_statement): every branch from statements built here,
// OK included, which no real receipt reaches (they are all below minimum work; review and cargo-mutants, 18.11.2026).

use swiftsync_verify::{hex32, words, Image, Verdict, MAX_FUTURE_BLOCK_TIME};

const NOW: u64 = 1_800_000_000;
/// Not the built-in (unpinned) id: a statement under it may reach OK.
const ID: [u8; 32] = [7; 32];

fn disp(b: &[u8; 32]) -> String {
    b.iter().rev().map(|x| format!("{x:02x}")).collect()
}

fn img(net: &str) -> &'static Image {
    Image::by_network(net).unwrap()
}

/// The network's minimum work, little-endian as the journal holds it.
fn min_work(i: &Image) -> [u8; 32] {
    let mut w = hex32(i.min_work_hex).unwrap();
    w.reverse();
    w
}

/// A statement that is OK for `net` under [`ID`] with H = T = the checkpoint, the work exactly Core's minimum.
fn good(net: &str) -> UtreexoStatement {
    let i = img(net);
    let h = i.checkpoint_height;
    UtreexoStatement {
        kind: KIND_UTREEXO,
        self_id: words(&ID),
        network: if net == "signet" { 4 } else { 0 },
        height: h,
        to: h,
        runs: 1,
        done: 1,
        block_hash: [0x11; 32],
        leaves: 3,
        roots: vec![[1; 32], [2; 32]],
        n_live: 3,
        core_id: words(&i.method_id()),
        anchor_id: hex32(i.genesis_anchor_hex).unwrap(),
        core_height: h,
        tip_hash: [0x11; 32],
        cum_work: min_work(i),
        max_time: NOW as u32,
        profile: i.profile,
        ..Default::default()
    }
}

fn at(z: &UtreexoStatement) -> (u32, String) {
    (z.height, disp(&z.block_hash))
}

/// The verdict for `z` checked as `net`, with block H named as the statement has it and the given tip.
fn v(net: &str, z: UtreexoStatement, tip: Option<(u32, String)>) -> Verdict {
    let a = at(&z);
    let r = judge_verified_statement(z, &ID, img(net), NOW, Some((a.0, a.1.as_str())), tip.as_ref().map(|(h, x)| (*h, x.as_str())));
    r.verdict
}

#[test]
fn ok_is_reachable_on_both_networks() {
    for net in NETWORKS {
        let r = judge_verified_statement(good(net), &ID, img(net), NOW, Some((at(&good(net)).0, at(&good(net)).1.as_str())), None);
        assert_eq!(r.verdict, Verdict::Ok, "{net}: {:?}", r.checks);
        assert!(r.identity_checked && r.image_source == Some(IMAGE_CALLER) && r.network == net);
        assert!(r.checks.iter().any(|c| c.1.contains("the one you named")), "{:?}", r.checks);
    }
}

#[test]
fn a_proof_of_another_network_fails() {
    assert_eq!(v("mainnet", good("signet"), None), Verdict::Fail);
    assert_eq!(v("signet", good("mainnet"), None), Verdict::Fail);
    // the network word alone, or the profile alone, of the other network
    let mut z = good("mainnet");
    z.network = 4;
    assert_eq!(v("mainnet", z, None), Verdict::Fail);
    let mut z = good("signet");
    z.network = 0;
    assert_eq!(v("signet", z, None), Verdict::Fail);
    let mut z = good("mainnet");
    z.profile |= swiftsync_verify::PROFILE_SIGNET;
    assert_eq!(v("mainnet", z, None), Verdict::Fail);
}

#[test]
fn every_statement_check_fails_alone() {
    let cases: Vec<(&str, fn(&mut UtreexoStatement))> = vec![
        ("kind", |z| z.kind ^= 1),
        ("not done", |z| z.done = 0),
        ("done, but not at H", |z| z.to = z.height - 1),
        ("self_id", |z| z.self_id[0] ^= 1),
        ("a root too few", |z| { z.roots.pop(); }),
        ("a root too many", |z| z.roots.push([3; 32])),
        ("a lineage", |z| z.lineage = [1; 32]),
        ("core_id", |z| z.core_id[0] ^= 1),
        ("profile", |z| z.profile ^= swiftsync_verify::PROFILE_TRIE),
        ("anchor", |z| z.anchor_id[0] ^= 1),
        ("time too new", |z| z.max_time = (NOW + MAX_FUTURE_BLOCK_TIME + 1) as u32),
    ];
    for net in NETWORKS {
        for (what, f) in &cases {
            let mut z = good(net);
            f(&mut z);
            assert_eq!(v(net, z, None), Verdict::Fail, "{net}: {what}");
        }
    }
    // the clock's edge: exactly 2 h ahead is allowed
    let mut z = good("mainnet");
    z.max_time = (NOW + MAX_FUTURE_BLOCK_TIME) as u32;
    assert_eq!(v("mainnet", z, None), Verdict::Ok);
}

#[test]
fn minimum_work_and_the_checkpoint_each_cap_at_demo() {
    for net in NETWORKS {
        // one unit of work below the minimum (the low byte of a minimum that is not a multiple of 256)
        let mut z = good(net);
        assert_ne!(z.cum_work[0], 0);
        z.cum_work[0] -= 1;
        assert_eq!(v(net, z, None), Verdict::Demo, "{net}: below minimum work");
        // one block below the checkpoint, enough work
        let mut z = good(net);
        z.height -= 1;
        z.to -= 1;
        z.core_height -= 1;
        assert_eq!(v(net, z, None), Verdict::Demo, "{net}: below the checkpoint");
        // above both
        let mut z = good(net);
        z.height += 1;
        z.to += 1;
        z.core_height += 1;
        z.cum_work[31] = 1;
        assert_eq!(v(net, z, None), Verdict::Ok, "{net}: above both");
    }
}

#[test]
fn block_h_and_the_core_tip() {
    let z = good("mainnet");
    let a = at(&z);
    let i = img("mainnet");
    // block H not named: DEMO; named wrong: FAIL
    assert_eq!(judge_verified_statement(z.clone(), &ID, i, NOW, None, None).verdict, Verdict::Demo);
    assert_eq!(judge_verified_statement(z.clone(), &ID, i, NOW, Some((a.0, disp(&[0x12; 32]).as_str())), None).verdict, Verdict::Fail);
    assert_eq!(judge_verified_statement(z.clone(), &ID, i, NOW, Some((a.0 + 1, a.1.as_str())), None).verdict, Verdict::Fail);
    // T = H: a given tip is compared too
    assert_eq!(v("mainnet", z.clone(), Some(a.clone())), Verdict::Ok);
    assert_eq!(v("mainnet", z.clone(), Some((a.0, disp(&[0x12; 32])))), Verdict::Fail);
    // T > H: OK only with the tip named, and named right
    let mut t = z.clone();
    t.core_height += 5;
    t.tip_hash = [0x22; 32];
    assert_eq!(v("mainnet", t.clone(), None), Verdict::Demo, "T > H, no tip: DEMO");
    assert_eq!(v("mainnet", t.clone(), Some((t.core_height, disp(&[0x22; 32])))), Verdict::Ok);
    assert_eq!(v("mainnet", t.clone(), Some((t.core_height, disp(&[0x23; 32])))), Verdict::Fail);
    assert_eq!(v("mainnet", t.clone(), Some((t.height, disp(&t.block_hash)))), Verdict::Fail, "block H named as the tip");
    // the right tip without block H named: DEMO (block H is what the roots are for), for T = H and T > H
    assert_eq!(judge_verified_statement(z.clone(), &ID, i, NOW, None, Some((a.0, a.1.as_str()))).verdict, Verdict::Demo);
    let tip = disp(&[0x22; 32]);
    assert_eq!(judge_verified_statement(t.clone(), &ID, i, NOW, None, Some((t.core_height, tip.as_str()))).verdict, Verdict::Demo);
    // T = H with no block H named and no tip: DEMO, not OK
    assert_eq!(judge_verified_statement(z, &ID, i, NOW, None, None).verdict, Verdict::Demo);
}

/// The engine's base proof (roots after block 100, tests/data/engine), under its id and pinned key.
const ENG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/engine");
const BASE_ID: &str = "340f6675bafcdd893c12e0dfae0391043cc7fd8919b526ff756ec13d5d76cc95";
const CORE_ID: &str = "c7fe0eb85d2d4c10532742191b059a2adcc3e24e22aa81c1e632e2ee718d2fc9";
const BASE_PIN: &str = "a966e0ebe4e542192436c292a0db4b1c1b4ac2f0208a3d985fd1f5563ecd730a";
const H100: &str = "000000007bc154e0fa7ea32218a72fe2c1bb9f86cf8c9ebf9a715ed27fdb229a";
const H200: &str = "000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320";
fn base100() -> (Vec<u8>, Vec<u8>) {
    (std::fs::read(format!("{ENG}/base100.hze")).unwrap(), std::fs::read(format!("{ENG}/base100.vk")).unwrap())
}

#[test]
fn the_caller_names_the_network() {
    let (f, vk) = base100();
    let key = swiftsync_verify::EngineKey { bytes: &vk, pin: hex32(BASE_PIN).unwrap() };
    let m = verify_utreexo_on("mainnet", &f, BASE_ID, &key, CORE_ID, NOW, Some((100, H100)), Some((200, H200)));
    assert_eq!(m.verdict, Verdict::Demo, "{:?}", m.checks);
    assert_eq!(m.network, "mainnet");
    // the default is mainnet
    assert_eq!(verify_utreexo(&f, BASE_ID, &key, CORE_ID, NOW, Some((100, H100)), Some((200, H200))).verdict, Verdict::Demo);
    // a mainnet proof checked as signet, or as a network without an image: FAIL
    let s = verify_utreexo_on("signet", &f, BASE_ID, &key, CORE_ID, NOW, Some((100, H100)), None);
    assert_eq!(s.verdict, Verdict::Fail);
    assert!(s.checks.iter().any(|c| !c.0 && c.1.contains("the proof is for signet")), "{:?}", s.checks);
    for n in ["regtest", "testnet", "", "Mainnet"] {
        assert_eq!(verify_utreexo_on(n, &f, BASE_ID, &key, CORE_ID, NOW, Some((100, H100)), None).verdict, Verdict::Fail, "{n:?}");
    }
}

#[test]
fn a_proof_with_trailing_bytes_fails() {
    let (mut f, vk) = base100();
    let key = swiftsync_verify::EngineKey { bytes: &vk, pin: hex32(BASE_PIN).unwrap() };
    assert_eq!(verify_utreexo(&f, BASE_ID, &key, CORE_ID, NOW, None, None).verdict, Verdict::Demo);
    f.push(0);
    let r = verify_utreexo(&f, BASE_ID, &key, CORE_ID, NOW, None, None);
    assert!(r.verdict == Verdict::Fail && r.statement.is_none());
}

/// A real proof that no longer verifies: FAIL with that as the ONLY check (its journal is not read), and the report
/// carries no statement.
#[test]
fn a_proof_that_does_not_verify_fails() {
    let (f, vk) = base100();
    let key = swiftsync_verify::EngineKey { bytes: &vk, pin: hex32(BASE_PIN).unwrap() };
    let mut bad = f.clone();
    bad[4096] ^= 1;
    let r = verify_utreexo(&bad, BASE_ID, &key, CORE_ID, NOW, Some((100, H100)), None);
    assert_eq!(r.verdict, Verdict::Fail);
    let failed: Vec<_> = r.checks.iter().filter(|c| !c.0).collect();
    assert!(failed.len() == 1 && failed[0].1.starts_with("the engine proof verifies"), "{:?}", r.checks);
    assert!(r.statement.is_none() && !r.identity_checked);
}
