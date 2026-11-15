//! liveness-anon (EXPERIMENT, not a published zyncAPP; docs/libzync/LIVENESS_ZAPP.md §6): proves, from a verified core
//! receipt, "I know a coin UNSPENT at the core tip, worth at least `min_value`, whose scriptPubKey satisfies `policy`",
//! and commits neither the coin nor its amount nor its script.
//!
//! Policies:
//!   0 (set): the script is a leaf of the Merkle tree `set_root` of depth `set_depth` (both committed): leaf =
//!     SHA-256(0x00 ‖ len u32 LE ‖ spk), node = SHA-256(0x01 ‖ L ‖ R), the path exactly `set_depth` long. The tags keep a
//!     node from passing as a leaf, and the committed depth fixes the tree (review 15.11.2026: an untagged leaf and a
//!     free path length let a node be presented as a "script"). No ownership is shown: anyone who knows such a coin can
//!     prove it, and there is no nullifier.
//!   1 (key): the coin is P2TR and the prover knows the secret of its output key (x(d·G) = the key in the script). The
//!     x-only key is met by d and n−d alike, so the secret is first normalised to the one whose point has even Y (as
//!     BIP340 does); nullifier = SHA-256("lzlv-anon/nullifier" ‖ context ‖ d_even ‖ txid ‖ vout): one per coin and
//!     context (review 15.11.2026: hashing d itself gave two), unlinkable to the coin without the secret.
//!
//! The core statement is checked against the network's PINNED genesis anchor (as every zAPP does). What the guest
//! cannot know, its VERIFIER must check from the journal: `core_id` is a core image it trusts (an allow-list) and
//! `lineage` is that image's; `cum_work` reaches the network's minimum chainwork and `tip_hash` is the network's block
//! at `height` (chain identity, from a source the verifier trusts); `max_time` is not in the future; `context` is the
//! one the verifier expects (its own, fixed in advance: a context the prover picks makes the nullifier worthless);
//! `min_value`, `policy`, `set_root` / `set_depth` are the ones it asked for.
//! `bench_key_only`: run the key policy's own work (sk·G, the nullifier) on its own and stop, for its cycle count.
#![no_main]
extern crate alloc;
use alloc::vec::Vec;
use risc0_zkvm::guest::env;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

risc0_zkvm::guest::entry!(main);

#[path = "../../../../../../libzync/src/coinlog.rs"]
mod coinlog;
#[path = "../../../../../../libzync/src/facts.rs"]
#[allow(dead_code)]
mod facts;

/// The core ChainState prefix (prover/methods/guest/src/main.rs), field for field.
#[derive(Deserialize)]
#[allow(dead_code)]
struct ChainState {
    kind: u32, version: u32, tip_hash: [u8; 32], utxo_roots: Vec<Option<[u8; 32]>>, utxo_leaves: u64, utxo_value: u64,
    cum_work: [u8; 32], height: u32, prev_nbits: u32, prev_time: u32, epoch_start: u32, recent_times: Vec<u32>,
    anchor_id: [u8; 32], self_id: [u32; 8], live_set: u32, live_hash: [u32; 16], trie_root: [u8; 32], smt_root: [u8; 32],
    max_time: u32, profile: u32, block_roots: Vec<Option<[u8; 32]>>, block_leaves: u64, lineage: [u8; 32],
}

#[derive(Deserialize)]
struct Coin { txid: [u8; 32], vout: u32, value: u64, script_pubkey: Vec<u8>, height: u32, coinbase: bool, tracked: bool, creation_mtp: u32 }

#[derive(Deserialize)]
struct Input {
    core_id: [u32; 8],
    journal: Vec<u8>,
    signet: bool,
    policy: u32,
    min_value: u64,
    set_root: [u8; 32],
    set_depth: u32,
    context: [u8; 32],
    // private from here on: never committed
    coin: Coin,
    pos: u64,
    path: Vec<[u8; 32]>,
    set_index: u64,
    set_path: Vec<[u8; 32]>,
    sk: [u8; 32],
    bench_key_only: bool,
}

#[derive(Serialize)]
struct AnonLive {
    kind: u32,
    core_id: [u32; 8],
    lineage: [u8; 32],
    anchor_id: [u8; 32],
    height: u32,
    tip_hash: [u8; 32],
    cum_work: [u8; 32],
    max_time: u32,
    profile: u32,
    policy: u32,
    min_value: u64,
    set_root: [u8; 32],
    set_depth: u32,
    context: [u8; 32],
    nullifier: [u8; 32],
}

/// The networks' genesis anchors (swiftsync-verify IMAGES `genesis_anchor_hex`, SOUNDNESS.md §3).
const ANCHOR_MAINNET: &str = "852c5c7e009317c2dd232dcf9b1a9e2babd58df324add461a6ff61824f390167";
const ANCHOR_SIGNET: &str = "3b67ead0618b7786b001525b68c7b088405d6a577292838361cc07e5bb938827";

fn anchor(hex: &str) -> [u8; 32] {
    core::array::from_fn(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
}

/// Key-bench output: the x-only key and the nullifier, so a host can check d and n−d agree.
#[derive(Serialize)]
struct KeyBench {
    x: [u8; 32],
    nullifier: [u8; 32],
}

const KIND_ANON_LIVE: u32 = 0x5A4C_0001;

fn mark(stage: &str, from: u64) -> u64 {
    let now = env::cycle_count();
    env::log(&alloc::format!("STAGE {stage} {}", now - from));
    now
}

/// The key policy: (x-only output key, nullifier).
fn key_work(sk: &[u8; 32], context: &[u8; 32], txid: &[u8; 32], vout: u32) -> ([u8; 32], [u8; 32]) {
    use k256::elliptic_curve::sec1::ToEncodedPoint;
    use k256::elliptic_curve::PrimeField;
    let d = Option::<k256::Scalar>::from(k256::Scalar::from_repr((*sk).into())).expect("sk is not a scalar");
    assert!(!bool::from(k256::elliptic_curve::Field::is_zero(&d)), "sk is zero");
    let p = (k256::ProjectivePoint::GENERATOR * d).to_affine().to_encoded_point(true);
    let x: [u8; 32] = p.as_bytes()[1..33].try_into().unwrap();
    // the even-Y secret: d if d·G has even Y (tag 0x02), else n − d. Both meet the same x-only key; only this one is hashed.
    let d_even = if p.as_bytes()[0] == 0x02 { d } else { -d };
    let d_even: [u8; 32] = d_even.to_repr().into();
    let mut h = Sha256::new();
    h.update(b"lzlv-anon/nullifier");
    h.update(context);
    h.update(d_even);
    h.update(txid);
    h.update(vout.to_le_bytes());
    (x, h.finalize().into())
}

fn main() {
    let t0 = env::cycle_count();
    let inp: Input = env::read();
    let mut t = mark("read", t0);
    if inp.bench_key_only {
        let (x, nullifier) = key_work(&inp.sk, &inp.context, &inp.coin.txid, inp.coin.vout);
        mark("key", t);
        env::commit(&KeyBench { x, nullifier });
        return;
    }
    env::verify(inp.core_id, &inp.journal).expect("the core receipt does not verify under core_id");
    assert!(inp.journal.len() % 4 == 0);
    let words: Vec<u32> = inp.journal.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    let j: ChainState = risc0_zkvm::serde::from_slice(&words).expect("core journal is not a ChainState");
    let st = facts::CoreStatement { kind: j.kind, version: j.version, profile: j.profile, self_id: &j.self_id, anchor_id: &j.anchor_id, live_set: j.live_set as u64 };
    // the network's PINNED genesis anchor (review 15.11.2026: comparing j.anchor_id with itself checked nothing)
    let (net, pinned) = if inp.signet { (facts::Network::Signet, anchor(ANCHOR_SIGNET)) } else { (facts::Network::Mainnet, anchor(ANCHOR_MAINNET)) };
    facts::check_core_statement(&st, &inp.core_id, 2, net, false, &pinned).unwrap_or_else(|e| panic!("core statement: {e}"));
    t = mark("core", t);
    let c = &inp.coin;
    let leaf = facts::coin_leaf(&c.txid, c.vout, c.value, &c.script_pubkey, c.height, c.coinbase, c.tracked, c.creation_mtp);
    assert!(facts::coin_unspent(&j.utxo_roots, j.utxo_leaves, inp.pos, &leaf, &inp.path), "the coin is not unspent at the tip");
    assert!(c.value >= inp.min_value, "the coin is worth less than min_value");
    t = mark("coin", t);
    let nullifier = match inp.policy {
        0 => {
            let mut h = Sha256::new();
            h.update([0u8]);
            h.update((c.script_pubkey.len() as u32).to_le_bytes());
            h.update(&c.script_pubkey);
            let mut node: [u8; 32] = h.finalize().into();
            assert!(inp.set_depth < 64 && inp.set_path.len() == inp.set_depth as usize, "the set path is not the committed depth");
            assert!(inp.set_index >> inp.set_depth == 0, "set index past the tree");
            for (k, s) in inp.set_path.iter().enumerate() {
                let mut h = Sha256::new();
                h.update([1u8]);
                if (inp.set_index >> k) & 1 == 0 { h.update(node); h.update(s); } else { h.update(s); h.update(node); }
                node = h.finalize().into();
            }
            assert!(node == inp.set_root, "the script is not in the set");
            mark("set", t);
            [0u8; 32]
        }
        1 => {
            let s = &c.script_pubkey;
            assert!(s.len() == 34 && s[0] == 0x51 && s[1] == 0x20, "the coin is not P2TR");
            let (x, n) = key_work(&inp.sk, &inp.context, &c.txid, c.vout);
            assert!(x[..] == s[2..], "sk is not the output key's secret");
            mark("key", t);
            n
        }
        _ => panic!("unknown policy"),
    };
    env::commit(&AnonLive {
        kind: KIND_ANON_LIVE, core_id: inp.core_id, lineage: j.lineage, anchor_id: j.anchor_id, height: j.height, tip_hash: j.tip_hash,
        cum_work: j.cum_work, max_time: j.max_time, profile: j.profile, policy: inp.policy, min_value: inp.min_value,
        set_root: if inp.policy == 0 { inp.set_root } else { [0; 32] }, set_depth: if inp.policy == 0 { inp.set_depth } else { 0 },
        context: inp.context, nullifier,
    });
}
