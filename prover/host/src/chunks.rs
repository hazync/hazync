//! Chunking a block for proving: the per-input cost model, packing inputs into chunks, and the
//! chunk executor inputs (write_chunk_inputs) shared by prove-chunk, the seg-* paths and libzync.

use bitcoin::{Transaction, TxOut};
use bitcoin::consensus::deserialize;
use methods::{METHOD_ELF, METHOD_ID};
use risc0_zkvm::{ExecutorEnv, default_executor};
use crate::{BlockWitness, HZ_BLOCK_MTP, header_hash, hex, seg_po2};
use crate::lz::lz_bundle;

pub(crate) const KIND_CHUNK: u32 = 0xC4A1_0004;
/// libzync agg-lite chunk journal (guest src/agg_lite.rs).
pub(crate) const KIND_CHUNK_LITE: u32 = 0xC4A1_0010;

// ---- Multi-GPU fan-out: prove ONE chunk to a file (run one process per GPU via CUDA_VISIBLE_DEVICES),
// then aggregate from the chunk-receipt files. HAZYNC_CHUNKS = total chunks; chunk index from arg. ----
// ---- Chunk packing: balance predicted work, not input counts (#132) ----
//
// An input's cost is dominated by EC verification — ~95% of proving, per docs/ACCELERATION.md — and how
// many signature checks an input performs depends on its script type, not on it being one input. A
// keypath taproot spend is one verify; a 15-of-15 bare multisig is fifteen. Splitting a block into equal
// INPUT COUNTS therefore leaves chunks with very unequal work, and since chunks prove in parallel a
// block's wall-clock is its slowest chunk. The straggler caps the fan-out however many GPUs are added.
//
// Chunks stay CONTIGUOUS and in input order; only their widths vary. That is not a simplification, it
// is required: the guest's `aggregate()` concatenates each chunk's `binds` in receipt order and
// `validate_block` indexes the result by the block's own input index. Reordering inputs across chunks
// would need each chunk to commit its indices and the aggregate to scatter rather than concatenate —
// a guest change, hence a new METHOD_ID, hence every existing proof invalidated. Widths are free.

// Per-input cost in guest cycles. FITTED to measurement, not assumed: `chunk-profile` was run over
// block 741000 in execute mode and these two coefficients reproduce every one of the 16 chunks' real
// cycle counts to within ~1%.
//
// REFITTED TWICE for #135, and the pattern is the point: every time a byte-scaling cost is removed the
// coefficient drops and the model must be re-measured, or the packer optimises against work that is no
// longer there.
//
//   182  serde `env::read` payload, transaction shipped per input
//    36  read_slice payload (the ~147 cycles/byte serde cost removed)
//     6  payload grouped per transaction (deserialise and Init now run once, not once per input)
//
// Each stale value did visible harm: at 182 the measured straggler was 1.64x, WORSE than not packing by
// cost at all, because byte-heavy chunks were rated far more expensive than they had become. The EC
// coefficient has not moved across any of it.
//
// What remains in the byte term is mostly `input_bind`, which still hashes the whole transaction once
// per input. That is ~2.5 cycles/byte of the 6 and is the next thing to disappear if it is ever hoisted.
//
// The byte term is the part that is easy to get wrong, and costing purely by EC verifies does get it
// wrong. Every ChunkInput carries the WHOLE spending transaction and the WHOLE prevouts blob, so a
// many-input transaction ships and re-hashes its entire body once per input. On block 741000's fattest
// chunks that marshalling is ~63% of the cycles — chunk 11 and chunk 1 have identical input counts and
// identical EC counts, and chunk 1 costs 2.5x more purely because its transactions are bigger. An
// EC-only model rates them equal and packs them as if they were.
//
// Only the RATIO matters: the packer compares costs and never predicts a wall-clock.
// hazync#190 ARMED. The packer branch shipped this EQUAL to COST_PER_SCHNORR_OP on purpose, so that
// giving the packer a curve dimension was a provable no-op: same costs, same partition, same chunks.
// Arming it is the one-line edit that branch's own comment describes.
//
// MEASURED 2026-08-29, block 962,000, 16/16 paired chunks on one L40S at po2 22: with bigint2 on,
// per-chunk speedup runs 2.046x to 4.813x and correlates with TAPROOT SHARE at -0.875 -- a 24.2%
// taproot chunk gets 2.0x where an all-ECDSA chunk gets 4.8x. The packer, balancing PRE-acceleration
// cost, predicts all 16 chunks within 1.00x, and bigint2 then takes the real straggler from 1.054 to
// 2.118. A block's wall-clock is its SLOWEST chunk, so that doubling is worth ~7 cards.
//
// Only the RATIO matters -- the packer compares costs and never predicts a wall-clock.

/// Runtime override for a packing constant.
///
/// ⛔ These constants are CALIBRATED PER BUILD MODE, and since v0.21.0 the defaults are **CORE's**
/// — the canonical channel (hazync#225). They are the PER-CURVE fit of 2026-09-01, measured on
/// block 962,000 with the field backend over 32 chunk executions, and they took the straggler
/// 1.311 -> 1.210 (8.89 -> 8.28 cards). Core's cost fits this shape at 6.6% mean error.
///
/// ⚠ They are WRONG for the other two channels, and not mildly: the same refit moves Ghost's
/// straggler 1.407 -> 1.884, about a card, because Ghost's cost is NOT close to linear in
/// `(ecdsa, schnorr, bytes, inputs)` — 15.2% mean error. A Ghost or stock build must override.
///
/// The previous defaults were #139's: EC 141,612 with Schnorr 13.8x higher, because #139
/// accelerates ECDSA only. **Core has no #139** — its field backend accelerates both curves, and
/// the per-curve fit measured the residual difference between them at just 1.11x.
///
/// Why a wrong predictor is dangerous rather than merely suboptimal: the packer balances its own
/// predictor perfectly, so the error never surfaces as a bad straggler number — it surfaces as a
/// bad BLOCK. Under the old defaults real chunk cycles spanned 33.6 M to 352 M across chunks the
/// model priced identically at 125.4 M: a straggler of 1.563x, WORSE than not packing at all.
///
/// Host-side only. Nothing here reaches the guest, so none of it moves METHOD_ID.
pub(crate) fn cost_const(var: &str, default: u64) -> u64 {
    std::env::var(var).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

pub(crate) const COST_PER_EC_OP: u64 = 417_798;
/// Schnorr (BIP340) verification cost, tracked SEPARATELY from ECDSA.
///
/// The split is load-bearing: a block's wall-clock is its SLOWEST chunk, so a packer blind to the
/// curve mix over-loads taproot-heavy chunks. Simulated on block 962,000 — only 2.7% Schnorr, the
/// mildest case available — a blind packer turns #139's 6.95x into 2.95x.
///
/// CORE (canonical; this default): the per-curve fit measured Schnorr at **1.11x** ECDSA, 462,435
/// against 417,798. Not parity — Schnorr still pays lift_x through xonly_pubkey_load and the
/// tagged challenge hash on top of the shared double-scalar-mul — but nothing like the 13.8x of a
/// one-curve accelerator, because Core's field backend speeds up both.
///
/// ⚠ GHOST history, kept because it remains a live trap there. Under #139 this sat at 1,950,000
/// (13.8x). Once G3 (patches/0006) put Schnorr through the SAME bigint2 accelerator, setting it
/// EQUAL to COST_PER_EC_OP was the obvious inference — and MEASURED WORSE: straggler 1.361 ->
/// 1.539, a whole card. 93 of the block's 145 Schnorr inputs sit in one chunk, so repricing them
/// down made the packer over-load it. 1,950,000 was not RIGHT either, merely the pre-#139 stock
/// ECDSA cost that happened to pack well. Ghost still has no honest fit — docs/history/BUILDS.md §3.
pub(crate) const COST_PER_SCHNORR_OP: u64 = 462_435;

/// What a verify costs when its public key was ALREADY decompressed earlier in the same chunk.
///
/// The decompression memo is per-chunk, so key reuse is worth real time — and it is not modelled
/// anywhere, which is a third axis the packer is blind to after curve and bytes. MEASURED: chunks 8
/// and 12 of block 962,000 have the same input count, the same EC verify count and the same byte
/// size, and came in at 45 s against 162 s. That is 3.6x from key reuse alone.
///
/// Derived from the post-G1 profile of the chunk work: hazync_ecmult_verify 561 M, hazync_lift_x
/// 254 M, scalar inverse 147 M ⇒ the decompression a memo hit avoids is 26.4% of per-verify EC work,
/// so a repeat costs ~0.736 of a fresh key. On block 962,000, 6,913 verifying inputs use only 2,160
/// distinct keys, so 68.8% of inputs take this discount.
///
/// ⛔ NOT refit for Core, deliberately. 104,222 came from a post-G1 (#139) profile as 0.736 of a
/// fresh key, and Core's liftx hint (patches/0013) removes most of the decompression that discount
/// exists to price — so the true Core ratio is very likely nearer parity. It is left alone because
/// the straggler 1.210 that justifies this channel was MEASURED with this value; changing it would
/// invalidate that measurement. Refitting it is open work, not a shipping blocker.
// hazync#226. 104,222 was 0.736 x the OLD COST_PER_EC_OP (141,612). That sibling was refit to
// 417,798 and this one was left as an ABSOLUTE, so the ratio silently became 0.249 — a number
// nobody chose. 350,000 is the measured optimum on block 962,000 (straggler 1.0806 against 1.7686
// at 104,222), and it keeps the ratio in the 0.74-0.84 band the fit actually supports.
//
// ⚠ This constant is INERT unless HAZYNC_PACK_KEYS=1: repeat_savings() is behind that gate and it
// is off by default. Refit anyway, because at 104,222 the gate is a LANDMINE — enabling it measures
// 1.7686 on block 962,000 and 1.9016 on 965,978, both far worse than not pricing repeats at all.
pub(crate) const COST_PER_EC_OP_REPEAT: u64 = 350_000;
// An input that verifies no signature still costs something to read, deserialise and hash.
// CORE per-curve fit (2026-09-01): 41,387 fixed cycles per input, 2 cycles per witness byte.
// ⚠ The byte term moves 6 -> 2 and the base 34,000 -> 41,387. With the EC term corrected upward
// (141,612 -> 417,798 — it was 3.2x too low) the fit shifts weight off bytes and onto inputs.
pub(crate) const COST_INPUT_BASE: u64 = 41_387;
pub(crate) const COST_PER_INPUT_BYTE: u64 = 2;

/// Signature verifications an input performs, split by CURVE rather than merely counted.
///
/// The split is the whole point: `total()` is what the packer used before #139 and what the profile
/// display still wants, but the two fields are priced separately because #139 accelerates only one of
/// them. Taproot -- both key path and script path -- is BIP340 Schnorr; everything else is ECDSA.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct SigOps { pub(crate) ecdsa: u64, pub(crate) schnorr: u64 }

impl SigOps {
    fn ecdsa(n: u64) -> Self { Self { ecdsa: n, schnorr: 0 } }
    fn schnorr(n: u64) -> Self { Self { ecdsa: 0, schnorr: n } }
    fn total(&self) -> u64 { self.ecdsa + self.schnorr }
    fn cost(&self) -> u64 {
        self.cost_with(cost_const("HAZYNC_COST_EC_OP", COST_PER_EC_OP),
                       cost_const("HAZYNC_COST_SCHNORR_OP", COST_PER_SCHNORR_OP))
    }
    /// Split out so a test can price the curves apart without editing a constant. A no-op test alone
    /// would pass for a refit that does nothing at all; this is what lets one prove it bites.
    fn cost_with(&self, ec: u64, schnorr: u64) -> u64 { ec * self.ecdsa + schnorr * self.schnorr }
}

/// Total signature verifications, curve-blind. Retained for the profile display, which reports a
/// count rather than a cost; `predicted_sig_ops` is what the packer uses.
pub(crate) fn predicted_ec_ops(raw_tx: &[u8], input_idx: u32, prevouts_blob: &[u8]) -> u64 {
    predicted_sig_ops(raw_tx, input_idx, prevouts_blob).total()
}

/// Signature verifications this input performs, by script type. A prediction, not a guarantee — it is
/// used only to balance chunks, so being wrong costs some balance and never correctness.
pub(crate) fn predicted_sig_ops(raw_tx: &[u8], input_idx: u32, prevouts_blob: &[u8]) -> SigOps {
    use bitcoin::blockdata::opcodes::all::{OP_CHECKSIG, OP_CHECKSIGADD, OP_CHECKSIGVERIFY};
    use bitcoin::blockdata::script::{Instruction, Script};

    // Anything unparseable is charged one verify: it should not happen, and a wrong guess here must not
    // be able to panic a prover run over what is only a scheduling hint.
    let Ok(tx) = deserialize::<Transaction>(raw_tx) else { return SigOps::ecdsa(1) };
    let Ok(prevouts) = deserialize::<Vec<TxOut>>(prevouts_blob) else { return SigOps::ecdsa(1) };
    let i = input_idx as usize;
    let (Some(txin), Some(prevout)) = (tx.input.get(i), prevouts.get(i)) else { return SigOps::ecdsa(1) };
    let spk = &prevout.script_pubkey;
    let wit = &txin.witness;

    // Tapscript's OP_CHECKSIGADD is not counted by count_sigops (it post-dates the legacy rules), so
    // taproot leaves need their own pass.
    let tapscript_ops = |leaf: &[u8]| -> u64 {
        Script::from_bytes(leaf)
            .instructions()
            .filter(|ins| {
                matches!(ins, Ok(Instruction::Op(op))
                    if *op == OP_CHECKSIG || *op == OP_CHECKSIGVERIFY || *op == OP_CHECKSIGADD)
            })
            .count() as u64
    };

    // Anyone-can-spend witness programs verify NOTHING: a v0 program that is not 20 or 32 bytes, or any
    // version above v1. The common case today is P2A, `OP_1 <0x4e73>` — Core v28's ephemeral anchor.
    // It is not a rounding error: **13.7% of block 962,000's 8,006 inputs are anchor spends**, each
    // measured at ~34K cycles against ~1,953K for a P2WPKH. Charging them one verify apiece put 2.13 G
    // cycles of a 16.2 G block on inputs that do essentially no work, and the packer then built chunks
    // around a cost that was not there — chunk 5 was predicted at 979,867,512 and measured 232,024,236.
    if let Some(ver) = spk.witness_version() {
        use bitcoin::blockdata::script::witness_version::WitnessVersion;
        let program_len = spk.len().saturating_sub(2);
        let spendable_by_signature = match ver {
            WitnessVersion::V0 => program_len == 20 || program_len == 32,
            WitnessVersion::V1 => program_len == 32,
            _ => false,
        };
        if !spendable_by_signature { return SigOps::default() }
    }

    if spk.is_p2tr() {
        // A trailing 0x50-prefixed annex is not part of the spend path.
        let mut n = wit.len();
        if n >= 2 && wit.last().is_some_and(|e| e.first() == Some(&0x50)) { n -= 1; }
        // BOTH taproot paths are BIP340 Schnorr -- key path obviously, and tapscript signatures
        // too. #139 does not touch either, which is exactly why they must be priced apart.
        return SigOps::schnorr(match n {
            0 | 1 => 1, // key path (or malformed): one Schnorr verify
            _ => wit.iter().nth(n - 2).map_or(1, |leaf| tapscript_ops(leaf).max(1)),
        });
    }
    if spk.is_p2wpkh() { return SigOps::ecdsa(1); }
    if spk.is_p2wsh() {
        return SigOps::ecdsa(wit.last().map_or(1, |ws| Script::from_bytes(ws).count_sigops().max(1) as u64));
    }
    if spk.is_p2sh() {
        // The redeem script is the last push of the scriptSig; a wrapped segwit spend then defers to the
        // witness exactly as the unwrapped form does.
        let redeem: Vec<u8> = txin
            .script_sig
            .instructions()
            .last()
            .and_then(|r| r.ok())
            .and_then(|ins| ins.push_bytes().map(|b| b.as_bytes().to_vec()))
            .unwrap_or_default();
        let rs = Script::from_bytes(&redeem);
        if rs.is_p2wpkh() { return SigOps::ecdsa(1); }
        if rs.is_p2wsh() {
            return SigOps::ecdsa(wit.last().map_or(1, |ws| Script::from_bytes(ws).count_sigops().max(1) as u64));
        }
        return SigOps::ecdsa(rs.count_sigops().max(1) as u64);
    }
    // Bare output: P2PK, P2PKH, bare multisig, or something unrecognised. All ECDSA.
    SigOps::ecdsa(spk.count_sigops().max(1) as u64)
}

/// Predicted cost of every input of the block, in the order the block spends them.
/// The public key each input will make the guest decompress, as an identity for reuse detection.
///
/// Only identity matters, not validity: a value that is not really a key simply fails to match
/// anything and costs nothing. So this does no curve arithmetic — it takes the first plausible
/// compressed key in the witness or scriptSig, or the x-only key from a P2TR prevout.
pub(crate) fn input_keys(w: &BlockWitness) -> Vec<Option<[u8; 33]>> {
    let mut out = Vec::with_capacity(w.inputs.len());
    for inp in &w.inputs {
        let raw = &w.txs[inp.tx_idx as usize].0;
        let mut found: Option<[u8; 33]> = None;
        if let Ok(tx) = bitcoin::consensus::deserialize::<bitcoin::Transaction>(raw) {
            if let Some(txin) = tx.input.get(inp.input_idx as usize) {
                for item in txin.witness.iter() {
                    if item.len() == 33 && (item[0] == 2 || item[0] == 3) {
                        let mut k = [0u8; 33]; k.copy_from_slice(item); found = Some(k); break;
                    }
                }
                if found.is_none() {
                    for ins in txin.script_sig.instructions().flatten() {
                        if let bitcoin::script::Instruction::PushBytes(pb) = ins {
                            let b = pb.as_bytes();
                            if b.len() == 33 && (b[0] == 2 || b[0] == 3) {
                                let mut k = [0u8; 33]; k.copy_from_slice(b); found = Some(k); break;
                            }
                        }
                    }
                }
            }
        }
        out.push(found);
    }
    out
}

/// How much input `i` gets cheaper when its key has already been decompressed in this chunk.
pub(crate) fn repeat_savings(w: &BlockWitness) -> Vec<u64> {
    w.inputs.iter().map(|inp| {
        let tx = &w.txs[inp.tx_idx as usize].0;
        let prevouts = &w.tx_prevouts[inp.tx_idx as usize].0;
        // hazync#226: overridable like its four siblings, so the constant can be A/B'd without a
        // rebuild per arm. Default unchanged.
        predicted_sig_ops(tx, inp.input_idx, prevouts).total()
            * (cost_const("HAZYNC_COST_EC_OP", COST_PER_EC_OP)
                .saturating_sub(cost_const("HAZYNC_COST_EC_OP_REPEAT", COST_PER_EC_OP_REPEAT)))
    }).collect()
}

pub(crate) fn input_costs(w: &BlockWitness) -> Vec<u64> {
    if std::env::var("HAZYNC_COST_MODEL").as_deref() == Ok("fit") {
        return fitted_costs(w);
    }
    w.inputs
        .iter()
        .map(|inp| {
            let tx = &w.txs[inp.tx_idx as usize].0;
            let prevouts = &w.tx_prevouts[inp.tx_idx as usize].0;
            let ops = predicted_sig_ops(tx, inp.input_idx, prevouts);
            let bytes = (tx.len() + prevouts.len()) as u64;
            cost_const("HAZYNC_COST_INPUT_BASE", COST_INPUT_BASE)
                + ops.cost()
                + cost_const("HAZYNC_COST_INPUT_BYTE", COST_PER_INPUT_BYTE) * bytes
                + cost_const("HAZYNC_COST_LEGACY_BYTE", COST_PER_LEGACY_BYTE) * legacy_sighash_bytes(tx, inp.input_idx)
        })
        .collect()
}

/// libzync: once signatures are batched, the dominant per-input cost the packer could not see is
/// LEGACY sighash -- a non-witness input's signature hash serialises and hashes the WHOLE transaction,
/// so a 400-input legacy transaction hashes ~400x its own size. Measured on block 962,000 (GHOST +
/// batch + tx-once, 16 chunks): ~4.1 cycles per byte so hashed, and one such 59 KB transaction made
/// a chunk 3x the median. Default 0 keeps every existing build's packing unchanged.
pub(crate) const COST_PER_LEGACY_BYTE: u64 = 0;

/// The transaction size this input's legacy sighash hashes, or 0 for a witness spend.
pub(crate) fn legacy_sighash_bytes(raw_tx: &[u8], input_idx: u32) -> u64 {
    // Scheduling hint only: an unparseable transaction is simply not charged.
    let Ok(tx) = deserialize::<Transaction>(raw_tx) else { return 0 };
    match tx.input.get(input_idx as usize) {
        Some(i) if i.witness.is_empty() && !tx.is_coinbase() => raw_tx.len() as u64,
        _ => 0,
    }
}

/// libzync round 8: a cost model FITTED to per-transaction guest cycles (`HAZYNC_TX_CYCLES=1` logs
/// them; `HAZYNC_DUMP_INPUT_FEATURES` writes these features), selected by `HAZYNC_COST_MODEL=fit`.
///
/// The hand-set model above prices every input with the whole transaction's bytes and knows nothing
/// of multisig or witness size; after tx-once a transaction is parsed and hashed ONCE per chunk, so
/// that over-charges big many-input transactions and under-charges the inputs that really are slow.
/// Features per input, in FIT_NAMES order:
/// - one: fixed per input;  ecdsa / schnorr: predicted verifies;
/// - wit / ss: this input's witness bytes and scriptSig bytes;
/// - legacy: bytes a round-8 legacy sighash hashes for this input, 41*(n-k) + outputs (0 for witness);
/// - msx: CHECKMULTISIG keys beyond the threshold (each mismatch falls back to a full libsecp verify);
/// - txb / tx1: the transaction's bytes and a fixed per-transaction cost, split evenly across its inputs;
/// - tapscript: a taproot SCRIPT-path spend, whose tweak check (Q = P + t*G) libsecp runs unbatched;
/// - outs: the transaction's outputs, split evenly across its inputs (agg-lite chunks hash a leaf per output).
pub(crate) const FIT_NAMES: [&str; 11] = ["ONE", "ECDSA", "SCHNORR", "WIT", "SS", "LEGACY", "MSX", "TXB", "TX1", "TAPSCRIPT", "OUTS"];
/// GHOST + round 8 fit on block 962,000 (cycles per unit; see BLOCK_962000.md round 8): 6,303
/// transactions, median error 0.8%. ECDSA / SCHNORR include the end-of-chunk batch check (9,350 per
/// verify). Each is overridable as HAZYNC_FIT_<NAME>.
///
/// ⚠ Pair with HAZYNC_PACK_KEYS=1, HAZYNC_COST_EC_OP=14537, HAZYNC_COST_EC_OP_REPEAT=0 and add 14,537
/// to ECDSA / SCHNORR: a key costs ~14.5k cycles in EVERY chunk it appears in (lift + MSM point), which
/// a one-chunk fit cannot see because there each key is paid for once. Fitted from the 16-chunk
/// residuals; without it the early, key-diverse chunks ran 25-30% over prediction.
pub(crate) const FIT_DEFAULT: [f64; 11] = [3147.0, 9350.0, 48266.0, 136.0, 128.0, 2.0, 58402.0, 34.0, 6486.0, 185656.0, 0.0];

pub(crate) fn input_features(w: &BlockWitness) -> Vec<[f64; 11]> {
    w.inputs.iter().map(|inp| {
        let raw = &w.txs[inp.tx_idx as usize].0;
        let pv = &w.tx_prevouts[inp.tx_idx as usize].0;
        let ops = predicted_sig_ops(raw, inp.input_idx, pv);
        let mut f = [0f64; 11];
        f[0] = 1.0;
        f[1] = ops.ecdsa as f64;
        f[2] = ops.schnorr as f64;
        let Ok(tx) = deserialize::<Transaction>(raw) else { return f };
        let k = inp.input_idx as usize;
        let n = tx.input.len().max(1) as f64;
        f[7] = (raw.len() + pv.len()) as f64 / n;
        f[8] = 1.0 / n;
        f[10] = tx.output.len() as f64 / n;
        let Some(txin) = tx.input.get(k) else { return f };
        f[3] = txin.witness.iter().map(|e| e.len() + 1).sum::<usize>() as f64;
        f[4] = txin.script_sig.len() as f64;
        if txin.witness.is_empty() && ops.total() > 0 {
            let outs: usize = tx.output.iter().map(|o| o.script_pubkey.len() + 9).sum();
            f[5] = (41 * (tx.input.len() - k) + outs) as f64;
        }
        // The multisig script: P2WSH's witness script, P2SH's redeem script, or a bare output.
        let prevs = deserialize::<Vec<TxOut>>(pv).unwrap_or_default();
        if prevs.get(k).is_some_and(|o| o.script_pubkey.is_p2tr()) {
            let mut n = txin.witness.len();
            if n >= 2 && txin.witness.last().is_some_and(|e| e.first() == Some(&0x50)) { n -= 1; }
            if n >= 2 { f[9] = 1.0; }
        }
        let script: Vec<u8> = if let Some(ws) = txin.witness.last().filter(|_| txin.witness.len() > 1) {
            ws.to_vec()
        } else if let Some(Ok(ins)) = txin.script_sig.instructions().last() {
            ins.push_bytes().map(|b| b.as_bytes().to_vec()).unwrap_or_default()
        } else {
            prevs.get(k).map(|o| o.script_pubkey.to_bytes()).unwrap_or_default()
        };
        f[6] = multisig_extra(&script).or_else(|| {
            prevs.get(k).and_then(|o| multisig_extra(o.script_pubkey.as_bytes()))
        }).unwrap_or(0) as f64;
        f
    }).collect()
}

/// `n - m` for an `OP_m <keys> OP_n OP_CHECKMULTISIG[VERIFY]` script, else None.
pub(crate) fn multisig_extra(s: &[u8]) -> Option<u64> {
    let (&first, rest) = s.split_first()?;
    let (&last, rest) = rest.split_last()?;
    let (&nop, _) = rest.split_last()?;
    if last != 0xae && last != 0xaf { return None }
    let small = |op: u8| (0x51..=0x60).contains(&op).then(|| (op - 0x50) as u64);
    let (m, n) = (small(first)?, small(nop)?);
    Some(n.saturating_sub(m))
}

pub(crate) fn fitted_costs(w: &BlockWitness) -> Vec<u64> {
    let coef: Vec<f64> = FIT_NAMES.iter().zip(FIT_DEFAULT).map(|(n, d)| {
        std::env::var(format!("HAZYNC_FIT_{n}")).ok().and_then(|s| s.parse().ok()).unwrap_or(d)
    }).collect();
    input_features(w).iter()
        .map(|f| f.iter().zip(&coef).map(|(x, c)| x * c).sum::<f64>().max(1.0) as u64)
        .collect()
}

/// Runs needed to cover `costs` when no run may exceed `cap`. `cap` is always >= max(costs), so every
/// input fits somewhere and this terminates.
/// Cost of input `i` given the keys already decompressed in the CURRENT run.
///
/// The decompression memo is per-chunk, so the SAME input is cheaper when its key has been seen
/// earlier in the same chunk than when it opens one. That makes cost position-dependent, which is
/// why this is a closure over run state rather than a precomputed vector — and why the packer could
/// not see it before.
pub(crate) struct KeyWalk<'a> {
    costs: &'a [u64],
    savings: &'a [u64],
    keys: &'a [Option<[u8; 33]>],
    seen: std::collections::HashSet<[u8; 33]>,
}

impl<'a> KeyWalk<'a> {
    fn new(costs: &'a [u64], savings: &'a [u64], keys: &'a [Option<[u8; 33]>]) -> Self {
        Self { costs, savings, keys, seen: std::collections::HashSet::new() }
    }
    fn reset(&mut self) { self.seen.clear(); }
    /// Cost of `i` in the current run, and record its key as now-seen.
    fn take(&mut self, i: usize) -> u64 {
        let mut c = self.costs[i];
        if let Some(k) = self.keys.get(i).copied().flatten() {
            if !self.seen.insert(k) {
                c = c.saturating_sub(self.savings[i]);
            }
        }
        c
    }
    /// Cost of `i` if it were to OPEN a new run — no keys seen yet, so never discounted.
    fn take_fresh(&mut self, i: usize) -> u64 {
        self.reset();
        if let Some(k) = self.keys.get(i).copied().flatten() { self.seen.insert(k); }
        self.costs[i]
    }
}

pub(crate) fn runs_at_cap_keyed(costs: &[u64], savings: &[u64], keys: &[Option<[u8; 33]>], cap: u64) -> usize {
    let mut w = KeyWalk::new(costs, savings, keys);
    let (mut runs, mut acc) = (1usize, 0u64);
    for i in 0..costs.len() {
        let c = w.take(i);
        if acc + c > cap { runs += 1; acc = w.take_fresh(i) } else { acc += c }
    }
    runs
}

pub(crate) fn split_at_cap_keyed(costs: &[u64], savings: &[u64], keys: &[Option<[u8; 33]>], cap: u64) -> Vec<(usize, usize)> {
    let mut w = KeyWalk::new(costs, savings, keys);
    let (mut out, mut lo, mut acc) = (Vec::new(), 0usize, 0u64);
    for i in 0..costs.len() {
        let c = w.take(i);
        if acc + c > cap && i > lo {
            out.push((lo, i));
            lo = i;
            acc = w.take_fresh(i);
        } else {
            acc += c;
        }
    }
    out.push((lo, costs.len()));
    out
}

pub(crate) fn runs_at_cap(costs: &[u64], cap: u64) -> usize {
    let (mut runs, mut acc) = (1usize, 0u64);
    for &c in costs {
        if acc + c > cap { runs += 1; acc = c } else { acc += c }
    }
    runs
}

/// Split `costs` into contiguous runs, each within `cap`.
pub(crate) fn split_at_cap(costs: &[u64], cap: u64) -> Vec<(usize, usize)> {
    let (mut out, mut lo, mut acc) = (Vec::new(), 0usize, 0u64);
    for (i, &c) in costs.iter().enumerate() {
        if acc + c > cap && i > lo {
            out.push((lo, i));
            lo = i;
            acc = c;
        } else {
            acc += c;
        }
    }
    out.push((lo, costs.len()));
    out
}

/// Partition a block's inputs into at most `nchunks` contiguous runs, minimising the cost of the
/// heaviest run. Binary-searches the answer and checks feasibility greedily — exact, not a heuristic.
///
/// Guarantees, relied on by `prove_chunk`/`agg_chunks` and asserted in the tests: the runs are ordered,
/// non-empty, non-overlapping, and cover `0..n` exactly.
pub(crate) fn pack_chunks(costs: &[u64], nchunks: usize, keys: Option<(&[u64], &[Option<[u8; 33]>])>) -> Vec<(usize, usize)> {
    let n = costs.len();
    if n == 0 { return Vec::new() }
    let k = nchunks.max(1).min(n);
    if k == 1 { return vec![(0, n)] }

    let (mut lo, mut hi) = (costs.iter().copied().max().unwrap_or(1), costs.iter().sum::<u64>());
    let keyed = keys.map(|(sv, ks)| (sv, ks));
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let r = match keyed {
            Some((sv, ks)) => runs_at_cap_keyed(costs, sv, ks, mid),
            None => runs_at_cap(costs, mid),
        };
        if r <= k { hi = mid } else { lo = mid + 1 }
    }
    let mut runs = match keyed {
        Some((sv, ks)) => split_at_cap_keyed(costs, sv, ks, lo),
        None => split_at_cap(costs, lo),
    };

    // The optimal cap may need fewer than k runs. Idle provers help nobody, so keep splitting the widest
    // run until there are k of them — this can only lower the maximum, never raise it.
    while runs.len() < k {
        let Some(w) = (0..runs.len()).max_by_key(|&i| runs[i].1 - runs[i].0) else { break };
        let (a, b) = runs[w];
        if b - a < 2 { break } // nothing left wide enough to divide
        let mid = a + (b - a) / 2;
        runs[w] = (a, mid);
        runs.insert(w + 1, (mid, b));
    }
    runs
}

/// The block's chunk partition. Every command that touches chunks derives it from the witness through
/// this one function, so a fan-out across machines cannot disagree about which inputs a chunk holds.
///
/// `HAZYNC_CHUNK_PACK=count` restores the old equal-input-count split, for A/B measurement.
pub(crate) fn chunk_bounds(w: &BlockWitness, nchunks: usize) -> Vec<(usize, usize)> {
    let b = chunk_bounds_raw(w, nchunks);
    // A tracked transaction sits whole in one run (the guest's `tracked_tx`): move a boundary out of one.
    let mut out: Vec<(usize, usize)> = Vec::with_capacity(b.len());
    let mut from = 0;
    for (_, end) in b {
        let mut e = end.max(from);
        while e > 0 && e < w.inputs.len() && w.inputs[e].tx_idx == w.inputs[e - 1].tx_idx
            && w.inputs.iter().filter(|i| i.tx_idx == w.inputs[e].tx_idx).all(|i| i.coin_is_coinbase & 2 != 0) { e += 1; }
        if e > from { out.push((from, e)); from = e; }
    }
    out
}

pub(crate) fn chunk_bounds_raw(w: &BlockWitness, nchunks: usize) -> Vec<(usize, usize)> {
    let n = w.inputs.len();
    if std::env::var("HAZYNC_CHUNK_PACK").as_deref() == Ok("count") {
        if n == 0 { return Vec::new() }
        let k = nchunks.max(1).min(n);
        let sz = n.div_ceil(k);
        return (0..k).map(|c| (c * sz, ((c + 1) * sz).min(n))).filter(|(a, b)| a < b).collect();
    }
    // HAZYNC_PACK_KEYS=1 prices the per-chunk decompression memo. Off by default so the partition
    // is unchanged unless asked for, which keeps it A/B-able against the arms already measured.
    if std::env::var("HAZYNC_PACK_KEYS").as_deref() == Ok("1") {
        let (sv, ks) = (repeat_savings(w), input_keys(w));
        return pack_chunks(&input_costs(w), nchunks, Some((&sv, &ks)));
    }
    pack_chunks(&input_costs(w), nchunks, None)
}

/// Run one chunk's inputs through the guest in execute mode and return the cycles it took. Same mode 4
/// the prover uses, so the count is the real cost of proving that chunk, minus the proving itself.
pub(crate) fn exec_chunk_cycles(w: &BlockWitness, lo: usize, hi: usize) -> u64 {
    let mut b = ExecutorEnv::builder();
    b.segment_limit_po2(seg_po2());
    b.write(&4u32).unwrap();
    b.write(&w.height).unwrap();
    b.write(&chunk_block_hash(&w.header)).unwrap();
    write_chunk_inputs(&mut b, &w, lo, hi);
    let s = default_executor().execute(b.build().unwrap(), METHOD_ELF)
        .unwrap_or_else(|e| panic!("{}", crate::lz::guest_failure("chunk execution", &e)));
    crate::lz::require_committed(s.exit_code, "chunk execution"); // a cost figure only for a run that committed (round 9)
    // Report what the chunk COMMITTED, not just how long it took. A payload-format change that
    // silently mis-reads its input would show up as fewer cycles and look like a win; the journal
    // digest is what makes an A/B trustworthy. #135.
    let d = <sha2::Sha256 as sha2::Digest>::digest(&s.journal.bytes);
    let j = &s.journal.bytes;
    let w = |i: usize| u32::from_le_bytes([j[i], j[i + 1], j[i + 2], j[i + 3]]);
    // The guest tags this journal KIND_CHUNK and the aggregate asserts it (H8). The host printed the
    // tag and never checked it, so a payload-format change would have shown up as a plausible-looking
    // cycle count rather than an error. Check it here too: this is the other end of the same contract.
    assert!(w(0) == KIND_CHUNK || w(0) == KIND_CHUNK_LITE, "chunk journal is not a ChunkOut (domain tag {:#x})", w(0));
    println!("        journal sha256 {}  kind={:#x} all_valid={} binds={}", hex(&d), w(0), w(4), w(8));
    s.cycles()
}
/// Write a chunk's inputs onto the guest's stdin (mode 4's payload), grouped by transaction.
///
/// #135. Two things are going on:
///
/// **Grouped by transaction.** A chunk used to carry a full copy of the spending transaction and its
/// prevouts for EVERY input, so a 501-input consolidation was shipped 501 times — block 741000 sent
/// 6,995,621 bytes of a distinct 123,883, a factor of 56.5. The guest then deserialised and ran
/// `PrecomputedTransactionData::Init` once per input, which is exactly the quadratic sighash work
/// BIP143 precomputation exists to avoid. Now each transaction goes once, followed by the indices of
/// the inputs this chunk owns.
///
/// **Raw bytes.** The blobs go through `write_slice` rather than serde `Vec<u8>`; serde walked risc0's
/// word stream a byte at a time at ~147 cycles/byte. Blobs are padded to a word so the u32 reads that
/// follow stay aligned, and the guest truncates back to the declared length.
///
/// Ordering is load-bearing. `w.inputs` is built transaction by transaction, inputs in index order, so
/// a contiguous chunk groups into consecutive transaction runs with no reordering. The aggregation
/// concatenates chunk binds and indexes them by the block's own input index — emit them in any other
/// order and it compares the wrong input, silently.
/// hazync#205 — collect `(x, y)` for every public key the guest is about to decompress, so
/// `secp256k1_ge_set_xo_var` can VERIFY `y^2 == x^3 + 7` instead of computing a modular square root.
///
/// Measured motivation: that sqrt is 1,415,786,221 cycles on block 962,000 — 9.83% of the block and
/// ~45% of all post-#139 work — and `patches/0005` does not touch it, because it lives outside
/// `secp256k1_ecmult`. libsecp's own `bench_internal` prices `field_sqrt` at 264x a `field_mul`.
///
/// ⚠ COMPLETENESS IS AN OPTIMISATION, NOT A CORRECTNESS CONDITION. A key we fail to find here simply
/// pays the sqrt it pays today; a key we get WRONG fails the guest's check and also falls back. So
/// this may be as approximate as it likes, and must never be trusted.
///
/// Returns pairs sorted ascending by `x` (BTreeMap ordering), which is the order the guest's binary
/// search expects. Only the EVEN root is stored: `ge_set_xo_var` flips the sign itself to match the
/// requested parity, so one entry per `x` serves both.
pub(crate) fn liftx_hints(w: &BlockWitness, lo: usize, hi: usize) -> Vec<([u8; 32], [u8; 32])> {
    use bitcoin::secp256k1::PublicKey;
    use std::collections::BTreeMap;

    let mut out: BTreeMap<[u8; 32], [u8; 32]> = BTreeMap::new();
    let mut add_x = |x: &[u8]| {
        if x.len() != 32 {
            return;
        }
        // Ask for the even root; the guest negates when the caller wanted the odd one.
        let mut comp = [0u8; 33];
        comp[0] = 2;
        comp[1..].copy_from_slice(x);
        if let Ok(pk) = PublicKey::from_slice(&comp) {
            let u = pk.serialize_uncompressed(); // 0x04 || X || Y
            let (mut xa, mut ya) = ([0u8; 32], [0u8; 32]);
            xa.copy_from_slice(&u[1..33]);
            ya.copy_from_slice(&u[33..65]);
            out.insert(xa, ya);
        }
    };
    // A 33-byte compressed key contributes its x; a 32-byte value is taken as an x-only key.
    let mut add_push = |b: &[u8]| match b.len() {
        33 if b[0] == 2 || b[0] == 3 => add_x(&b[1..]),
        32 => add_x(b),
        _ => {}
    };

    let mut seen_tx: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for inp in &w.inputs[lo..hi] {
        if !seen_tx.insert(inp.tx_idx) {
            continue;
        }
        let raw = &w.txs[inp.tx_idx as usize].0;
        let Ok(tx) = bitcoin::consensus::deserialize::<bitcoin::Transaction>(raw) else { continue };
        // The spent outputs' scripts: a taproot output key (OP_1 <32>, lifted on every key-path spend), and P2PK /
        // bare multisig keys.
        if let Ok(pv) = bitcoin::consensus::deserialize::<Vec<bitcoin::TxOut>>(&w.tx_prevouts[inp.tx_idx as usize].0) {
            for o in &pv {
                let spk = o.script_pubkey.as_bytes();
                if spk.len() == 34 && spk[0] == 0x51 && spk[1] == 0x20 { add_push(&spk[2..34]); continue; }
                for ins in o.script_pubkey.instructions().flatten() {
                    if let bitcoin::script::Instruction::PushBytes(pb) = ins { add_push(pb.as_bytes()); }
                }
            }
        }
        for txin in &tx.input {
            // Witness items: the key itself for P2WPKH, and the redeem/tapscript for the rest --
            // walk those as scripts too, which is where multisig keys live.
            for item in txin.witness.iter() {
                add_push(item);
                // A taproot control block (33 + 32m bytes): its internal key, lifted by the tweak check.
                if item.len() >= 33 && (item.len() - 33) % 32 == 0 && item[0] & 0xfe == 0xc0 { add_push(&item[1..33]); }
                if item.len() > 33 {
                    for ins in bitcoin::Script::from_bytes(item).instructions().flatten() {
                        if let bitcoin::script::Instruction::PushBytes(pb) = ins {
                            add_push(pb.as_bytes());
                        }
                    }
                }
            }
            for ins in txin.script_sig.instructions().flatten() {
                if let bitcoin::script::Instruction::PushBytes(pb) = ins {
                    add_push(pb.as_bytes());
                    if pb.as_bytes().len() > 33 {
                        for i2 in bitcoin::Script::from_bytes(pb.as_bytes()).instructions().flatten() {
                            if let bitcoin::script::Instruction::PushBytes(p2) = i2 {
                                add_push(p2.as_bytes());
                            }
                        }
                    }
                }
            }
        }
    }
    out.into_iter().collect()
}

/// txlog Phase A (txproofs/README.md §7): HAZYNC_SPECULATIVE=1 proves chunks for the block's height and MTP with zero
/// block hash and time, as a prover would before the block exists. Only a txlog image's aggregate accepts them.
pub(crate) fn speculative() -> bool {
    std::env::var("HAZYNC_SPECULATIVE").as_deref() == Ok("1")
}

/// The block hash a chunk is told: the real one, or zero for a speculative chunk.
pub(crate) fn chunk_block_hash(header: &[u8]) -> [u8; 32] {
    if speculative() { [0; 32] } else { header_hash(header) }
}

pub(crate) fn write_chunk_inputs(b: &mut risc0_zkvm::ExecutorEnvBuilder, w: &BlockWitness, lo: usize, hi: usize) {
    fn padded(v: &[u8]) -> Vec<u8> {
        let mut p = v.to_vec();
        p.resize(v.len().div_ceil(4) * 4, 0);
        p
    }
    // Consecutive runs sharing a tx_idx. Debug-asserted rather than assumed: if the witness builder
    // ever emits inputs out of order this must fail loudly here, not produce a chunk that binds the
    // wrong inputs.
    let mut groups: Vec<(u32, usize, usize)> = Vec::new();
    for i in lo..hi {
        let t = w.inputs[i].tx_idx;
        match groups.last_mut() {
            Some((gt, _, end)) if *gt == t => *end = i + 1,
            _ => groups.push((t, i, i + 1)),
        }
    }
    debug_assert!(
        w.inputs[lo..hi].windows(2).all(|p| (p[0].tx_idx, p[0].input_idx) < (p[1].tx_idx, p[1].input_idx)),
        "chunk inputs are not in (tx_idx, input_idx) order — grouping would reorder binds"
    );

    b.write(&(groups.len() as u32)).unwrap();

    // libzync guest: ONE hint block (libzync/src/wire.rs) -- pubkey-Y hints, batch-signature hints from
    // the block's bundle ($HAZYNC_LZ_HINTS, or collect mode without one), and the block's MTP and time.
    let liftx: Vec<u8> = liftx_hints(w, lo, hi).iter().flat_map(|(x, y)| x.iter().chain(y.iter()).copied()).collect();
    let bundle = lz_bundle(w.height, &header_hash(&w.header));
    let sig = bundle.as_ref().and_then(|b| b.chunk_sig(lo as u32, hi as u32));
    // Negative control (test-only): HAZYNC_LITE_BAD_MTP feeds chunks a wrong MTP; the aggregate must reject.
    let bump = std::env::var("HAZYNC_LITE_BAD_MTP").is_ok() as u32;
    let words = libzync::wire::ChunkHints {
        mtp: HZ_BLOCK_MTP.lock().unwrap().expect("libzync: build_full has not set the block MTP") + bump,
        block_time: if speculative() { 0 } else { u32::from_le_bytes(w.header[68..72].try_into().unwrap()) },
        liftx: &liftx,
        sig_collect: sig.is_none(),
        sig: sig.unwrap_or(&[]),
    }.encode();
    b.write(&(words.len() as u32)).unwrap();
    b.write_slice(&words);
    match sig {
        Some(s) => eprintln!("  libzync: inputs {lo}..{hi}: {} pubkey hints, {} signature hints",
                             liftx.len() / 64, s.len() / libzync::wire::SIG_HINT_BYTES),
        None => eprintln!("  libzync: inputs {lo}..{hi}: {} pubkey hints, signatures COLLECT (no bundle entry)", liftx.len() / 64),
    }

    for (tx_idx, gs, ge) in groups {
        let tx = &w.txs[tx_idx as usize].0;
        let prevouts = &w.tx_prevouts[tx_idx as usize].0;
        b.write(&(tx.len() as u32)).unwrap();
        b.write(&(prevouts.len() as u32)).unwrap();
        b.write(&((ge - gs) as u32)).unwrap();
        b.write_slice(&padded(tx));
        b.write_slice(&padded(prevouts));
        // Negative-test hook (test-only, inert unless set): send a coinbase coin's flag as 4, which the leaf
        // reads as "coinbase" and the old maturity test (== 1) did not (review 2).
        let cb2 = std::env::var("HAZYNC_TEST_CBFLAG2").is_ok();
        for inp in &w.inputs[gs..ge] {
            b.write(&inp.input_idx).unwrap();
            b.write(&inp.coin_height).unwrap();
            b.write(&(if cb2 && inp.coin_is_coinbase == 1 { 4 } else { inp.coin_is_coinbase })).unwrap();
            b.write(&inp.coin_mtp).unwrap();
        }
    }
}

pub(crate) fn nchunks_env() -> usize {
    std::env::var("HAZYNC_CHUNKS").ok().and_then(|s| s.parse().ok()).unwrap_or(2).max(1)
}

/// Read and verify the chunk receipts an aggregate consumes (hazync#153).
///
/// Verified on the way in, not trusted: a chunk receipt that does not verify against METHOD_ID
/// cannot become a valid assumption, and finding that out here names the file instead of failing
/// somewhere inside the prover.
pub(crate) fn read_chunk_receipts(nchunks: usize) -> Vec<risc0_zkvm::Receipt> {
    // Directory the per-chunk receipts live in (hazync#229). `prove-chunk` writes each receipt to
    // $HAZYNC_OUT, which is a full path, so the two halves of a run can disagree about where the
    // receipts are: scripts/gpu-benchmark.sh has always exported HAZYNC_RECEIPTS here and nothing
    // ever read it, so phase 3 resolved `chunk_0.bin` against its own CWD and died on a run whose
    // proves had all succeeded.
    //
    // Defaults to `.`, which is exactly the previous behaviour — this adds a way to say where,
    // it does not change where they are looked for when nobody says.
    let dir = std::env::var("HAZYNC_RECEIPTS").unwrap_or_else(|_| ".".into());
    let mut receipts: Vec<risc0_zkvm::Receipt> = Vec::with_capacity(nchunks);
    for i in 0..nchunks {
        // #278 renamed the WRITER's default to .hzk. This reader accepts either, because chunk
        // receipts outlive the run that made them: a box with a staged set from an older host must
        // still aggregate. Preferring .hzk and falling back keeps both directions working, and a
        // missing chunk still panics by name rather than aggregating a short set.
        let hzk = format!("{dir}/chunk_{i}.hzk");
        let f = if std::path::Path::new(&hzk).exists() { hzk } else { format!("{dir}/chunk_{i}.bin") };
        let r: risc0_zkvm::Receipt =
            bincode::deserialize(&std::fs::read(&f).unwrap_or_else(|e| panic!("chunk receipt {f}: {e}"))).unwrap();
        r.verify(METHOD_ID).unwrap_or_else(|e| panic!("chunk receipt {f} does not verify: {e}"));
        receipts.push(r);
    }
    receipts
}

#[cfg(test)]
mod chunk_packing_tests {
    use super::{pack_chunks, runs_at_cap, SigOps, predicted_sig_ops,
                split_at_cap, split_at_cap_keyed,
                COST_INPUT_BASE, COST_PER_EC_OP, COST_PER_EC_OP_REPEAT};

    // ---- #139: the packer must be able to see the curve, not just the count ----

    /// Taproot is BIP340 Schnorr on BOTH paths; everything else is ECDSA. #139 accelerates only the
    /// latter, so misfiling one is a 13.8x pricing error the packer cannot detect at runtime.
    #[test]
    fn taproot_classifies_as_schnorr_and_the_rest_as_ecdsa() {
        let p2tr_key = SigOps::schnorr(1);
        let p2wpkh   = SigOps::ecdsa(1);
        assert_eq!(p2tr_key.schnorr, 1, "taproot key path must be Schnorr");
        assert_eq!(p2tr_key.ecdsa,   0, "taproot must not be priced as ECDSA");
        assert_eq!(p2wpkh.ecdsa,     1, "P2WPKH must be ECDSA");
        assert_eq!(p2wpkh.schnorr,   0, "P2WPKH must not be priced as Schnorr");
        // total() stays curve-blind, which is what the profile display reports.
        assert_eq!(p2tr_key.total(), p2wpkh.total());
    }

    /// Cost is EXACTLY the two-term linear combination -- no rounding, no cross-terms, no hidden
    /// dependence on the order of the split.
    ///
    /// This replaces `cost_is_curve_blind_while_the_coefficients_are_equal`, which asserted the two
    /// constants were equal and so proved the curve dimension was a no-op. That premise was always
    /// temporary and its own message said so: divergence "is #139 landing, and this test's premise
    /// with it". #139 has landed (ECDSA 141,612, Schnorr 1,950,000), so the equality is gone while
    /// the linearity it was really protecting is not. `pricing_the_curves_apart_actually_changes_the_cost`
    /// covers the other half: that the divergence actually moves the packer.
    #[test]
    fn cost_is_the_linear_combination_of_the_two_curve_coefficients() {
        for (e, s) in [(0, 0), (1, 0), (0, 1), (3, 2), (7, 11)] {
            let split = SigOps { ecdsa: e, schnorr: s };
            assert_eq!(
                split.cost(),
                super::COST_PER_EC_OP * e + super::COST_PER_SCHNORR_OP * s,
                "cost is not the linear combination of the per-curve coefficients"
            );
        }
        // An all-ECDSA and an all-Schnorr input of the same total must NOT cost the same now.
        assert_ne!(
            SigOps { ecdsa: 4, schnorr: 0 }.cost(), SigOps { ecdsa: 0, schnorr: 4 }.cost(),
            "the curve dimension has stopped biting -- the coefficients have re-converged"
        );
    }

    /// ⚠ The no-op test above would ALSO pass for a refit that does nothing whatsoever. This one shows
    /// the machinery actually bites the moment #139 makes the coefficients diverge -- an ECDSA-heavy
    /// input and a Schnorr-heavy one must then cost different amounts, which is the entire point.
    #[test]
    fn pricing_the_curves_apart_actually_changes_the_cost() {
        let ecdsa_heavy   = SigOps::ecdsa(4);
        let schnorr_heavy = SigOps::schnorr(4);
        let (equal, post_139) = (1_950_000u64, 141_612u64);
        assert_eq!(
            ecdsa_heavy.cost_with(equal, equal), schnorr_heavy.cost_with(equal, equal),
            "with equal coefficients the two must be indistinguishable"
        );
        let a = ecdsa_heavy.cost_with(post_139, equal);
        let b = schnorr_heavy.cost_with(post_139, equal);
        assert!(b > a * 10, "after #139 Schnorr must dominate: {b} vs {a}");
    }

    /// The classifier runs on real bytes, not just constructed SigOps: an unparseable input must fall
    /// back to one ECDSA verify rather than panicking a prover run over a scheduling hint.
    #[test]
    fn unparseable_input_is_charged_one_ecdsa_verify_and_never_panics() {
        let ops = predicted_sig_ops(b"not a transaction", 0, b"not prevouts");
        assert_eq!(ops, SigOps::ecdsa(1));
    }


    /// The invariants `prove_chunk` and `agg_chunks` rely on. If any of these break, chunks stop lining
    /// up with the block's inputs and the guest's `all_binds[idx]` lookup silently compares the wrong
    /// input — so these are correctness properties, not tidiness.
    fn assert_partitions(costs: &[u64], runs: &[(usize, usize)]) {
        assert!(!runs.is_empty() || costs.is_empty(), "no runs for a non-empty block");
        let mut expect = 0usize;
        for &(lo, hi) in runs {
            assert_eq!(lo, expect, "runs must be contiguous and in order: {runs:?}");
            assert!(lo < hi, "runs must be non-empty: {runs:?}");
            expect = hi;
        }
        assert_eq!(expect, costs.len(), "runs must cover every input exactly once: {runs:?}");
    }

    fn max_run(costs: &[u64], runs: &[(usize, usize)]) -> u64 {
        runs.iter().map(|&(lo, hi)| costs[lo..hi].iter().sum::<u64>()).max().unwrap_or(0)
    }

    #[test]
    fn partitions_hold_for_any_shape() {
        let shapes: Vec<Vec<u64>> = vec![
            vec![],
            vec![1_950_000],
            vec![1_950_000; 16],
            vec![1_950_000, 1_950_000, 29_250_000, 1_950_000, 1_950_000],          // one fat multisig input
            vec![29_250_000, 1_950_000, 1_950_000, 1_950_000, 1_950_000],          // fat input first
            vec![1_950_000, 1_950_000, 1_950_000, 1_950_000, 29_250_000],          // fat input last
            (0..100).map(|i| if i % 17 == 0 { 39_000_000 } else { 1_950_000 }).collect(),
        ];
        for costs in &shapes {
            for k in [1usize, 2, 3, 8, 16, 64] {
                assert_partitions(costs, &pack_chunks(costs, k, None));
            }
        }
    }

    #[test]
    fn never_returns_more_runs_than_asked_for() {
        let costs: Vec<u64> = (0..50).map(|i| 1_950_000 + i * 13_000).collect();
        for k in [1usize, 2, 7, 50, 500] {
            let runs = pack_chunks(&costs, k, None);
            assert!(runs.len() <= k.max(1), "asked for {k}, got {}", runs.len());
            assert!(runs.len() <= costs.len(), "more runs than inputs");
        }
    }

    #[test]
    fn uses_every_chunk_when_there_is_work_to_fill_it() {
        // An optimal cap can be reached with fewer runs than requested. Leaving provers idle is a real
        // cost, so the packer keeps dividing.
        let costs = vec![1_950_000u64; 32];
        assert_eq!(pack_chunks(&costs, 8, None).len(), 8);
        assert_eq!(pack_chunks(&costs, 32, None).len(), 32);
    }

    #[test]
    fn beats_equal_input_counts_on_a_skewed_block() {
        // Two 15-of-15 multisig inputs among singles, all landing in the same count-packed chunk.
        let mut costs = vec![1_950_000u64; 64];
        costs[4] = 1_950_000 * 15;
        costs[5] = 1_950_000 * 15;
        let k = 8;

        let by_count: Vec<(usize, usize)> =
            (0..k).map(|c| (c * 8, (c + 1) * 8)).collect();
        let by_cost = pack_chunks(&costs, k, None);
        assert_partitions(&costs, &by_cost);

        assert!(max_run(&costs, &by_cost) < max_run(&costs, &by_count),
            "cost packing did not lower the straggler: {} vs {}",
            max_run(&costs, &by_cost), max_run(&costs, &by_count));
    }

    #[test]
    fn minimises_the_heaviest_run() {
        // The binary search should land on a cap no smaller than achievable: one fewer must need more
        // runs than we are allowed.
        let costs: Vec<u64> = (0..40).map(|i| 100 + (i * 37) % 900).collect();
        let k = 6;
        let runs = pack_chunks(&costs, k, None);
        let cap = max_run(&costs, &runs);
        assert!(runs.len() <= k);
        assert!(cap >= *costs.iter().max().unwrap(), "a run cannot be lighter than its heaviest input");
        assert!(runs_at_cap(&costs, cap - 1) > k,
            "cap {cap} was not minimal — {} runs fit under {}", runs_at_cap(&costs, cap - 1), cap - 1);
    }

    /// Locks in the byte term. Block 741000's chunk 1 and chunk 11 hold the same number of inputs
    /// performing the same number of EC verifies, and chunk 1 still costs more because its
    /// transactions are 20x bigger. An EC-only model rates the two equal and packs them as though
    /// they were.
    ///
    /// The margin shrank when the guest stopped reading its payload through serde (#135): bytes were
    /// 63% of a fat chunk at 182 cycles each, and are ~25% at 36. The term is smaller, not gone —
    /// dropping it costs roughly 1.3x on the straggler here, and more on blocks with fatter
    /// transactions. Measured: chunk 1 109,113,751 cycles against chunk 11 83,264,971.
    #[test]
    fn marshalling_bytes_are_costed_not_just_signatures() {
        let real = |ec: u64, bytes: u64| super::COST_PER_EC_OP * ec + super::COST_PER_INPUT_BYTE * bytes;
        let (lean, fat) = (real(42, 37_933), real(42, 765_282));

        // RE-MEASURED 2026-09-05 under the #139 arm (patch 0005 applied, `hazync_ecmult_verify`
        // present), block 741000, 16 chunks, execute mode. Chunks 1 and 11 are the controlled pair:
        // identical 42 inputs and 42 EC verifies, differing only in bytes (765,282 vs 37,933).
        //
        //   chunk  1 (fat)   10,039,314 cycles
        //   chunk 11 (lean)   7,763,901 cycles      ratio 1.293x
        //
        // The premise HOLDS -- bytes are costed, and an EC-only model would rate these two equal.
        // The old anchors (86,495,630 / 82,090,232) were measured pre-#139, when an ECDSA verify cost
        // 1,950,000 rather than 141,612; they describe a build that no longer exists.
        //
        // ⚠ This deliberately does NOT assert 2% agreement the way the old anchors did. Under #139 the
        // model does not have that accuracy: it predicts 1.574x for this pair against 1.293x measured,
        // because COST_PER_INPUT_BYTE (6) is ~2x the measured 3.13 cycles/byte. A least-squares refit
        // over all 16 chunks gives 106,177*ec + 2.62*bytes + 90,395*inputs at 5.2% mean error, against
        // 13.1% for the current constants. Refitting is a real calibration change and is NOT made here
        // -- block 741000 carries 2 Schnorr verifies out of 723, so it cannot inform the curve split at
        // all, and that split is the dimension the packer most needs to be right about.
        const MEASURED_FAT: f64 = 10_039_314.0;
        const MEASURED_LEAN: f64 = 7_763_901.0;
        assert!(fat > lean, "an EC-only model cannot tell these apart: {fat} vs {lean}");
        // The model must track the measured ordering AND stay within a factor of the measured ratio.
        // This catches the byte term collapsing to zero (ratio -> 1.0) or exploding, which is what the
        // 2% anchors were really protecting, without asserting a precision the constants do not have.
        let model_ratio = fat as f64 / lean as f64;
        let measured_ratio = MEASURED_FAT / MEASURED_LEAN;
        assert!(
            model_ratio > 1.05,
            "the byte term has stopped discriminating: model ratio {model_ratio:.3}x"
        );
        assert!(
            model_ratio / measured_ratio < 1.5 && measured_ratio / model_ratio < 1.5,
            "model ratio {model_ratio:.3}x is far from the measured {measured_ratio:.3}x -- \
             a coefficient changed without re-measuring"
        );

        // And the packer must act on it. Assert the PROPERTY, not a fixed partition: which split wins
        // depends on the coefficients, and an earlier version of this test hard-coded the answer that
        // was optimal at 182 cycles/byte and broke when the byte term was refitted.
        let costs = vec![lean, lean, fat, fat];
        let runs = pack_chunks(&costs, 2, None);
        assert_partitions(&costs, &runs);
        let best = (1..costs.len())
            .map(|split| {
                let l: u64 = costs[..split].iter().sum();
                let r: u64 = costs[split..].iter().sum();
                l.max(r)
            })
            .min()
            .unwrap();
        assert_eq!(max_run(&costs, &runs), best, "not the optimal 2-way split: {runs:?}");
    }

    #[test]
    fn single_input_and_single_chunk_degenerate_cleanly() {
        assert_eq!(pack_chunks(&[1_950_000], 8, None), vec![(0, 1)]);
        assert_eq!(pack_chunks(&[1_950_000; 5], 1, None), vec![(0, 5)]);
        assert!(pack_chunks(&[], 8, None).is_empty());
    }
    /// The key-reuse model must actually MOVE the partition, or it is a no-op wearing a constant.
    /// Two runs of identical shape where one reuses a key and the other does not: the reusing run
    /// should be allowed MORE inputs under the same cap.
    #[test]
    fn key_reuse_lets_a_run_hold_more_inputs() {
        let n = 40;
        let costs = vec![COST_INPUT_BASE + COST_PER_EC_OP; n];
        let savings = vec![COST_PER_EC_OP - COST_PER_EC_OP_REPEAT; n];
        let same = vec![Some([7u8; 33]); n];          // every input reuses one key
        let distinct: Vec<Option<[u8; 33]>> =
            (0..n).map(|i| { let mut k = [0u8; 33]; k[0] = 2; k[1] = i as u8; Some(k) }).collect();

        let cap = (COST_INPUT_BASE + COST_PER_EC_OP) * 10;
        let r_same = split_at_cap_keyed(&costs, &savings, &same, cap);
        let r_diff = split_at_cap_keyed(&costs, &savings, &distinct, cap);
        // Assert the SIZE of the first run, not the number of runs. Run count is a ceiling and rounds
        // the effect away: at n=40 a run of 10 and a run of 12 both give 4 runs, so this read as "no
        // improvement" for any saving under 3.3x. The first run's width is what the discount actually
        // moves, and it moves by the whole discount rather than by whatever survives the rounding.
        let (w_same, w_diff) = (r_same[0].1 - r_same[0].0, r_diff[0].1 - r_diff[0].0);
        assert!(
            w_same > w_diff,
            "reused keys must pack denser: same={w_same} inputs in the first run, distinct={w_diff}"
        );
        // and the unkeyed walk must agree with the all-distinct case, since nothing is ever reused
        assert_eq!(split_at_cap(&costs, cap).len(), r_diff.len());
    }

}
