//! The BIP30-tracked flag (coin leaf flag bit 1, docs/libzync/UTREEXO_FORMAT.md §2a), derived on the host as the guest
//! derives it, instead of read from the block file.
//!
//! The block files of the deployed pipeline (`prover/fetch_block_rpc.py`) carry no `coinbase_flags` and only bit 0
//! (coinbase) in a prevout's `coin_is_coinbase`: an RPC node does not know which coins the BIP30 tree tracks. The host
//! then gave tracked coins no bit 1 while the guest sets it itself for a coinbase (`agg_lite.rs`: mined before BIP34 and
//! indicating a later height, or the repeat of a REPEATABLE one). From mainnet block 147,396 (coinbase `cec2a159…`, the
//! first tracked coin) prep's coin log, live-coin hash and block forest were not what the guest commits, so no leaf
//! proved on prep's state at 147,396 or later could join the leaf of 147,396 (#721 follow-up, 15.11.2026).
//!
//! [`annotate`] writes the derived flags into the block JSON, the shape the core-rules rig and `mkblock.py` already
//! write, so every reader of the JSON (prep, a leaf bundle's block, `build_full`) sees the guest's flags:
//! - `coinbase_flags` = 1 | 2·(tracked), from the coinbase bytes and the BIP30 tree before this block;
//! - each prevout's `coin_is_coinbase` bit 1: for a coin this block creates, its transaction's flag; for an earlier
//!   coin, whether the coin log holds the coin's leaf WITH bit 1 (the log is the chain's, so its leaf is the answer).
//!   Only coins whose txid the BIP30 tree holds can be tracked, so other coins cost no coin-log lookup.
//! A transaction is tracked when it has inputs and all of them are tracked coins (`agg_lite.rs` `tracked_tx`).
//! A flag the file already carries must agree, or the block is refused (a rig file is checked, never trusted).

use crate::Smt;
use bitcoin::consensus::deserialize;
use bitcoin::hashes::Hash as _;
use bitcoin::Transaction;
use hazync_coinbase_smt::bip30::{REPEATABLE, REPEAT_HORIZON};

/// The first block that creates a tracked coin: mainnet 147,396 (`cec2a159…`, reproduce/libzync-tracked.json, scanned
/// 15.11.2026). Below it the flags the old host gave are the guest's; from it on they are not. Signet has none (BIP34
/// from block 1, so no coinbase indicates a later height before it); regtest (the rig) may have one anywhere.
pub(crate) fn first_tracked_height() -> u32 {
    match methods::NETWORK { "signet" => u32::MAX, "regtest" => 1, _ => 147_396 }
}

/// The save-point marker of a prep that derives the tracked flags (`state_<h>.flags` beside `state_<h>.json`). A prep
/// continues from a state at or above [`first_tracked_height`] only with it: a store written by a host that read the
/// flags from the block files (before 15.11.2026) is wrong from there on, and nothing in its files says so otherwise.
pub(crate) const FLAGS_MARK: &str = "T1\n";

pub(crate) fn flags_mark_path(state_json: &str) -> String {
    format!("{}.flags", state_json.strip_suffix(".json").unwrap_or(state_json))
}

/// Refuses to continue from `state_json` at `height` when it lacks the marker (see [`FLAGS_MARK`]).
pub(crate) fn check_flags_mark(state_json: &str, height: u32) {
    if height < first_tracked_height() { return; }
    let m = std::fs::read_to_string(flags_mark_path(state_json)).unwrap_or_default();
    assert!(m == FLAGS_MARK, "{state_json}: no tracked-flag marker ({}): this state is at {height}, at or above the first \
            tracked coin ({}), and was written by a host that took the BIP30-tracked flag from the block files, so its coin \
            log, live-coin hash and block forest are not the guest's. Prep again from below {} (from genesis)",
            flags_mark_path(state_json), first_tracked_height(), first_tracked_height());
}

/// A leaf bundle's block JSON at or above [`first_tracked_height`] must carry the derived flags (`coinbase_flags`, which
/// [`annotate`] always writes): a bundle cut by a prep that read them from the file cannot be proved by mistake.
pub(crate) fn check_bundle_json(height: u32, block_json: &[u8]) {
    if height < first_tracked_height() { return; }
    let j: serde_json::Value = serde_json::from_slice(block_json).expect("leaf bundle: the block JSON does not parse");
    assert!(j.get("coinbase_flags").is_some(), "leaf bundle for block {height}: its block JSON has no coinbase_flags, so it was \
            cut by a prep that read the BIP30-tracked flag from the block files; its witnesses are not the guest's. Cut it again");
}

/// The network's BIP34 height (libzync/src/net.rs).
pub(crate) fn bip34_height() -> u32 {
    match methods::NETWORK { "signet" => libzync::net::SIGNET.bip34_height, "regtest" => libzync::net::REGTEST.bip34_height,
                             _ => libzync::net::MAINNET.bip34_height }
}

/// The height a coinbase's scriptSig indicates under BIP34's encoding: its scriptSig must begin with exactly the bytes
/// of Core's `CScript() << Y`, Y read from the first push (OP_1..OP_16, or a push of 1-5 bytes, little-endian). 0 when
/// it indicates nothing; u32::MAX above 2^31 - 1. The guest's `hz_indicated_height` (verify_input.cpp), line for line.
pub(crate) fn indicated_height(coinbase: &Transaction) -> u32 {
    let Some(ss) = coinbase.input.first().map(|i| i.script_sig.as_bytes()) else { return 0 };
    if ss.is_empty() {
        return 0;
    }
    let op = ss[0] as usize;
    let v: i64 = if (0x51..=0x60).contains(&op) {
        op as i64 - 0x50
    } else if (1..=5).contains(&op) && ss.len() > op {
        let mut v = 0i64;
        for i in (1..=op).rev() {
            v = (v << 8) | ss[i] as i64;
        }
        v
    } else {
        return 0;
    };
    let expect = bitcoin::script::Builder::new().push_int(v).into_script();
    if ss.len() < expect.len() || &ss[..expect.len()] != expect.as_bytes() {
        return 0;
    }
    if v > 0x7fff_ffff { u32::MAX } else { v as u32 }
}

/// The coinbase's leaf flags at `height`: bit 0, and bit 1 when the guest tracks it. `prior` is the coinbase txid's
/// slot in the BIP30 tree before this block; the two grandfathered overwrites (91,842 / 91,880) are never a repeat.
pub(crate) fn coinbase_flags(height: u32, bip34_height: u32, coinbase: &Transaction, prior: Option<u32>, grandfathered: bool) -> u32 {
    let y = indicated_height(coinbase);
    let root = height < bip34_height && y > height && y < REPEAT_HORIZON;
    let repeat = !grandfathered && prior == Some(REPEATABLE);
    1 | ((root || repeat) as u32) << 1
}

/// `j` with `coinbase_flags` and every prevout's `coin_is_coinbase` bit 1 set as the guest derives them, and the txids
/// of this block's tracked transactions (coinbase first). `smt`: the BIP30 tree BEFORE this block.
/// `tracked_coin(txid, vout, leaf)`: whether an earlier block's coin is tracked; `leaf` is its coin leaf WITH bit 1, so a
/// caller holding the coin log answers `log holds leaf`, and one replaying from genesis answers from its tracked txids.
pub(crate) fn annotate(j: &serde_json::Value, smt: &Smt, bip34_height: u32, grandfathered: bool,
                       tracked_coin: &dyn Fn(&[u8; 32], u32, &[u8; 32]) -> bool) -> (serde_json::Value, Vec<[u8; 32]>) {
    let mut j = j.clone();
    let height = j["height"].as_u64().expect("block JSON: height") as u32;
    let coinbase: Transaction = deserialize(&crate::hx(j["coinbase_hex"].as_str().expect("block JSON: coinbase_hex"))).expect("coinbase");
    let cb_txid = coinbase.compute_txid().to_byte_array();
    let cbf = coinbase_flags(height, bip34_height, &coinbase, smt.get(&cb_txid), grandfathered);
    if let Some(f) = j.get("coinbase_flags").and_then(|v| v.as_u64()) {
        assert_eq!(f as u32, cbf, "block {height}: the file's coinbase_flags {f} is not the guest's {cbf}");
    }
    j["coinbase_flags"] = cbf.into();
    // this block's transactions so far: txid -> its outputs' flags (bit 1 = tracked)
    let mut created: std::collections::HashMap<[u8; 32], u32> = std::collections::HashMap::new();
    created.insert(cb_txid, cbf);
    let mut tracked_txids: Vec<[u8; 32]> = if cbf & 2 != 0 { vec![cb_txid] } else { Vec::new() };
    for t in j["txs"].as_array_mut().expect("block JSON: txs") {
        let tx: Transaction = deserialize(&crate::hx(t["raw"].as_str().expect("tx raw"))).expect("tx");
        let txid = tx.compute_txid().to_byte_array();
        let prevouts = t["prevouts"].as_array_mut().expect("tx prevouts");
        assert_eq!(prevouts.len(), tx.input.len(), "block {height}: prevouts do not match the inputs");
        let mut all_tracked = !tx.input.is_empty();
        for (inp, p) in tx.input.iter().zip(prevouts.iter_mut()) {
            let op = inp.previous_output;
            let ptxid = op.txid.to_byte_array();
            let file = p["coin_is_coinbase"].as_u64().expect("prevout coin_is_coinbase") as u32;
            let bit = if let Some(f) = created.get(&ptxid) {
                f & 2
            } else if smt.get(&ptxid).is_some() {
                let leaf = hazync_utreexo::coin_leaf_flags(&ptxid, op.vout, p["value"].as_u64().expect("prevout value"),
                    &crate::hx(p["spk"].as_str().expect("prevout spk")), p["coin_height"].as_u64().expect("prevout coin_height") as u32,
                    ((file & 1) | 2) as u8, p["coin_mtp"].as_u64().expect("prevout coin_mtp") as u32);
                if tracked_coin(&ptxid, op.vout, &leaf) { 2 } else { 0 }
            } else {
                0
            };
            assert!(file & 2 == 0 || bit == 2, "block {height}: the file marks prevout {}:{} tracked; the coin log does not",
                    crate::hex(&{ let mut r = ptxid; r.reverse(); r }), op.vout);
            p["coin_is_coinbase"] = ((file & 1) | bit).into();
            all_tracked &= bit == 2;
        }
        created.insert(txid, (all_tracked as u32) << 1);
        if all_tracked { tracked_txids.push(txid); }
    }
    (j, tracked_txids)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CB_147396: &str = "01000000010000000000000000000000000000000000000000000000000000000000000000ffffffff0a04cd9a830002c3050105ffffffff044605f108000000001976a914f27b35cd5c2b18e326f1e3fcad8f9fa2184112c188ac7e645810000000001976a9146a61a4a4b25c46b819636ba236f648a09382fe9388acd141a54a000000001976a914c02052ef85d5c7144c6bcc32d047897840efbf2d88ac3b241ec6000000001976a914dd34473bf0607dcb0483e92e6b3422a86979300588ac00000000";

    fn cb_with(script_sig: &str) -> Transaction {
        // a coinbase: version 1, one null input with this scriptSig, one output
        let ss = crate::hx(script_sig);
        let mut raw = vec![1, 0, 0, 0, 1];
        raw.extend([0u8; 32]);
        raw.extend([0xff; 4]);
        raw.push(ss.len() as u8);
        raw.extend(&ss);
        raw.extend([0xff; 4]);
        raw.push(1);
        raw.extend(5_000_000_000u64.to_le_bytes());
        raw.extend([1, 0x51]);
        raw.extend([0; 4]);
        deserialize(&raw).unwrap()
    }

    fn tx_spending(prev: &[([u8; 32], u32)]) -> Transaction {
        use bitcoin::{absolute::LockTime, transaction::Version, Amount, OutPoint, ScriptBuf, Sequence, TxIn, TxOut, Txid, Witness};
        Transaction {
            version: Version::ONE, lock_time: LockTime::ZERO,
            input: prev.iter().map(|(t, v)| TxIn { previous_output: OutPoint { txid: Txid::from_byte_array(*t), vout: *v },
                script_sig: ScriptBuf::new(), sequence: Sequence::MAX, witness: Witness::new() }).collect(),
            output: vec![TxOut { value: Amount::from_sat(1000), script_pubkey: ScriptBuf::from_bytes(vec![0x51]) }; 2],
        }
    }

    fn prevout(cb: u32) -> serde_json::Value {
        serde_json::json!({"value": 1000, "spk": "51", "coin_height": 100, "coin_is_coinbase": cb, "coin_mtp": 7})
    }

    #[test]
    fn annotate_sets_the_guests_flags() {
        use bitcoin::consensus::serialize;
        let cb: Transaction = deserialize(&crate::hx(CB_147396)).unwrap();
        let cb_txid = cb.compute_txid().to_byte_array();
        let (old_t, old_u) = ([0x11u8; 32], [0x22u8; 32]); // earlier coins: T tracked, U not
        let mut smt = Smt::new();
        smt.insert(old_t, 2);
        smt.insert(old_u, 1);
        let t1 = tx_spending(&[(old_t, 0)]); // all inputs tracked: tracked
        let t2 = tx_spending(&[(t1.compute_txid().to_byte_array(), 0)]); // in-block spend of t1: tracked
        let t3 = tx_spending(&[(old_t, 1), (old_u, 0)]); // one untracked input: not tracked
        let j = serde_json::json!({"height": 147_396, "coinbase_hex": crate::hex(&serialize(&cb)), "txs": [
            {"raw": crate::hex(&serialize(&t1)), "prevouts": [prevout(1)]},
            {"raw": crate::hex(&serialize(&t2)), "prevouts": [prevout(0)]},
            {"raw": crate::hex(&serialize(&t3)), "prevouts": [prevout(1), prevout(0)]}]});
        let (a, new) = annotate(&j, &smt, 227_931, false, &|t, _, _| *t == old_t);
        assert_eq!(a["coinbase_flags"], 3);
        let flags = |k: usize| a["txs"][k]["prevouts"].as_array().unwrap().iter().map(|p| p["coin_is_coinbase"].as_u64().unwrap()).collect::<Vec<_>>();
        assert_eq!((flags(0), flags(1), flags(2)), (vec![3], vec![2], vec![3, 0]));
        assert_eq!(new, vec![cb_txid, t1.compute_txid().to_byte_array(), t2.compute_txid().to_byte_array()]);
        // after BIP34 the same coinbase is not tracked; a coin the BIP30 tree does not hold is never looked up
        let mut k = j.clone();
        k["height"] = 300_000.into();
        let (a, _) = annotate(&k, &Smt::new(), 227_931, false, &|_, _, _| panic!("looked up a coin the tree does not hold"));
        assert_eq!((a["coinbase_flags"].as_u64(), a["txs"][0]["prevouts"][0]["coin_is_coinbase"].as_u64()), (Some(1), Some(1)));
        // a file's flags are checked, never trusted
        let mut bad = j.clone();
        bad["coinbase_flags"] = 1.into();
        assert!(std::panic::catch_unwind(|| annotate(&bad, &smt, 227_931, false, &|t, _, _| *t == old_t)).is_err());
        let mut bad = j.clone();
        bad["txs"][2]["prevouts"][1]["coin_is_coinbase"] = 2.into();
        assert!(std::panic::catch_unwind(|| annotate(&bad, &smt, 227_931, false, &|t, _, _| *t == old_t)).is_err());
    }

    #[test]
    fn old_states_and_bundles_are_refused_from_the_first_tracked_coin() {
        if methods::NETWORK != "mainnet" { return; }
        let d = std::env::temp_dir().join(format!("hz-tracked-mark-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let st = d.join("state_147396.json").to_string_lossy().to_string();
        check_flags_mark(&st, 147_395); // below: no marker needed
        assert!(std::panic::catch_unwind(|| check_flags_mark(&st, 147_396)).is_err(), "no marker at 147,396");
        std::fs::write(flags_mark_path(&st), FLAGS_MARK).unwrap();
        check_flags_mark(&st, 147_396);
        std::fs::write(flags_mark_path(&st), "U0\n").unwrap();
        assert!(std::panic::catch_unwind(|| check_flags_mark(&st, 600_000)).is_err(), "another marker");
        let _ = std::fs::remove_dir_all(&d);
        let old = br#"{"height": 147396, "txs": []}"#;
        check_bundle_json(147_395, old);
        assert!(std::panic::catch_unwind(|| check_bundle_json(147_396, old)).is_err());
        check_bundle_json(147_396, br#"{"height": 147396, "coinbase_flags": 3, "txs": []}"#);
    }

    #[test]
    fn indicated_heights_as_the_guest() {
        // mainnet 147,396's real coinbase, the first tracked coin: a 4-byte push cd9a8300 (the 0x00 is CScriptNum's sign
        // pad), 8,624,845 > 147,396; txid cec2a159… (reproduce/libzync-tracked.json)
        let real: Transaction = deserialize(&crate::hx(CB_147396)).unwrap();
        let mut id = real.compute_txid().to_byte_array();
        id.reverse();
        assert_eq!(crate::hex(&id), "cec2a159481a406d84da03cf5de1cd4b8b9cedc4eef8e5b6e13eeaa34a023907");
        assert_eq!(indicated_height(&real), 8_624_845);
        assert_eq!(coinbase_flags(147_396, 227_931, &real, None, false), 3);
        // another tracked one (a9f87755…, block ~170k): a push of 1f7e3201, 20,086,303
        assert_eq!(indicated_height(&cb_with("041f7e3201")), 20_086_303);
        // the common early form `04 ffff001d …` reads as 0x1d00ffff: not below the horizon
        assert_eq!(indicated_height(&cb_with("04ffff001d0104")), 0x1d00ffff);
        assert_eq!(indicated_height(&cb_with("51")), 1); // OP_1
        assert_eq!(indicated_height(&cb_with("03a08601")), 100_000); // BIP34's own form
        assert_eq!(indicated_height(&cb_with("04a0860100")), 0, "not minimal: CScript() << 100000 is 03a08601");
        assert_eq!(indicated_height(&cb_with("0180")), 0, "0x80 alone is -0: CScript() << 128 is 028000");
        assert_eq!(indicated_height(&cb_with("028000")), 128);
        assert_eq!(indicated_height(&cb_with("06aabbccddeeff")), 0, "a 6-byte push indicates nothing");
        assert_eq!(indicated_height(&cb_with("")), 0);
        let f = |h, ss: &str, prior, g| coinbase_flags(h, 227_931, &cb_with(ss), prior, g);
        assert_eq!(f(147_396, "041f7e3201", None, false), 3);
        assert_eq!(f(227_931, "041f7e3201", None, false), 1, "from BIP34 on, no root");
        assert_eq!(f(20_086_303, "041f7e3201", None, false), 1, "at the indicated height itself, not tracked");
        assert_eq!(f(1000, "04ffff001d0104", None, false), 1);
        assert_eq!(f(1000, "04ffff001d0104", Some(REPEATABLE), false), 3, "the repeat of a REPEATABLE coinbase");
        assert_eq!(f(91_880, "04ffff001d0104", Some(REPEATABLE), true), 1, "a grandfathered overwrite is no repeat");
    }
}
