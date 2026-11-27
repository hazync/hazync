//! The assume zAPPs' fuzz properties (docs/libzync/ZAPP_TESTING.md §3.4, §4.4), one function per target, so the
//! fuzz targets and the stable corpus replay (tests/replay.rs) run the same code. A property that fails panics.
//!
//! - `snapshot_decode`: any bytes into `read_snapshot`: no panic, no allocation beyond the input; a snapshot it accepts
//!   re-encodes (Core's encodings, written here from Core's serialize.h / compressor.cpp, not from the reader) to the
//!   same bytes, and the re-encoding reads back as the same coins.
//! - `snapshot_check`: mutations of Core's real dumptxoutset at 200 checked against the real 1..200 proof: OK only for
//!   the exact file.
//! - `side_inputs`: the headers file, the tracked list and the assumevalid block against the real proof: a header run
//!   that is accepted is a suffix of the real headers, an assumevalid answer names the real block at that height, and a
//!   snapshot accepted with a tracked list never has a listed outpoint among its coins (none is tracked at 200).
//! - `core_receipt`: mutations of the real core proof (the engine chain's, mainnet 1..200) through
//!   zapp_corecheck::verify_core: a verdict other than FAIL only for exactly the real file.

use std::sync::OnceLock;
use swiftsync_verify::{ChainState, Image, Inputs, Verdict};
use zapp_assume::{HeaderChain, SnapCoin, SnapshotMeta};

pub const RCPT: &[u8] = include_bytes!("../../../../swiftsync-verify/vectors/mainnet_1_200_engine.hze");
const VK: &[u8] = include_bytes!("../../../../swiftsync-verify/vectors/mainnet_1_200_engine.vk");
const ID: &str = include_str!("../../../../swiftsync-verify/vectors/mainnet_1_200_engine.image_id");
pub const SNAP_200: &[u8] = include_bytes!("../../vectors/utxo_200.dat");
pub const HDRS: &[u8] = include_bytes!("../../vectors/headers_0_200.bin");
pub const TIP_200: &str = "000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320";
pub const NOW: u64 = 1_800_000_000;

/// The engine chain's mainnet 1..200 image with its key, once.
pub fn image() -> &'static Image {
    static I: OnceLock<&'static Image> = OnceLock::new();
    I.get_or_init(|| Image::by_network("mainnet").unwrap().with_method_id(ID.trim()).unwrap().with_engine_key_unchecked(VK.to_vec()))
}

/// The real proof's verified state (DEMO: below minimum work), once.
pub fn state() -> &'static ChainState {
    static S: OnceLock<ChainState> = OnceLock::new();
    S.get_or_init(|| {
        let r = swiftsync_verify::verify(&Inputs { receipt: RCPT, image: image(), anchor: None, now: NOW });
        assert_eq!(r.verdict, Verdict::Demo, "{:?}", r.checks);
        r.state.unwrap()
    })
}

fn headers() -> &'static HeaderChain {
    static H: OnceLock<HeaderChain> = OnceLock::new();
    H.get_or_init(|| HeaderChain::bind(HDRS, &state().tip_hash, state().height).unwrap())
}

fn real_coins() -> &'static Vec<SnapCoin> {
    static C: OnceLock<Vec<SnapCoin>> = OnceLock::new();
    C.get_or_init(|| {
        let mut v = Vec::new();
        zapp_assume::read_snapshot(SNAP_200, |c| { v.push(c.clone()); Ok(()) }).unwrap();
        v
    })
}

// ---- Core's encodings, written from Core (serialize.h WriteCompactSize / WriteVarInt, compressor.cpp) -------------

fn compact_size(o: &mut Vec<u8>, n: u64) {
    match n {
        0..=252 => o.push(n as u8),
        253..=0xffff => { o.push(253); o.extend_from_slice(&(n as u16).to_le_bytes()) }
        0x1_0000..=0xffff_ffff => { o.push(254); o.extend_from_slice(&(n as u32).to_le_bytes()) }
        _ => { o.push(255); o.extend_from_slice(&n.to_le_bytes()) }
    }
}

/// WriteVarInt, VarIntMode::DEFAULT.
fn var_int(o: &mut Vec<u8>, mut n: u64) {
    let mut tmp = [0u8; 10];
    let mut len = 0usize;
    loop {
        tmp[len] = (n & 0x7f) as u8 | if len > 0 { 0x80 } else { 0 };
        if n <= 0x7f { break; }
        n = (n >> 7) - 1;
        len += 1;
    }
    for i in (0..=len).rev() { o.push(tmp[i]); }
}

/// CompressAmount.
fn compress_amount(mut n: u64) -> u64 {
    if n == 0 { return 0; }
    let mut e = 0;
    while n % 10 == 0 && e < 9 { n /= 10; e += 1; }
    if e < 9 {
        let d = n % 10;
        n /= 10;
        1 + (n * 9 + d - 1) * 10 + e
    } else {
        1 + (n - 1) * 10 + 9
    }
}

/// secp256k1: y^2 = x^3 + 7 (mod p), for CompressScript's IsFullyValid on an uncompressed key.
fn on_curve(x: &[u8], y: &[u8]) -> bool {
    use num_bigint::BigUint;
    let p = BigUint::parse_bytes(b"fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f", 16).unwrap();
    let (x, y) = (BigUint::from_bytes_be(x), BigUint::from_bytes_be(y));
    x < p && y < p && (&y * &y) % &p == (&x * &x * &x + 7u32) % &p
}

/// ScriptCompression::Ser (CompressScript, else the raw script with VARINT(size + 6)).
fn script(o: &mut Vec<u8>, s: &[u8]) {
    if s.len() == 25 && s[..3] == [0x76, 0xa9, 20] && s[23..] == [0x88, 0xac] {
        o.push(0); o.extend_from_slice(&s[3..23]);
    } else if s.len() == 23 && s[..2] == [0xa9, 20] && s[22] == 0x87 {
        o.push(1); o.extend_from_slice(&s[2..22]);
    } else if s.len() == 35 && s[0] == 33 && (s[1] == 2 || s[1] == 3) && s[34] == 0xac {
        o.push(s[1]); o.extend_from_slice(&s[2..34]);
    } else if s.len() == 67 && s[0] == 65 && s[1] == 4 && s[66] == 0xac && on_curve(&s[2..34], &s[34..66]) {
        o.push(4 | (s[65] & 1)); o.extend_from_slice(&s[2..34]);
    } else {
        var_int(o, s.len() as u64 + 6);
        o.extend_from_slice(s);
    }
}

/// A snapshot file as Core's dumptxoutset writes it (one group per txid run).
pub fn encode_snapshot(m: &SnapshotMeta, coins: &[SnapCoin]) -> Vec<u8> {
    let mut o = Vec::new();
    o.extend_from_slice(&zapp_assume::SNAPSHOT_MAGIC);
    o.extend_from_slice(&m.version.to_le_bytes());
    o.extend_from_slice(&m.network_magic);
    o.extend_from_slice(&m.base_blockhash);
    o.extend_from_slice(&m.coins_count.to_le_bytes());
    let mut i = 0;
    while i < coins.len() {
        let j = coins[i..].iter().position(|c| c.txid != coins[i].txid).map_or(coins.len(), |k| i + k);
        o.extend_from_slice(&coins[i].txid);
        compact_size(&mut o, (j - i) as u64);
        for c in &coins[i..j] {
            compact_size(&mut o, c.vout as u64);
            var_int(&mut o, ((c.height as u64) << 1) | c.coinbase as u64);
            var_int(&mut o, compress_amount(c.value));
            script(&mut o, &c.script_pubkey);
        }
        i = j;
    }
    o
}

fn read_all(b: &[u8]) -> Result<(SnapshotMeta, Vec<SnapCoin>), String> {
    let mut v = Vec::new();
    let m = zapp_assume::read_snapshot(b, |c| { v.push(c.clone()); Ok(()) })?;
    Ok((m, v))
}

// ---- the properties ---------------------------------------------------------------------------------------------

pub fn snapshot_decode(data: &[u8]) {
    let Ok((m, coins)) = read_all(data) else { return };
    assert_eq!(m.coins_count, coins.len() as u64, "accepted a count that is not the coins read");
    let enc = encode_snapshot(&m, &coins);
    let back = read_all(&enc).unwrap_or_else(|e| panic!("ROUNDTRIP: Core's encoding of the accepted coins is refused: {e}"));
    assert_eq!(back, (m, coins), "ROUNDTRIP: Core's encoding reads back as other coins");
    assert!(enc == data, "NONCANONICAL: accepted a snapshot that is not Core's encoding of its coins ({} bytes, Core's {})", data.len(), enc.len());
}

pub fn snapshot_check(data: &[u8]) {
    let cs = state();
    let r = zapp_assume::check_snapshot(cs, data, headers(), &Default::default());
    if r.is_ok() {
        if data != SNAP_200 {
            let coins = read_all(data).map(|x| x.1);
            panic!("WRONG-OK: a snapshot other than Core's file at 200 is the proven UTXO set (same coins: {:?})",
                   coins.as_ref().map(|c| c == real_coins()));
        }
    }
}

fn real_hashes() -> &'static Vec<[u8; 32]> {
    static H: OnceLock<Vec<[u8; 32]>> = OnceLock::new();
    H.get_or_init(|| headers().hashes.clone())
}

/// `[u32 LE n][n bytes of headers][text]`; the text's first line `av:tip` or `av:<hash>` asks assumevalid, the rest
/// is the tracked list.
pub fn side_inputs(data: &[u8]) {
    if data.len() < 4 { return; }
    let n = (u32::from_le_bytes(data[..4].try_into().unwrap()) as usize).min(data.len() - 4);
    let hdr = &data[4..4 + n];
    let Ok(text) = std::str::from_utf8(&data[4 + n..]) else { return };
    let (av, tracked) = match text.split_once('\n') {
        Some((first, rest)) if first.starts_with("av:") => (Some(first), rest),
        _ if text.starts_with("av:") => (Some(text), ""),
        _ => (None, text),
    };
    let av = match av.map(|a| &a[3..]) {
        None => None,
        Some("tip") => Some(None),
        Some(h) => match zapp_assume::parse_display_hash(h) { Some(x) => Some(Some(x)), None => return },
    };
    let cs = state();
    let o = zapp_assume::run_assume(cs, zapp_assume::AssumeRequest {
        assumevalid: av,
        headers: Some(hdr).filter(|h| !h.is_empty()),
        snapshot: Some(SNAP_200),
        tracked: Some(tracked).filter(|t| !t.is_empty()),
    });
    let suffix = hdr.is_empty() || (hdr.len() % 80 == 0 && HDRS.ends_with(hdr));
    if let Some(v) = &o.valid {
        assert!(v.height <= cs.height && real_hashes().get(v.height as usize) == Some(&v.block),
                "WRONG-OK: assumevalid says block {} is at height {}", zapp_assume::display(&v.block), v.height);
        assert!(suffix, "WRONG-OK: assumevalid accepted with headers that are not the real chain's");
    }
    if o.utxo.is_some() {
        assert!(suffix, "WRONG-OK: assumeutxo accepted with headers that are not the real chain's");
        if let Ok(t) = zapp_assume::parse_tracked(tracked) {
            assert!(!real_coins().iter().any(|c| t.contains(&(c.txid, c.vout))),
                    "WRONG-OK: assumeutxo accepted a coin listed as BIP30-tracked (none is at 200)");
        }
    }
}

pub fn core_receipt(data: &[u8]) {
    let expect = format!("200:{TIP_200}");
    let c = zapp_corecheck::verify_core(data, image(), None, NOW, Some(&expect));
    if c.report.verdict != Verdict::Fail {
        // an engine proof file has no slack: bytes after it are journal bytes, which the public values refuse
        assert!(data == RCPT, "WRONG-OK: a file other than the real proof verifies ({:?}, {} bytes)", c.report.verdict, data.len());
    }
}
