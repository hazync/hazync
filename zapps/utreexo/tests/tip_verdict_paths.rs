//! The tip statement's verdict as a pure function of the statement (judge_tip_statement, UTREEXO_ZAPP.md §10.7): OK
//! from a statement built here (no real receipt reaches it: all are below minimum work), and each check failing alone.
#![cfg(feature = "tip")]
use swiftsync_verify::{hex32, words, Image, Verdict};
use zapp_utreexo::*;

const NOW: u64 = 1_800_000_000;
/// Neither the built-in tip nor the built-in base id: statements under them may reach OK.
const TIP: [u8; 32] = [7; 32];
const BASE: [u8; 32] = [9; 32];

fn disp(b: &[u8; 32]) -> String {
    b.iter().rev().map(|x| format!("{x:02x}")).collect()
}

fn img(net: &str) -> &'static Image {
    Image::by_network(net).unwrap()
}

fn good(net: &str) -> TipStatement {
    let i = img(net);
    let h = i.checkpoint_height;
    let mut w = hex32(i.min_work_hex).unwrap();
    w.reverse();
    TipStatement {
        kind: KIND_UTREEXO_TIP,
        self_id: words(&risc0_zkvm::sha::Digest::from_bytes(TIP)),
        base_id: words(&risc0_zkvm::sha::Digest::from_bytes(BASE)),
        network: if net == "signet" { 4 } else { 0 },
        base_height: 100,
        base_core_id: words(&i.method_id()),
        steps: 3,
        from: h - 6,
        height: h,
        block_hash: [0x11; 32],
        leaves: 3,
        roots: vec![[1; 32], [0; 32]],
        core_id: words(&i.method_id()),
        anchor_id: hex32(i.genesis_anchor_hex).unwrap(),
        core_height: h,
        tip_hash: [0x11; 32],
        cum_work: w,
        max_time: NOW as u32,
        profile: i.profile,
        ..Default::default()
    }
}

fn v(net: &str, z: TipStatement, tip: Option<(u32, String)>) -> Verdict {
    let a = (z.height, disp(&z.block_hash));
    judge_tip_statement(z, &TIP, &BASE, img(net), NOW, Some((a.0, a.1.as_str())), tip.as_ref().map(|(h, x)| (*h, x.as_str()))).report.verdict
}

#[test]
fn ok_is_reachable_on_both_networks() {
    for net in ["mainnet", "signet"] {
        assert_eq!(v(net, good(net), None), Verdict::Ok, "{net}");
    }
}

#[test]
fn each_check_fails_alone() {
    let cases: Vec<(&str, fn(&mut TipStatement))> = vec![
        ("kind 'ZU' 1 (a base)", |z| z.kind = KIND_UTREEXO),
        ("another tip image", |z| z.self_id[0] ^= 1),
        ("another base image", |z| z.base_id[0] ^= 1),
        ("zero steps", |z| z.steps = 0),
        ("from = height", |z| z.from = z.height),
        ("base above from", |z| z.base_height = z.from + 1),
        ("a root missing", |z| { z.roots.pop(); }),
        ("signet's network word", |z| z.network = 4),
        ("the base on another core image", |z| z.base_core_id[0] ^= 1),
        ("this step on another core image", |z| z.core_id[0] ^= 1),
        ("a foreign lineage", |z| z.lineage = [5; 32]),
        ("another profile", |z| z.profile ^= 2),
        ("another anchor", |z| z.anchor_id[0] ^= 1),
        ("a block 3 h ahead", |z| z.max_time = (NOW + 3 * 3600) as u32),
    ];
    for (what, f) in cases {
        let mut z = good("mainnet");
        f(&mut z);
        assert_eq!(v("mainnet", z, None), Verdict::Fail, "{what}");
    }
}

#[test]
fn demo_paths() {
    let i = img("mainnet");
    // below minimum work
    let mut z = good("mainnet");
    z.cum_work = [0; 32];
    assert_eq!(v("mainnet", z, None), Verdict::Demo);
    // T > H: OK only with the tip named, FAIL with a wrong one
    let mut z = good("mainnet");
    z.height -= 3;
    z.from -= 3;
    assert_eq!(v("mainnet", z.clone(), None), Verdict::Demo);
    assert_eq!(v("mainnet", z.clone(), Some((i.checkpoint_height, disp(&[0x11; 32])))), Verdict::Ok);
    assert_eq!(v("mainnet", z, Some((i.checkpoint_height, disp(&[0x12; 32])))), Verdict::Fail);
    // block H not named: DEMO
    let z = good("mainnet");
    let r = judge_tip_statement(z, &TIP, &BASE, i, NOW, None, None);
    assert_eq!(r.report.verdict, Verdict::Demo);
    // the built-in base image: OK once it is pinned, DEMO while it is not
    let base = hex32(UTREEXO_IMAGE_ID).unwrap();
    let mut z = good("mainnet");
    z.base_id = words(&risc0_zkvm::sha::Digest::from_bytes(base));
    let a = (z.height, disp(&z.block_hash));
    let want = if UTREEXO_IMAGE_PINNED { Verdict::Ok } else { Verdict::Demo };
    assert_eq!(judge_tip_statement(z, &TIP, &base, i, NOW, Some((a.0, a.1.as_str())), None).report.verdict, want);
}

#[test]
fn the_value_view_is_floresta_s() {
    let z = good("mainnet");
    let f = z.as_value().floresta_rust("mainnet");
    assert!(f.contains("BitcoinNodeHash::Empty,") && f.contains("leaves: 3,"), "{f}");
}

/// The built-in ids are the published ones (reproduce/LIBZYNC_IDS, the reproducible container's output), and pinned.
#[test]
fn built_in_ids_are_the_published_ones() {
    let ids = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../reproduce/LIBZYNC_IDS")).unwrap();
    let id = |name: &str| ids.lines().find(|l| l.starts_with(&format!("{name} mainnet METHOD_ID "))).map(|l| l.split_whitespace().nth(3).unwrap().to_string());
    assert_eq!(id("zyncapp-utreexo").as_deref(), Some(UTREEXO_IMAGE_ID));
    assert_eq!(id("zyncapp-utreexo-tip").as_deref(), Some(UTREEXO_TIP_IMAGE_ID));
    assert!(UTREEXO_IMAGE_PINNED && UTREEXO_TIP_IMAGE_PINNED);
}

/// Review H1: a composite receipt nested 300,000 levels deep (each level: no segments, one assumption receipt that is
/// again composite) must be refused before any decoding. Decoding it first overflowed the stack (abort, exit 134).
pub fn nested_composite(levels: usize) -> Vec<u8> {
    let mut b = 0u32.to_le_bytes().to_vec(); // InnerReceipt::Composite
    for _ in 0..levels {
        b.extend_from_slice(&0u64.to_le_bytes()); // segments: []
        b.extend_from_slice(&1u64.to_le_bytes()); // assumption_receipts: [one]
        b.extend_from_slice(&0u32.to_le_bytes()); // InnerAssumptionReceipt::Composite
    }
    b
}

#[test]
fn a_deeply_nested_composite_receipt_is_refused_without_decoding() {
    let r = nested_composite(300_000);
    assert_eq!(journal_kind(&r), None);
    let rep = verify_utreexo_tip_on("mainnet", &r, UTREEXO_TIP_IMAGE_ID, UTREEXO_IMAGE_ID, None, NOW, None, None);
    assert_eq!(rep.report.verdict, Verdict::Fail);
    assert!(rep.report.checks.iter().any(|c| !c.0 && c.1.contains("composite")), "{:?}", rep.report.checks);
    assert_eq!(verify_utreexo(&r, UTREEXO_IMAGE_ID, None, NOW, None, None).verdict, Verdict::Fail);
}
