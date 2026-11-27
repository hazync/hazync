//! The assume checkers' verdict paths (docs/libzync/ZAPP_TESTING.md §3.5, the cargo-mutants pass of 17.11.2026): the
//! checks no test reached, one test each, on the library's public API. In a file of its own, not the crate's test
//! module: a line added to src/lib.rs moves what the WASM module embeds (its published sha).
//!
//! Snapshots here are written byte by byte (Core's format: utxo_snapshot.h, coins.h, compressor.h, serialize.h), so a
//! boundary is exactly the one named; the real ones are Core v31.1.0's (vectors/).

use sha2::{Digest, Sha256};
use swiftsync_verify::{ChainState, PROFILE_LIBZYNC};
use zapp_assume::*;

const RCPT: &[u8] = include_bytes!("../../../swiftsync-verify/vectors/mainnet_1_200_engine.hze");
const SNAP_200: &[u8] = include_bytes!("../vectors/utxo_200.dat");
const HDRS: &[u8] = include_bytes!("../vectors/headers_0_200.bin");
const MAINNET: [u8; 4] = [0xf9, 0xbe, 0xb4, 0xd9];
const MAX_MONEY: u64 = 21_000_000 * 100_000_000;

fn state() -> ChainState {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let id = include_str!("../../../swiftsync-verify/vectors/mainnet_1_200_engine.image_id").trim();
    let vk = include_bytes!("../../../swiftsync-verify/vectors/mainnet_1_200_engine.vk").to_vec();
    let image = swiftsync_verify::Image::by_network("mainnet").unwrap().with_method_id(id).unwrap().with_engine_key_unchecked(vk);
    let rep = swiftsync_verify::verify(&swiftsync_verify::Inputs { receipt: RCPT, image, anchor: None, now });
    assert_eq!(rep.verdict, swiftsync_verify::Verdict::Demo, "{:?}", rep.checks);
    rep.state.unwrap()
}

fn dsha(b: &[u8]) -> [u8; 32] {
    Sha256::digest(Sha256::digest(b)).into()
}

/// A snapshot header (mainnet, base block zero) naming `count` coins, then `body`.
fn snap(count: u64, body: &[u8]) -> Vec<u8> {
    let mut v = SNAPSHOT_MAGIC.to_vec();
    v.extend(SNAPSHOT_VERSION.to_le_bytes());
    v.extend(MAINNET);
    v.extend([0u8; 32]);
    v.extend(count.to_le_bytes());
    v.extend(body);
    v
}

/// One txid group of one coin: txid, CompactSize 1, the vout's CompactSize bytes, VARINT code, VARINT compressed amount,
/// then the script as Core compresses it (`script`: its VARINT type / size and its bytes).
fn group(txid: [u8; 32], vout_cs: &[u8], code: u64, camount: u64, script: &[u8]) -> Vec<u8> {
    let mut v = txid.to_vec();
    v.push(1);
    v.extend(vout_cs);
    v.extend(varint_bytes(code));
    v.extend(varint_bytes(camount));
    v.extend(script);
    v
}

/// A raw (uncompressed) script of `n` bytes, as the snapshot writes it: VARINT(n + 6), the bytes.
fn raw_script(n: usize) -> Vec<u8> {
    [varint_bytes(n as u64 + 6), vec![0x51; n]].concat()
}

/// Core's CompressAmount (compressor.cpp).
fn compress(mut n: u64) -> u64 {
    if n == 0 { return 0 }
    let mut e = 0;
    while n % 10 == 0 && e < 9 { n /= 10; e += 1; }
    if e < 9 { let d = n % 10; n /= 10; 1 + (n * 9 + d - 1) * 10 + e } else { 1 + (n - 1) * 10 + 9 }
}

fn coins_of(b: &[u8]) -> Result<Vec<SnapCoin>, String> {
    let mut v = Vec::new();
    read_snapshot(b, |c| { v.push(c.clone()); Ok(()) })?;
    Ok(v)
}

/// Doubling a point held with z != 1 (P + Q - Q, then + P) gives what doubling the affine point gives; a sum that
/// cancels keeps its count.
#[test]
fn ecmh_doubling_off_affine_and_counts() {
    let (px, py) = ecmh_point(&[1u8; 32]).unwrap();
    let (qx, qy) = ecmh_point(&[2u8; 32]).unwrap();
    let mut a = Ecmh::default();
    a.add_affine(&px, &py);
    a.add_affine(&px, &py);
    let mut b = Ecmh::default();
    b.add_affine(&px, &py);
    b.add_affine(&qx, &qy);
    b.add_affine(&qx, &(-qy));
    b.add_affine(&px, &py);
    assert_eq!(a.committed(), b.committed(), "2P by both paths");
    assert_eq!(a.committed().0, 1, "2P is not the identity");
    assert_eq!((a.count, b.count), (2, 4));
    let mut c = Ecmh::default();
    c.add_affine(&px, &py);
    c.add_affine(&px, &(-py));
    assert_eq!((c.committed(), c.count), ((0, [0; 16]), 2), "P - P: the identity, two leaves counted");
}

/// Headers: whole 80-byte headers only (one byte more, or none, is refused).
#[test]
fn headers_are_whole_80_byte_records() {
    let genesis = dsha(&HDRS[..80]);
    assert_eq!(HeaderChain::bind(&HDRS[..80], &genesis, 0).unwrap().hashes, vec![genesis]);
    assert!(HeaderChain::bind(&HDRS[..81], &genesis, 0).unwrap_err().contains("multiple of 80"));
    assert!(HeaderChain::bind(&HDRS[..80 * 2 + 1], &dsha(&HDRS[80..160]), 1).unwrap_err().contains("multiple of 80"));
    assert!(HeaderChain::bind(&[], &genesis, 0).unwrap_err().contains("multiple of 80"));
}

/// The MTP from a run that starts above genesis: the same as from the whole chain, wherever the eleven are in the run.
#[test]
fn mtp_from_a_run_above_genesis() {
    let cs = state();
    let full = HeaderChain::bind(HDRS, &cs.tip_hash, cs.height).unwrap();
    for start in [1u32, 5, 150] {
        let part = HeaderChain::bind(&HDRS[80 * start as usize..], &cs.tip_hash, cs.height).unwrap();
        assert_eq!(part.first_height, start);
        for h in [start + 11, start + 12, 199, 200] {
            assert_eq!(part.mtp_before(h), full.mtp_before(h), "run from {start}, MTP before {h}");
        }
        assert!(part.mtp_before(start + 10).is_err(), "needs a header below the run");
    }
}

/// The tracked list: display-order txids (any case) to internal order, comments and blank lines skipped; bad lines refused.
#[test]
fn tracked_lists_parse() {
    let a = "d5d27987d2a3dfc724e359870c6644b40e497bdc0589a033220fe15429d88599";
    let b = "e3bf3d07d4b0375638d5f1db5255fe07ba2c4cb067cd81b84ee974b6585fb468";
    let mut ai = [0u8; 32];
    for i in 0..32 { ai[i] = u8::from_str_radix(&a[62 - 2 * i..64 - 2 * i], 16).unwrap(); }
    assert_eq!(parse_display_hash(a), Some(ai));
    assert_eq!(display(&ai), a);
    let t = parse_tracked(&format!("# BIP30\n\n{a}:0\n  {}:1   # the second\n", b.to_uppercase())).unwrap();
    assert_eq!(t.len(), 2);
    assert!(t.contains(&(ai, 0)));
    assert!(t.contains(&(parse_display_hash(b).unwrap(), 1)));
    assert!(!t.contains(&(ai, 1)));
    assert!(parse_tracked("").unwrap().is_empty());
    for bad in [format!("{a}"), format!("{a}:x"), format!("{a}:-1"), format!("{}:0", &a[1..]), "zz:0".to_string()] {
        assert!(parse_tracked(&bad).unwrap_err().contains("line 1"), "{bad}");
    }
}

/// assumevalid: a state of a non-libzync image, and headers that do not end at the proven tip (another last hash, or
/// another height), are refused, also for a block the headers hold.
#[test]
fn assumevalid_refuses_other_images_and_headers() {
    let cs = state();
    let hc = HeaderChain::bind(HDRS, &cs.tip_hash, cs.height).unwrap();
    assert_eq!(assume_valid(&cs, Some(&hc), Some(hc.hashes[170])).unwrap().height, 170);
    let mut c = cs.clone();
    c.profile &= !PROFILE_LIBZYNC;
    assert!(assume_valid(&c, None, None).unwrap_err().contains("not a libzync image"));
    let mut last = hc.hashes.clone();
    *last.last_mut().unwrap() = [9u8; 32];
    let other_tip = HeaderChain { first_height: 0, hashes: last, times: hc.times.clone() };
    assert!(assume_valid(&cs, Some(&other_tip), Some(hc.hashes[170])).unwrap_err().contains("do not end at the proven tip"));
    let shifted = HeaderChain { first_height: 1, hashes: hc.hashes.clone(), times: hc.times.clone() };
    assert!(assume_valid(&cs, Some(&shifted), Some(hc.hashes[170])).unwrap_err().contains("do not end at the proven tip"));
}

/// check_snapshot: headers from genesis with the right height but another last hash are refused.
#[test]
fn snapshot_headers_must_end_at_the_tip() {
    let cs = state();
    let hc = HeaderChain::bind(HDRS, &cs.tip_hash, cs.height).unwrap();
    assert!(check_snapshot(&cs, SNAP_200, &hc, &Tracked::new()).is_ok());
    let mut last = hc.hashes.clone();
    *last.last_mut().unwrap() = [9u8; 32];
    let other = HeaderChain { first_height: 0, hashes: last, times: hc.times.clone() };
    assert!(check_snapshot(&cs, SNAP_200, &other, &Tracked::new()).unwrap_err().contains("from genesis to the proven tip"));
}

/// A truncated snapshot names the byte where the read that failed started (fixed-size reads and a script's bytes).
#[test]
fn truncation_names_the_byte() {
    let e = read_snapshot(&snap(1, &[])[..], |_| Ok(())).unwrap_err();
    assert!(e.contains("truncated at byte 51 "), "{e}");
    let g = group([1; 32], &[0], 2, 0, &raw_script(5));
    let cut = snap(1, &g[..g.len() - 3]);
    let e = read_snapshot(&cut[..], |_| Ok(())).unwrap_err();
    let at = 51 + g.len() - 5;
    assert!(e.contains(&format!("truncated at byte {at} ")), "{e}");
}

/// CompactSize: at most MAX_SIZE (0x02000000), both sides of it (a vout).
#[test]
fn compact_size_max() {
    let cs = |v: u32| [&[0xfe][..], &v.to_le_bytes()].concat();
    let ok = coins_of(&snap(1, &group([1; 32], &cs(0x0200_0000), 2, 0, &raw_script(1)))).unwrap();
    assert_eq!(ok[0].vout, 0x0200_0000);
    let e = coins_of(&snap(1, &group([1; 32], &cs(0x0200_0001), 2, 0, &raw_script(1)))).unwrap_err();
    assert!(e.contains("exceeds MAX_SIZE"), "{e}");
}

/// VARINT into u32 (the height / coinbase code): u32::MAX is read, u32::MAX + 1 and above are refused.
#[test]
fn varint_range() {
    let c = coins_of(&snap(1, &group([1; 32], &[0], u32::MAX as u64, 0, &raw_script(1)))).unwrap();
    assert_eq!((c[0].height, c[0].coinbase), (u32::MAX >> 1, true));
    for code in [u32::MAX as u64 + 1, 1u64 << 33, 1u64 << 40] {
        let e = coins_of(&snap(1, &group([1; 32], &[0], code, 0, &raw_script(1)))).unwrap_err();
        assert!(e.contains("VARINT") && e.contains("too large"), "{code}: {e}");
    }
}

/// The compressed script forms 0..5 and raw scripts up to MAX_SCRIPT_SIZE (10,000 bytes), not over it.
#[test]
fn script_forms_and_size() {
    let h20 = [0x11u8; 20];
    let x = [0x22u8; 32];
    let cases: [(Vec<u8>, Vec<u8>); 4] = [
        ([&[0u8][..], &h20].concat(), [&[0x76, 0xa9, 20][..], &h20, &[0x88, 0xac]].concat()),
        ([&[1u8][..], &h20].concat(), [&[0xa9, 20][..], &h20, &[0x87]].concat()),
        ([&[2u8][..], &x].concat(), [&[33, 2][..], &x, &[0xac]].concat()),
        ([&[3u8][..], &x].concat(), [&[33, 3][..], &x, &[0xac]].concat()),
    ];
    for (enc, want) in cases {
        let c = coins_of(&snap(1, &group([1; 32], &[0], 2, 0, &enc))).unwrap();
        assert_eq!(c[0].script_pubkey, want, "type {}", enc[0]);
    }
    assert_eq!(coins_of(&snap(1, &group([1; 32], &[0], 2, 0, &raw_script(10_000)))).unwrap()[0].script_pubkey.len(), 10_000);
    let e = coins_of(&snap(1, &group([1; 32], &[0], 2, 0, &raw_script(10_001)))).unwrap_err();
    assert!(e.contains("MAX_SCRIPT_SIZE"), "{e}");
}

/// A txid group of no coins, or of more coins than the header has left, is refused.
#[test]
fn group_sizes() {
    let mut zero = [1u8; 32].to_vec();
    zero.push(0);
    let e = coins_of(&snap(1, &[zero, group([2; 32], &[0], 2, 0, &raw_script(1))].concat())).unwrap_err();
    assert!(e.contains("0 coins for txid"), "{e}");
    let mut two = group([1; 32], &[0], 2, 0, &raw_script(1));
    two[32] = 2;
    two.extend([&[1u8][..], &varint_bytes(2), &varint_bytes(0), &raw_script(1)].concat());
    let e = coins_of(&snap(1, &two)).unwrap_err();
    assert!(e.contains("2 coins for txid") && e.contains("1 coins left"), "{e}");
    assert_eq!(coins_of(&snap(2, &two)).unwrap().len(), 2, "the same group under a count of 2");
}

/// Amounts: MAX_MONEY is read, one satoshi more is refused.
#[test]
fn amount_range() {
    assert_eq!(compress(MAX_MONEY), 0x1406f40, "Core's compress_tests vector");
    let c = coins_of(&snap(1, &group([1; 32], &[0], 2, compress(MAX_MONEY), &raw_script(1)))).unwrap();
    assert_eq!(c[0].value, MAX_MONEY);
    let e = coins_of(&snap(1, &group([1; 32], &[0], 2, compress(MAX_MONEY + 1), &raw_script(1)))).unwrap_err();
    assert!(e.contains("MoneyRange"), "{e}");
}

/// hash_serialized_3 against a preimage written out here: TxOutSer with each CompactSize form of the script length.
#[test]
fn hash_serialized_3_preimage() {
    for (n, len_bytes) in [(252usize, vec![252u8]), (253, vec![253, 253, 0]), (300, vec![253, 0x2c, 0x01]), (0x1_0000, vec![254, 0, 0, 1, 0])] {
        let c = SnapCoin { txid: [3; 32], vout: 7, height: 100, coinbase: true, value: 5_000_000_000, script_pubkey: vec![0x6a; n] };
        let mut h = HashSerialized3::default();
        h.add(&c);
        let pre = [&c.txid[..], &7u32.to_le_bytes(), &201u32.to_le_bytes(), &5_000_000_000u64.to_le_bytes(), &len_bytes, &c.script_pubkey].concat();
        let mut d = dsha(&pre);
        d.reverse();
        let want: String = d.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(h.finish(), want, "script of {n} bytes");
    }
}

/// hash_snapshot reports how many coins the tracked list marked.
#[test]
fn tracked_coins_are_counted() {
    let cs = state();
    let hc = HeaderChain::bind(HDRS, &cs.tip_hash, cs.height).unwrap();
    let all = coins_of(SNAP_200).unwrap();
    let mut t = Tracked::new();
    t.insert((all[3].txid, all[3].vout));
    t.insert((all[9].txid, all[9].vout));
    t.insert(([0xee; 32], 0)); // not in the snapshot
    assert_eq!(hash_snapshot(SNAP_200, &hc, &t, 200).unwrap().tracked_used, 2);
    assert_eq!(hash_snapshot(SNAP_200, &hc, &Tracked::new(), 200).unwrap().tracked_used, 0);
}

/// run_assume, the one flow of the CLI, the C ABI and WASM: every check line, and each way it stops.
#[test]
fn run_assume_end_to_end() {
    let cs = state();
    let hc = HeaderChain::bind(HDRS, &cs.tip_hash, cs.height).unwrap();
    let req = |av: Option<Option<[u8; 32]>>, h: Option<&'static [u8]>, s: Option<&'static [u8]>, t: Option<&'static str>| {
        AssumeRequest { assumevalid: av, headers: h, snapshot: s, tracked: t }
    };
    let o = run_assume(&cs, req(Some(Some(hc.hashes[170])), Some(HDRS), Some(SNAP_200), Some("# none\n")));
    assert!(o.checks.len() == 3 && o.checks.iter().all(|c| c.0), "{:?}", o.checks);
    assert_eq!(o.valid.as_ref().unwrap().height, 170);
    assert_eq!(o.utxo.as_ref().unwrap().coins, 204);
    // the tip, no headers: one check
    let o = run_assume(&cs, req(Some(None), None, None::<&[u8]>, None));
    assert!(o.checks.len() == 1 && o.checks[0].0 && o.valid.unwrap().height == 200, "{:?}", o.checks);
    // headers that do not reach the tip: one failed check, nothing else checked
    let o = run_assume(&cs, req(Some(None), Some(&HDRS[..80 * 200]), Some(SNAP_200), None));
    assert!(o.checks.len() == 1 && !o.checks[0].0 && o.valid.is_none() && o.utxo.is_none(), "{:?}", o.checks);
    // a snapshot without headers; a tracked list that does not parse
    let o = run_assume(&cs, req(None, None, Some(SNAP_200), None));
    assert!(o.checks.len() == 1 && !o.checks[0].0 && o.checks[0].1.contains("required"), "{:?}", o.checks);
    let o = run_assume(&cs, req(None, Some(HDRS), Some(SNAP_200), Some("not a line")));
    assert!(o.checks.iter().any(|c| !c.0 && c.1.contains("tracked list")) && o.utxo.is_none(), "{:?}", o.checks);
    // a block not in the chain: that check fails, the snapshot is still checked
    let o = run_assume(&cs, req(Some(Some([7; 32])), Some(HDRS), Some(SNAP_200), None));
    assert!(!o.checks[1].0 && o.checks[2].0 && o.valid.is_none() && o.utxo.is_some(), "{:?}", o.checks);
}

/// The byte a truncation names counts the bytes of every raw script read before it.
#[test]
fn truncation_after_a_raw_script() {
    let g1 = group([1; 32], &[0], 2, 0, &raw_script(5));
    let e = read_snapshot(&snap(2, &[&g1[..], &[2u8; 10]].concat())[..], |_| Ok(())).unwrap_err();
    assert!(e.contains(&format!("truncated at byte {} ", 51 + g1.len())), "{e}");
}

/// The network a snapshot must name: mainnet's message start, signet's, and under the tests-only regtest-test feature
/// (zapps/corecheck's regtest image, profile bit 3) Core's regtest pchMessageStart.
#[test]
fn network_magic_by_profile() {
    assert_eq!(network_magic(0x71), MAINNET);
    assert_eq!(network_magic(0x71 | swiftsync_verify::PROFILE_SIGNET), [0x0a, 0x03, 0xcf, 0x40]);
    #[cfg(feature = "regtest-test")]
    assert_eq!(network_magic(0x79), [0xfa, 0xbf, 0xb5, 0xda]);
    #[cfg(not(feature = "regtest-test"))]
    assert_eq!(network_magic(0x79), MAINNET, "no regtest without the tests-only feature");
}
