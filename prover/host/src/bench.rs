//! Measurement and self-test commands: exec-time, chunk-profile, segment-mem, segment-size, vb-stages,
//! test-merkle, test-locks, check-full, check-ibd, check-bip30, adversarial, regress, prove-chain-bad,
//! script-flags-test, bundle-roundtrip-test.

use bitcoin::{Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness, absolute,
    transaction};
use bitcoin::consensus::{deserialize, serialize};
use bitcoin::hashes::Hash as _;
use hazync_coinbase_smt::Smt;
use hazync_utreexo::{Forest, Hash, coin_leaf, hash_leaf};
use methods::{METHOD_ELF, METHOD_ID};
use risc0_zkvm::{ExecutorEnv, ProverOpts, default_executor, default_prover};
use serde::Deserialize;
use crate::{Bip30Del, Bip30Overwrite, BlockInput, BlockWitness, Bundle, CB170, CB171, ChainState,
    GENESIS_TIME, HASH169, HASH170, PackedBytes, PackedHash, PackedHashes, WireProof, WireStump,
    advance_recent, arr, build_block, build_block_carried, build_full, build_header, build_header_v,
    cb_spendable_outputs, cb_spends_from, chain_step, genesis_anchor, header_170, hex, hx, ibd_range,
    median_u32, out_leaf_of, out_spendable, prove_segment_resilient, prove_step, push_mtp, read_block_json,
    rev, script_flags, seed_and_anchor, seg_po2, smt_advance, smt_witness_standalone, spend_170,
    state_journal_bytes, wire_proof, wire_stump, work_u128};
use crate::chunks::{FIT_NAMES, chunk_block_hash, chunk_bounds, exec_chunk_cycles, input_costs,
    input_features, input_keys, nchunks_env, pack_chunks, predicted_ec_ops, predicted_sig_ops,
    repeat_savings, write_chunk_inputs};

// Real block 91842 (coinbase-only) — a BIP30 grandfathered block: its coinbase duplicates block 91812's
// still-unspent coinbase (merkle == that coinbase's txid). Used by check-bip30 (F3).
pub(crate) const CB91842: &str = "01000000010000000000000000000000000000000000000000000000000000000000000000ffffffff060456720e1b00ffffffff0100f2052a010000004341046896ecfc449cb8560594eb7f413f199deb9b4e5d947a142e7dc7d2de0b811b8e204833ea2a2fd9d4c7b153a8ca7661d0a0b7fc981df1f42f55d64b26b3da1e9cac00000000";
pub(crate) const PREV91842: &str = "00000000000a1e92acbcbdf594cac25d1095544d5fbf5113bfec85a9eb4b1120";
pub(crate) const MERKLE91842: &str = "d5d27987d2a3dfc724e359870c6644b40e497bdc0589a033220fe15429d88599";

// CHECK-FULL: execute-mode (no proving) validation of the HAZYNC_BLOCK — runs the exact same guest
// consensus path as prove_full (mode 2, is_base=1). Guest asserts block_valid, so a clean execute ==
// every rule passed (scripts, no-inflation, PoW, retarget, merkle, subsidy, weight, sigops, witness
// commitment, BIP34, BIP30, and now REAL maturity/BIP68 from the S2 metadata). Cheap pre-flight before
// Isolated exerciser for the real maturity/BIP68 relative-lock check (guest mode 8). Builds a minimal
// v2 tx with one input carrying the given nSequence, then runs `check_input_locks` with real MTP
// numbers supplied via env vars. Lets us drive the time-based branch (which no tested block exercises)
// with real mainnet MTP data. Return codes: 1 valid, -40 immature coinbase, -41 height-lock unmet,
// -42 time-lock unmet.
// COV-2 negative test: demonstrate the merkle mutation (CVE-2012-2459) check. An honest 3-tx list
// [A,B,C] and a malleated 4-tx list [A,B,C,C] (last tx duplicated) produce the SAME merkle root — the
// classic malleability — but the real Core ComputeMerkleRoot flags the second as `mutated`. Our
// `merkle_ok` requires `mutated == 0`, so the malleated block is rejected.
pub(crate) fn test_merkle_cmd() {
    let run = |txids: &[[u8; 32]]| -> ([u8; 32], u8) {
        let flat: Vec<u8> = txids.iter().flatten().copied().collect();
        let mut b = ExecutorEnv::builder();
        b.write(&9u32).unwrap();
        b.write(&flat).unwrap();
        let s = default_executor().execute(b.build().unwrap(), METHOD_ELF).expect("exec");
        s.journal.decode().unwrap()
    };
    let (a, bb, c) = ([0x11u8; 32], [0x22u8; 32], [0x33u8; 32]);
    let (root_n, mut_n) = run(&[a, bb, c]);      // honest 3-tx block
    let (root_m, mut_m) = run(&[a, bb, c, c]);   // malleated: last tx duplicated (CVE-2012-2459)
    println!("normal  [A,B,C]   : merkle {}  mutated={}  (merkle_ok: {})", hex(&root_n), mut_n, mut_n == 0);
    println!("mutated [A,B,C,C] : merkle {}  mutated={}  (merkle_ok: {})", hex(&root_m), mut_m, mut_m == 0);
    println!("SAME root (CVE collision): {}  -> the malleated block is REJECTED on merkle_ok (mutated=1)",
        root_n == root_m);
}

/// ROADMAP I1: Core's own script / transaction test vectors through the guest (mode 10). Cases from
/// `scripts/libzync/core_vectors.py <bitcoin-core> <cases.json>`; each is compared with Core's expectation (validity, and
/// for script_tests the exact ScriptError; BADTX = CheckTransaction refused). Prints every mismatch and a total.
///   host core-vectors <cases.json> [cases per execution, default 500]
pub(crate) fn core_vectors_cmd(args: &[String]) {
    #[derive(Deserialize)]
    struct Case { kind: u32, tx: String, prevouts: String, input: u32, flags: u32, label: String,
                  valid: Option<bool>, code: Option<i32> }
    let i = args.iter().position(|a| a == "core-vectors").unwrap();
    let path = args.get(i + 1).expect("usage: core-vectors <cases.json> [batch]");
    let batch: usize = args.get(i + 2).map(|x| x.parse().expect("batch")).unwrap_or(500);
    let cases: Vec<Case> = serde_json::from_slice(&std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}")))
        .expect("cases JSON");
    let unhex = |h: &str| -> Vec<u8> { (0..h.len()).step_by(2).map(|j| u8::from_str_radix(&h[j..j + 2], 16).unwrap()).collect() };
    let (mut agree, mut bad, mut cycles) = (0usize, 0usize, 0u64);
    for chunk in cases.chunks(batch) {
        let mut b = ExecutorEnv::builder();
        b.write(&10u32).unwrap();
        b.write(&(chunk.len() as u32)).unwrap();
        for c in chunk {
            b.write(&(c.kind, unhex(&c.tx), unhex(&c.prevouts), c.input, c.flags)).unwrap();
        }
        let s = default_executor().execute(b.build().unwrap(), METHOD_ELF).expect("core-vectors: guest execution failed");
        cycles += s.cycles();
        let got: Vec<i32> = s.journal.decode().expect("mode 10 journal");
        assert_eq!(got.len(), chunk.len());
        for (c, rc) in chunk.iter().zip(got) {
            let ok = match (c.code, c.valid) {
                (Some(code), _) => rc == code,
                (None, Some(true)) => rc == 1,
                (None, Some(false)) => rc != 1 && rc != -1000,
                (None, None) => panic!("{}: no expectation", c.label),
            };
            if ok { agree += 1 } else {
                bad += 1;
                println!("MISMATCH {}: guest {rc}, want {}", c.label,
                         c.code.map(|x| x.to_string()).unwrap_or_else(|| if c.valid == Some(true) { "valid".into() } else { "invalid".into() }));
            }
        }
        eprintln!("  {} / {} cases, {} mismatches", agree + bad, cases.len(), bad);
    }
    println!("CORE_VECTORS {agree} agree, {bad} differ, of {} ({} cycles)", cases.len(), cycles);
    if bad > 0 { std::process::exit(1); }
}

pub(crate) fn test_locks_cmd() {
    let ev = |k: &str, d: u32| std::env::var(k).ok().and_then(|s| s.parse().ok()).unwrap_or(d);
    let seq = ev("HAZYNC_LOCK_SEQ", 0);
    let coin_mtp = ev("HAZYNC_LOCK_COINMTP", 0);
    let spend_mtp = ev("HAZYNC_LOCK_SPENDMTP", 0);
    let coin_h = ev("HAZYNC_LOCK_COINH", 100);
    let spend_h = ev("HAZYNC_LOCK_SPENDH", 200);
    let cb = ev("HAZYNC_LOCK_CB", 0);
    // Real-tx mode: if HAZYNC_LOCK_RAWTX (hex) is set, feed the ACTUAL mainnet tx bytes to the real
    // check_input_locks (its version + vin[idx].nSequence are read from these). Else build a minimal
    // synthetic v2 tx carrying HAZYNC_LOCK_SEQ.
    let input_idx: u32 = std::env::var("HAZYNC_LOCK_IDX").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let raw: Vec<u8> = if let Ok(h) = std::env::var("HAZYNC_LOCK_RAWTX") {
        hx(h.trim())
    } else {
        let mut raw: Vec<u8> = Vec::new();
        raw.extend_from_slice(&2u32.to_le_bytes()); // version 2
        raw.push(1); // vin count
        raw.extend_from_slice(&[0u8; 32]); // prev txid
        raw.extend_from_slice(&0u32.to_le_bytes()); // prev vout
        raw.push(0); // scriptSig len
        raw.extend_from_slice(&seq.to_le_bytes()); // nSequence
        raw.push(1); // vout count
        raw.extend_from_slice(&0u64.to_le_bytes()); // value
        raw.push(0); // scriptPubKey len
        raw.extend_from_slice(&0u32.to_le_bytes()); // locktime
        raw
    };
    let mut b = ExecutorEnv::builder();
    b.write(&8u32).unwrap();
    b.write(&raw).unwrap();
    b.write(&input_idx).unwrap(); // input_idx
    b.write(&coin_h).unwrap();
    b.write(&cb).unwrap();
    b.write(&coin_mtp).unwrap();
    b.write(&spend_h).unwrap();
    b.write(&spend_mtp).unwrap();
    let s = default_executor().execute(b.build().unwrap(), METHOD_ELF).expect("exec");
    let rc: i32 = s.journal.decode().unwrap();
    let meaning = match rc { 1 => "VALID", -40 => "REJECT immature-coinbase", -41 => "REJECT height-lock-unmet", -42 => "REJECT time-lock-unmet", _ => "?" };
    // display the ACTUAL nSequence the check read (from the real tx bytes, not the synthetic var)
    let disp_seq = deserialize::<Transaction>(&raw).map(|t| t.input[input_idx as usize].sequence.0).unwrap_or(seq);
    println!("LOCKS rc={} ({})  [nSequence={:#010x} coin_mtp={} spend_mtp={} coin_h={} spend_h={} cb={}]",
        rc, meaning, disp_seq, coin_mtp, spend_mtp, coin_h, spend_h, cb);
}

// committing a multi-GPU prove; a false flag panics here in seconds-to-minutes on CPU, not hours on GPU.
pub(crate) fn check_full() {
    use std::time::Instant;
    let (anchor, w) = build_full();
    if std::env::var("HAZYNC_WITNESS_SIZES").is_ok() {
        // MEASURE THE WIRE, NOT THE SOURCE.
        //
        // The previous version of this report divided SOURCE byte counts (`t.0.len()`,
        // `siblings.len() * 32`) by a WIRE total (`to_vec(&w) * 4`). Those are different units, and
        // the mismatch is not uniform: risc0 serde spends one 32-bit word per byte for a plain
        // `[u8; 32]` or `Vec<u8>`, so anything going through that path costs 4x on the wire, while a
        // `PackedBytes` costs 1x. Dividing 1x numerators by a 4x-inflated denominator understates
        // exactly the structures that pay the penalty -- which is the whole question here.
        //
        // Everything below is measured by running the SAME `to_vec` the executor uses, per
        // sub-structure, so every figure shares one unit and the amplification is visible directly.
        // TEN_MINUTE_BLOCK.md 7.8 asked for this before anyone writes an encoder; the 3.13x
        // per-input figure it quotes is arithmetic over a residual, and this replaces it.
        let wire = |n: usize| n * 4;
        let tot = wire(risc0_zkvm::serde::to_vec(&w).unwrap().len());
        let n = w.inputs.len();
        let pct = |x: usize| if tot > 0 { x as f64 / tot as f64 * 100.0 } else { 0.0 };

        let w_txs = wire(risc0_zkvm::serde::to_vec(&w.txs).unwrap().len());
        let w_prevouts = wire(risc0_zkvm::serde::to_vec(&w.tx_prevouts).unwrap().len());
        let w_inputs = wire(risc0_zkvm::serde::to_vec(&w.inputs).unwrap().len());
        let w_txids = wire(risc0_zkvm::serde::to_vec(&w.txids).unwrap().len());
        let w_header = wire(risc0_zkvm::serde::to_vec(&w.header).unwrap().len());
        let w_cb = wire(risc0_zkvm::serde::to_vec(&w.coinbase_tx).unwrap().len());
        let w_stumps = wire(risc0_zkvm::serde::to_vec(&w.root_prev).unwrap().len())
            + wire(risc0_zkvm::serde::to_vec(&w.root_next).unwrap().len());
        let w_bip30 = wire(risc0_zkvm::serde::to_vec(&w.bip30).unwrap().len());
        let w_smt = wire(risc0_zkvm::serde::to_vec(&w.smt).unwrap().len());

        // Inside `inputs`: the two WireProofs against everything else. Each proof is a `[u8; 32]`
        // leaf plus a `Vec<[u8; 32]>` of siblings, and every one of those bytes is a word on the
        // wire. This split is what decides whether an encoder is worth writing.
        let mut w_proofs = 0usize;
        let mut src_proofs = 0usize;
        for i in &w.inputs {
            w_proofs += wire(risc0_zkvm::serde::to_vec(&i.proof_i).unwrap().len());
            w_proofs += wire(risc0_zkvm::serde::to_vec(&i.proof_last).unwrap().len());
            src_proofs += 32 + i.proof_i.siblings.0.len() * 32 + 8;
            src_proofs += 32 + i.proof_last.siblings.0.len() * 32 + 8;
        }
        let w_scalars = w_inputs.saturating_sub(w_proofs);

        let src_txs: usize = w.txs.iter().map(|t| t.0.len()).sum();
        let src_prevouts: usize = w.tx_prevouts.iter().map(|t| t.0.len()).sum();
        let src_txids = w.txids.0.len() * 32;
        let amp = |wire_b: usize, src_b: usize| if src_b > 0 { wire_b as f64 / src_b as f64 } else { 0.0 };

        println!("WITNESS block {} inputs={n} txs(deduped)={} total={tot}B (wire)", w.height, w.txs.len());
        println!("  field            wire B      %     source B    amplification");
        println!("  inputs        {w_inputs:>10} {:>6.1}%          --            --", pct(w_inputs));
        println!("    proofs      {w_proofs:>10} {:>6.1}%  {src_proofs:>10}         {:>5.2}x", pct(w_proofs), amp(w_proofs, src_proofs));
        println!("    scalars     {w_scalars:>10} {:>6.1}%          --            --", pct(w_scalars));
        println!("  txs           {w_txs:>10} {:>6.1}%  {src_txs:>10}         {:>5.2}x", pct(w_txs), amp(w_txs, src_txs));
        println!("  tx_prevouts   {w_prevouts:>10} {:>6.1}%  {src_prevouts:>10}         {:>5.2}x", pct(w_prevouts), amp(w_prevouts, src_prevouts));
        println!("  txids         {w_txids:>10} {:>6.1}%  {src_txids:>10}         {:>5.2}x", pct(w_txids), amp(w_txids, src_txids));
        println!("  smt           {w_smt:>10} {:>6.1}%", pct(w_smt));
        println!("  stumps        {w_stumps:>10} {:>6.1}%", pct(w_stumps));
        println!("  header+cbtx   {:>10} {:>6.1}%", w_header + w_cb, pct(w_header + w_cb));
        println!("  bip30         {w_bip30:>10} {:>6.1}%", pct(w_bip30));
        if n > 0 {
            println!("  per input: {} wire B, of which {} B is the two proofs", w_inputs / n, w_proofs / n);
        }

        // What a packed encoding of the proofs would actually buy, on measured numbers rather than
        // on the residual. Packing takes those bytes from 4 wire bytes each to 1, so the floor is
        // the source size; everything else on the wire is untouched.
        let projected = tot - w_proofs + src_proofs;
        println!("  IF proofs were packed: {projected}B vs {tot}B  =>  {:.3}x on witness bytes",
            if projected > 0 { tot as f64 / projected as f64 } else { 0.0 });
        println!("  (witness bytes only -- the cycle win follows only if deserialisation stays at \
            ~347 cyc/B; that is the next thing to measure, not to assume)");
        return;
    }
    println!("=== CHECK-FULL (execute, no proof) block {} — {} inputs ===", w.height, w.inputs.len());
    let mut b = ExecutorEnv::builder();
    b.write(&2u32).unwrap();
    b.write(&state_journal_bytes(&anchor)).unwrap();
    b.write(&w).unwrap();
    b.write(&1u32).unwrap();
    b.write(&METHOD_ID).unwrap();
    let t = Instant::now();
    // Do NOT collapse every executor error into "the guest rejected the block". An environment failure
    // and a consensus rejection need opposite responses, and reporting the first as the second sends you
    // hunting a consensus bug that does not exist: a missing r0vm binary reported itself as "guest
    // asserted a consensus flag false", which cost two rounds of diagnosis. IO errors name a file or a
    // process; a guest rejection never does.
    let s = match default_executor().execute(b.build().unwrap(), METHOD_ELF) {
        Ok(s) => s,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("No such file or directory")
                || msg.contains("os error")
                || msg.contains("Permission denied")
                || msg.contains("server version")
            {
                eprintln!("CHECK-FULL could not RUN. This is an ENVIRONMENT failure, not a consensus");
                eprintln!("rejection -- the block was never validated.");
                eprintln!("  underlying: {msg}");
                eprintln!();
                eprintln!("  Most likely the external r0vm server is missing: default_executor() spawns");
                eprintln!("  it unless the binary was built with an in-process prover. Run inside the");
                eprintln!("  build container, or use a binary with the prover linked in.");
                std::process::exit(2);
            }
            panic!("CHECK-FULL FAILED: the guest REJECTED the block -- a consensus flag came back false \
                    (see the guest message above).\n  underlying: {msg}");
        }
    };
    let tip: ChainState = s.journal.decode().unwrap();
    println!(">>> BLOCK {} VALID (execute {:.0}s, {} cycles) — all consensus flags true.",
        w.height, t.elapsed().as_secs_f64(), s.cycles());
    println!("  tip_hash {}  cum_work {}  UTXO leaves {}", hex(&tip.tip_hash), work_u128(&tip.cum_work), tip.utxo_leaves);
}

// CHECK-IBD: execute-mode fold from the genesis anchor over [from..=to]. Validates every block's full
// consensus AND the real carried-accumulator transition + chain linkage + retarget — no proving.
pub(crate) fn check_ibd() {
    use std::time::Instant;
    let (dir, from, to) = ibd_range();
    println!("=== CHECK-IBD (execute) genesis-anchor → fold blocks {from}..={to} from {dir} ===");
    let mut forest = Forest::new();
    let mut state = genesis_anchor();
    let t = Instant::now();
    let mut total_cyc = 0u64;
    let mut block_mtp: Vec<u32> = vec![GENESIS_TIME]; // index = height; [0] = genesis
    let mut win: Vec<u32> = vec![GENESIS_TIME];       // rolling ≤11 block times for MTP
    for h in from..=to {
        let j = read_block_json(&dir, h);
        push_mtp(&j, &mut win, &mut block_mtp);
        let w = build_block_carried(&mut forest, &j, &block_mtp);
        let (ns, cyc) = chain_step(&state, &w, if h == from { 1 } else { 0 });
        assert_eq!(ns.height, h, "block {h}: height did not advance");
        state = ns;
        total_cyc += cyc;
        if h % 200 == 0 || h == to {
            println!("  folded {h}: tip {} leaves {} cum_work {} ({:.0}s)", hex(&state.tip_hash), state.utxo_leaves, work_u128(&state.cum_work), t.elapsed().as_secs_f64());
        }
    }
    println!(">>> CHECK-IBD {from}..{to} VALID ({:.0}s, {}M cyc). tip_hash {}  cum_work {}  UTXO leaves {}",
        t.elapsed().as_secs_f64(), total_cyc / 1_000_000, hex(&state.tip_hash), work_u128(&state.cum_work), state.utxo_leaves);
}

pub(crate) fn dump_input_features(w: &BlockWitness, path: &str) {
    use std::fmt::Write as _;
    let mut s = format!("tx_idx,input_idx,tx_len,{}\n", FIT_NAMES.join(","));
    for (inp, f) in w.inputs.iter().zip(input_features(w)) {
        let _ = write!(s, "{},{},{}", inp.tx_idx, inp.input_idx, w.txs[inp.tx_idx as usize].0.len());
        for x in f { let _ = write!(s, ",{x}"); }
        s.push('\n');
    }
    std::fs::write(path, s).expect("write HAZYNC_DUMP_INPUT_FEATURES");
    eprintln!("input features → {path}");
}

// `chunk-profile`: report how work is spread across a block's chunks, under both packing strategies.
// Prediction alone is free and needs no GPU; `HAZYNC_PROFILE_EXEC=1` additionally runs each chunk in
// execute mode for its real cycle count, which is the measurement #132 asks for before any GPU time is
// bought. Execute mode is slow but costs nothing but wall-clock.
pub(crate) fn chunk_profile() {
    let (_anchor, w) = build_full();
    let n = w.inputs.len();
    let nchunks = nchunks_env();
    let costs = input_costs(&w);
    let keys = input_keys(&w);   // for the per-chunk repeat count reported below
    let exec = std::env::var("HAZYNC_PROFILE_EXEC").is_ok();
    if let Ok(p) = std::env::var("HAZYNC_DUMP_INPUT_FEATURES") { dump_input_features(&w, &p); }

    println!("=== CHUNK PROFILE block {}: {} inputs → {} chunks ===", w.height, n, nchunks);
    let total_ec: u64 = w.inputs.iter().map(|inp| predicted_ec_ops(
        &w.txs[inp.tx_idx as usize].0, inp.input_idx, &w.tx_prevouts[inp.tx_idx as usize].0)).sum();
    println!("predicted EC verifies: {} across {} inputs ({:.2} per input)",
        total_ec, n, total_ec as f64 / n.max(1) as f64);

    for (label, bounds) in [
        ("count-packed (old)", {
            let k = nchunks.max(1).min(n.max(1));
            let sz = n.div_ceil(k.max(1));
            (0..k).map(|c| (c * sz, ((c + 1) * sz).min(n))).filter(|(a, b)| a < b).collect::<Vec<_>>()
        }),
        // hazync#226: honour HAZYNC_PACK_KEYS here too. This arm hardcoded `None`, so the memo
        // term — and with it COST_PER_EC_OP_REPEAT — could never influence the partition that
        // chunk-profile reports. The A/B the issue asks for was therefore a guaranteed null: every
        // value of the constant produced byte-identical output, which reads as "no effect" rather
        // than "not exercised". Measured before this change: 104,222 and 413,800 both gave
        // straggler 1.2104. → gotcha_checks_that_cannot_fail_and_logs_that_cannot_speak
        ("cost-packed (new)", {
            if std::env::var("HAZYNC_PACK_KEYS").as_deref() == Ok("1") {
                let (sv, ks) = (repeat_savings(&w), input_keys(&w));
                pack_chunks(&costs, nchunks, Some((&sv, &ks)))
            } else {
                pack_chunks(&costs, nchunks, None)
            }
        }),
    ] {
        // libzync: HAZYNC_PROFILE_LAYOUT=cost runs only the cost-packed layout (the one proving uses).
        if std::env::var("HAZYNC_PROFILE_LAYOUT").as_deref() == Ok("cost") && label.starts_with("count") {
            continue;
        }
        println!("\n--- {label}: {} chunks ---", bounds.len());
        let mut predicted: Vec<u64> = Vec::new();
        let mut measured: Vec<u64> = Vec::new();
        for (i, &(lo, hi)) in bounds.iter().enumerate() {
            let c: u64 = costs[lo..hi].iter().sum();
            predicted.push(c);
            let bytes: u64 = w.inputs[lo..hi].iter().map(|inp|
                (w.txs[inp.tx_idx as usize].0.len() + w.tx_prevouts[inp.tx_idx as usize].0.len()) as u64).sum();
            let ec: u64 = w.inputs[lo..hi].iter().map(|inp| predicted_ec_ops(
                &w.txs[inp.tx_idx as usize].0, inp.input_idx, &w.tx_prevouts[inp.tx_idx as usize].0)).sum();
            // ⛔ Report the two curves SEPARATELY, not just their total. Fitting a cost model against
            // the combined count cannot separate them, so a refit has to reuse whatever ECDSA:Schnorr
            // ratio it started with -- and that ratio is build-dependent. Rescaling Ghost's stale
            // 13.77x ratio is exactly what made its straggler WORSE (1.407 -> 1.884). The two columns
            // below are what a per-curve fit needs.
            let (ecdsa_n, schnorr_n) = w.inputs[lo..hi].iter().fold((0u64, 0u64), |(e, s), inp| {
                let o = predicted_sig_ops(&w.txs[inp.tx_idx as usize].0, inp.input_idx,
                                          &w.tx_prevouts[inp.tx_idx as usize].0);
                (e + o.ecdsa, s + o.schnorr)
            });
            // ⛔ The dimension a static (ecdsa, schnorr, bytes, inputs) fit CANNOT capture. Cost is
            // position-dependent inside a chunk: an input whose pubkey was already decompressed
            // earlier in the SAME chunk is cheaper, which is what COST_PER_EC_OP_REPEAT prices and
            // what KeyWalk walks. On block 962,000 two chunks with near-identical ecdsa counts (210
            // vs 205) and byte sizes (14.7 M vs 14.9 M) measured 28.5 M against 80.4 M cycles -- 2.8x
            // apart, and no model over those four columns can fit both. `repeats` is that term.
            let repeats = {
                let mut seen = std::collections::HashSet::new();
                let mut n = 0u64;
                for k in keys[lo..hi].iter().flatten() {
                    if !seen.insert(*k) { n += 1; }
                }
                n
            };
            let cycles = if exec { Some(exec_chunk_cycles(&w, lo, hi)) } else { None };
            if let Some(cy) = cycles { measured.push(cy); }
            match cycles {
                Some(cy) => println!("  chunk {i:>3}  inputs {:>5}  ec {:>5}  ecdsa {:>5}  schnorr {:>5}  repeats {:>5}  bytes {:>9}  predicted {:>10}  cycles {:>14}", hi - lo, ec, ecdsa_n, schnorr_n, repeats, bytes, c, cy),
                None     => println!("  chunk {i:>3}  inputs {:>5}  ec {:>5}  ecdsa {:>5}  schnorr {:>5}  repeats {:>5}  bytes {:>9}  predicted {:>10}", hi - lo, ec, ecdsa_n, schnorr_n, repeats, bytes, c),
            }
        }
        // The straggler ratio is the number that matters: a block's wall-clock is its slowest chunk, so
        // this is the factor by which fan-out falls short of the work being evenly shared.
        //
        // ⛔ REPORT IT ON MEASURED CYCLES WHENEVER WE HAVE THEM. Computed on PREDICTED cost this is a
        // check that cannot fail: the cost packer balances the predictor by construction, so it
        // reports a perfect 1.00x however wrong the predictor is -- it measures the packer against
        // itself. On block 962,000 with the field backend it reported 1.00x while real chunk cycles
        // spanned 33.6 M to 352 M, an actual straggler of 1.563x, WORSE than the naive count packer.
        let strag = |v: &[u64], what: &str| {
            let max = v.iter().copied().max().unwrap_or(0);
            let mean = v.iter().sum::<u64>() as f64 / v.len().max(1) as f64;
            let min = v.iter().copied().min().unwrap_or(0);
            println!("  straggler ({what}): max {} vs mean {:.0} = {:.3}x   [spread {:.1}x]",
                     max, mean, max as f64 / mean.max(1.0), max as f64 / min.max(1) as f64);
        };
        strag(&predicted, "predicted");
        if !measured.is_empty() {
            strag(&measured, "MEASURED");
            // A packer is only as good as its cost model; this is the number that says whether it is.
            let pm: f64 = predicted.iter().sum::<u64>() as f64 / predicted.len().max(1) as f64;
            let mm: f64 = measured.iter().sum::<u64>() as f64 / measured.len().max(1) as f64;
            let worst = predicted.iter().zip(&measured)
                .map(|(p, m)| (*m as f64 / mm) / (*p as f64 / pm))
                .fold(0.0f64, |a, b| a.max(b));
            println!("  cost-model error: worst chunk is {:.2}x its predicted share of the block", worst);
        }
    }
}

// ADVERSARIAL S1: prove a valid base (170), then attempt to fold 171 with a WRONG self_id.
// The guest's `assert(prev.self_id == self_id)` (and the unresolvable composition) must reject it.
pub(crate) fn prove_chain_bad() {
    let (mut forest, anchor) = seed_and_anchor();
    let mut recent = anchor.recent_times.clone();
    let w170 = build_block(&mut forest, header_170(), 170, CB170, &[spend_170()], median_u32(&recent));
    let r170 = prove_step(state_journal_bytes(&anchor), None, &w170, 1);
    advance_recent(&mut recent, 1_231_731_025);
    let cb171: Transaction = deserialize(&hx(CB171)).unwrap();
    let hdr171 = build_header(HASH170, &cb171.compute_txid().to_byte_array(), 1_231_731_401, 0x1d00ffff, 653_436_935);
    let w171 = build_block(&mut forest, hdr171, 171, CB171, &[], median_u32(&recent));
    let mut bad_id = METHOD_ID;
    bad_id[0] ^= 1; // corrupt the image id
    println!("=== ADVERSARIAL S1: folding block 171 with a WRONG self_id (must be rejected) ===");
    let mut b = ExecutorEnv::builder();
    b.segment_limit_po2(seg_po2());
    b.add_assumption(r170.clone());
    b.write(&2u32).unwrap();
    b.write(&r170.journal.bytes).unwrap();
    b.write(&w171).unwrap();
    b.write(&0u32).unwrap();
    b.write(&bad_id).unwrap(); // WRONG
    match default_prover().prove(b.build().unwrap(), METHOD_ELF) {
        Ok(_) => { println!(">>> ADVERSARIAL S1: FAIL — wrong self_id ACCEPTED (soundness hole!)"); std::process::exit(1); }
        Err(e) => println!(">>> ADVERSARIAL S1: wrong self_id REJECTED ✓  ({})", format!("{e}").lines().next().unwrap_or("")),
    }
}

// C1: fast, self-contained, EXECUTE-mode regression — no proving, no GPU, no external files. Runs
// block 170 through the whole consensus path (scripts, checks, accumulator, PoW, merkle, subsidy,
// BIP34/30, witness, self_id) and asserts the committed tip. Any consensus-logic regression trips
// either a chain_step assertion (execute → Err) or the tip mismatch.
pub(crate) fn regress() {
    let (mut forest, anchor) = seed_and_anchor();
    let w = build_block(&mut forest, header_170(), 170, CB170, &[spend_170()], median_u32(&anchor.recent_times));
    let mut b = ExecutorEnv::builder();
    b.write(&2u32).unwrap();
    b.write(&state_journal_bytes(&anchor)).unwrap();
    b.write(&w).unwrap();
    b.write(&1u32).unwrap();
    b.write(&METHOD_ID).unwrap();
    let s = default_executor().execute(b.build().unwrap(), METHOD_ELF).expect("regress: block 170 execute failed");
    let tip: ChainState = s.journal.decode().unwrap();
    let ok = tip.tip_hash == arr(rev(hx(HASH170))) && tip.height == 170 && tip.self_id == METHOD_ID;
    println!("[regress] block 170 chain_step (execute): tip {} height {} self_id-ok {}",
        if tip.tip_hash == arr(rev(hx(HASH170))) { "MATCH" } else { "MISMATCH" }, tip.height, tip.self_id == METHOD_ID);
    println!(">>> REGRESSION {}", if ok { "PASS ✓" } else { "FAIL ✗" });
    if !ok { std::process::exit(1); }
}

// ============================================================================================
// ADVERSARIAL SOUNDNESS SUITE — execute-mode, self-contained, no GPU, no external files. Each case
// builds a witness that exploits a specific hole from the 2026-07 soundness audit and asserts the
// guest REJECTS it, alongside an honest baseline that must be ACCEPTED (so a broken baseline can't
// make a malicious case look "rejected" for the wrong reason). Run with `host adversarial`; wired
// into CI. Holes: #1 host-controlled height, #3 in-block double-spend / ordering, #4 coinbase checks.
// (#2 segmented flag/witness binding needs proven chunks -> GPU box, see `prove-chunk-badheight`.)
// ============================================================================================

// Decode of the guest's mode-1 BlockOutput journal (field order MUST match the guest's BlockOutput).
#[derive(Deserialize)]
pub(crate) struct BlockOut {
    _script_results: Vec<i32>, _tx_checks: Vec<i32>, _coin_leaves: Vec<[u8; 32]>, _total_fee: i64,
    _pow_ok: bool, _merkle_ok: bool, _coinbase_val: i64, _subsidy: i64, _subsidy_ok: bool,
    all_ok: bool, _root_matches: bool,
}

// Execute one block witness in mode-1 (block_proof) and return the committed `all_ok` — which is
// independent of PoW/merkle, so a synthetic block with a dummy header still exercises the accumulator,
// script, coinbase and in-block-spend logic. A guest panic surfaces as Err => treated as rejected.
pub(crate) fn block_all_ok(w: &BlockWitness) -> bool {
    let mut b = ExecutorEnv::builder();
    b.write(&1u32).unwrap();
    b.write(w).unwrap();
    match default_executor().execute(b.build().unwrap(), METHOD_ELF) {
        Ok(s) => { let o: BlockOut = s.journal.decode().unwrap(); o.all_ok }
        Err(_) => false,
    }
}

// Build a synthetic OP_TRUE block: a coinbase, then `txs` (each a single-input tx) where inblock[i]
// marks txs[i] as spending an in-block-created coin (tx A's output 0) rather than the external coin C.
// Height 1000 => script flags 0, so bare OP_TRUE spends validate. Values: C=50.00001 BTC funds A
// (out 50 BTC), B/D spend A:0 (out 49.99999 / 49.99998), coinbase = 50 BTC + 2000 sat fees.
pub(crate) const SYNTH_H: u32 = 1000;
pub(crate) const SYNTH_T: u32 = 1_400_000_000;
pub(crate) fn synth_block(cb: &Transaction, txs: &[&Transaction], inblock: &[bool]) -> BlockWitness {
    let optrue = || ScriptBuf::from_bytes(vec![0x51]);
    let c_leaf = coin_leaf(&[0x11u8; 32], 0, 5_000_001_000, &[0x51], 1, false, 0);
    let mut forest = Forest::new();
    for i in 0..4u64 { forest.add(hash_leaf(&[b"pre".as_slice(), &i.to_le_bytes()].concat())); }
    forest.add(c_leaf);
    for i in 0..2u64 { forest.add(hash_leaf(&[b"post".as_slice(), &i.to_le_bytes()].concat())); }
    let root_prev = wire_stump(&forest);

    let cb_txid = cb.compute_txid().to_byte_array();
    let mut txids = vec![cb_txid];
    let mut wtxids: Vec<[u8; 32]> = vec![[0u8; 32]]; // coinbase wtxid = zeros (BIP141)
    let mut inputs: Vec<BlockInput> = Vec::new();
    let mut wtxs: Vec<PackedBytes> = Vec::new();
    let mut wtx_prevs: Vec<PackedBytes> = Vec::new();
    for (i, tx) in txs.iter().enumerate() {
        txids.push(tx.compute_txid().to_byte_array());
        wtxids.push(tx.compute_wtxid().to_byte_array());
        if inblock[i] {
            // spends A:0 (in-block coin, created THIS block at height H, mtp = block_time). No
            // accumulator proof needed — the guest skips the delete for in-block coins.
            let prevouts = serialize(&vec![TxOut { value: Amount::from_sat(5_000_000_000), script_pubkey: optrue() }]);
            wtxs.push(PackedBytes(serialize(*tx))); wtx_prevs.push(PackedBytes(prevouts));
            inputs.push(BlockInput {
                tx_idx: i as u32, input_idx: 0,
                global_pos: 0, coin_height: SYNTH_H, coin_is_coinbase: 0, coin_mtp: SYNTH_T, tx_first: 1,
                proof_i: WireProof { leaf: PackedHash([0u8; 32]), position: 0, siblings: PackedHashes(vec![]) },
                proof_last: WireProof { leaf: PackedHash([0u8; 32]), position: 0, siblings: PackedHashes(vec![]) },
            });
        } else {
            // external: spends C from the accumulator (real inclusion proof).
            let prevouts = serialize(&vec![TxOut { value: Amount::from_sat(5_000_001_000), script_pubkey: optrue() }]);
            let pos = forest.find(&c_leaf).expect("C in accumulator");
            let last = forest.leaves.len() - 1;
            wtxs.push(PackedBytes(serialize(*tx))); wtx_prevs.push(PackedBytes(prevouts));
            inputs.push(BlockInput {
                tx_idx: i as u32, input_idx: 0,
                global_pos: pos as u64, coin_height: 1, coin_is_coinbase: 0, coin_mtp: 0, tx_first: 1,
                proof_i: wire_proof(&forest.prove(pos)), proof_last: wire_proof(&forest.prove(last)),
            });
            forest.delete(pos);
        }
    }
    let root_next = wire_stump(&forest); // approximate — root_matches is not part of all_ok
    let header = build_header_v(1, HASH169, &[0u8; 32], SYNTH_T, 0x1d00ffff, 0);
    let (in_smt_root, smt) = smt_witness_standalone(cb_txid, cb_spendable_outputs(cb),
        &cb_spends_from(&inputs, &wtxs));
    BlockWitness { header, height: SYNTH_H, coinbase_tx: serialize(cb), txids: PackedHashes(txids), root_prev, txs: wtxs, tx_prevouts: wtx_prevs, inputs, root_next, bip30: None, in_smt_root, smt }
}

// The four OP_TRUE transactions (built via the real bitcoin crate so txids/serialization are correct).
pub(crate) fn synth_txs() -> (Transaction, Transaction, Transaction, Transaction, Transaction) {
    let optrue = || ScriptBuf::from_bytes(vec![0x51]);
    let txout = |sat: u64| TxOut { value: Amount::from_sat(sat), script_pubkey: optrue() };
    let vin = |txid: Txid, vout: u32, ss: Vec<u8>| TxIn {
        previous_output: OutPoint { txid, vout }, script_sig: ScriptBuf::from_bytes(ss),
        sequence: Sequence::MAX, witness: Witness::new(),
    };
    let mktx = |input: Vec<TxIn>, output: Vec<TxOut>| Transaction {
        version: transaction::Version(1), lock_time: absolute::LockTime::ZERO, input, output,
    };
    let c_txid = Txid::from_byte_array([0x11u8; 32]);
    let a = mktx(vec![vin(c_txid, 0, vec![])], vec![txout(5_000_000_000)]);
    let a_txid = a.compute_txid();
    let b = mktx(vec![vin(a_txid, 0, vec![])], vec![txout(4_999_999_000)]);
    let d = mktx(vec![vin(a_txid, 0, vec![])], vec![txout(4_999_998_000)]);
    let cb_ok = mktx(vec![vin(Txid::all_zeros(), 0xffff_ffff, vec![0x51, 0x51])], vec![txout(5_000_002_000)]);
    let cb_bad = mktx(vec![vin(Txid::all_zeros(), 0xffff_ffff, vec![0x51; 101])], vec![txout(5_000_002_000)]);
    (cb_ok, cb_bad, a, b, d)
}

// #5: a 2-input tx whose FIRST input's prevouts blob carries a phantom high-value coin (the fee blob
// the host supplies is not, entry-for-entry, bound to accumulator-authenticated coins). `phantom=false`
// is the honest baseline (both inputs share the real [C1,C2] blob); `phantom=true` puts a ~21M BTC
// fake coin at position 1 of the first input's blob to inflate the fee -> mint via the coinbase. The
// #5 pre-pass must reject it (the two inputs' blobs differ => group check fails => all_ok=false).
pub(crate) fn synth_unbound_prevouts(phantom: bool) -> BlockWitness {
    let optrue = || ScriptBuf::from_bytes(vec![0x51]);
    let (v1, v2): (u64, u64) = (3_000_000_000, 2_000_000_000); // C1 + C2 = 50 BTC
    let c1_txid = Txid::from_byte_array([0x21u8; 32]);
    let c2_txid = Txid::from_byte_array([0x22u8; 32]);
    let c1_leaf = coin_leaf(&[0x21u8; 32], 0, v1, &[0x51], 1, false, 0);
    let c2_leaf = coin_leaf(&[0x22u8; 32], 0, v2, &[0x51], 1, false, 0);
    // 2-input tx spending C1 and C2, one OP_TRUE output (fee 1000 sat).
    let vin = |txid: Txid| TxIn { previous_output: OutPoint { txid, vout: 0 },
        script_sig: ScriptBuf::new(), sequence: Sequence::MAX, witness: Witness::new() };
    let t = Transaction { version: transaction::Version(1), lock_time: absolute::LockTime::ZERO,
        input: vec![vin(c1_txid), vin(c2_txid)],
        output: vec![TxOut { value: Amount::from_sat(v1 + v2 - 1000), script_pubkey: optrue() }] };
    let t_raw = serialize(&t);
    // coinbase: subsidy(1000)=50 BTC + 1000 sat fee.
    let cb = Transaction { version: transaction::Version(1), lock_time: absolute::LockTime::ZERO,
        input: vec![TxIn { previous_output: OutPoint { txid: Txid::all_zeros(), vout: 0xffff_ffff },
            script_sig: ScriptBuf::from_bytes(vec![0x51, 0x51]), sequence: Sequence::MAX, witness: Witness::new() }],
        output: vec![TxOut { value: Amount::from_sat(5_000_001_000), script_pubkey: optrue() }] };

    let real_blob = serialize(&vec![
        TxOut { value: Amount::from_sat(v1), script_pubkey: optrue() },
        TxOut { value: Amount::from_sat(v2), script_pubkey: optrue() }]);
    let phantom_blob = serialize(&vec![
        TxOut { value: Amount::from_sat(v1), script_pubkey: optrue() },
        TxOut { value: Amount::from_sat(2_100_000_000_000_000), script_pubkey: optrue() }]); // ~21M BTC fake

    let mut forest = Forest::new();
    for i in 0..4u64 { forest.add(hash_leaf(&[b"pre".as_slice(), &i.to_le_bytes()].concat())); }
    forest.add(c1_leaf); forest.add(c2_leaf);
    for i in 0..2u64 { forest.add(hash_leaf(&[b"post".as_slice(), &i.to_le_bytes()].concat())); }
    let root_prev = wire_stump(&forest);

    let mk_input = |forest: &mut Forest, idx: u32, leaf: Hash| -> BlockInput {
        let pos = forest.find(&leaf).expect("coin in accumulator");
        let last = forest.leaves.len() - 1;
        let bi = BlockInput { tx_idx: 0, input_idx: idx,
            global_pos: pos as u64, coin_height: 1, coin_is_coinbase: 0, coin_mtp: 0, tx_first: (idx == 0) as u32,
            proof_i: wire_proof(&forest.prove(pos)), proof_last: wire_proof(&forest.prove(last)) };
        forest.delete(pos); bi
    };
    // Both inputs share the tx's SINGLE prevouts blob (dedup). The phantom-fee attack now lives in that
    // shared blob: prevouts[1] is a fake ~21M-BTC coin instead of the real C2 — so input 1's guest-computed
    // coin leaf no longer matches the accumulator-proven C2, and the block is rejected (accumulator binding,
    // the same soundness the per-input blob-equality check used to enforce).
    let in0 = mk_input(&mut forest, 0, c1_leaf);
    let in1 = mk_input(&mut forest, 1, c2_leaf);
    let shared_blob = if phantom { phantom_blob } else { real_blob.clone() };

    let header = build_header_v(1, HASH169, &[0u8; 32], SYNTH_T, 0x1d00ffff, 0);
    // Both inputs here are non-coinbase, so the transition is just the coinbase insert.
    let unbound_smt = smt_witness_standalone(cb.compute_txid().to_byte_array(),
        cb_spendable_outputs(&cb), &[]);
    BlockWitness { header, height: SYNTH_H, coinbase_tx: serialize(&cb),
        txids: PackedHashes(vec![cb.compute_txid().to_byte_array(), t.compute_txid().to_byte_array()]),
        root_prev, txs: vec![PackedBytes(t_raw.clone())], tx_prevouts: vec![PackedBytes(shared_blob)],
        inputs: vec![in0, in1], root_next: wire_stump(&forest), bip30: None,
        in_smt_root: unbound_smt.0, smt: unbound_smt.1 }
}

// #1: on the real block-170 chain step, downgrade the host-supplied height. The guest must reject
// (its `w.height == prev.height+1` assert fires => execute Err).
pub(crate) fn adv_height_rejected() -> bool {
    let (mut forest, anchor) = seed_and_anchor();
    let mut w = build_block(&mut forest, header_170(), 170, CB170, &[spend_170()], median_u32(&anchor.recent_times));
    w.height = 1; // attacker: height 1 turns every soft-fork flag off + subsidy -> 50 BTC
    let mut b = ExecutorEnv::builder();
    b.write(&2u32).unwrap();
    b.write(&state_journal_bytes(&anchor)).unwrap();
    b.write(&w).unwrap();
    b.write(&1u32).unwrap(); // is_base = 1
    b.write(&METHOD_ID).unwrap();
    default_executor().execute(b.build().unwrap(), METHOD_ELF).is_err()
}

pub(crate) fn adversarial() {
    println!("=== HAZYNC ADVERSARIAL SOUNDNESS SUITE (execute-mode) — every malicious witness must be REJECTED ===\n");
    let verdict = |rejected: bool| if rejected { "REJECTED ✓" } else { "ACCEPTED ✗ (SOUNDNESS HOLE)" };
    let mut pass = true;
    let (cb_ok, cb_bad, a, b, d) = synth_txs();

    // Baseline: an honest in-block spend (B spends A's output created in the same block) must be ACCEPTED.
    let honest = block_all_ok(&synth_block(&cb_ok, &[&a, &b], &[false, true]));
    println!("[baseline] honest in-block-spend block accepted ......... {}", if honest { "yes ✓" } else { "NO ✗ (baseline broken — fix before trusting the rejects)" });
    pass &= honest;

    let r1 = adv_height_rejected();
    println!("#1 host-controlled height (flag/subsidy downgrade) ...... {}", verdict(r1));
    pass &= r1;

    // #3a: B and D both spend A:0 in the same block (double-spend -> inflation).
    let r3a = !block_all_ok(&synth_block(&cb_ok, &[&a, &b, &d], &[false, true, true]));
    println!("#3 in-block coin spent twice (inflation) ................ {}", verdict(r3a));
    pass &= r3a;

    // #3b: B (spending A:0) placed BEFORE A creates it (spend-before-create / ordering).
    let r3b = !block_all_ok(&synth_block(&cb_ok, &[&b, &a], &[true, false]));
    println!("#3 spend-before-create ordering violation ............... {}", verdict(r3b));
    pass &= r3b;

    // #4: coinbase with a 101-byte scriptSig (bad-cb-length) now runs through CheckTransaction.
    let r4 = !block_all_ok(&synth_block(&cb_bad, &[&a, &b], &[false, true]));
    println!("#4 malformed coinbase (never CheckTransaction'd before) . {}", verdict(r4));
    pass &= r4;

    // #5: unbound fee-prevouts on a 2-input tx. Baseline (honest shared blob) must pass; the phantom
    // ~21M BTC coin in the first input's blob must be rejected.
    let honest2 = block_all_ok(&synth_unbound_prevouts(false));
    println!("[baseline] honest 2-input tx accepted ................... {}", if honest2 { "yes ✓" } else { "NO ✗ (baseline broken)" });
    pass &= honest2;
    let r5 = !block_all_ok(&synth_unbound_prevouts(true));
    println!("#5 unbound fee-prevouts (phantom coin -> inflation) ..... {}", verdict(r5));
    pass &= r5;

    println!("\n>>> ADVERSARIAL SUITE {}", if pass { "PASS ✓ — all holes closed" } else { "FAIL ✗ — a hole is OPEN" });
    if !pass { std::process::exit(1); }
}

// Execute one witness in mode-1 and return (all_ok, root_matches).
pub(crate) fn block_out(w: &BlockWitness) -> (bool, bool) {
    let mut b = ExecutorEnv::builder();
    b.write(&1u32).unwrap();
    b.write(w).unwrap();
    match default_executor().execute(b.build().unwrap(), METHOD_ELF) {
        Ok(s) => { let o: BlockOut = s.journal.decode().unwrap(); (o.all_ok, o._root_matches) }
        Err(_) => (false, false),
    }
}

// F3: the BIP30 grandfathered overwrite, tested on REAL block 91842 (coinbase-only, whose coinbase
// duplicates block 91812's still-unspent coinbase outpoint). The honest overwrite must ACCEPT with a
// matching root (superseded leaf deleted, new one added); skipping it, or claiming the wrong old height,
// must REJECT. Needs block_91842.json (fetch_block.py 91842).
pub(crate) fn check_bip30() {
    let (height, time, bits, nonce): (u32, u32, u32, u32) = (91842, 1_289_768_691, 453_931_606, 3_778_549_762);
    let header = build_header_v(1, PREV91842, &arr(rev(hx(MERKLE91842))), time, bits, nonce);
    let cb_hex = CB91842;
    let coinbase: Transaction = deserialize(&hx(cb_hex)).unwrap();
    let cb_txid = coinbase.compute_txid().to_byte_array();
    let old_height: u32 = 91812;          // block 91842 duplicates 91812's coinbase
    let (old_mtp, new_mtp) = (time, time); // test: seed and witness use the same value (a real run uses MTP(h-1))

    // root_prev = fillers + the SUPERSEDED coinbase outputs (this coinbase at old_height/old_mtp).
    let mut forest = Forest::new();
    for i in 0..4u64 { forest.add(hash_leaf(&[b"pre".as_slice(), &i.to_le_bytes()].concat())); }
    let mut superseded: Vec<Hash> = Vec::new();
    for v in 0..coinbase.output.len() {
        if !out_spendable(coinbase.output[v].script_pubkey.as_bytes()) { continue; }
        let l = out_leaf_of(&coinbase, &cb_txid, v, old_height, true, old_mtp);
        forest.add(l); superseded.push(l);
    }
    for i in 0..2u64 { forest.add(hash_leaf(&[b"post".as_slice(), &i.to_le_bytes()].concat())); }
    let root_prev = wire_stump(&forest);

    // overwrite: delete the superseded leaves (against root_prev), then add the NEW coinbase outputs.
    let mut dels: Vec<Bip30Del> = Vec::new();
    for l in &superseded {
        let pos = forest.find(l).expect("superseded coin present");
        let last = forest.leaves.len() - 1;
        dels.push(Bip30Del { global_pos: pos as u64, proof_i: wire_proof(&forest.prove(pos)), proof_last: wire_proof(&forest.prove(last)) });
        forest.delete(pos);
    }
    for v in 0..coinbase.output.len() {
        if !out_spendable(coinbase.output[v].script_pubkey.as_bytes()) { continue; }
        forest.add(out_leaf_of(&coinbase, &cb_txid, v, height, true, new_mtp));
    }
    let root_next = wire_stump(&forest);

    // AUDIT #3 F-1. This used to build a transition against a FRESH, EMPTY tree — where absence proves
    // trivially, which is the one state these blocks are never in. Their duplicated coinbase is
    // present and UNSPENT (that is why the utreexo overwrite below has leaves to delete at all), so
    // the fixture now models exactly that: seed the tree with the duplicate, then take the
    // grandfathered overwrite path. Driving the real block from an empty tree is what let the SMT's
    // reject-valid behaviour on 91842/91880 sit green through a whole test suite.
    let f3_smt = {
        let cb_out = cb_spendable_outputs(&coinbase);
        let mut t = Smt::new();
        t.insert(cb_txid, cb_out.max(1));
        let root_in = t.root();
        (root_in, smt_advance(&mut t, cb_txid, cb_out, &[], true).0)
    };
    let mk = |bip30: Option<Bip30Overwrite>| BlockWitness {
        header: header.clone(), height, coinbase_tx: hx(cb_hex), txids: PackedHashes(vec![cb_txid]),
        root_prev: root_prev.clone(), txs: vec![], tx_prevouts: vec![], inputs: vec![], root_next: root_next.clone(), bip30,
        in_smt_root: f3_smt.0, smt: f3_smt.1.clone(),
    };
    let honest = block_out(&mk(Some(Bip30Overwrite { old_height, old_mtp, dels: dels.clone() })));
    let skip = block_out(&mk(None));
    let wrong = block_out(&mk(Some(Bip30Overwrite { old_height: 91722, old_mtp, dels: dels.clone() }))); // wrong pair
    println!("=== F3 BIP30 grandfathered overwrite — REAL block {} (dup of 91812) ===", height);
    println!("[honest overwrite] accepted + root matches ... all_ok={} root_matches={}  (both true)", honest.0, honest.1);
    println!("[skip overwrite]   rejected (mandatory) ...... all_ok={}  (must be false)", skip.0);
    println!("[wrong old_height] rejected (delete misses) .. all_ok={}  (must be false)", wrong.0);
    let pass = honest.0 && honest.1 && !skip.0 && !wrong.0;
    println!(">>> F3 BIP30 OVERWRITE TEST {}", if pass { "PASS ✓" } else { "FAIL ✗" });
    if !pass { std::process::exit(1); }
}

// Regression test for the v0.9.0 bundle-parse bug. `PackedBytes` serialises via `serialize_bytes`, which
// serde_json emits as a JSON array (a *sequence*) since JSON has no native bytes type — so the deserialiser
// must accept a sequence, not only a bytes type. Any block with a non-empty `txs`/`tx_prevouts` (i.e. every
// block with a real spend; block 170 is the first) exercises this path. The whole adversarial+regression
// suite passed while this was broken because those prove from in-memory witnesses; ONLY the bridge →
// bundle_<n>.json → prove-range-bridge round-trip (this exact `to_vec` → `from_slice`) hit it. This test
// closes that coverage gap: it must PASS after the fix and would panic ("invalid type: sequence") before it.
pub(crate) fn bundle_roundtrip_test() {
    let mut fails = 0;
    // 1) PackedBytes alone, across cases that stress the JSON sequence path: empty, single, the full 0..=255
    //    range (bytes > 127 are where a signed/unsigned mixup would show), and a hand-picked mix.
    for case in [Vec::new(), vec![0u8], (0u8..=255).collect::<Vec<u8>>(), vec![255, 128, 0, 1, 127, 200]] {
        let j = serde_json::to_vec(&PackedBytes(case.clone())).expect("serialise PackedBytes");
        let back: PackedBytes = serde_json::from_slice(&j)
            .expect("PackedBytes JSON round-trip must parse (regression: v0.9.0 'invalid type: sequence')");
        if back.0 != case { println!(">>> BUNDLE-ROUNDTRIP: FAIL — PackedBytes {} bytes mismatch", case.len()); fails += 1; }
    }
    // 2) A full Bundle carrying a spend tx — the exact struct prove-range-bridge parses. Non-empty txs +
    //    tx_prevouts with bytes spanning the range, so this is the production shape that used to fail.
    let raw_tx: Vec<u8> = (0u8..=255).cycle().take(400).collect();
    let prevouts: Vec<u8> = (0u8..=255).rev().cycle().take(140).collect();
    let w = BlockWitness {
        header: vec![1, 2, 3], height: 170, coinbase_tx: vec![9, 9, 9],
        txids: PackedHashes(vec![[7u8; 32], [8u8; 32]]),
        root_prev: WireStump { roots: vec![], num_leaves: 0 },
        txs: vec![PackedBytes(raw_tx.clone())], tx_prevouts: vec![PackedBytes(prevouts.clone())],
        inputs: vec![],
        root_next: WireStump { roots: vec![Some([2u8; 32])], num_leaves: 1 }, bip30: None,
        // Round-trip test only — never executed, so a bare standalone transition is enough.
        in_smt_root: [0u8; 32], smt: smt_witness_standalone([1u8; 32], 1, &[]).1,
    };
    let b = Bundle {
        height: 170, in_tip: [1u8; 32], in_roots: vec![None, Some([2u8; 32])], in_leaves: 42,
        in_nbits: 0x1d00_ffff, in_time: 1_231_731_025, in_epoch_start: 1_231_006_505, in_recent: vec![1, 2, 3],
        witness: w,
    };
    let j = serde_json::to_vec(&b).expect("serialise Bundle");
    let back: Bundle = serde_json::from_slice(&j)
        .expect("Bundle JSON round-trip must parse (regression: the v0.9.0 spend-block parse bug)");
    if back.witness.txs.first().map(|p| &p.0) != Some(&raw_tx) {
        println!(">>> BUNDLE-ROUNDTRIP: FAIL — Bundle.witness.txs mismatch"); fails += 1;
    }
    if back.witness.tx_prevouts.first().map(|p| &p.0) != Some(&prevouts) {
        println!(">>> BUNDLE-ROUNDTRIP: FAIL — Bundle.witness.tx_prevouts mismatch"); fails += 1;
    }
    // 3) The packed HASH types. These were added by the 2026-08-28 wire-packing change with their own
    //    byte-visitor that omitted `visit_seq`, so every bundle carrying `txids` failed to parse -- the
    //    v0.9.0 bug, reintroduced. Parsing the Bundle above already exercises it, but assert the VALUES
    //    too: `PackedHashes` serialises as one flat n*32 blob, so a flat/nested or chunking mixup would
    //    still parse while yielding the wrong hashes.
    if back.witness.txids.0 != vec![[7u8; 32], [8u8; 32]] {
        println!(">>> BUNDLE-ROUNDTRIP: FAIL — Bundle.witness.txids mismatch"); fails += 1;
    }
    for case in [Vec::new(), vec![[3u8; 32]], (0u8..9).map(|i| [i; 32]).collect::<Vec<_>>()] {
        let j = serde_json::to_vec(&PackedHashes(case.clone())).expect("serialise PackedHashes");
        let back: PackedHashes = serde_json::from_slice(&j)
            .expect("PackedHashes JSON round-trip must parse (regression: missing visit_seq)");
        if back.0 != case { println!(">>> BUNDLE-ROUNDTRIP: FAIL — PackedHashes {} hashes", case.len()); fails += 1; }
    }
    // 4) The LEGACY nested shape, which the coordinator's 220,001 stored bundles are written in.
    //    Pre-2026-08-28, `Vec<[u8; 32]>` serialised as a list of 32-element lists; it is now one flat
    //    n*32 blob. The reader must accept both or every stored bundle becomes unreadable, which is
    //    74 GB of regeneration to arrive back at identical leaf values.
    for hashes in [vec![], vec![[7u8; 32]], vec![[1u8; 32], [255u8; 32], [0u8; 32]]] {
        let legacy = serde_json::to_vec(&hashes).expect("serialise legacy nested");
        let back: PackedHashes = serde_json::from_slice(&legacy)
            .expect("legacy NESTED PackedHashes must still parse (the coordinator's stored bundles)");
        if back.0 != hashes {
            println!(">>> BUNDLE-ROUNDTRIP: FAIL — legacy nested {} hashes", hashes.len()); fails += 1;
        }
        // and the flat shape must still agree on the same value
        let flat: PackedHashes = serde_json::from_slice(
            &serde_json::to_vec(&PackedHashes(hashes.clone())).expect("serialise flat")
        ).expect("flat PackedHashes must parse");
        if flat.0 != hashes {
            println!(">>> BUNDLE-ROUNDTRIP: FAIL — flat {} hashes", hashes.len()); fails += 1;
        }
    }
    // A mixture is not a real wire shape and must be refused rather than silently half-read.
    if serde_json::from_slice::<PackedHashes>(b"[1,2,[3,3,3]]").is_ok() {
        println!(">>> BUNDLE-ROUNDTRIP: FAIL — a mixed byte/hash sequence was accepted"); fails += 1;
    }
    {
        let case = PackedHash([0xABu8; 32]);
        let j = serde_json::to_vec(&case).expect("serialise PackedHash");
        let back: PackedHash = serde_json::from_slice(&j)
            .expect("PackedHash JSON round-trip must parse (regression: missing visit_seq)");
        if back != case { println!(">>> BUNDLE-ROUNDTRIP: FAIL — PackedHash mismatch"); fails += 1; }
    }
    if fails == 0 { println!(">>> BUNDLE-ROUNDTRIP: PASS — PackedBytes + PackedHash(es) + full spend-block Bundle JSON round-trip"); }
    else { std::process::exit(1); }
}

// Differential audit of the guest's script-flag schedule (script_flags::block_script_flags — the SAME
// module the guest compiles) against Core's canonical mainnet GetBlockScriptFlags. The guest applies
// P2SH|WITNESS|TAPROOT retroactively to genesis — stricter than Core's height-gated activation
// (P2SH@173805, segwit@481824, taproot@709632) — so the from-genesis prover need not special-case
// pre-activation blocks. That is SOUND because extra flags can only REJECT more, never accept a
// Core-invalid block, so a Hazync proof still implies Core-validity. This test proves that safety
// property (guest ⊇ Core at every boundary), that the buried soft-forks (DERSIG/CLTV/CSV/NULLDUMMY)
// flip at Core's EXACT heights, and that the two script_flag_exception blocks behave (BIP16 → no flags;
// taproot → TAPROOT cleared, base retained).
pub(crate) fn script_flags_test() {
    use script_flags::*;
    const P2SH_HEIGHT: u32 = 173_805; // BIP16Height (chainparams.cpp)
    const TAPROOT_HEIGHT: u32 = 709_632;
    // Core's GetBlockScriptFlags for a NON-exception block at height h (each flag active iff h >= its height).
    let core_flags = |h: u32| -> u32 {
        let mut f = 0u32;
        if h >= P2SH_HEIGHT { f |= P2SH; }
        if h >= BIP66_HEIGHT { f |= DERSIG; }
        if h >= BIP65_HEIGHT { f |= CLTV; }
        if h >= CSV_HEIGHT { f |= CSV; }
        if h >= SEGWIT_HEIGHT { f |= WITNESS | NULLDUMMY; }
        if h >= TAPROOT_HEIGHT { f |= TAPROOT; }
        f
    };
    let non_exc = [0u8; 32]; // any non-exception hash → the base schedule
    let (mut fails, mut checked) = (0u32, 0u32);
    let mut heights: Vec<u32> = vec![0, 1, 100_000, 227_931, 959_617];
    for h in [P2SH_HEIGHT, BIP66_HEIGHT, BIP65_HEIGHT, CSV_HEIGHT, SEGWIT_HEIGHT, TAPROOT_HEIGHT] {
        heights.push(h.saturating_sub(1)); heights.push(h); heights.push(h + 1);
    }
    for &h in &heights {
        let g = block_script_flags(h, &non_exc);
        let c = core_flags(h);
        checked += 1;
        // (a) monotonic strictness / soundness: the guest never CLEARS a flag Core sets.
        if c & !g != 0 { fails += 1; println!("  SOUNDNESS FAIL h={h}: core={c:#x} guest={g:#x} missing={:#x}", c & !g); }
        // (b) buried soft-forks EXACT: guest bit == Core bit at every boundary.
        for (bit, name) in [(DERSIG, "DERSIG"), (CLTV, "CLTV"), (CSV, "CSV"), (NULLDUMMY, "NULLDUMMY")] {
            if (g & bit) != (c & bit) { fails += 1; println!("  BURIED FAIL h={h} {name}: guest={:#x} core={:#x}", g & bit, c & bit); }
        }
        // (c) retroactive base (P2SH|WITNESS|TAPROOT) always on for non-exception blocks.
        if g & (P2SH | WITNESS | TAPROOT) != (P2SH | WITNESS | TAPROOT) { fails += 1; println!("  BASE FAIL h={h}: guest={g:#x}"); }
    }
    // Exception blocks — keyed by the guest-computed block hash (unforgeable).
    let bip16 = block_script_flags(BIP16_EXCEPTION_HEIGHT, &BIP16_EXCEPTION);
    if bip16 != 0 { fails += 1; println!("  BIP16 EXCEPTION FAIL: expected 0 (SCRIPT_VERIFY_NONE), got {bip16:#x}"); }
    let tap = block_script_flags(TAPROOT_EXCEPTION_HEIGHT, &TAPROOT_EXCEPTION);
    if tap & TAPROOT != 0 { fails += 1; println!("  TAPROOT EXCEPTION FAIL: TAPROOT must be OFF, got {tap:#x}"); }
    if tap & (P2SH | WITNESS) != (P2SH | WITNESS) { fails += 1; println!("  TAPROOT EXCEPTION FAIL: P2SH|WITNESS must be ON, got {tap:#x}"); }
    // Core's mapped exception value must be a subset of the guest's (guest ⊇ Core → still sound).
    if (0u32 & !bip16) != 0 || ((P2SH | WITNESS) & !tap) != 0 { fails += 1; println!("  EXCEPTION SUBSET FAIL"); }
    checked += 2;
    println!("script-flags differential: checked {checked} height/exception cases (guest ⊇ Core, buried forks exact)");
    if fails == 0 {
        println!(">>> SCRIPT-FLAGS TEST PASS ✓");
    } else {
        println!(">>> SCRIPT-FLAGS TEST FAIL — {fails} discrepancies");
        std::process::exit(1);
    }
}

/// `vb-stages` — hazync: cost each phase of the aggregate's `validate_block` by subtraction.
///
/// The aggregate is 3,636,355,430 cycles on block 962,000 — 3.8x a single chunk, 21% of the block's
/// total work, and SERIAL where chunks are parallel. Optimising it without knowing which phase it is
/// would be guessing, and this codebase has a habit of punishing that.
///
/// Runs the AGGREGATION path (scripts not re-verified), so the numbers describe mode 5 and not mode 1.
/// Execute mode: no GPU, no chunk receipts.
pub(crate) fn vb_stages_cmd() {
    let (_anchor, w) = build_full();
    let stages: &[(u32, &str)] = &[
        (0, "read witness + header/version"),
        (1, "+ per-tx output leaves (tx_out_leaves)"),
        (2, "+ created_at in-block-coin map"),
        (3, "+ input loop: binds & per-tx checks (NO utreexo delete)"),
        (4, "+ utreexo deletes"),
        (5, "+ utreexo adds + root compare"),
        (6, "+ merkle root"),
        (u32::MAX, "+ wtxids & witness commitment (FULL)"),
        // Inside the input loop, which is 73% of the total. Each `continue`s after one more call, so
        // the deltas isolate the per-input work that the phase ladder above could only report in bulk.
        (20, "  [loop] bare iteration (resolve tx/prevouts only)"),
        (21, "  [loop] + coin_leaf_only + bind digest"),
        (22, "  [loop] + per-TX check_tx & is_final_tx"),
        (23, "  [loop] + check_input_locks"),
        (24, "  [loop] + created_at lookup & in-block bookkeeping"),
        (3,  "  [loop] + utreexo proof build (NO delete)"),
        (4,  "  [loop] + utreexo delete"),
        // Not a phase: asserts the batched leaves and locks equal the per-input ones for EVERY input.
        // It costs more than the full run (it does both), which is the point — correctness, not speed.
        (30, "DIFFERENTIAL: batch vs per-input, every input"),
        (31, "DIFFERENTIAL: chunk-supplied leaves/seqs/wtxids vs recomputed"),
    ];
    println!("=== validate_block phase costs — block {} ===", w.height);
    println!("{:<52} {:>16} {:>16}", "phase", "cumulative", "this phase");
    let mut prev = 0u64;
    for (stage, label) in stages {
        let mut b = ExecutorEnv::builder();
        b.segment_limit_po2(seg_po2());
        b.write(&12u32).unwrap();
        b.write(&stage).unwrap();
        b.write(&w).unwrap();
        let s = default_executor().execute(b.build().unwrap(), METHOD_ELF).expect("execute mode 12");
        let c = s.cycles();
        let delta = c.saturating_sub(prev);
        println!("{label:<52} {c:>16} {delta:>16}");
        prev = c;
    }
    println!();
    println!("The largest 'this phase' is where the aggregate's time goes. Anything that is per-TX");
    println!("and pure is a parallelisation candidate; utreexo is sequential accumulator state.");
}

/// `segment-size` — how big is one segment on the wire?
///
/// Decides whether segment-level distribution is a LAN-only architecture or something ordinary nodes
/// could join. A worker proving a segment needs that segment; if it is hundreds of MB the network is
/// the bottleneck rather than the GPU, and the idea is a rack, not a network.
pub(crate) fn segment_size_cmd() {
    use risc0_zkvm::ExecutorImpl;
    let idx: usize = std::env::var("HAZYNC_CHUNK").ok().and_then(|s| s.parse().ok()).unwrap_or(9);
    let (_a, w) = build_full();
    let bounds = chunk_bounds(&w, nchunks_env());
    let (lo, hi) = *bounds.get(idx).expect("chunk index");
    let mut b = ExecutorEnv::builder();
    b.segment_limit_po2(seg_po2());
    b.write(&4u32).unwrap();
    b.write(&w.height).unwrap();
    b.write(&chunk_block_hash(&w.header)).unwrap();
    write_chunk_inputs(&mut b, &w, lo, hi);
    let session = ExecutorImpl::from_elf(b.build().unwrap(), METHOD_ELF).unwrap().run().unwrap();
    let n = session.segments.len();
    let mut sizes: Vec<usize> = Vec::new();
    for r in session.segments.iter() {
        let seg = r.resolve().expect("resolve");
        sizes.push(bincode::serialize(&seg).expect("serialize").len());
    }
    sizes.sort_unstable();
    let tot: usize = sizes.iter().sum();
    println!("=== segment wire size — block {} chunk {} at po2 {} ===", w.height, idx, seg_po2());
    println!("  segments          {n}");
    println!("  total             {:.1} MB", tot as f64 / 1e6);
    println!("  mean              {:.2} MB", tot as f64 / n as f64 / 1e6);
    println!("  min / median / max  {:.2} / {:.2} / {:.2} MB",
        sizes[0] as f64 / 1e6, sizes[n / 2] as f64 / 1e6, sizes[n - 1] as f64 / 1e6);
    println!();
    println!("  A whole block is ~16x this. To distribute segment proving, each worker must receive");
    println!("  its segment: that is the mean above per segment proved, against ~4.2 s of GPU work.");
    let mean_mb = tot as f64 / n as f64 / 1e6;
    // MB -> Mbit is x8, then divide by the seconds of work it buys. (An earlier version of this line
    // multiplied by a further 1000 and printed a figure 1000x too large.)
    println!("  Break-even at 4.2 s/segment: {:.2} Mbit/s keeps one worker saturated.",
        mean_mb * 8.0 / 4.2);
    println!("  So the constraint on distributing segments is not bandwidth. It is the prover's");
    println!("  working set per segment, which this does not measure.");
}

// How much of a chunk's wall clock is EXECUTION rather than PROVING?
//
// This decides whether distributing segment proving can work at all. Segments are proved
// independently — that is the whole idea — but they are PRODUCED by one sequential executor pass.
// That pass is a serial floor no number of workers can cross: with P proving seconds spread over N
// cards, a chunk cannot finish faster than E + P/N.
//
// Measured here on the CPU that runs the harness, which is the conservative direction: this laptop is
// slower than the prover box, so the execution share reported is an UPPER bound on the real one.
pub(crate) fn exec_time_cmd() {
    use risc0_zkvm::ExecutorImpl;
    use std::time::Instant;
    let idx: usize = std::env::var("HAZYNC_CHUNK").ok().and_then(|s| s.parse().ok()).unwrap_or(9);

    let t0 = Instant::now();
    let (_a, w) = build_full();
    let t_build = t0.elapsed().as_secs_f64();

    let bounds = chunk_bounds(&w, nchunks_env());
    let (lo, hi) = *bounds.get(idx).expect("chunk index");
    let mut b = ExecutorEnv::builder();
    b.segment_limit_po2(seg_po2());
    b.write(&4u32).unwrap();
    b.write(&w.height).unwrap();
    b.write(&chunk_block_hash(&w.header)).unwrap();
    write_chunk_inputs(&mut b, &w, lo, hi);
    let env = b.build().unwrap();

    let t1 = Instant::now();
    let session = ExecutorImpl::from_elf(env, METHOD_ELF).unwrap().run().unwrap();
    let t_exec = t1.elapsed().as_secs_f64();

    let n = session.segments.len();
    let po2 = seg_po2();
    let cyc_cap = (n as f64) * (1u64 << po2) as f64;

    // The one calibration point we have: chunk 9 of this block proved in 915 s on a B200
    // (948,436,992 cycles -> 1.036 M cycles/s). Anyone re-running on other hardware should
    // override it rather than trust the default.
    let prove_s: f64 = std::env::var("HAZYNC_PROVE_S").ok().and_then(|s| s.parse().ok()).unwrap_or(915.0);

    println!("=== execute vs prove — block {} chunk {} at po2 {} ===", w.height, idx, po2);
    println!("  witness build     {:.1} s", t_build);
    println!("  EXECUTION         {:.1} s   ({} segments, <= {:.0} M cycles)", t_exec, n, cyc_cap / 1e6);
    println!("  proving (B200)    {:.1} s   [override with HAZYNC_PROVE_S]", prove_s);
    println!();
    println!("  execution share of a 1-card chunk: {:.1}%", 100.0 * t_exec / (t_exec + prove_s));
    println!();
    println!("  Serial floor with segment proving spread over N cards (E + P/N):");
    for n_cards in [1usize, 4, 16, 30, 64, 1000] {
        let t = t_exec + prove_s / n_cards as f64;
        println!("    N={:<5} {:7.1} s   (speedup {:.1}x, ceiling {:.1}x)",
            n_cards, t, (t_exec + prove_s) / t, (t_exec + prove_s) / t_exec);
    }
    println!();
    println!("  The ceiling column is what execution alone caps the chunk at. If that number is");
    println!("  small, distributing segments cannot reach 10 minutes no matter how many cards join,");
    println!("  and the executor has to be parallelised or split before anything else matters.");
}

// Peak memory to prove ONE segment — the number that decides who can be a worker.
//
// §29 settled bandwidth for distributing segment proving (0.28 MB mean, 0.53 Mbit/s keeps a worker
// saturated) and §31 settled the serial floor (execution is 2.2%). The remaining unknown is the
// prover's WORKING SET per segment. It decides two different things:
//
//   CPU  — whether an ordinary Bitcoin Ghost node can take a segment. Nodes run in 3.87 GB.
//   CUDA — how many proves share one card, which is the #97 OOM in a different disguise.
//
// Reports VmHWM, the kernel's peak-RSS high-water mark. It is monotonic across the process, so the
// FIRST prove's increment is the real per-segment cost and later ones mostly reuse allocations; both
// are printed so that is visible rather than assumed. VRAM is not visible from inside the process —
// sample `nvidia-smi` alongside it.
pub(crate) fn segment_mem_cmd() {
    use risc0_zkvm::{ExecutorImpl, VerifierContext, get_prover_server};
    use std::time::Instant;

    fn kb(field: &str) -> f64 {
        std::fs::read_to_string("/proc/self/status").ok()
            .and_then(|s| s.lines().find(|l| l.starts_with(field))
                .and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse::<f64>().ok())))
            .unwrap_or(0.0) / 1e6  // kB -> GB
    }

    let idx: usize = std::env::var("HAZYNC_CHUNK").ok().and_then(|s| s.parse().ok()).unwrap_or(9);
    let nseg: usize = std::env::var("HAZYNC_NSEG").ok().and_then(|s| s.parse().ok()).unwrap_or(3);

    let (_a, w) = build_full();
    let bounds = chunk_bounds(&w, nchunks_env());
    let (lo, hi) = *bounds.get(idx).expect("chunk index");
    let mut b = ExecutorEnv::builder();
    b.segment_limit_po2(seg_po2());
    b.write(&4u32).unwrap();
    b.write(&w.height).unwrap();
    b.write(&chunk_block_hash(&w.header)).unwrap();
    write_chunk_inputs(&mut b, &w, lo, hi);

    let t_exec = Instant::now();
    let session = ExecutorImpl::from_elf(b.build().unwrap(), METHOD_ELF).unwrap().run().unwrap();
    let exec_s = t_exec.elapsed().as_secs_f64();
    let total = session.segments.len();

    let backend = std::env::var("RISC0_PROVER").unwrap_or_else(|_| "local".into());
    println!("=== per-segment prover working set — block {} chunk {} po2 {} ===", w.height, idx, seg_po2());
    println!("  backend {backend}   segments {total}   execution {exec_s:.1} s");
    println!("  after execute, before any prove:  VmHWM {:.2} GB   VmRSS {:.2} GB", kb("VmHWM:"), kb("VmRSS:"));
    println!();

    let opts = ProverOpts::default();
    let server = get_prover_server(&opts).expect("prover server");
    let ctx = VerifierContext::default();

    // Sample across the chunk rather than the first N: segment cost is not uniform, and the first
    // segment of a session is the least representative one.
    println!("  {:>4}  {:>8}  {:>10}  {:>10}  {:>10}", "seg", "prove s", "VmHWM GB", "VmRSS GB", "wire MB");
    let mut peak_delta = 0.0f64;
    let before_all = kb("VmHWM:");
    for k in 0..nseg.min(total) {
        let si = if nseg <= 1 { total / 2 } else { k * (total - 1) / (nseg - 1) };
        let seg = session.segments[si].resolve().expect("resolve");
        let wire = bincode::serialize(&seg).map(|v| v.len()).unwrap_or(0) as f64 / 1e6;
        let before = kb("VmHWM:");
        let t = Instant::now();
        let receipt = server.prove_segment(&ctx, &seg).expect("prove segment");
        let s = t.elapsed().as_secs_f64();
        receipt.verify_integrity_with_context(&ctx).expect("segment receipt integrity");
        let hwm = kb("VmHWM:");
        peak_delta = peak_delta.max(hwm - before);
        println!("  {:>4}  {:>8.1}  {:>10.2}  {:>10.2}  {:>10.2}", si, s, hwm, kb("VmRSS:"), wire);
    }

    // Lift and join are the price of a SMALL po2. Per-CYCLE segment proving throughput is flat
    // across po2 (147.6 / 145.9 / 147.8 us per cycle at 18 / 19 / 20) — ⚠ which is NOT the same as
    // total block time being flat, because the fold tree above it is not. So shrinking segments to fit a
    // node's RAM looks free -- until you count recursion. Every segment must be lifted to a succinct
    // receipt and then joined pairwise, and if lift cost is per-SEGMENT rather than per-CYCLE then
    // halving po2 doubles the recursion bill. That is the number that decides whether po2 18 holds up.
    if std::env::var("HAZYNC_LIFT").ok().as_deref() == Some("1") {
        // CONSECUTIVE segments, not one segment twice. join() checks continuity -- segment a's post
        // state must be segment b's pre state -- so join(x, x) fails an equality check on the state
        // digest rather than timing anything. A segment does not follow itself.
        let i = total / 2;
        let mut lifted = Vec::new();
        let mut lift_s = 0.0f64;
        for k in [i, i + 1] {
            let seg = session.segments[k].resolve().expect("resolve");
            let sr = prove_segment_resilient(&server, &ctx, &seg, &format!("segment {i}"));
            let t = Instant::now();
            lifted.push(server.lift(&sr).expect("lift"));
            lift_s = lift_s.max(t.elapsed().as_secs_f64());
        }
        println!();
        println!("  lift  {:.1} s  (per segment)", lift_s);
        let t = Instant::now();
        server.join(&lifted[0], &lifted[1]).expect("join");
        let join_s = t.elapsed().as_secs_f64();
        println!("  join  {:.1} s  (per PAIR, so ~1 per segment over a whole tree)", join_s);
        println!("  recursion for {} segments in this chunk: {:.0} s   ({:.0} s lift + {:.0} s join)",
            total, total as f64 * lift_s + (total - 1) as f64 * join_s,
            total as f64 * lift_s, (total - 1) as f64 * join_s);
        println!("  peak RSS after recursion: {:.2} GB", kb("VmHWM:"));
    }

    println!();
    println!("  peak RSS overall         {:.2} GB", kb("VmHWM:"));
    println!("  largest single-prove rise {:.2} GB  (first prove pays setup; later ones reuse)", peak_delta);
    println!("  rise across all proves    {:.2} GB", kb("VmHWM:") - before_all);
    println!();
    println!("  A Bitcoin Ghost node runs in 3.87 GB. The figure that has to fit under that is the");
    println!("  peak RSS of a worker that ONLY proves segments — it never executes, so it never holds");
    println!("  the witness. Subtract the pre-prove VmHWM above to get that.");
    println!("  VRAM is invisible from inside the process: sample nvidia-smi alongside for CUDA.");
}
