//! `ts-verify [--network mainnet|signet|regtest] [--image <hex>] [--anchor <A.json>] [--now <unix s>] <receipt>`
//!
//! EXPERIMENTAL prototype (docs/libzync/TRUSTED_STATE.md §8.1, §14.3), on branch feat/trusted-state only. It wraps the
//! shared verifier library `swiftsync-verify` without changing it, and adds what §4 asks of a verifier for a Trusted
//! State proof: a PINNED anchor table `(network, H, block, hash_serialized_3, anchor_id)` and a verdict between OK and
//! DEMO:
//!
//! - `TS_VERIFY_OK` (exit 0): a genesis-anchored proof; exactly swiftsync-verify's OK (this tool adds nothing to it).
//! - `TS_VERIFY_ANCHORED` (exit 3): every check of swiftsync-verify passes with the given anchor A, A's link digest is
//!   the PINNED anchor_id of (network, A.height), A's tip is the pinned block, and the chain has the work and
//!   checkpoint an OK verdict would need. Printed with the §4 wording: valid ASSUMING Core's UTXO set at H.
//! - `TS_VERIFY_DEMO` (exit 2): valid from the given anchor, but the anchor is not a pinned one (or its pin is
//!   PENDING), or low work; as swiftsync-verify's DEMO.
//! - `TS_VERIFY_FAIL` (exit 1): anything else.
//!
//! What it does NOT do (TRUSTED_STATE.md §8.1, open): the joined statement after the join receipt; history facts
//! relabelled "since H" (no fact is checked here at all); the block forest's 0x02 leaves below H for coin and tx facts.
//! The pin itself is trusted: it must come from a reproducible `ts-anchor build` of Core's snapshot, and from the
//! ingest receipt before it is relied on (§11.1).
use swiftsync_verify as sv;

/// One pinned Trusted State anchor. `anchor_id_hex` is the anchor's link digest (what the guest commits as
/// `anchor_id` after the base step on it); `None` = PENDING: the anchor is chosen but its id is not known yet, and
/// a proof on it can never be ANCHORED.
#[derive(Clone, Copy)]
pub struct TrustedAnchor {
    pub network: &'static str,
    pub height: u32,
    /// Display order, as Core's chainparams and `getblockhash` write it.
    pub block_hex: &'static str,
    /// As Core's chainparams (`AssumeutxoHash{uint256{"…"}}`) and `gettxoutsetinfo` print it.
    pub hash_serialized_3: &'static str,
    pub anchor_id_hex: Option<&'static str>,
    pub source: &'static str,
}

pub const ANCHORS: &[TrustedAnchor] = &[
    TrustedAnchor {
        network: "mainnet",
        height: 965_000,
        block_hex: "00000000000000000001595977e6000ce56129f5c9b4073e31ccc30b90b97da9",
        hash_serialized_3: "4a8d794337118c0c615b574f817c7306c687584a537184b8d233df42bf477ec2",
        // PENDING: the anchor_id is the output of `ts-anchor build` on Core's dumptxoutset at 965,000, which has not been
        // taken (TRUSTED_STATE.md §9). It must be filled ONLY from that build, reproduced by a second party, and never
        // before the ingest receipt of the same snapshot exists (§11.1).
        anchor_id_hex: None,
        source: "Bitcoin Core m_assumeutxo_data (v32.0+, commit fed50258); anchor_id PENDING (snapshot not taken)",
    },
    TrustedAnchor {
        network: "regtest",
        height: 104,
        block_hex: "6abf594e169df9042a5484b6703707acfda03eee3ab2d63fe8ebe22767242fb8",
        hash_serialized_3: "97aa05659a2add03eed3dfc24eefa036e153467d5bc4b0f2d68f58e969639015",
        anchor_id_hex: Some("8fce9ea699a50fbba21f1514df2e223ed88378e6710940cd6a1b0de68caff71e"),
        source: "TEST ENTRY: the zAPP regtest harness chain (scripts/zapp-regtest), Core 28.0 utxo_104.dat; NOT a network pin",
    },
];

/// The regtest image of the zAPP harness (eb2cd424, CI's host_reg, main 797b777e): not in swiftsync-verify's table
/// (regtest is not a network it serves), so the prototype carries it, for its controls only.
pub static REGTEST: sv::Image = sv::Image {
    network: "regtest",
    method_id_hex: "eb2cd424b94a4aa74f3458efc0d5edb1d3c35d46615eab8363696f1fa4b47ccd",
    profile: 0x79, // the regtest product profile (TRUSTED_STATE.md §3.1), checked against the receipt by swiftsync-verify
    genesis_hash_hex: "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206",
    genesis_time: 1_296_688_602,
    genesis_bits: 0x207f_ffff,
    genesis_work: 2,
    min_work_hex: "0000000000000000000000000000000000000000000000000000000000000000",
    genesis_anchor_hex: "99509fc506d022f06dfd72bb146fc894718ce8047e69e10b201e1a78d4bccf63",
    checkpoint_height: 0,
    lineage: &[],
};

pub fn pinned(network: &str, height: u32) -> Option<&'static TrustedAnchor> {
    let p = ANCHORS.iter().find(|a| a.network == network && a.height == height)?;
    // CONTROLS BUILD only (`--features controls`, verify_controls.sh): TS_VERIFY_REGTEST_PIN=<hex>|pending replaces the
    // regtest TEST ENTRY's anchor_id, so the pinned-id and PENDING checks are each shown to bite on a real receipt. A
    // real build has no such hook and refuses the variable.
    #[cfg(feature = "controls")]
    if let (Ok(v), "regtest") = (std::env::var("TS_VERIFY_REGTEST_PIN"), network) {
        let id: Option<&'static str> = if v == "pending" { None } else { Some(Box::leak(v.into_boxed_str())) };
        return Some(Box::leak(Box::new(TrustedAnchor { anchor_id_hex: id, source: "CONTROLS BUILD: a replaced regtest pin", ..*p })));
    }
    #[cfg(not(feature = "controls"))]
    assert!(std::env::var_os("TS_VERIFY_REGTEST_PIN").is_none(), "TS_VERIFY_REGTEST_PIN needs the controls build (--features controls)");
    Some(p)
}

#[derive(Debug, PartialEq, Eq)]
pub enum TsVerdict { Ok, Anchored, Demo, Fail }

impl TsVerdict {
    pub fn exit_code(&self) -> i32 { match self { TsVerdict::Ok => 0, TsVerdict::Anchored => 3, TsVerdict::Demo => 2, TsVerdict::Fail => 1 } }
}

/// The outcome and the lines to print (checks first, the verdict line last).
pub fn ts_verify(receipt: &[u8], image: &'static sv::Image, anchor: Option<sv::ChainState>, now: u64) -> (TsVerdict, Vec<String>) {
    let mut out: Vec<String> = Vec::new();
    let rep = sv::verify(&sv::Inputs { receipt, image, anchor: anchor.clone(), now });
    for (ok, what) in &rep.checks { out.push(format!("  [{}] {what}", if *ok { "ok" } else { "FAIL" })); }
    let fail = |mut out: Vec<String>, why: String| { out.push(format!("TS_VERIFY_FAIL: {why}")); (TsVerdict::Fail, out) };
    if rep.verdict == sv::Verdict::Fail {
        return fail(out, "a check above failed".into());
    }
    let cs = rep.state.clone().expect("a verdict other than Fail carries the state");
    let Some(a) = anchor else {
        // no anchor: swiftsync-verify's own verdict, unchanged
        return match rep.verdict {
            sv::Verdict::Ok => { out.push(format!("TS_VERIFY_OK: blocks 0..{} valid, from genesis (swiftsync-verify's OK)", cs.height)); (TsVerdict::Ok, out) }
            _ => { out.push(format!("TS_VERIFY_DEMO: a valid chain from genesis to {}, below the minimum work or the checkpoint", cs.height)); (TsVerdict::Demo, out) }
        };
    };
    // From here the proof rests on A: swiftsync-verify checked cs.anchor_id == anchor_id(A) and calls it DEMO.
    let demo = |mut out: Vec<String>, why: String| { out.push(format!("TS_VERIFY_DEMO: {why}")); (TsVerdict::Demo, out) };
    let aid = sv::anchor_id(&a);
    let Some(p) = pinned(image.network, a.height) else {
        return demo(out, format!("valid FROM THE GIVEN ANCHOR at {} only: no Trusted State anchor is pinned for {} at that height", a.height, image.network));
    };
    let mut tip = a.tip_hash;
    tip.reverse();
    if sv::hex(&tip) != p.block_hex {
        return fail(out, format!("the anchor's block {} is not the pinned block {} at {}", sv::hex(&tip), p.block_hex, p.height));
    }
    let Some(pid) = p.anchor_id_hex else {
        return demo(out, format!("the {} anchor at {} is PENDING (its anchor_id is not pinned yet: {}); this proof's anchor is {}",
                                 p.network, p.height, p.source, sv::hex(&aid)));
    };
    if sv::hex(&aid) != pid {
        return demo(out, format!("the anchor's link digest {} is not the pinned anchor_id {pid} for {} at {}: not the pinned Trusted State",
                                 sv::hex(&aid), p.network, p.height));
    }
    out.push(format!("  [ok] the anchor's link digest is the pinned Trusted State anchor_id {pid} ({} at {}; {})", p.network, p.height, p.source));
    if cs.height <= a.height {
        return fail(out, format!("the proof's tip {} is not above the anchor height {}", cs.height, a.height));
    }
    if rep.low_work {
        return demo(out, "on the pinned anchor, but below the network's minimum chainwork or checkpoint".into());
    }
    let mut t = cs.tip_hash;
    t.reverse();
    // §4's label, before the join
    out.push(format!(
        "TS_VERIFY_ANCHORED: TRUSTED STATE: blocks {}..{} (tip {}) are valid under Bitcoin Core v28.0's rules (image {}), \
         ASSUMING the UTXO set at block {} ({}) is the one Bitcoin Core pins in its chainparams (hash_serialized_3 {}), \
         and the anchor's side inputs (headers 0..{}, the BIP30-tracked coins and REPEATABLE transactions). \
         Not yet joined to a proof from genesis. History facts on this proof start at {} (not checked by this prototype).",
        a.height + 1, cs.height, sv::hex(&t), image.method_id_hex, a.height, p.block_hex, p.hash_serialized_3, a.height, a.height));
    (TsVerdict::Anchored, out)
}

fn main() {
    std::panic::set_hook(Box::new(|i| { println!("TS_VERIFY_FAIL: {}", format!("{i}").escape_debug()); std::process::exit(1); }));
    let mut args = std::env::args().skip(1);
    let (mut network, mut image_hex, mut anchor, mut now_arg, mut files) = ("mainnet".to_string(), None, None, None, Vec::new());
    let usage = || -> ! { eprintln!("usage: ts-verify [--network mainnet|signet|regtest] [--image <hex>] [--anchor <A.json>] [--now <unix s>] <receipt>"); std::process::exit(1) };
    while let Some(a) = args.next() {
        match a.as_str() {
            "--network" => network = args.next().unwrap_or_else(|| usage()),
            "--image" => image_hex = Some(args.next().unwrap_or_else(|| usage())),
            "--anchor" => anchor = Some(args.next().unwrap_or_else(|| usage())),
            "--now" => now_arg = Some(args.next().unwrap_or_else(|| usage())),
            _ if a.starts_with("--") => usage(),
            _ => files.push(a),
        }
    }
    let [receipt] = files.as_slice() else { usage() };
    let image: &'static sv::Image = if network == "regtest" {
        if image_hex.as_deref().is_some_and(|h| h != REGTEST.method_id_hex) { panic!("regtest: only the harness image {}", REGTEST.method_id_hex) }
        &REGTEST
    } else {
        match &image_hex { Some(h) => sv::Image::by_id(&network, h), None => sv::Image::by_network(&network) }
            .unwrap_or_else(|| panic!("no built-in {network} image{}", image_hex.as_ref().map(|h| format!(" {h}")).unwrap_or_default()))
    };
    let anchor: Option<sv::ChainState> = anchor.map(|p| serde_json::from_slice(&std::fs::read(&p).unwrap_or_else(|e| panic!("{p}: {e}")))
        .unwrap_or_else(|e| panic!("{p}: not a ChainState: {e}")));
    let mut now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    if let Some(v) = now_arg { let t: u64 = v.parse().expect("--now: unix seconds"); assert!(t <= now, "--now may only set the clock back"); now = t; }
    let bytes = std::fs::read(receipt).unwrap_or_else(|e| panic!("{receipt}: {e}"));
    let (v, lines) = ts_verify(&bytes, image, anchor, now);
    for l in lines { println!("{l}"); }
    if cfg!(feature = "controls") { println!("  <<< CONTROLS BUILD (--features controls): its pins can be replaced; never verify anything with it >>>"); }
    std::process::exit(v.exit_code());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The prototype's regtest image pins the genesis anchor the harness's receipts commit (block_104.rcpt's anchor_id).
    #[test]
    fn regtest_genesis_anchor_is_computed() {
        assert_eq!(sv::hex(&sv::anchor_id(&sv::genesis_anchor(&REGTEST))), REGTEST.genesis_anchor_hex);
    }

    /// The table: one entry per (network, H), 64-hex fields, and the mainnet entry is the chainparams triple of §9 with
    /// its anchor_id PENDING (no value may be pinned before the snapshot exists).
    #[test]
    fn table_is_well_formed_and_mainnet_is_pending() {
        let hex64 = |s: &str| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
        for (i, a) in ANCHORS.iter().enumerate() {
            assert!(hex64(a.block_hex) && hex64(a.hash_serialized_3) && a.anchor_id_hex.map_or(true, hex64), "entry {i}");
            assert!(ANCHORS.iter().filter(|b| b.network == a.network && b.height == a.height).count() == 1, "entry {i} twice");
        }
        let m = pinned("mainnet", 965_000).unwrap();
        assert_eq!(m.block_hex, "00000000000000000001595977e6000ce56129f5c9b4073e31ccc30b90b97da9");
        assert_eq!(m.hash_serialized_3, "4a8d794337118c0c615b574f817c7306c687584a537184b8d233df42bf477ec2");
        assert!(m.anchor_id_hex.is_none(), "the 965,000 anchor_id must stay PENDING until the snapshot's build exists");
    }
}
