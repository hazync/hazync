use bitcoin::consensus::{deserialize, serialize};
use bitcoin::hashes::Hash as _;
use bitcoin::{Amount, ScriptBuf, Transaction, TxOut};
use hazync_utreexo::{coin_leaf, hash_leaf, Forest, Hash};
use hazync_coinbase_smt::{Proof as SmtProof, Smt};
use methods::{METHOD_ELF, METHOD_ID};
use risc0_zkvm::{ExecutorEnv, ProverOpts};
use serde::{Deserialize, Serialize};

// The host is libzync-only: the CORE design (a per-signature guest, image 37987b85) is in history/, and this host
// writes only the libzync input streams. Refuse to build against any other guest.
const _: () = assert!(methods::LIBZYNC, "prover/host is libzync-only: build it with HAZYNC_LIBZYNC=1 (history/README.md)");

mod bench;
mod bridge;
mod chunks;
mod lz;
mod trie;
mod seg;
mod verify;
mod prep;
mod lz_coord;

use bench::{chunk_profile, exec_time_cmd, segment_mem_cmd, segment_size_cmd, test_locks_cmd, test_merkle_cmd};
use bridge::{cmd_bridge, cmd_dump_snapshot, snapshot_emit_cmd};
use chunks::{chunk_block_hash, chunk_bounds, nchunks_env, write_chunk_inputs};
use lz::{agg_chunks, agg_exec_fake, execute_chunks_parallel, libzync_hints_cmd, libzync_snark_wrap_cmd,
    libzync_verify_cmd, libzync_verify_fail_hook, lz_execute, lz_prove_range_cmd};
use seg::{seg_connect_cmd, seg_coordinate_cmd, seg_coordinate_tree_cmd, seg_distribute_cmd, seg_join_cmd,
    seg_prove_one_cmd, seg_serve_cmd, seg_work_cmd};
use verify::receipt_digest_cmd;

// A byte blob (de)serialised via risc0 serde's PACKED byte path (serialize_bytes → 4 bytes/word) instead
// of the default one-word-per-byte. Must match the guest's PackedBytes. Wire encoding only. Used for the
// shared, de-duplicated per-tx raw_tx / prevouts blobs.
#[derive(Clone)]
struct PackedBytes(Vec<u8>);
impl serde::Serialize for PackedBytes {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> { s.serialize_bytes(&self.0) }
}
impl<'de> serde::Deserialize<'de> for PackedBytes {
    // Delegates to the shared `packed_bytes` visitor. This used to carry its own copy, and when
    // PackedHash/PackedHashes were added they got a *second* visitor that silently omitted `visit_seq`
    // -- reintroducing the v0.9.0 JSON parse bug for every bundle carrying `txids`. One visitor only.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(PackedBytes(packed_bytes(d)?))
    }
}

// H8 domain tags — first committed field of each recursion-consumed journal (must match the guest).
/// Mirrors the guest's CHAIN_STATE_VERSION.
const CHAIN_STATE_VERSION: u32 = 2;
const KIND_CHAIN: u32 = 0xC4A1_0012;
/// libzync agg-lite: the block's median-time-past, set by `build_full` for `write_chunk_inputs`.
// Per block, not per process: `libzync-prove-range` runs many blocks in one process.
static HZ_BLOCK_MTP: std::sync::Mutex<Option<u32>> = std::sync::Mutex::new(None);

// A 32-byte hash, and a vector of them, carried over risc0 serde's PACKED byte path rather than the
// default one-word-per-byte. MEASURED on block 962,000 (docs/WITNESS_WIRE_PROFILE_2026-08-28.md):
// `[u8; 32]` and `Vec<[u8; 32]>` amplify 4.00x on the wire, and the two WireProofs alone were 60.0%
// of the whole witness at 3.73x. Same trick PackedBytes already uses for the tx blobs -- the values
// on both sides are byte-identical, only the encoding changes.
//
// The LEAF matters as much as the siblings here: measured source is ~73 B per proof (32 leaf +
// 8 position + ~1 sibling), so at 4x the leaf alone is ~2.0 MB of the 4.35 MB the proofs cost.
// Packing siblings without the leaf would leave most of it on the table.
#[derive(Clone, Default, PartialEq)]
struct PackedHash([u8; 32]);
impl serde::Serialize for PackedHash {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> { s.serialize_bytes(&self.0) }
}
impl<'de> serde::Deserialize<'de> for PackedHash {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = packed_bytes(d)?;
        if v.len() != 32 { return Err(serde::de::Error::custom("PackedHash must be 32 bytes")); }
        let mut h = [0u8; 32]; h.copy_from_slice(&v); Ok(PackedHash(h))
    }
}
#[derive(Clone, Default)]
struct PackedHashes(Vec<[u8; 32]>);
impl serde::Serialize for PackedHashes {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        // Vec<[u8; 32]> is already n*32 contiguous bytes, so this is a view, not a copy.
        let flat: &[u8] = unsafe { std::slice::from_raw_parts(self.0.as_ptr() as *const u8, self.0.len() * 32) };
        s.serialize_bytes(flat)
    }
}
/// One element of a `PackedHashes` JSON sequence: a byte of the packed blob, or a whole 32-byte hash
/// from a LEGACY pre-packing bundle. Unambiguous — a JSON number is a byte, a JSON array is a hash —
/// and only ever reached through JSON, because risc0's binary serde lands on `visit_bytes`.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum ByteOrHash { Byte(u8), Hash([u8; 32]) }

impl<'de> serde::Deserialize<'de> for PackedHashes {
    /// Accepts BOTH wire shapes, on purpose.
    ///
    /// The 2026-08-28 packing changed `Vec<[u8; 32]>` from a nested list-of-lists to ONE flat n*32
    /// blob. That is a read-side incompatibility with every bundle written before it — and the
    /// coordinator holds **220,001 of them, 74 GB** (measured 2026-09-05), all nested. Regenerating
    /// those means re-running the bridge over 220k blocks against a full node; the leaf values did
    /// not change, only the encoding did, so that would be a lot of work to arrive back where we are.
    ///
    /// So the reader accepts the legacy shape. Nothing WRITES it — `serialize` still emits the flat
    /// blob — and the binary path is untouched, so this costs the wire nothing and saves rewriting
    /// 74 GB on a production box.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Vec<[u8; 32]>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("packed 32-byte hashes (flat blob, or a legacy list of 32-byte lists)")
            }
            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                if v.len() % 32 != 0 { return Err(E::custom("PackedHashes must be a multiple of 32")); }
                Ok(v.chunks_exact(32).map(|c| { let mut h = [0u8; 32]; h.copy_from_slice(c); h }).collect())
            }
            fn visit_byte_buf<E: serde::de::Error>(self, v: Vec<u8>) -> Result<Self::Value, E> {
                self.visit_bytes(&v)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let (mut flat, mut out) = (Vec::<u8>::new(), Vec::<[u8; 32]>::new());
                while let Some(e) = seq.next_element::<ByteOrHash>()? {
                    match e {
                        ByteOrHash::Byte(b) => flat.push(b),
                        ByteOrHash::Hash(h) => out.push(h),
                    }
                }
                // A sequence is one shape or the other, never a mixture.
                if !flat.is_empty() && !out.is_empty() {
                    return Err(serde::de::Error::custom("PackedHashes mixes packed bytes and whole hashes"));
                }
                if !out.is_empty() { return Ok(out); }
                if flat.len() % 32 != 0 {
                    return Err(serde::de::Error::custom("PackedHashes must be a multiple of 32"));
                }
                Ok(flat.chunks_exact(32).map(|c| { let mut h = [0u8; 32]; h.copy_from_slice(c); h }).collect())
            }
        }
        Ok(PackedHashes(d.deserialize_byte_buf(V)?))
    }
}
/// The byte-visitor both of the above share.
fn packed_bytes<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    struct V;
    impl<'de> serde::de::Visitor<'de> for V {
        type Value = Vec<u8>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str("bytes") }
        fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Vec<u8>, E> { Ok(v.to_vec()) }
        fn visit_byte_buf<E: serde::de::Error>(self, v: Vec<u8>) -> Result<Vec<u8>, E> { Ok(v) }
        // JSON (serde_json) has no native bytes type: `serialize_bytes` emits a sequence of u8, so the
        // bundle round-trip (bridge writes bundle_<n>.json, prove-range-bridge reads it) lands here.
        // risc0's binary serde still hits visit_bytes/visit_byte_buf above; both paths yield Vec<u8>.
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<u8>, A::Error> {
            let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0));
            while let Some(b) = seq.next_element::<u8>()? { out.push(b); }
            Ok(out)
        }
    }
    d.deserialize_byte_buf(V)
}

// ---- Wire format: MUST match the guest structs field-for-field, in order. ----
#[derive(Serialize, Deserialize, Clone)]
struct WireProof { leaf: PackedHash, position: u64, siblings: PackedHashes }
#[derive(Serialize, Deserialize)]
struct BlockInput {
    // flags removed: script flags are guest-derived (block_script_flags), never host-supplied.
    // raw_tx + prevouts are de-duplicated into BlockWitness.txs / tx_prevouts; this input refers to its
    // tx by index (a multi-input tx would otherwise repeat its full bytes once per input).
    tx_idx: u32, input_idx: u32,
    global_pos: u64, coin_height: u32, coin_is_coinbase: u32, coin_mtp: u32, tx_first: u32,
    proof_i: WireProof, proof_last: WireProof,
}
#[derive(Serialize, Deserialize, Clone)]
struct WireStump { roots: Vec<Option<[u8; 32]>>, num_leaves: u64 }
#[derive(Serialize, Deserialize, Clone)]
struct Bip30Del { global_pos: u64, proof_i: WireProof, proof_last: WireProof }
#[derive(Serialize, Deserialize, Clone)]
struct Bip30Overwrite { old_height: u32, old_mtp: u32, dels: Vec<Bip30Del> } // F3: superseded coinbase deletes
#[derive(Serialize, Deserialize)]
struct BlockWitness {
    header: Vec<u8>, height: u32, coinbase_tx: Vec<u8>, txids: PackedHashes,
    root_prev: WireStump, txs: Vec<PackedBytes>, tx_prevouts: Vec<PackedBytes>,
    inputs: Vec<BlockInput>, root_next: WireStump,
    bip30: Option<Bip30Overwrite>,
    // #54 — the coinbase-SMT root this block starts from, and the sequenced proofs that advance it.
    // Serialised LAST so the field order matches the guest's, which must stay in step for every wire
    // struct here.
    #[serde(default)]
    in_smt_root: [u8; 32],
    smt: SmtBlockWitness,
}
#[derive(Serialize, Deserialize, Clone)]
struct ChainState {
    kind: u32, // H8: == KIND_CHAIN
    // == CHAIN_STATE_VERSION. Default 0 for a JSON written before the field, which the guest refuses: an old anchor
    // file cannot silently enter a chain with a zero utxo_value.
    #[serde(default)]
    version: u32,
    tip_hash: [u8; 32], utxo_roots: Vec<Option<[u8; 32]>>, utxo_leaves: u64,
    // Satoshis held by the coin forest (the UTXO set's total at H). Mirrors the guest.
    #[serde(default)]
    utxo_value: u64,
    cum_work: [u8; 32], height: u32,
    prev_nbits: u32, prev_time: u32, epoch_start: u32, recent_times: Vec<u32>,
    anchor_id: [u8; 32], // S5: dsha256 of the base anchor; verifier pins == dsha256(genesis_anchor)
    self_id: [u32; 8],  // S1: image id recursed against; verifier asserts == METHOD_ID
    // libzync (E2): the multiset hash of the LIVE coins (profile bit 6; live_set 0 = the identity) and the counting
    // trie's root (bit 5). Zero without those bits. Mirrors the guest's ChainState field for field. The aliases read a
    // state JSON written before the rename (ss_set / ss_acc / ss_hints, the retired SwiftSync design B's names).
    #[serde(alias = "ss_set")] live_set: u32,
    #[serde(alias = "ss_acc")] live_hash: [u32; 16],
    #[serde(alias = "ss_hints")] trie_root: [u8; 32],
    // libzync (review 2): the coinbase-txid SMT root after this block, carried so each block's BIP30 check
    // runs against the chain's own tree (genesis: empty). Mirrors the guest.
    smt_root: [u8; 32],
    // libzync: the largest block timestamp so far (the verifier applies time-too-new to it) and the image's
    // profile word (bit 0 libzync, bit 1 scripts SKIPPED, bit 2 signet). Mirrors the guest. Default 0 so an
    // anchor JSON written before these fields still loads; prove-range fills max_time from its timestamps.
    #[serde(default)]
    max_time: u32,
    #[serde(default)]
    profile: u32,
    // libzync: the block forest and lineage digest (docs/libzync/HANDOVER.md). Mirrors the guest. Default empty so a
    // block file's anchor, which has no history, still loads; `demo_block_forest` then fills a placeholder.
    #[serde(default)]
    block_roots: Vec<Option<[u8; 32]>>,
    #[serde(default)]
    block_leaves: u64,
    #[serde(default)]
    lineage: [u8; 32],
}

// The guest's own pin file: this host and that guest cannot disagree about the lineage.
mod handover_pin { include!("../../methods/guest/src/handover_pin.rs"); }
// Cross-version handover: every vendored RISC Zero recursion circuit (the guest compiles only the ones it pins).
#[path = "../../methods/guest/src/xver/mod.rs"] pub mod xver;
// Every variant's lineage file; `pinned_lineage` picks this host's (the guest compiles only its own).
#[path = "../../methods/guest/src/lineage/mainnet.rs"] mod lineage_mainnet;
#[path = "../../methods/guest/src/lineage/signet.rs"] mod lineage_signet;
#[path = "../../methods/guest/src/lineage/regtest.rs"] mod lineage_regtest;
#[path = "../../methods/guest/src/lineage/mainnet_coinlog.rs"] mod lineage_mainnet_coinlog;
#[path = "../../methods/guest/src/lineage/signet_coinlog.rs"] mod lineage_signet_coinlog;
#[path = "../../methods/guest/src/lineage/regtest_coinlog.rs"] mod lineage_regtest_coinlog;

/// The handovers this host's image pins (the guest's `lineage::LINEAGE`).
fn pinned_lineage() -> &'static [handover_pin::Handover] {
    match (methods::NETWORK, methods::COINLOG) {
        ("signet", false) => lineage_signet::LINEAGE,
        ("regtest", false) => lineage_regtest::LINEAGE,
        ("signet", true) => lineage_signet_coinlog::LINEAGE,
        ("regtest", true) => lineage_regtest_coinlog::LINEAGE,
        (_, true) => lineage_mainnet_coinlog::LINEAGE,
        _ => lineage_mainnet::LINEAGE,
    }
}

fn sha256_host(b: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(b).into()
}

/// The lineage digest of a list of handovers (guest `lineage_of`).
fn lineage_of(list: &[handover_pin::Handover]) -> [u8; 32] {
    list.iter().fold([0u8; 32], |l, h| { let (p, n) = handover_pin::lineage_step(&l, h); sha256_host(&p[..n]) })
}

/// Stamping by era (docs/HANDOVER.md §5). Under the lineage `[(I0, h1), (I1, h2), ...]`, block h is proved by the
/// first `I_k` with `h < h_{k+1}`'s entry, else by this image: entry k's image proved every block below its
/// `first_height` that an earlier entry did not.
pub(crate) fn image_at(h: u32) -> [u32; 8] {
    pinned_lineage().iter().find(|e| h < e.first_height).map(|e| e.image).unwrap_or(METHOD_ID)
}

/// The lineage digest a state after block h carries: every handover at or below h.
pub(crate) fn lineage_at(h: u32) -> [u8; 32] {
    let l = pinned_lineage();
    lineage_of(&l[..l.iter().take_while(|e| e.first_height <= h).count()])
}

/// A prep state S(h) as the image proving block h commits it: that image's id and the lineage at h. Genesis (h = 0)
/// is never stamped: its self_id and lineage are zero in every image, so GENESIS_TIP never moves. With an empty
/// lineage this is exactly what prep did before (self_id = METHOD_ID). Prep states are untrusted starting points, and
/// a leaf's guest checks every witness against them, so normalising one on read cannot weaken anything.
pub(crate) fn stamp(s: &mut ChainState) {
    if s.height == 0 { return; }
    // The rehearsal's control (experimental/libzync-bench/handover/board_rehearsal.sh --control): without stamping, the
    // 11-11 failure must come back. A dev-mode test hook, refused anywhere else.
    if std::env::var("HAZYNC_TEST_NO_STAMP").as_deref() == Ok("1") {
        assert!(crate::verify::dev_mode_env(), "HAZYNC_TEST_NO_STAMP is a dev-mode test hook");
        return;
    }
    s.self_id = image_at(s.height);
    s.lineage = lineage_at(s.height);
    s.profile = crate::lz::expected_profile();
}

/// `libzync-lineage`: the handovers this image pins, for the coordinator, the worker and the runbook.
fn lineage_cmd() {
    let l = pinned_lineage();
    println!("METHOD_ID {}", method_id_hex());
    if l.is_empty() { println!("HANDOVER none"); }
    for (k, e) in l.iter().enumerate() {
        let v = if e.verifier == [0u8; 32] { "same".to_string() } else {
            format!("{} ({})", hex(&e.verifier), xver::version(&e.verifier).map_or("NOT CARRIED BY THIS HOST", |v| v.name))
        };
        println!("HANDOVER k={k} image={} first_height={} verifier={v}", hex_u32x8(&e.image), e.first_height);
    }
    println!("LINEAGE {}", hex(&lineage_of(l)));
}

fn hex_u32x8(w: &[u32; 8]) -> String {
    hex(&risc0_zkvm::sha::Digest::from(*w).as_bytes().try_into().unwrap_or([0u8; 32]))
}

/// A predecessor's receipt verified the way the lineage entry says: env-free `Receipt::verify` for the same RISC
/// Zero, the vendored old circuit (xver) for an older one. Returns the journal.
pub(crate) fn verify_pred_receipt(r: &risc0_zkvm::Receipt, e: &handover_pin::Handover) -> Result<(), String> {
    if e.verifier == [0u8; 32] {
        return r.verify(e.image).map_err(|x| format!("does not verify against predecessor {}: {x}", hex_u32x8(&e.image)));
    }
    let risc0_zkvm::InnerReceipt::Succinct(s) = &r.inner else {
        return Err("a cross-version predecessor receipt must be succinct".into());
    };
    if s.verifier_parameters.as_bytes() != &e.verifier[..] {
        return Err(format!("receipt's verifier parameters {} are not the pinned version's {}", s.verifier_parameters, hex(&e.verifier)));
    }
    xver::verify(&e.verifier, e.image, &r.journal.bytes, &xver::encode(s), &xver::soft_suite()).map_err(|x| x.to_string())
}

/// `xver-selftest <receipt> [<image hex>]`: the cross-version verifier on a real receipt, natively, with its negative
/// controls (a seal word, the image, the journal: each changed must be refused). Also recomputes each vendored
/// version's verifier-parameter digest from its circuit.
fn xver_selftest_cmd(path: &str, image: Option<&str>) {
    use risc0_zkvm::sha::Digestible;
    for v in xver::KNOWN {
        let p = risc0_zkvm::SuccinctReceiptVerifierParameters {
            control_root: v.control_root,
            inner_control_root: None,
            proof_system_info: risc0_zkp::adapter::PROOF_SYSTEM_INFO,
            circuit_info: v.circuit_info,
        }.digest();
        let ok = p.as_bytes() == &v.params[..];
        println!("XVER {} params {} computed {} {}", v.name, hex(&v.params), p, if ok { "OK" } else { "MISMATCH" });
        assert!(ok, "{}: the declared verifier-parameter digest is not the vendored circuit's", v.name);
    }
    let r: risc0_zkvm::Receipt = bincode::deserialize(&std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}")))
        .unwrap_or_else(|e| panic!("{path}: not a receipt: {e}"));
    let risc0_zkvm::InnerReceipt::Succinct(s) = &r.inner else { panic!("{path}: not a succinct receipt") };
    let params: [u8; 32] = s.verifier_parameters.as_bytes().try_into().unwrap();
    let v = xver::version(&params).unwrap_or_else(|| panic!("{path}: verifier parameters {} are not a vendored version", s.verifier_parameters));
    let claim = s.claim.as_value().expect("claim pruned");
    let img: [u32; 8] = match image {
        Some(h) => { let b = hx(h); let b: [u8; 32] = b.try_into().expect("image: 32 bytes hex"); risc0_zkvm::sha::Digest::from(b).into() }
        None => claim.pre.digest().into(),
    };
    assert!(xver::expected_claim(img, &r.journal.bytes) == s.claim.digest(),
            "the receipt's own claim is not ReceiptClaim::ok(image, journal) (wrong image given?)");
    let words = xver::encode(s);
    let suite = xver::soft_suite();
    let t = std::time::Instant::now();
    xver::verify(&params, img, &r.journal.bytes, &words, &suite).unwrap_or_else(|e| panic!("VALID receipt refused: {e}"));
    println!("XVER {}: {path} ({} bytes, image {}) VERIFIES natively in {:.2} s", v.name, std::fs::metadata(path).unwrap().len(),
             hex_u32x8(&img), t.elapsed().as_secs_f64());
    let mut seal = words.clone();
    let i = seal.len() - 100; seal[i] ^= 1;
    let mut img2 = img; img2[0] ^= 1;
    let mut j2 = r.journal.bytes.clone(); j2[0] ^= 1;
    let cases: [(&str, Result<(), &str>); 3] = [
        ("tampered seal", xver::verify(&params, img, &r.journal.bytes, &seal, &suite)),
        ("wrong image", xver::verify(&params, img2, &r.journal.bytes, &words, &suite)),
        ("wrong journal", xver::verify(&params, img, &j2, &words, &suite)),
    ];
    for (name, res) in cases {
        match res {
            Err(e) => println!("XVER negative control {name}: REFUSED ({e})"),
            Ok(()) => panic!("XVER negative control {name}: ACCEPTED"),
        }
    }
    println!("XVER-SELFTEST-OK");
}

/// A mid-chain anchor from a block file has no record of the blocks before it, but the guest needs a forest of the
/// right SIZE to append to. This fills one with placeholder roots. A proof built on it is anchored, so verifiers can
/// only ever report DEMO, and its block forest does not commit to real history: it exists for measurement and tests.
fn demo_block_forest(s: &mut ChainState, warn: bool) {
    if s.block_leaves != 0 { return; }
    let n = s.height as u64 + 1;
    s.block_leaves = n;
    s.block_roots = (0..64 - n.leading_zeros()).map(|i| ((n >> i) & 1 == 1).then(|| sha256_host(&[b"hazync demo block forest root", &[i as u8][..]].concat()))).collect();
    if warn { eprintln!("WARNING: anchor at {} has no block forest; using PLACEHOLDER roots (demonstration only)", s.height); }
}

fn hx(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}
fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }

// Proving segment size (2^n cycles). risc0 4.0.5 has a preflight bug: for ~10% of blocks a segment packs
// right to its 2^po2 boundary and the assertion `cycles <= 1 << segment.po2` overflows -> the prove panics
// (on CPU AND cuda — shared witgen). It's a liveness bug, never a wrong proof. Smaller segments repartition
// the work and clear the boundary, so the caller retries a failed prove with a decremented HAZYNC_SEG_PO2.
// EVERY prove path calls this so the retry is honoured everywhere. Host-side executor config only — the
// guest is untouched by this knob, so it does not affect METHOD_ID.
// Default depends on the backend, because the trade-off does. Bigger segments = fewer of them = less
// recursion/fold overhead: on GPU that measured ~6% faster than 20 on block 130000 (23.0s vs 24.4s).
// ⛔ THIS COMMENT USED TO SAY "flat to 22, so 21 is the sweet spot (same speed as 22)". THAT IS WRONG,
// and reading it cost an evening of wrong advice. It was measured on block 130,000 — a 2011 block with
// almost no transactions and ~30 segments, where there is barely any fold tree to save. On a TIP block
// po2 22 measured ~11.5% faster (block 962,000 on an L40S, docs/history/TOPOLOGY_AND_SETTINGS.md §3.1), and the
// saving grows with segment count because that is what the fold tree is built from. 21 is the DEFAULT
// because it peaks ~22 GB and fits a 24 GB card; 22 peaks ~40.6 GB and needs ≥48 GB. Judge po2 on a
// block the size of the one you are proving, never on a young one.
// A po2-21 segment also needs ~2x the working memory of 20, and on CPU that is a pure cost — the
// speed win was never measured there, while an
// 11 GB box proving block 170 (a 2.3M-cycle block!) hit 8.7 GB RSS and went to swap. Swapping is not a
// prove *failure*, so the retry ladder below never fires; it just crawls. So: 21 with the cuda feature,
// the risc0 default of 20 otherwise. HAZYNC_SEG_PO2 overrides either way.
fn seg_po2() -> u32 {
    std::env::var("HAZYNC_SEG_PO2").ok().and_then(|s| s.parse().ok())
        .unwrap_or(if cfg!(feature = "cuda") { 21 } else { 20 })
}

// This host's guest image id (METHOD_ID) as the canonical RISC0 hex digest.
fn method_id_hex() -> String { risc0_zkvm::Digest::from(METHOD_ID).to_string() }

fn rev(mut v: Vec<u8>) -> Vec<u8> { v.reverse(); v }
fn arr(v: Vec<u8>) -> [u8; 32] { v.try_into().unwrap() }

fn wire_proof(p: &hazync_utreexo::Proof) -> WireProof {
    WireProof { leaf: PackedHash(p.leaf), position: p.position, siblings: PackedHashes(p.siblings.clone()) }
}
fn wire_stump(f: &Forest) -> WireStump { WireStump { roots: f.roots(), num_leaves: f.leaves.len() as u64 } }
// Strip trailing empty root slots (mirrors the guest `normalize`) so two representations of the same
// accumulator (e.g. the empty genesis forest) compare equal regardless of padding.
fn normalize_host(mut v: Vec<Option<[u8; 32]>>) -> Vec<Option<[u8; 32]>> {
    while v.last() == Some(&None) { v.pop(); }
    v
}
// Block hash (internal/LE order) = double-SHA256 of the 80-byte header, matching the guest's dsha256.
fn header_hash(header: &[u8]) -> [u8; 32] { bitcoin::hashes::sha256d::Hash::hash(header).to_byte_array() }
// Core CScript::IsUnspendable(): OP_RETURN (0x6a) or script > MAX_SCRIPT_SIZE (10000). Unspendable
// outputs never enter the UTXO set, so the accumulator (host + guest) must skip them (H3).
fn out_spendable(spk: &[u8]) -> bool { !((!spk.is_empty() && spk[0] == 0x6a) || spk.len() > 10_000) }

fn out_leaf_of(tx: &Transaction, txid: &[u8; 32], vout: usize, height: u32, flags: impl Into<u32>, mtp: u32) -> Hash {
    let o = &tx.output[vout];
    hazync_utreexo::coin_leaf_flags(txid, vout as u32, o.value.to_sat(), o.script_pubkey.as_bytes(), height, flags.into() as u8, mtp)
}
fn build_header_v(version: i32, prev_disp: &str, merkle_internal: &[u8; 32], time: u32, bits: u32, nonce: u32) -> Vec<u8> {
    let mut h = Vec::with_capacity(80);
    h.extend_from_slice(&version.to_le_bytes());     // real block version (versionbits post-BIP9)
    h.extend_from_slice(&rev(hx(prev_disp)));        // prev (internal)
    h.extend_from_slice(merkle_internal);            // merkle (internal)
    h.extend_from_slice(&time.to_le_bytes());
    h.extend_from_slice(&bits.to_le_bytes());
    h.extend_from_slice(&nonce.to_le_bytes());
    h
}

// Serialize a ChainState to the exact bytes env::commit(&state) would produce (LE u32 words).
/// The guest's link digest (ROADMAP H-B): sha256d of the state's journal with anchor_id zeroed; what a base step on
/// `s` commits as its anchor_id and what a join requires of the left part.
fn link_digest(s: &ChainState) -> [u8; 32] {
    let mut z = s.clone();
    z.anchor_id = [0; 32];
    bitcoin::hashes::sha256d::Hash::hash(&state_journal_bytes(&z)).to_byte_array()
}

fn state_journal_bytes(s: &ChainState) -> Vec<u8> {
    risc0_zkvm::serde::to_vec(s).unwrap().iter().flat_map(|w| w.to_le_bytes()).collect()
}

fn work_u128(w: &[u8; 32]) -> u128 {
    let mut low = [0u8; 16];
    low.copy_from_slice(&w[0..16]); // arith_uint256 internal = little-endian
    u128::from_le_bytes(low)
}

// Build the anchor + full block witness from the HAZYNC_BLOCK JSON (coinbase + N multi-input txs, real prevouts).
// Coins without coin_height/coinbase/mtp get benign values (maturity/BIP68 no-op).
fn build_full() -> (ChainState, BlockWitness) {
    let path = std::env::var("HAZYNC_BLOCK").unwrap_or_else(|_| "block_full.json".into());
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let height = j["height"].as_u64().unwrap() as u32;
    let bits = j["bits"].as_u64().unwrap() as u32;
    let time = j["time"].as_u64().unwrap() as u32;
    let nonce = j["nonce"].as_u64().unwrap() as u32;
    let version = j["version"].as_i64().unwrap_or(1) as i32; // real versionbits value (fallback v1)
    let prev = j["prev"].as_str().unwrap();
    let merkle = j["merkle"].as_str().unwrap();
    let cb_hex = j["coinbase_hex"].as_str().unwrap();
    let header = build_header_v(version, prev, &arr(rev(hx(merkle))), time, bits, nonce);

    let ch: u32 = height.saturating_sub(10_000); // benign mature height (coins marked non-coinbase)
    let coinbase: Transaction = deserialize(&hx(cb_hex)).unwrap();
    let cb_txid = coinbase.compute_txid().to_byte_array();

    // Parse each non-coinbase tx: raw + full prevout set + per-coin (height, is_coinbase, creation-MTP)
    // — real from the fetcher/bridge (S2), or benign fallback for JSONs lacking it (pre-S2 vectors like
    // 130000/140000). coin_mtp arrives from the archive-node bridge (-hazyncwitness hook) and closes
    // BIP68 time-based relative locks; the fetcher omits it (fallback 0 = conservative, no false reject).
    struct Ptx { raw: Vec<u8>, tx: Transaction, prevouts: Vec<TxOut>, meta: Vec<(u32, u32, u32)>, txid: [u8; 32] }
    let mut ptxs: Vec<Ptx> = Vec::new();
    for tx in j["txs"].as_array().unwrap() {
        let raw = hx(tx["raw"].as_str().unwrap());
        let t: Transaction = deserialize(&raw).unwrap();
        let mut prevouts = Vec::new();
        let mut meta = Vec::new();
        for p in tx["prevouts"].as_array().unwrap() {
            prevouts.push(TxOut {
                value: Amount::from_sat(p["value"].as_u64().unwrap()),
                script_pubkey: ScriptBuf::from_bytes(hx(p["spk"].as_str().unwrap())),
            });
            let h = p["coin_height"].as_u64().map(|x| x as u32).unwrap_or(ch);
            let cb = p["coin_is_coinbase"].as_u64().map(|x| x as u32).unwrap_or(0);
            let mtp = p["coin_mtp"].as_u64().map(|x| x as u32).unwrap_or(0);
            meta.push((h, cb, mtp));
        }
        let txid = t.compute_txid().to_byte_array();
        ptxs.push(Ptx { raw, tx: t, prevouts, meta, txid });
    }
    let leaf_of = |op: &bitcoin::OutPoint, o: &TxOut, h: u32, cb: u32, mtp: u32| hazync_utreexo::coin_leaf_flags(&op.txid.to_byte_array(), op.vout, o.value.to_sat(), o.script_pubkey.as_bytes(), h, cb as u8, mtp);
    // A coin's leaf flag, bit 1: tracked by the BIP30 tree. The block file carries it, as it carries the coin's height;
    // the guest derives it again and a wrong one does not match the coin's leaf.
    let cb_flags = j["coinbase_flags"].as_u64().unwrap_or(1) as u32;
    let tx_flags = |p: &Ptx| -> u32 { (!p.meta.is_empty() && p.meta.iter().all(|m| m.1 & 2 != 0)) as u32 * 2 };

    // recent_times (prev-11) + this block's creation-MTP — needed EARLY: the guest detects in-block
    // spends (H1) by LEAF membership in this block's created-output set (main.rs `created_at`), and the
    // leaf commits the creation-MTP. Keying on txid instead would false-positive a spend of an earlier
    // coin whose funding tx shares a txid with a tx in this block (pre-BIP34 coinbase-txid collisions).
    let mut recent_times: Vec<u32> = {
        let rt: Vec<u32> = j["recent_times"].as_array()
            .map(|a| a.iter().filter_map(|v| v.as_u64().map(|x| x as u32)).collect())
            .unwrap_or_default();
        if rt.is_empty() { (0..11).map(|i| time.saturating_sub(2000) + i * 100).collect() } else { rt }
    };
    // COV-1 negative-test hook (test-only, inert unless HAZYNC_COV1_BADTIME): prev-11 MTP == this block's
    // time so `time_ok` fails. Applied to recent_times so cmtp stays consistent host↔guest. NEVER in prod.
    if std::env::var("HAZYNC_COV1_BADTIME").is_ok() { recent_times = vec![time; 11]; }
    let cmtp = median_u32(&recent_times);
    *HZ_BLOCK_MTP.lock().unwrap() = Some(cmtp);
    // This block's created output leaves (coinbase + every tx, unspendable skipped) — the guest's in-block
    // set, keyed by LEAF (height-bearing, so a colliding old coinbase's leaf differs and stays external).
    let mut created: std::collections::HashSet<[u8; 32]> = std::collections::HashSet::new();
    for v in 0..coinbase.output.len() {
        if !out_spendable(coinbase.output[v].script_pubkey.as_bytes()) { continue; }
        created.insert(out_leaf_of(&coinbase, &cb_txid, v, height, cb_flags, cmtp));
    }
    for p in &ptxs {
        for v in 0..p.tx.output.len() {
            if !out_spendable(p.tx.output[v].script_pubkey.as_bytes()) { continue; }
            created.insert(out_leaf_of(&p.tx, &p.txid, v, height, tx_flags(p), cmtp));
        }
    }
    let mut spent_in_block: std::collections::HashSet<[u8; 32]> = std::collections::HashSet::new();
    // Seed the accumulator: filler + every EXTERNAL input's spent coin (in-block coins never entered it) + filler.
    let mut forest = Forest::new();
    for i in 0..4u64 { forest.add(hash_leaf(&[b"pre".as_slice(), &i.to_le_bytes()].concat())); }
    for p in &ptxs {
        for (i, o) in p.prevouts.iter().enumerate() {
            let coin = leaf_of(&p.tx.input[i].previous_output, o, p.meta[i].0, p.meta[i].1, p.meta[i].2);
            if created.contains(&coin) { continue; } // in-block: ephemeral, never in the accumulator
            forest.add(coin);
        }
    }
    for i in 0..2u64 { forest.add(hash_leaf(&[b"post".as_slice(), &i.to_le_bytes()].concat())); }

    let mut anchor = ChainState {
        kind: KIND_CHAIN, version: CHAIN_STATE_VERSION, utxo_value: 0,
        tip_hash: arr(rev(hx(prev))), utxo_roots: forest.roots(), utxo_leaves: forest.leaves.len() as u64,
        cum_work: [0u8; 32], height: height - 1,
        // #83: use the REAL in-boundary retarget inputs when the fixture carries them. The old values
        // were fabricated — prev_time as "this block minus 600s", epoch_start as "minus 1000 blocks".
        // Harmless at a non-retarget height, where the guest carries nbits through unchanged; fatal at
        // a retarget height, where calc_next_bits consumes epoch_start and the expected target is then
        // derived from a timestamp that never existed. Block 481824 came back `block_valid=true
        // retarget_ok=false` for exactly that reason: the block was fine, the fixture could not
        // express it. Pre-#83 fixtures (130000/140000/741000) carry none of these and keep the old
        // behaviour, which is correct for them because none is a retarget height.
        prev_nbits: j["prev_bits"].as_u64().map(|v| v as u32).unwrap_or(bits),
        prev_time: j["prev_time"].as_u64().map(|v| v as u32).unwrap_or_else(|| time.saturating_sub(600)),
        epoch_start: j["epoch_start"].as_u64().map(|v| v as u32)
            .unwrap_or_else(|| time.saturating_sub(600 * 1000)),
        // Real prev-11 block timestamps (median = MTP(height-1), the spend block's BIP68-time/BIP113
        // window) when the fetcher/bridge supplies them; else the benign placeholder for pre-S2 vectors
        // (130000/140000) that carry no `recent_times` — computed above (with the COV-1 hook applied).
        recent_times: recent_times.clone(),
        anchor_id: [0u8; 32], self_id: METHOD_ID, live_set: 0, live_hash: [0; 16], trie_root: [0; 32], smt_root: hazync_coinbase_smt::empty_root(),
        max_time: recent_times.iter().copied().max().unwrap_or(0), profile: 0,
        block_roots: Vec::new(), block_leaves: 0, lineage: [0; 32],
    };

    // Build the witness: per tx a shared full-prevouts blob; per input a BlockInput (tx_first on input 0).
    let root_prev = wire_stump(&forest);
    let mut txids = vec![cb_txid];
    let mut wtxids: Vec<[u8; 32]> = vec![[0u8; 32]]; // coinbase wtxid = zeros (BIP141)
    let mut inputs: Vec<BlockInput> = Vec::new();
    let mut txs: Vec<PackedBytes> = Vec::new();          // de-duplicated: one raw_tx blob per tx
    let mut tx_prevouts: Vec<PackedBytes> = Vec::new();  // parallel: one prevouts blob per tx
    // SEC-2 negative-test hook (test-only, inert unless HAZYNC_SEC2_BADPOS is set): corrupt the FIRST
    // spend's claimed global position while leaving its inclusion proof honest — the exact inconsistency
    // an honest witness-builder cannot express (both fields normally derive from the same `pos`). The
    // guest's hardened `delete` must reject it (`all_ok=false`, and the accumulator diverges so
    // `root_matches=false`). See SECURITY.md / ROADMAP (SEC-2). NEVER set in production.
    let sec2_bad = std::env::var("HAZYNC_SEC2_BADPOS").is_ok();
    for (tx_i, p) in ptxs.iter().enumerate() {
        txids.push(p.txid);
        wtxids.push(p.tx.compute_wtxid().to_byte_array());
        let prevouts_blob = serialize(&p.prevouts);
        txs.push(PackedBytes(p.raw.clone()));            // this tx's shared raw_tx blob (index == tx_i)
        tx_prevouts.push(PackedBytes(prevouts_blob));    // this tx's shared prevouts blob
        for i in 0..p.tx.input.len() {
            let (ch_i, cb_i, mtp_i) = p.meta[i];
            let coin = leaf_of(&p.tx.input[i].previous_output, &p.prevouts[i], ch_i, cb_i, mtp_i);
            if created.contains(&coin) {
                // IN-BLOCK spend (H1): leaf matches a coin created earlier in this block (the guest's exact
                // rule) — it never entered the accumulator: dummy proof, no delete. Script still verifies.
                spent_in_block.insert(coin);
                inputs.push(BlockInput {
                    tx_idx: tx_i as u32, input_idx: i as u32,
                    global_pos: 0, coin_height: ch_i, coin_is_coinbase: cb_i, coin_mtp: mtp_i, tx_first: (i == 0) as u32,
                    proof_i: WireProof { leaf: PackedHash(coin), position: 0, siblings: PackedHashes(vec![]) },
                    proof_last: WireProof { leaf: PackedHash(coin), position: 0, siblings: PackedHashes(vec![]) },
                });
                continue;
            }
            let pos = forest.find(&coin).expect("input coin in accumulator");
            let last = forest.leaves.len() - 1;
            let mut global_pos = pos as u64;
            if sec2_bad && inputs.is_empty() {
                // a different but in-range index -> membership proof stays valid, position is a lie
                global_pos = if (pos as u64) < last as u64 { pos as u64 + 1 } else { (pos as u64).saturating_sub(1) };
                eprintln!("[SEC2-TEST] corrupting first spend global_pos {} -> {} (proof_i left honest)", pos, global_pos);
            }
            inputs.push(BlockInput {
                tx_idx: tx_i as u32, input_idx: i as u32,
                global_pos, coin_height: ch_i, coin_is_coinbase: cb_i, coin_mtp: mtp_i, tx_first: (i == 0) as u32,
                proof_i: wire_proof(&forest.prove(pos)), proof_last: wire_proof(&forest.prove(last)),
            });
            forest.delete(pos);
        }
    }
    // Add the SURVIVING created outputs (cmtp computed above) — unspendable skipped AND in-block-spent
    // cancelled (leaf ∈ spent_in_block), matching the guest's surviving set exactly.
    let mut new_outputs: Vec<[u8; 32]> = Vec::new();
    for v in 0..coinbase.output.len() { if !out_spendable(coinbase.output[v].script_pubkey.as_bytes()) { continue; } let l = out_leaf_of(&coinbase, &cb_txid, v, height, cb_flags, cmtp); if spent_in_block.contains(&l) { continue; } forest.add(l); new_outputs.push(l); }
    for p in &ptxs { for v in 0..p.tx.output.len() { if !out_spendable(p.tx.output[v].script_pubkey.as_bytes()) { continue; } let l = out_leaf_of(&p.tx, &p.txid, v, height, tx_flags(p), cmtp); if spent_in_block.contains(&l) { continue; } forest.add(l); new_outputs.push(l); } }
    let root_next = wire_stump(&forest);
    let spend_tracked: Vec<bool> = inputs.iter().filter(|i| i.coin_is_coinbase != 0).map(|i| i.coin_is_coinbase & 2 != 0).collect();
    let tracked_new: Vec<([u8; 32], u32)> = ptxs.iter().filter(|p| tx_flags(p) != 0)
        .map(|p| (p.txid, p.tx.output.iter().filter(|o| out_spendable(o.script_pubkey.as_bytes())).count() as u32))
        .filter(|t| t.1 > 0).collect();
    let (in_smt_root, smt) = smt_witness_standalone_ex(height, cb_txid, cb_spendable_outputs(&coinbase),
        &cb_spends_from(&inputs, &txs), &spend_tracked, &tracked_new);
    let mut w = BlockWitness { header, height, coinbase_tx: hx(cb_hex), txids: PackedHashes(txids), root_prev, txs, tx_prevouts, inputs, root_next, bip30: None, in_smt_root, smt };
    // A single-block run is a base step on this anchor, so the anchor carries the tree the witness was cut from.
    anchor.smt_root = w.in_smt_root;
    // --- reject-path negative-test hooks (test-only, inert unless the env var is set; NEVER in production) ---
    // Each corrupts exactly one consensus input so check-full drives the matching guest flag false, closing
    // the retarget / block-weight / sigop-cost coverage gap so those reject-paths are continuously CI-enforced.
    if std::env::var("HAZYNC_TEST_BADNBITS").is_ok() {
        // Non-boundary block: retarget_ok requires nBits == prev_nbits. Corrupt prev_nbits → retarget_ok=false.
        anchor.prev_nbits = anchor.prev_nbits.wrapping_add(0x1000);
    }
    if std::env::var("HAZYNC_TEST_BADWEIGHT").is_ok() {
        // Append a ~1.1 MB OP_RETURN output to the coinbase → block weight > MAX_BLOCK_WEIGHT → weight_ok=false.
        let mut cb: Transaction = deserialize(&hx(cb_hex)).unwrap();
        cb.output.push(TxOut { value: Amount::from_sat(0), script_pubkey: ScriptBuf::from_bytes(vec![0x6au8; 1_100_000]) });
        w.coinbase_tx = serialize(&cb);
    }
    if std::env::var("HAZYNC_TEST_BADSIGOPS").is_ok() {
        // Append an output of 30001 OP_CHECKSIG → sigop cost 30001*4 = 120004 > 80000 → sigops_ok=false.
        let mut cb: Transaction = deserialize(&hx(cb_hex)).unwrap();
        cb.output.push(TxOut { value: Amount::from_sat(0), script_pubkey: ScriptBuf::from_bytes(vec![0xacu8; 30_001]) });
        w.coinbase_tx = serialize(&cb);
    }
    if std::env::var("HAZYNC_TEST_BADSUBSIDY").is_ok() {
        // Inflate the coinbase output by one satoshi over the allowed subsidy + fees → subsidy_ok=false.
        // ONE satoshi on purpose: an obviously-huge value could be caught by an overflow guard rather
        // than by the subsidy rule, which would make this test pass for the wrong reason.
        let mut cb: Transaction = deserialize(&hx(cb_hex)).unwrap();
        if let Some(o) = cb.output.first_mut() {
            o.value = Amount::from_sat(o.value.to_sat().saturating_add(1));
        }
        w.coinbase_tx = serialize(&cb);
    }
    if std::env::var("HAZYNC_TEST_BADMERKLE").is_ok() {
        // Flip one bit of the header's merkle root → merkle_ok=false. The guest recomputes the root
        // from the txids it was given, so this is the check that the header is BOUND to the block body.
        w.header[36] ^= 0x01;
    }
    if std::env::var("HAZYNC_TEST_BADPOW").is_ok() {
        // Flip a nonce bit → the block hash no longer meets the target → pow_ok=false.
        w.header[76] ^= 0x01;
    }
    if std::env::var("HAZYNC_TEST_BADBIP34").is_ok() {
        // BIP34 requires the coinbase scriptSig to begin with a push of the block height. Corrupt the
        // pushed height (not the opcode) → bip34_ok=false above the activation height.
        let mut cb: Transaction = deserialize(&hx(cb_hex)).unwrap();
        if let Some(i) = cb.input.first_mut() {
            let mut b = i.script_sig.to_bytes();
            if b.len() > 2 { b[1] ^= 0x01; i.script_sig = ScriptBuf::from_bytes(b); }
        }
        w.coinbase_tx = serialize(&cb);
    }
    // Silent: most callers discard this anchor (prove-range carries the real state); the ones that keep it are
    // block-file measurements, which are anchored demonstrations anyway.
    demo_block_forest(&mut anchor, false);
    (anchor, w)
}

// ===================== Genesis and per-block witnesses from block_<h>.json ===========================
// Mainnet genesis (the unconditional trusted anchor — its hash/params are consensus constants).
const GENESIS_HASH: &str = "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f";
const GENESIS_TIME: u32 = 1_231_006_505;
const GENESIS_BITS: u32 = 0x1d00ffff;
const GENESIS_WORK: u128 = 4_295_032_833; // GetBlockProof(0x1d00ffff): cumulative work through block 0.

fn arr_u128(x: u128) -> [u8; 32] { let mut a = [0u8; 32]; a[..16].copy_from_slice(&x.to_le_bytes()); a }

// State just after genesis (height 0), before block 1. UTXO set empty: the genesis coinbase is
// unspendable and (per Core) never enters the UTXO set, so the accumulator starts empty.
/// (genesis hash, time, bits, GetBlockProof(bits)) of the embedded guest's network.
fn genesis_params() -> (&'static str, u32, u32, u128) {
    match methods::NETWORK {
        // Core's CSigNetParams genesis: time 1598918400, nBits 0x1e0377ae.
        "signet" => (libzync::net::SIGNET.genesis_hex, 1_598_918_400, 0x1e03_77ae, 4_838_420),
        "regtest" => (libzync::net::REGTEST.genesis_hex, 1_296_688_602, 0x207f_ffff, 2),
        _ => (GENESIS_HASH, GENESIS_TIME, GENESIS_BITS, GENESIS_WORK),
    }
}

/// The coin-state digest of the genesis state: no coins (the genesis coinbase is unspendable), value 0.
fn empty_utxo_state() -> [u8; 32] {
    let mut st: Vec<u8> = Vec::new();
    handover_pin::utxo_state_preimage(0, 0, &[], &mut |b: &[u8]| st.extend_from_slice(b));
    sha256_host(&st)
}

fn genesis_anchor() -> ChainState {
    let (g_hash, g_time, g_bits, g_work) = genesis_params();
    ChainState {
        kind: KIND_CHAIN, version: CHAIN_STATE_VERSION, utxo_value: 0,
        tip_hash: arr(rev(hx(g_hash))), utxo_roots: Forest::new().roots(), utxo_leaves: 0,
        cum_work: arr_u128(g_work), height: 0,
        prev_nbits: g_bits, prev_time: g_time, epoch_start: g_time,
        // self_id is zero, not this image: anchor_id must stay the same across a handover (HANDOVER.md).
        recent_times: vec![g_time], anchor_id: [0u8; 32], self_id: [0; 8], live_set: 0, live_hash: [0; 16], trie_root: [0; 32], smt_root: hazync_coinbase_smt::empty_root(),
        max_time: g_time, profile: 0,
        block_roots: vec![Some(sha256_host(&handover_pin::block_leaf_preimage(&arr(rev(hx(g_hash))), 0, &empty_utxo_state())))],
        block_leaves: 1, lineage: [0; 32],
    }
}

fn read_block_json(dir: &str, h: u32) -> serde_json::Value {
    let p = format!("{dir}/block_{h}.json");
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|_| panic!("missing {p}"))).unwrap()
}

// Build a block witness from a bridge/fetcher JSON block, against and ADVANCING the running
// accumulator `forest`. Multi-input, multi-tx; root_prev = current forest, root_next = after this
// block's external spends + created outputs. (No in-block-spend handling: the guest deletes external
// inputs then adds outputs — a spent coin created in the same block would fail the position lookup
// below with a clear panic. Absent in the early chain this targets.)
// Advance the median-time-past window with this block and append `block_mtp[height] = MTP(height)`
// (median of the ≤11 most recent block timestamps, Core's `GetMedianTimePast`). Assumes blocks are fed
// in order from the genesis anchor (block_mtp is indexed by absolute height).
fn median_u32(v: &[u32]) -> u32 { let mut s = v.to_vec(); s.sort_unstable(); s[s.len() / 2] }

fn push_mtp(j: &serde_json::Value, win: &mut Vec<u32>, block_mtp: &mut Vec<u32>) {
    // block_mtp[h] = MTP(h-1): median of the window through the PREVIOUS block, matching Core's BIP68
    // creation time GetMedianTimePast(coinHeight-1) and the guest's median(prev.recent_times).
    block_mtp.push(median_u32(win));
    let bt = j["time"].as_u64().unwrap() as u32;
    win.push(bt);
    if win.len() > 11 { let n = win.len() - 11; win.drain(0..n); }
}

// `block_mtp[h]` = GetMedianTimePast() of the block at height h (the host derives it from the chain it
// has already processed — same value an archive node holds for free). Used as the coin's creation-MTP
// on BOTH sides: committed when an output is created here, and looked up by the coin's committed height
// when it is spent. This is the real BIP68-time value (Core's median-time-past), replacing the earlier
// raw-block-timestamp proxy, and it stays consistent so the accumulator leaf matches across the coin's
// life.
fn build_block_carried(forest: &mut Forest, j: &serde_json::Value, block_mtp: &[u32]) -> BlockWitness {
    let height = j["height"].as_u64().unwrap() as u32;
    let bits = j["bits"].as_u64().unwrap() as u32;
    let time = j["time"].as_u64().unwrap() as u32;
    let nonce = j["nonce"].as_u64().unwrap() as u32;
    let version = j["version"].as_i64().unwrap_or(1) as i32;
    let prev = j["prev"].as_str().unwrap();
    let merkle = j["merkle"].as_str().unwrap();
    let cb_hex = j["coinbase_hex"].as_str().unwrap();
    let header = build_header_v(version, prev, &arr(rev(hx(merkle))), time, bits, nonce);
    let coinbase: Transaction = deserialize(&hx(cb_hex)).unwrap();
    let cb_txid = coinbase.compute_txid().to_byte_array();

    let root_prev = wire_stump(forest);
    let mut txids: Vec<[u8; 32]> = vec![cb_txid];
    let mut wtxids: Vec<[u8; 32]> = vec![[0u8; 32]]; // coinbase wtxid convention (pre-segwit: unused)
    let mut inputs: Vec<BlockInput> = Vec::new();
    let mut txs: Vec<PackedBytes> = Vec::new();
    let mut tx_prevouts: Vec<PackedBytes> = Vec::new();

    struct P { raw: Vec<u8>, tx: Transaction, prevouts: Vec<TxOut>, meta: Vec<(u32, bool, u32)>, txid: [u8; 32] }
    let mut ptxs: Vec<P> = Vec::new();
    for tx in j["txs"].as_array().unwrap() {
        let raw = hx(tx["raw"].as_str().unwrap());
        let t: Transaction = deserialize(&raw).unwrap();
        let mut prevouts = Vec::new();
        let mut meta = Vec::new();
        for p in tx["prevouts"].as_array().unwrap() {
            prevouts.push(TxOut {
                value: Amount::from_sat(p["value"].as_u64().unwrap()),
                script_pubkey: ScriptBuf::from_bytes(hx(p["spk"].as_str().unwrap())),
            });
            let h = p["coin_height"].as_u64().map(|x| x as u32).unwrap_or(0);
            let cb = p["coin_is_coinbase"].as_u64().map(|x| x != 0).unwrap_or(false);
            // Real BIP68-time value: the median-time-past of the coin's CREATION block, derived by the
            // host (not the JSON's raw block timestamp). Matches what was committed when the coin was
            // created (below), so the leaf is found in the accumulator.
            let mtp = block_mtp.get(h as usize).copied().unwrap_or(0);
            meta.push((h, cb, mtp));
        }
        let txid = t.compute_txid().to_byte_array();
        ptxs.push(P { raw, tx: t, prevouts, meta, txid });
    }

    // This block's created output leaves (coinbase + every tx, unspendable skipped) — the guest detects
    // in-block spends (H1) by LEAF membership here (main.rs `created_at`), NOT by txid. A leaf carries the
    // coin's height, so a spend of an earlier coin whose funding tx shares a txid with a tx in this block
    // (pre-BIP34 coinbase-txid collision) has a DIFFERENT leaf and stays correctly external — keying on
    // txid would false-positive it, diverge from the guest, and stall the frontier at that block.
    let self_mtp = block_mtp.get(height as usize).copied().unwrap_or(time);
    let mut created: std::collections::HashSet<Hash> = std::collections::HashSet::new();
    for v in 0..coinbase.output.len() {
        if !out_spendable(coinbase.output[v].script_pubkey.as_bytes()) { continue; }
        created.insert(out_leaf_of(&coinbase, &cb_txid, v, height, true, self_mtp));
    }
    for p in &ptxs {
        for v in 0..p.tx.output.len() {
            if !out_spendable(p.tx.output[v].script_pubkey.as_bytes()) { continue; }
            created.insert(out_leaf_of(&p.tx, &p.txid, v, height, false, self_mtp));
        }
    }
    let mut spent_in_block: std::collections::HashSet<Hash> = std::collections::HashSet::new();

    for (tx_i, p) in ptxs.iter().enumerate() {
        txids.push(p.txid);
        wtxids.push(p.tx.compute_wtxid().to_byte_array());
        let prevouts_blob = serialize(&p.prevouts);
        txs.push(PackedBytes(p.raw.clone()));
        tx_prevouts.push(PackedBytes(prevouts_blob));
        for i in 0..p.tx.input.len() {
            let (ch, cb, mtp) = p.meta[i];
            let o = &p.prevouts[i];
            let op = p.tx.input[i].previous_output;
            let coin = coin_leaf(&op.txid.to_byte_array(), op.vout, o.value.to_sat(), o.script_pubkey.as_bytes(), ch, cb, mtp);
            if created.contains(&coin) {
                // IN-BLOCK (H1): leaf matches a coin created earlier in this block (the guest's exact
                // rule) — never entered the accumulator: dummy proof, no delete. Script still verifies.
                spent_in_block.insert(coin);
                inputs.push(BlockInput {
                    tx_idx: tx_i as u32, input_idx: i as u32,
                    global_pos: 0, coin_height: ch, coin_is_coinbase: cb as u32, coin_mtp: mtp, tx_first: (i == 0) as u32,
                    proof_i: WireProof { leaf: PackedHash(coin), position: 0, siblings: PackedHashes(vec![]) },
                    proof_last: WireProof { leaf: PackedHash(coin), position: 0, siblings: PackedHashes(vec![]) },
                });
            } else {
                // EXTERNAL: prove inclusion in the carried forest, delete.
                let pos = forest.find(&coin)
                    .expect("spent coin not in carried accumulator (bad metadata)");
                let last = forest.leaves.len() - 1;
                inputs.push(BlockInput {
                    tx_idx: tx_i as u32, input_idx: i as u32,
                    global_pos: pos as u64, coin_height: ch, coin_is_coinbase: cb as u32, coin_mtp: mtp, tx_first: (i == 0) as u32,
                    proof_i: wire_proof(&forest.prove(pos)), proof_last: wire_proof(&forest.prove(last)),
                });
                forest.delete(pos);
            }
        }
    }

    // Created coins: coinbase outputs then each tx's outputs, in canonical order — skipping unspendable
    // outputs (H3) and coins spent within this same block (H1). Must match the guest's surviving set.
    // This block's creation-MTP (committed on every output leaf); a later block spending these coins
    // looks up the identical value by height, so the leaves match.
    // self_mtp computed above (with `created`). SURVIVING outputs only: unspendable skipped AND
    // in-block-spent cancelled (leaf ∈ spent_in_block), matching the guest's surviving set exactly.
    let mut new_outputs = Vec::new();
    let add_out = |tx: &Transaction, txid: &[u8; 32], is_cb: bool, forest: &mut Forest, no: &mut Vec<Hash>| {
        for v in 0..tx.output.len() {
            if !out_spendable(tx.output[v].script_pubkey.as_bytes()) { continue; }
            let l = out_leaf_of(tx, txid, v, height, is_cb, self_mtp);
            if spent_in_block.contains(&l) { continue; }
            forest.add(l);
            no.push(l);
        }
    };
    // F3: BIP30 grandfathered duplicate-coinbase overwrite. Blocks 91842 (dup of 91812) and 91880 (dup
    // of 91722) re-use an earlier still-unspent coinbase's outpoint; Core OVERWRITES the old coin. Delete
    // the superseded coinbase output leaf(s) — this same coinbase's spendable outputs committed at the OLD
    // height/MTP — from the carried forest BEFORE add_out re-adds them at the current height, and hand the
    // guest the deletion witness. Without this the bridge emits bip30:None and the guest (which MANDATES
    // the overwrite at these two hashes) rejects the block, making 91842/91880 unprovable via the bridge.
    // old_mtp = block_mtp[old_height] = MTP(old_height-1) = exactly what the superseded leaf committed at
    // creation, so the leaf is found. Height-gated here; the guest cross-checks the block HASH
    // (BIP30_OVERWRITE_A/B), so a spurious height hit on a non-matching block is still rejected there.
    let bip30 = if height == 91842 || height == 91880 {
        let old_height: u32 = if height == 91842 { 91812 } else { 91722 };
        let old_mtp = block_mtp.get(old_height as usize).copied().unwrap_or(0);
        let mut dels: Vec<Bip30Del> = Vec::new();
        for v in 0..coinbase.output.len() {
            if !out_spendable(coinbase.output[v].script_pubkey.as_bytes()) { continue; }
            let l = out_leaf_of(&coinbase, &cb_txid, v, old_height, true, old_mtp);
            let pos = forest.find(&l)
                .expect("BIP30 superseded coinbase leaf not in carried accumulator");
            let last = forest.leaves.len() - 1;
            dels.push(Bip30Del { global_pos: pos as u64, proof_i: wire_proof(&forest.prove(pos)), proof_last: wire_proof(&forest.prove(last)) });
            forest.delete(pos);
        }
        Some(Bip30Overwrite { old_height, old_mtp, dels })
    } else {
        None
    };
    add_out(&coinbase, &cb_txid, true, forest, &mut new_outputs);
    for p in &ptxs {
        add_out(&p.tx, &p.txid, false, forest, &mut new_outputs);
    }
    let root_next = wire_stump(forest);
    let (in_smt_root, smt) = smt_witness_standalone(cb_txid, cb_spendable_outputs(&coinbase),
        &cb_spends_from(&inputs, &txs));
    BlockWitness { header, height, coinbase_tx: hx(cb_hex), txids: PackedHashes(txids), root_prev, txs, tx_prevouts, inputs, root_next, bip30, in_smt_root, smt }
}

// `prove-chunk <i>`: prove chunk i's scripts, write the receipt to chunk_<i>.hzk (or $HAZYNC_OUT).
fn prove_chunk(idx: usize) {
    use std::time::Instant;
    let (_anchor, w) = build_full();
    let bounds = chunk_bounds(&w, nchunks_env());
    let (lo, hi) = *bounds.get(idx).unwrap_or_else(|| panic!(
        "prove-chunk {idx}: block has only {} chunks at HAZYNC_CHUNKS={}", bounds.len(), nchunks_env()));
    let mut b = ExecutorEnv::builder();
    b.segment_limit_po2(seg_po2());
    b.write(&4u32).unwrap();
    b.write(&w.height).unwrap();
    b.write(&chunk_block_hash(&w.header)).unwrap(); // block hash for flag exceptions
    write_chunk_inputs(&mut b, &w, lo, hi);
    let t = Instant::now();
    // EXECUTE FIRST, then prove (#145). This printed nothing until it finished, so while a chunk was
    // running its log was EMPTY and "wedged" was indistinguishable from "working". That is what hid
    // hazync#147 for 76 minutes and then 3h38m: the only thing that gave it away was nvidia-smi
    // showing 0% with the process alive, which is a thing you have to already suspect to go and look
    // at. A start line and periodic progress make a hang visible in the place anyone would look first.
    let opts = ProverOpts::succinct();
    let server = risc0_zkvm::get_prover_server(&opts).expect("prover server");
    let ctx = risc0_zkvm::VerifierContext::default();
    let session = risc0_zkvm::ExecutorImpl::from_elf(b.build().unwrap(), METHOD_ELF)
        .unwrap().run().unwrap();
    crate::lz::require_committed(session.exit_code, "chunk execution");
    let nseg = session.segments.len();
    println!("chunk {idx}: {} inputs, {nseg} segments at po2 {} — proving", hi - lo, seg_po2());
    // SCALING: prove the chunk to a SUCCINCT receipt (not the default composite). This runs the
    // STARK-to-STARK "lift" NOW, in parallel across the chunk fleet — so agg-chunks resolves each
    // assumption cheaply instead of lifting all N composite receipts sequentially.
    //
    // #237: driven segment-by-segment so every segment goes through the #119 retry. A fault used to
    // cost the whole chunk; it now costs one segment.
    let receipt = prove_session_resilient(&server, &ctx, &session);
    receipt.verify(METHOD_ID).unwrap();
    let out = std::env::var("HAZYNC_OUT").unwrap_or_else(|_| format!("chunk_{idx}.hzk"));
    // Write then rename: seg-serve's HAZYNC_AGG_EARLY polls for this file and must never read half of it.
    std::fs::write(format!("{out}.tmp"), bincode::serialize(&receipt).unwrap()).unwrap();
    std::fs::rename(format!("{out}.tmp"), &out).unwrap();
    println!("chunk {idx} ({} inputs) proved in {:.0}s -> {out}", hi - lo, t.elapsed().as_secs_f64());
}

// ============ ARCHIVE-NODE BRIDGE — persistent forest driver over a local bitcoind ============
// `host bridge`: drive ONE resident Forest forward over the chain from a local bitcoind, emitting per
// block a bundle {in-boundary, witness} with the REAL root_prev + inclusion proofs — so a prover can
// prove [n..n] with NO bridge_pass replay (kills the quadratic; closes S3). Reuses build_block_carried +
// push_mtp exactly; a parallel UTXO-metadata map (outpoint -> value,spk,creation-height,coinbase) supplies
// the prevouts build_block_carried needs.
/// One coinbase-output spend, with the proof of that coinbase's count under the root as it stands
/// when the spend is applied. Mirrors `hazync_coinbase_smt::bip30::Spend`.
#[derive(Serialize, Deserialize, Clone)]
struct SmtSpend { coinbase_txid: [u8; 32], current_count: u32, proof: SmtProof, tracked: bool }
/// A tracked transaction this block creates. Mirrors `hazync_coinbase_smt::bip30::TrackedTx`.
#[derive(Serialize, Deserialize, Clone)]
struct SmtTracked { txid: [u8; 32], outputs: u32, prior: Option<u32>, proof: SmtProof }

/// Everything the guest needs to run the BIP30 transition for one block.
///
/// The guest takes ONLY the proofs from here. `coinbase_txid`, `coinbase_outputs` and the spend list
/// are all things it derives for itself from data it already validates — anything it read instead of
/// derived would be something a prover could lie about, and the whole point of the structure is that
/// the check cannot be talked out of.
/// `coinbase_txid`/`coinbase_outputs` are CROSS-CHECKS, not inputs: the guest derives both for itself
/// and refuses the block if they disagree. They exist because a disagreement otherwise surfaces as an
/// unexplained `BadProof` three steps later — which is exactly what happened the first time the guest
/// counted the coinbase's outputs at the wrong point in the function.
#[derive(Serialize, Deserialize, Clone)]
struct SmtBlockWitness {
    coinbase_txid: [u8; 32],
    coinbase_outputs: u32,
    absence_proof: SmtProof,
    spends: Vec<SmtSpend>,
    /// `Some(prior_count)` ONLY at the two blocks BIP30 grandfathers (91842, 91880), where the
    /// duplicated coinbase was still UNSPENT so no absence proof can exist. LAST, because risc0's
    /// serde is positional and appending is the only change that does not renumber every field.
    smt_overwrite: Option<u32>,
    coinbase_prior: Option<u32>,
    tracked: Vec<SmtTracked>,
}

/// The coinbase txid of every input that spends a coinbase output, in the block's tx-then-input
/// order — derived the same way the guest derives it, from the same raw transaction bytes.
///
/// The guest does NOT take this list from the witness; it reads each prevout out of the transaction
/// itself. This exists so the host can build a witness whose proofs line up with what the guest will
/// independently compute, and any disagreement shows up as a refused proof rather than a bad one.
fn cb_spends_from(inputs: &[BlockInput], txs: &[PackedBytes]) -> Vec<[u8; 32]> {
    let mut out = Vec::new();
    for inp in inputs {
        if inp.coin_is_coinbase == 0 { continue; }
        let raw = txs.get(inp.tx_idx as usize)
            .unwrap_or_else(|| panic!("cb_spends_from: tx_idx {} out of range ({} txs)", inp.tx_idx, txs.len()));
        let tx: Transaction = deserialize(&raw.0)
            .unwrap_or_else(|e| panic!("cb_spends_from: tx_idx {} did not parse: {e}", inp.tx_idx));
        let i = tx.input.get(inp.input_idx as usize)
            .unwrap_or_else(|| panic!("cb_spends_from: input_idx {} out of range ({} inputs)",
                                      inp.input_idx, tx.input.len()));
        out.push(i.previous_output.txid.to_byte_array());
    }
    out
}

/// Count of a coinbase's SPENDABLE outputs — the value the guest derives from its own leaf gather.
fn cb_spendable_outputs(coinbase: &Transaction) -> u32 {
    coinbase.output.iter().filter(|o| out_spendable(o.script_pubkey.as_bytes())).count() as u32
}

/// A self-consistent SMT transition for a block considered in ISOLATION, for the fixture and
/// synthetic-block paths that have no chain history behind them.
///
/// The tree is seeded with exactly the coinbases this block spends, so the transition is valid on its
/// own terms. It is NOT the chain's real SMT state and must never be used for proving — only the
/// bridge, which replays from genesis, knows that. Its purpose is to keep the negative tests testing
/// what they are about (script flags, merkle mutation, unbound prevouts) rather than failing on an
/// unrelated BIP30 input they were never written to exercise.
fn smt_witness_standalone(
    coinbase_txid: [u8; 32],
    coinbase_outputs: u32,
    coinbase_spends: &[[u8; 32]],
) -> ([u8; 32], SmtBlockWitness) {
    let mut t = Smt::new();
    for s in coinbase_spends {
        let c = t.get(s).unwrap_or(0);
        t.insert(*s, c + 1);
    }
    let root_in = t.root();
    let w = smt_advance(&mut t, coinbase_txid, coinbase_outputs, coinbase_spends, false).0;
    (root_in, w)
}

/// `smt_witness_standalone`, aware of the BIP30-grandfathered heights (91,842 / 91,880: the duplicated coinbase is still
/// UNSPENT, so the witness carries the overwrite claim; the guest gates that on the block HASH) and of blocks that spend
/// or create tracked coins (bip30.rs).
fn smt_witness_standalone_ex(height: u32, coinbase_txid: [u8; 32], coinbase_outputs: u32, coinbase_spends: &[[u8; 32]],
                             spend_tracked: &[bool], tracked_new: &[([u8; 32], u32)]) -> ([u8; 32], SmtBlockWitness) {
    let mut t = Smt::new();
    for s in coinbase_spends {
        let c = t.get(s).unwrap_or(0);
        t.insert(*s, c + 1);
    }
    // Mainnet's two BIP30 overwrites. On another network these are ordinary heights.
    let grandfathered = methods::NETWORK == "mainnet" && (height == 91_842 || height == 91_880);
    if grandfathered { t.insert(coinbase_txid, coinbase_outputs.max(1)); }
    let root_in = t.root();
    (root_in, smt_advance_ex(&mut t, coinbase_txid, coinbase_outputs, coinbase_spends, grandfathered, spend_tracked, tracked_new).0)
}

/// Advance the coinbase SMT by one block and emit the proofs the guest will need.
///
/// PURE AND SEPARATE FROM THE BRIDGE ON PURPOSE. The order of operations here has to match
/// `bip30::apply_block` exactly — every proof is against the root as it stands at that step, not
/// against the incoming root — and a mismatch is refused by the guest rather than mis-folded, so it
/// shows up as a stalled board with no obvious cause. Extracting it means the agreement between the
/// two is a native test (`smt_emission_round_trips_through_apply_block`) instead of a comment.
///
/// `coinbase_spends` is the coinbase txid of every input in this block that spends a coinbase output,
/// in the block's own tx-then-input order.
///
/// # What the sequencing actually constrains — narrower than it looks
///
/// A proof for key `k` is made of the siblings OFF `k`'s path, so it is completely unaffected by any
/// change to `k`'s own value. Two consequences, both established by running the mistakes as positive
/// controls rather than by reasoning about them:
///
///   * Taking the absence proof after its own insert changes nothing — the two are byte-identical.
///   * Chained spends of the SAME coinbase do not invalidate each other's proofs either.
///
/// What does bite is ordering across DISTINCT keys: the coinbase insert sits on a path that is a
/// sibling of every other key that branches off it, so a spend proof taken before that insert is
/// stale and is refused. That is the one real constraint here, and it is the one the round-trip test
/// is built to catch — the first two controls passed against a deliberately broken implementation,
/// which is exactly why they are recorded here instead of being trusted as tests.
fn smt_advance(
    smt: &mut Smt,
    coinbase_txid: [u8; 32],
    coinbase_outputs: u32,
    coinbase_spends: &[[u8; 32]],
    grandfathered: bool,
) -> (SmtBlockWitness, Vec<([u8; 32], Option<u32>)>) {
    smt_advance_ex(smt, coinbase_txid, coinbase_outputs, coinbase_spends, grandfathered, &[], &[])
}

/// `smt_advance` with tracked coins: `spend_tracked` is parallel to `coinbase_spends`, or empty for none, and
/// `tracked_new` lists the tracked transactions this block creates, in block order.
fn smt_advance_ex(
    smt: &mut Smt,
    coinbase_txid: [u8; 32],
    coinbase_outputs: u32,
    coinbase_spends: &[[u8; 32]],
    grandfathered: bool,
    spend_tracked: &[bool],
    tracked_new: &[([u8; 32], u32)],
) -> (SmtBlockWitness, Vec<([u8; 32], Option<u32>)>) {
    use hazync_coinbase_smt::bip30::{after_spend, is_live, REPEATABLE, SPENT};
    // 1. Normally: absence, against the INCOMING root, before any of this block's own updates.
    //
    //    At 91842/91880 (audit#3 F-1) the duplicated coinbase is still UNSPENT — that is why BIP30
    //    exists and why Core exempts these two heights — so absence is unprovable and the witness
    //    carries a MEMBERSHIP proof of the prior count instead. The guest gates this on the block
    //    HASH it derives, so a spurious height match here is still rejected there.
    // ⚠ CAPTURED BEFORE ANY MUTATION (hazync#484). One (key, prior) pair per key this function
    // touches inverts all three of its shapes — the coinbase insert, the grandfathered remove, and
    // each spend's decrement — without the rewind having to know which happened.
    let mut prior: Vec<([u8; 32], Option<u32>)> = vec![(coinbase_txid, smt.get(&coinbase_txid))];
    let absence_proof = smt.prove(&coinbase_txid);
    let smt_overwrite = if grandfathered {
        let prior = smt.get(&coinbase_txid).unwrap_or_else(|| panic!(
            "bridge: grandfathered BIP30 block duplicates coinbase {} which the SMT does not hold — \
             the tree and the chain have diverged",
            coinbase_txid.iter().map(|b| format!("{b:02x}")).collect::<String>()));
        Some(prior)
    } else {
        None
    };
    let may_repeat = |v: Option<u32>| v.filter(|c| *c == REPEATABLE);
    let coinbase_prior = if grandfathered { None } else { may_repeat(smt.get(&coinbase_txid)) };
    if coinbase_outputs > 0 { smt.insert(coinbase_txid, coinbase_outputs); }
    else if grandfathered { smt.remove(&coinbase_txid); }

    // Tracked transactions, proved in the order `apply_block` applies them. A live entry is emitted as absent: the
    // guest's check then fails, which is its bad-txns-BIP30.
    let mut tracked = Vec::with_capacity(tracked_new.len());
    for (t, n) in tracked_new {
        if !prior.iter().any(|(k, _)| k == t) { prior.push((*t, smt.get(t))); }
        tracked.push(SmtTracked { txid: *t, outputs: *n, prior: may_repeat(smt.get(t)), proof: smt.prove(t) });
        smt.insert(*t, *n);
    }

    // 2. Decrement each spend, proving against the root as it stands at that step. This is what lets
    //    a block spend two outputs of the same coinbase: the second proof sees the first's effect.
    let mut spends = Vec::with_capacity(coinbase_spends.len());
    // libzync filters by the tracked bit, so it must have one per spend: an empty list would drop tracked decrements.
    assert!(coinbase_spends.is_empty() || spend_tracked.len() == coinbase_spends.len(),
            "smt_advance_ex: libzync needs the tracked bit of every coinbase spend ({} spends, {} bits)", coinbase_spends.len(), spend_tracked.len());
    for (i, t) in coinbase_spends.iter().enumerate() {
        let is_tracked = spend_tracked.get(i).copied().unwrap_or(false);
        // libzync: only tracked coins are counted (agg_lite.rs: an untracked coinbase's count changes no verdict).
        if !is_tracked { continue; }
        let cur = smt.get(t).unwrap_or(0);
        // ⚠ FIRST TOUCH ONLY. A block may spend two outputs of the same coinbase; the second sees
        // the first's decrement, so recording it too would restore the intermediate value.
        if !prior.iter().any(|(k, _)| k == t) { prior.push((*t, smt.get(t))); }
        // HAZYNC_RIG: the differential rig gives the guest invalid blocks on purpose. The guest must be the one to
        // refuse them, so emit the honest witness of a spend from nothing and let `apply_block` reject it.
        if std::env::var("HAZYNC_RIG").is_ok() && !is_live(cur) {
            spends.push(SmtSpend { coinbase_txid: *t, current_count: cur, proof: smt.prove(t), tracked: is_tracked });
            continue;
        }
        let _ = SPENT;
        assert!(is_live(cur),
            "bridge: a block spends coinbase {} which the SMT holds at zero — the tree and the UTXO \
             set have diverged, which is a bug here and not a property of the chain",
            t.iter().map(|b| format!("{b:02x}")).collect::<String>());
        spends.push(SmtSpend { coinbase_txid: *t, current_count: cur, proof: smt.prove(t), tracked: is_tracked });
        smt.insert(*t, after_spend(cur, is_tracked));
    }
    (SmtBlockWitness { coinbase_txid, coinbase_outputs, absence_proof, spends, smt_overwrite, coinbase_prior, tracked }, prior)
}

#[derive(Serialize, Deserialize)]
struct Bundle {
    height: u32, in_tip: [u8; 32],
    in_roots: Vec<Option<[u8; 32]>>, in_leaves: u64,
    in_nbits: u32, in_time: u32, in_epoch_start: u32, in_recent: Vec<u32>,
    witness: BlockWitness,
}

fn main() {
    let _report_119_on_exit = Report119OnExit;
    // Prove IN-PROCESS unless the operator asked for something else.
    //
    // risc0's default backend shells out to `r0vm`, a separate ~109 MB binary that is not part of this
    // release. A contributor who downloads only the prebuilt host — which is exactly what CONTRIBUTING
    // tells them to do — therefore gets `No such file or directory (os error 2)` on their first prove,
    // naming nothing. It affected the CPU binary specifically: the CUDA build links its prover in, so
    // the GPU path worked and the "no GPU still works" path did not.
    //
    // The binary is now built with risc0-zkvm's `prove` feature (see Cargo.toml), so the local prover
    // is linked in and default_prover() already selects it when RISC0_PROVER is unset. This makes the
    // intent explicit and survives a future features change; set only when unset, so RISC0_PROVER=ipc
    // / bonsai still work for anyone who wants them.
    if std::env::var_os("RISC0_PROVER").is_none() {
        std::env::set_var("RISC0_PROVER", "local");
    }
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::filter::EnvFilter::from_default_env())
        .init();

    // A prover memory failure surfaces as a raw unwrap deep inside the GPU HAL:
    //
    //   panicked at risc0-zkp/src/hal/cuda.rs:246: called `Result::unwrap()` on an `Err` value:
    //   allocation failed on evaluated: 134217728 bytes  Caused by: "out of memory"
    //
    // naming neither the knob that fixes it nor the fact that a smaller segment would just work. On a
    // CUDA 13 driver the default po2 21 fails on an OTHERWISE IDLE L40S with 45 GB free (#97), so this
    // is the first thing a contributor on UpCloud's current GPU image meets. `coordinator/hazync` walks
    // a retry ladder and absorbs it; a direct `host prove-block` gets the bare panic, which is what
    // docs/history/RELEASE_PLAN.md §1.4 says must not happen — an OOM has to name HAZYNC_SEG_PO2 and a value to try.
    //
    // The hook ADDS the remedy and leaves the original panic intact. The underlying message is still
    // the most accurate description of what happened, and replacing it would trade one unhelpful
    // failure for another.
    let prior = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        prior(info);
        let msg = info.payload().downcast_ref::<String>().map(|s| s.as_str())
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or("");
        if msg.contains("out of memory") || msg.contains("allocation failed") {
            let po2 = seg_po2();
            eprintln!("\nhazync: that is a prover memory failure, not a bad block.");
            eprintln!("  The usual cause is TWO proves sharing one card, not a segment that is too big.");
            eprintln!("  Measured on an L40S (46 GB): one prove at HAZYNC_SEG_PO2=21 peaks near 22 GB, so");
            eprintln!("  a single prove fits easily and two do not (#97). `hazync run` serialises GPU work");
            eprintln!("  through a lock; a direct `host prove-*` does not, so it can land on top of one.");
            eprintln!("  Look for another prove first:");
            eprintln!("      nvidia-smi --query-compute-apps=pid,used_memory --format=csv");
            eprintln!("  If the card really is yours alone, drop a rung — each step down roughly halves");
            eprintln!("  the working memory one segment needs, at a few seconds' cost per prove:");
            eprintln!("      HAZYNC_SEG_PO2={} <the same command again>", po2.saturating_sub(1));
        }
    }));

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "agg-exec-fake") {
        agg_exec_fake();
        return;
    }
    if let Some(p) = args.iter().position(|a| a == "exec-chunk") {
        // libzync negative tests: execute ONE chunk with the current hint env and report its verdict.
        let idx: usize = args.get(p + 1).and_then(|s| s.parse().ok()).expect("exec-chunk <index>");
        let (_, w) = build_full();
        let bounds = chunk_bounds(&w, nchunks_env());
        let (r, log) = execute_chunks_parallel(&w, &bounds[idx..idx + 1], 1).pop().expect("one chunk");
        for l in String::from_utf8_lossy(&log).lines().filter(|l| l.contains("batch:")) { println!("  {l}"); }
        let j = &r.journal.bytes;
        let word = |k: usize| u32::from_le_bytes(j[4 * k..4 * k + 4].try_into().unwrap());
        use sha2::Digest;
        println!("exec-chunk {idx} [{}..{}): kind={:#x} all_valid={} journal sha256 {}", bounds[idx].0, bounds[idx].1,
                 word(0), word(1), hex(&sha2::Sha256::digest(j)));
        return;
    }
    if let Some(p) = args.iter().position(|a| a == "libzync-anchor-json") {
        // libzync-anchor-json <out> | --genesis <out>: the network's genesis state (what a real proof starts
        // from), or the state BEFORE $HAZYNC_BLOCK as build_full derives it from the block file: tip, difficulty
        // and time context, a forest holding filler and that block's spent coins, zero work, a zero SwiftSync
        // total. The latter is a DEMONSTRATION anchor; libzync-verify reports proofs on it as DEMO, never OK.
        if args.get(p + 1).map(|s| s.as_str()) == Some("--genesis") {
            std::fs::write(&args[p + 2], serde_json::to_string(&genesis_anchor()).unwrap()).unwrap();
            println!("genesis anchor ({}) -> {}", methods::NETWORK, args[p + 2]);
            return;
        }
        let (mut anchor, w) = build_full();
        // Block files list the prior 11 times NEWEST first (mkblock.py); a ChainState window is OLDEST first,
        // because each step appends the new time and drops element 0. A single block only uses the median, so
        // the order never mattered before; a chain of proofs on this anchor would drop the wrong time and every
        // later block's MTP would differ from its chunks' (found by the pre-H dry run, 969,997).
        anchor.recent_times.reverse();
        std::fs::write(&args[p + 1], serde_json::to_string(&anchor).unwrap()).unwrap();
        println!("anchor for block {} (state at {}) -> {}", w.height, anchor.height, args[p + 1]);
        return;
    }
    if let Some(p) = args.iter().position(|a| a == "journal-sha") {
        let f = args.get(p + 1).expect("journal-sha <rcpt>");
        let r: risc0_zkvm::Receipt = bincode::deserialize(&std::fs::read(f).unwrap_or_else(|e| panic!("{f}: {e}"))).expect("receipt");
        r.verify(METHOD_ID).unwrap_or_else(|e| panic!("{f} does not verify against this image: {e}"));
        println!("JOURNAL_SHA256 {}", lz::journal_sha(&r));
        return;
    }
    if let Some(p) = args.iter().position(|a| a == "libzync-prep") {
        let n = |k: usize| -> u32 { args.get(p + k).and_then(|s| s.parse().ok()).expect("libzync-prep <lo> <hi> <json dir> <anchor.json|genesis> <out dir>") };
        prep::lz_prep_cmd(n(1), n(2), &args[p + 3], &args[p + 4], &args[p + 5]);
        return;
    }
    if let Some(p) = args.iter().position(|a| a == "libzync-join") {
        let f = |k: usize| args.get(p + k).cloned().expect("libzync-join <left.rcpt> <right.rcpt> <out.rcpt>");
        lz::lz_join_cmd(&f(1), &f(2), &f(3));
        return;
    }
    if let Some(p) = args.iter().position(|a| a == "libzync-prove-range") {
        let n = |k: usize| -> u32 { args.get(p + k).and_then(|s| s.parse().ok()).expect("libzync-prove-range <lo> <hi> <json dir> <anchor.json> <out dir>") };
        lz_prove_range_cmd(n(1), n(2), &args[p + 3], &args[p + 4], &args[p + 5]);
        return;
    }
    if let Some(p) = args.iter().position(|a| a == "signet-check-test") {
        // signet-check-test <block json>: the BIP325 check on the real block (must pass), then with ONE
        // other txid changed (the signed commitment then covers a different merkle root: must fail).
        assert!(methods::NETWORK == "signet", "signet-check-test needs a signet guest");
        std::env::set_var("HAZYNC_BLOCK", &args[p + 1]);
        let (_, w) = build_full();
        let mut txids: Vec<u8> = Vec::new();
        let j = read_block_json(std::path::Path::new(&args[p + 1]).parent().unwrap().to_str().unwrap(), w.height);
        for t in j["txs"].as_array().unwrap() {
            let tx: Transaction = deserialize(&hx(t["raw"].as_str().unwrap())).unwrap();
            txids.extend_from_slice(&tx.compute_txid().to_byte_array());
        }
        let run = |ids: &[u8]| -> u32 {
            let mut b = ExecutorEnv::builder();
            b.write(&16u32).unwrap();
            b.write(&w.header).unwrap();
            b.write(&w.coinbase_tx).unwrap();
            b.write(&ids.to_vec()).unwrap();
            lz_execute(b.build().unwrap()).expect("mode 16").journal.decode::<u32>().unwrap()
        };
        let good = run(&txids);
        println!("signet-check-test block {}: real block -> {good}", w.height);
        let bad = if txids.is_empty() {
            // coinbase-only: add a phantom transaction instead
            run(&[7u8; 32])
        } else {
            let mut t = txids.clone();
            t[0] ^= 1;
            run(&t)
        };
        println!("signet-check-test block {}: one txid changed -> {bad}", w.height);
        println!("{}", if good == 1 && bad == 0 { "SIGNET_CHECK_OK" } else { "SIGNET_CHECK_FAIL" });
        return;
    }
    if let Some(p) = args.iter().position(|a| a == "libzync-snark-wrap") {
        libzync_snark_wrap_cmd(args.get(p + 1).expect("libzync-snark-wrap <receipt> <out>"),
                               args.get(p + 2).expect("libzync-snark-wrap <receipt> <out>"));
        return;
    }
    if let Some(p) = args.iter().position(|a| a == "libzync-verify") {
        libzync_verify_fail_hook();
        // Positional: <receipt> [anchor.json]. Flags anywhere after: --expect <h>:<hash>, --mempool,
        // --mempool-url <base> (chain identity, as swiftsync-verify).
        let mut pos: Vec<&str> = Vec::new();
        let (mut expect, mut mempool, mut cli): (Option<String>, Option<String>, Option<String>) = (None, None, None);
        let mut it = args[p + 1..].iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--expect" => expect = Some(it.next().expect("--expect <height>:<hash>").clone()),
                "--mempool" => mempool = Some(String::new()),
                "--mempool-url" => mempool = Some(it.next().expect("--mempool-url <base>").clone()),
                "--bitcoin-cli" => cli = Some(it.next().expect("--bitcoin-cli \"<command>\"").clone()),
                x if x.starts_with("--") => panic!("libzync-verify: unknown flag {x} (a mistyped identity flag must not silently skip the check)"),
                x => pos.push(x),
            }
        }
        let usage = "libzync-verify <receipt> [anchor.json] [--expect <h>:<hash>] [--mempool | --mempool-url <base>] [--bitcoin-cli \"<command>\"]";
        assert!(pos.len() <= 2, "{usage}");
        // The retired hints-file argument ("-" or a .hints file) must not be read as an anchor.
        assert!(pos.get(1).is_none_or(|a| *a != "-" && !a.ends_with(".hints")), "libzync-verify takes no hints file (E2): {usage}");
        libzync_verify_cmd(pos.first().expect(usage), pos.get(1).copied(), expect.as_deref(), mempool.as_deref(), cli.as_deref());
        return;
    }
    if let Some(p) = args.iter().position(|a| a == "libzync-hints") {
        libzync_hints_cmd(args.get(p + 1).expect("libzync-hints <out dir>"));
        return;
    }
    if let Some(p) = args.iter().position(|a| a == "prove-chunk") {
        let idx: usize = args.get(p + 1).and_then(|s| s.parse().ok()).expect("prove-chunk <index>");
        prove_chunk(idx);
        return;
    }
    if let Some(p) = args.iter().position(|a| a == "dump-snapshot") {
        let out = args.get(p + 1).expect("dump-snapshot <out-file>");
        cmd_dump_snapshot(out);
        return;
    }
    if args.iter().any(|a| a == "segment-size") {
        segment_size_cmd();
        return;
    }
    if let Some(p) = args.iter().position(|a| a == "receipt-digest") {
        let f = args.get(p + 1).expect("receipt-digest <file>");
        receipt_digest_cmd(f);
        return;
    }
    if args.iter().any(|a| a == "seg-coordinate-tree") {
        seg_coordinate_tree_cmd();
        return;
    }
    if args.iter().any(|a| a == "seg-serve") {
        seg_serve_cmd();
        return;
    }
    if let Some(p) = args.iter().position(|a| a == "seg-connect") {
        let addr = args.get(p + 1).expect("seg-connect <host:port>");
        seg_connect_cmd(addr);
        return;
    }
    if args.iter().any(|a| a == "seg-prove-one") {
        seg_prove_one_cmd();
        return;
    }
    if args.iter().any(|a| a == "seg-join") {
        seg_join_cmd();
        return;
    }
    if args.iter().any(|a| a == "seg-work") {
        seg_work_cmd();
        return;
    }
    if args.iter().any(|a| a == "seg-coordinate") {
        seg_coordinate_cmd();
        return;
    }
    if args.iter().any(|a| a == "seg-distribute") {
        seg_distribute_cmd();
        return;
    }
    if args.iter().any(|a| a == "segment-mem") {
        segment_mem_cmd();
        return;
    }
    if args.iter().any(|a| a == "exec-time") {
        exec_time_cmd();
        return;
    }
    if args.iter().any(|a| a == "chunk-profile") {
        chunk_profile();
        return;
    }
    if args.iter().any(|a| a == "agg-chunks") {
        agg_chunks();
        return;
    }
    if args.iter().any(|a| a == "core-vectors") {
        bench::core_vectors_cmd(&args);
        return;
    }
    if args.iter().any(|a| a == "test-locks") {
        test_locks_cmd();
        return;
    }
    if args.iter().any(|a| a == "test-merkle") {
        test_merkle_cmd();
        return;
    }
    if let Some(p) = args.iter().position(|a| a == "fold-range") {
        let (l, r, o) = (args.get(p + 1).expect("left"), args.get(p + 2).expect("right"), args.get(p + 3).expect("out"));
        // the original coordinator's fold is a libzync join (lz_coord.rs)
        lz::lz_join_cmd(l, r, o);
        return;
    }
    if args.iter().any(|a| a == "libzync-lineage") {
        lineage_cmd();
        return;
    }
    if let Some(p) = args.iter().position(|a| a == "xver-selftest") {
        xver_selftest_cmd(args.get(p + 1).expect("xver-selftest <receipt> [<image hex>]"), args.get(p + 2).map(|s| s.as_str()));
        return;
    }
    if let Some(p) = args.iter().position(|a| a == "xver-encode") {
        // the words a cross-version handover feeds the guest (for the in-guest test, experimental/libzync-bench/xver)
        let (i, o) = (args.get(p + 1).expect("xver-encode <receipt> <out>"), args.get(p + 2).expect("xver-encode <receipt> <out>"));
        let r: risc0_zkvm::Receipt = bincode::deserialize(&std::fs::read(i).unwrap()).expect("receipt");
        let risc0_zkvm::InnerReceipt::Succinct(s) = &r.inner else { panic!("not succinct") };
        let w: Vec<u8> = xver::encode(s).iter().flat_map(|x| x.to_le_bytes()).collect();
        std::fs::write(o, w).unwrap();
        return;
    }
    if let Some(p) = args.iter().position(|a| a == "libzync-handover") {
        // libzync-handover <pred receipt> <leaf_H.bin> <out>: block H as the handover step (docs/HANDOVER.md §5)
        let u = "libzync-handover <predecessor spine receipt> <leaf_H.bin> <out range file>";
        lz_coord::handover_cmd(args.get(p + 1).expect(u), args.get(p + 2).expect(u), args.get(p + 3).expect(u));
        return;
    }
    if args.iter().any(|a| a == "libzync-genesis-tip") {
        lz_coord::genesis_tip_cmd();
        return;
    }
    if args.iter().any(|a| a == "method-id") {
        // Print THIS host's guest image id so a contributor can check it matches a proof's guest.
        println!("METHOD_ID {}", method_id_hex());
        println!("  u32x8   {:?}", METHOD_ID);
        return;
    }
    if args.iter().any(|a| a == "seg-po2") {
        // The segment po2 this binary would use, so the CLI's retry ladder can start from the binary's
        // own per-backend default (21 cuda / 20 CPU) instead of guessing and wasting a duplicate attempt.
        println!("{}", seg_po2());
        return;
    }
    if let Some(p) = args.iter().position(|a| a == "snapshot-emit") {
        snapshot_emit_cmd(args.get(p + 1).map(|s| s.as_str()).unwrap_or("/root/bridge_bundles"),
                          args.get(p + 2).expect("snapshot-emit <bridge_dir> <out.snap>"));
        return;
    }
    if let Some(p) = args.iter().position(|a| a == "verify-any") {
        let f = args.get(p + 1).expect("verify-any <bin> [<lo>] [--check-end]");
        let (mut lo, mut check_end) = (None, false);
        for a in &args[p + 2..] {
            if a == "--check-end" { check_end = true; }
            else if lo.is_none() { lo = Some(a.parse().expect("verify-any <bin> [<lo>] [--check-end]: lo is a height")); }
            else { panic!("verify-any <bin> [<lo>] [--check-end]: unexpected argument {a}"); }
        }
        lz_coord::verify_any_cmd(f, lo, check_end);
        return;
    }
    if args.iter().any(|a| a == "bridge") {
        cmd_bridge();
        return;
    }
    if let Some(p) = args.iter().position(|a| a == "prove-range-bridge") {
        let n: u32 = args.get(p + 1).and_then(|s| s.parse().ok()).expect("prove-range-bridge <n>");
        lz_coord::prove_bridge_cmd(n);
        return;
    }
    // No command matched. The CORE design's demonstrations, block modes and range, spine and SNARK commands are
    // in history/ (a libzync image refuses those guest modes); prover/README.md lists this host's commands.
    eprintln!("usage: host <command> [args]  (see prover/README.md)");
    std::process::exit(2);
}

#[cfg(test)]
mod smt_bridge {
    use super::*;
    use hazync_coinbase_smt::bip30::{apply_block, BlockUpdate, Spend, TrackedTx, REPEATABLE};

    fn txid(n: u64) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut b = [0u8; 32];
        b[..8].copy_from_slice(&n.to_le_bytes());
        Sha256::digest(b).into()
    }

    fn to_update(cb: [u8; 32], nout: u32, w: &SmtBlockWitness) -> BlockUpdate {
        BlockUpdate {
            coinbase_prior: w.coinbase_prior,
            tracked: w.tracked.iter()
                .map(|t| TrackedTx { txid: t.txid, outputs: t.outputs, prior: t.prior, proof: t.proof.clone() })
                .collect(),
            coinbase_txid: cb,
            coinbase_outputs: nout,
            absence_proof: w.absence_proof.clone(),
            overwrite: w.smt_overwrite,
            spends: w.spends.iter()
                .map(|s| Spend { coinbase_txid: s.coinbase_txid, current_count: s.current_count,
                                 proof: s.proof.clone(), tracked: s.tracked })
                .collect(),
        }
    }

    /// THE test this refactor exists for: what the bridge emits is what the guest can consume.
    ///
    /// The coupling is invisible in either half alone — the bridge produces well-formed proofs and
    /// `apply_block` verifies well-formed proofs, but if the two disagree about the ORDER the roots
    /// are taken in, every proof is against the wrong intermediate state. That fails closed, so it
    /// would present as a board that silently stops advancing rather than as a wrong answer.
    #[test]
    fn smt_emission_round_trips_through_apply_block() {
        let mut bridge = Smt::new();
        let mut guest_root = bridge.root();

        // A chain where coinbases accumulate and later blocks spend them, including two outputs of
        // the SAME coinbase inside one block — the case that only works if proofs are sequenced.
        for h in 0..40u64 {
            let cb = txid(h);
            let nout = if h % 5 == 0 { 2 } else { 1 };
            let spends: Vec<[u8; 32]> = if h >= 10 {
                if h % 5 == 0 { vec![txid(h - 10), txid(h - 10)] } else { vec![txid(h - 10)] }
            } else { Vec::new() };
            // txid(h-10) has 2 outputs exactly when (h-10) % 5 == 0, i.e. when h % 5 == 0.

            // every spend tracked: the libzync bridge needs a tracked bit per coinbase spend, and CORE ignores them
            let w = smt_advance_ex(&mut bridge, cb, nout, &spends, false, &vec![true; spends.len()], &[]).0;
            guest_root = apply_block(&guest_root, &to_update(cb, nout, &w))
                .unwrap_or_else(|e| panic!("guest refused the bridge's own block at height {h}: {e:?}"));
            assert_eq!(guest_root, bridge.root(), "roots diverged at height {h}");
        }
    }

    /// A block whose coinbase duplicates one that still has unspent outputs must be refused, using
    /// exactly the witness the bridge would have produced for it.
    #[test]
    fn the_bridge_cannot_produce_a_witness_that_passes_a_real_bip30_violation() {
        let mut bridge = Smt::new();
        let cb = txid(1);
        let w0 = smt_advance(&mut bridge, cb, 1, &[], false).0;
        let root0 = apply_block(&Smt::new().root(), &to_update(cb, 1, &w0)).unwrap();

        // Now try the same coinbase again while it is still unspent.
        let mut replay = bridge.clone();
        let w1 = smt_advance(&mut replay, cb, 1, &[], false).0;
        assert!(apply_block(&root0, &to_update(cb, 1, &w1)).is_err(),
                "a duplicate of an UNSPENT coinbase was accepted");
    }

    /// AUDIT #3 F-1 — the REAL shape of 91842/91880, against a RESIDENT tree.
    ///
    /// The fixture that stood here drove the guest from a fresh, empty `Smt`, where absence proves
    /// trivially. That is the one state the real blocks are not in: their duplicated coinbases were
    /// overwritten while still UNSPENT, which is the entire reason BIP30 exists. Because the fixture
    /// never reproduced the precondition, the whole suite stayed green while a from-genesis prover
    /// would have stalled at 91841.
    #[test]
    fn the_grandfathered_overwrite_round_trips_against_a_resident_tree() {
        let dup = txid(91812);
        let mut bridge = Smt::new();
        bridge.insert(dup, 1);                       // present and UNSPENT — the real precondition
        let root = bridge.root();

        let w = smt_advance(&mut bridge, dup, 1, &[], true).0;
        assert_eq!(w.smt_overwrite, Some(1), "the bridge did not emit an overwrite claim");

        let out = apply_block(&root, &to_update(dup, 1, &w))
            .expect("the guest rejected real block 91842 — a from-genesis prover stalls at 91841");
        assert_eq!(out, root, "a byte-identical overwrite must leave the root unchanged");
        assert_eq!(out, bridge.root(), "bridge and guest diverged across the overwrite");
    }

    /// The exception must not be reachable on an ordinary block: without the grandfather flag the same
    /// state is a plain BIP30 violation and must be refused.
    #[test]
    fn the_same_state_without_the_grandfather_is_still_refused() {
        let dup = txid(91812);
        let mut bridge = Smt::new();
        bridge.insert(dup, 1);
        let root = bridge.root();
        let w = smt_advance(&mut bridge.clone(), dup, 1, &[], false).0;
        assert!(apply_block(&root, &to_update(dup, 1, &w)).is_err(),
                "a duplicate of an UNSPENT coinbase was accepted outside the two grandfathered blocks");
    }

    /// BIP30 since libzync #26 (29188b5), exact: a fully spent coinbase may be duplicated only if it is TRACKED (one that
    /// can ever repeat: pre-BIP34, indicating a later height, or itself a repeat). Spent to zero it is held REPEATABLE and
    /// the duplicate is an ordinary insert; an untracked one is held SPENT and a duplicate is refused (on chain it cannot
    /// occur: its txid can never be produced again). This test asserted the pre-#26 rule (any fully spent coinbase may
    /// repeat) until review round 2 (11-05).
    #[test]
    fn a_fully_spent_coinbase_repeats_only_if_tracked() {
        for tracked in [true, false] {
            let mut bridge = Smt::new();
            let dup = txid(91812);
            let w0 = smt_advance(&mut bridge, dup, 1, &[], false).0;
            let mut root = apply_block(&Smt::new().root(), &to_update(dup, 1, &w0)).unwrap();

            // Spend it to zero via an ordinary later block.
            let w1 = smt_advance_ex(&mut bridge, txid(999), 1, &[dup], false, &[tracked], &[]).0;
            root = apply_block(&root, &to_update(txid(999), 1, &w1)).unwrap();
            // libzync counts only tracked coins (agg_lite.rs), so an untracked spend leaves the count where it was;
            // either way the duplicate of an untracked coinbase must be refused below.
            assert_eq!(bridge.get(&dup), Some(if tracked { REPEATABLE } else { 1 }));

            let w2 = smt_advance(&mut bridge, dup, 1, &[], false).0;
            let r = apply_block(&root, &to_update(dup, 1, &w2));
            if tracked {
                assert_eq!(w2.coinbase_prior, Some(REPEATABLE));
                assert_eq!(r.expect("duplicating a fully spent TRACKED coinbase was rejected (legal under BIP30)"), bridge.root());
            } else {
                assert!(r.is_err(), "a duplicate of an untracked (SPENT) coinbase was accepted");
            }
        }
    }
}

// Survive hazync#119: the prover intermittently returned a segment proof that failed its own
// internal verify. ROOT CAUSE FOUND AND FIXED (#245, v0.21.1): rv32im accum phase 3 added prev[k]
// to LogUp cells the instruction never wrote (still INVALID); when prev[k] == 0 that gave
// 0x87FFFFFE >= P, which broke one NTT butterfly pair. Fixed in vendor/risc0-circuit-rv32im-sys;
// upstream risc0 #3804.
//
// The retry STAYS, as defence in depth. The failure is DETECTABLE -- prove_segment verifies before
// returning -- and re-proving a segment is cheap next to losing a chunk. With the root cause fixed,
// a retry here is NEW information, not the old fault: which is exactly why it must stay loud.
//
// LOUD ON PURPOSE. A silent retry would turn a known prover fault into an invisible one and hide any
// change in its rate, which is the number #119 actually needs. Every retry prints, and the total is
// reported at the end of a run, so the rate stays measurable. If a segment fails REPEATEDLY that is
// not #119 and the error is propagated unchanged.
fn prove_segment_resilient(
    server: &std::rc::Rc<dyn risc0_zkvm::ProverServer>,
    ctx: &risc0_zkvm::VerifierContext,
    seg: &risc0_zkvm::Segment,
    what: &str,
) -> risc0_zkvm::SegmentReceipt {
    const ATTEMPTS: usize = 3;
    let mut last: Option<String> = None;
    for attempt in 1..=ATTEMPTS {
        match server.prove_segment(ctx, seg) {
            Ok(r) => {
                if attempt > 1 {
                    println!("  [#119] {what}: succeeded on attempt {attempt}");
                    RETRIES_119.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                return r;
            }
            Err(e) => {
                let msg = format!("{e:#}");
                // Only retry the fault we know is transient. An OOM, a malformed segment or a
                // missing zkr will fail identically every time, and retrying those wastes a card
                // for three times as long before saying the same thing.
                let transient = msg.contains("verification indicates proof is invalid")
                    || msg.contains("verify segment");
                println!("  [#119] {what}: attempt {attempt}/{ATTEMPTS} failed: {}",
                    msg.lines().next().unwrap_or("?"));
                if !transient {
                    panic!("{what}: not a #119 fault, not retrying: {msg}");
                }
                last = Some(msg);
            }
        }
    }
    panic!("{what}: failed {ATTEMPTS} times, last error: {}", last.unwrap_or_default());
}

fn prove_session_resilient(
    server: &std::rc::Rc<dyn risc0_zkvm::ProverServer>,
    ctx: &risc0_zkvm::VerifierContext,
    session: &risc0_zkvm::Session,
) -> risc0_zkvm::Receipt {
    use std::time::Instant;
    let nseg = session.segments.len();
    // ~20 progress lines whatever the size, matching the cadence `FoldProgress` used when this went
    // through `prove_session`. The line format is deliberately unchanged: recorded benchmark logs
    // are parsed for it.
    // #256: the board worker sets HAZYNC_PROGRESS_EVERY=1 and watches these lines -- a completed
    // segment is the only honest "still making progress" signal (silence is not: healthy provers on
    // block 39,318 were once killed for being quiet). Unset keeps the ~20-line cadence above.
    let every = std::env::var("HAZYNC_PROGRESS_EVERY").ok().and_then(|v| v.parse::<usize>().ok())
        .filter(|&v| v > 0).unwrap_or((nseg / 20).max(1));
    let t = Instant::now();
    let mut receipts: Vec<risc0_zkvm::SegmentReceipt> = Vec::with_capacity(nseg);
    for (i, sref) in session.segments.iter().enumerate() {
        let seg = sref.resolve().expect("resolve segment");
        receipts.push(prove_segment_resilient(server, ctx, &seg, &format!("segment {i}")));
        let n = i + 1;
        if n % every == 0 || n == nseg {
            let el = t.elapsed().as_secs_f64();
            // Rate from work actually done, not from a guess — early estimates are poor and say so
            // by being obviously early rather than by being hidden.
            let eta = el / n as f64 * (nseg - n) as f64;
            println!("    segment {n}/{nseg}  {el:.0}s elapsed, ~{eta:.0}s left");
        }
    }
    // ASSEMBLY: lift every segment receipt and join them into one. #256 taught the board worker's
    // watchdog about the segment loop above and left THIS phase silent, which is where block 39,318
    // actually died -- 880/880 segments proved in 3,102s, then no output at all, killed at 600s,
    // retried, killed again, re-claimed, forever. Assembly is ~one lift and ~one join per segment, so
    // its cost scales with segment count: above roughly 600 segments every block died this way and the
    // frontier could not pass one. Silence was the bug, not the duration.
    //
    // A heartbeat thread rather than a callback because `assemble_from_segment_receipts` is a single
    // blocking call with no progress hook. It prints on the same stdout the worker already parses;
    // Rust's stdout is a LineWriter, so each line flushes through the pipe as it is written.
    let asm_t = Instant::now();
    let asm_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let asm_flag = asm_done.clone();
    let every = std::env::var("HAZYNC_ASSEMBLY_TICK").ok()
        .and_then(|v| v.parse::<u64>().ok()).filter(|&v| v > 0).unwrap_or(30);
    let hb = std::thread::spawn(move || {
        let t = Instant::now();
        let mut next = every;
        while !asm_flag.load(std::sync::atomic::Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let el = t.elapsed().as_secs();
            if el >= next {
                println!("    assembling {nseg} segment receipts (lift + join)  {el}s elapsed");
                next = el + every;
            }
        }
    });
    let assembled = server.assemble_from_segment_receipts(ctx, session, receipts);
    asm_done.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = hb.join();
    println!("    assembled {nseg} segment receipts in {:.0}s", asm_t.elapsed().as_secs_f64());
    assembled.expect("assemble from segment receipts").receipt
}

static RETRIES_119: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn report_119_retries() {
    let n = RETRIES_119.load(std::sync::atomic::Ordering::Relaxed);
    if n > 0 {
        println!("  [#119] {n} segment(s) needed a retry this run — the rate is worth recording on the issue");
    }
}

/// Reports the #119 retry total however `main` exits.
///
/// ⛔ `prove_segment_resilient`'s own comment says the total "is reported at the end of a run, so the
/// rate stays measurable" — and it never was: `report_119_retries` had no caller, so every retry was
/// counted and then discarded. #119 is still open and still unexplained, and the retry RATE is the
/// number it actually needs.
///
/// A call at the end of `main` would not have worked: `main` dispatches a dozen commands with early
/// `return`s, so it would fire for almost none of them. A drop guard fires on every normal exit path.
struct Report119OnExit;
impl Drop for Report119OnExit {
    fn drop(&mut self) { report_119_retries(); }
}
