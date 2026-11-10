//! Measurement and self-test commands: exec-time, chunk-profile, segment-mem, segment-size, test-merkle,
//! test-locks, core-vectors. (The CORE design's check-*, adversarial, regress and bundle tests were removed with it.)

use bitcoin::Transaction;
use bitcoin::consensus::deserialize;
use methods::METHOD_ELF;
use risc0_zkvm::{ExecutorEnv, ProverOpts, default_executor};
use serde::Deserialize;
use crate::{BlockWitness, build_full, hex, hx, prove_segment_resilient, seg_po2};
use crate::chunks::{FIT_NAMES, chunk_block_hash, chunk_bounds, exec_chunk_cycles, input_costs,
    input_features, input_keys, nchunks_env, pack_chunks, predicted_ec_ops, predicted_sig_ops,
    repeat_savings, write_chunk_inputs};

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
///   host core-vectors <cases.json> [cases per execution, default 500] [--sig honest|forged]
/// --sig runs the BATCHED signature path, as a chunk does: a collect run learns each case's deferred verifies, then the
/// case runs on hints and the batch check. honest (R = u1*G + u2*Q): must still agree with Core exactly. forged (every
/// verify claimed VALID with an R that passes the per-signature check): no case Core rejects may pass; a case Core
/// accepts may fail (a lying host only costs liveness), counted apart.
pub(crate) fn core_vectors_cmd(args: &[String]) {
    #[derive(Deserialize)]
    struct Case { kind: u32, tx: String, prevouts: String, input: u32, flags: u32, label: String,
                  valid: Option<bool>, code: Option<i32> }
    let i = args.iter().position(|a| a == "core-vectors").unwrap();
    let path = args.get(i + 1).expect("usage: core-vectors <cases.json> [batch] [--sig honest|forged]");
    let batch: usize = args.get(i + 2).filter(|x| !x.starts_with("--")).map(|x| x.parse().expect("batch")).unwrap_or(500);
    // None: libsecp. Some(None): honest hints. Some(Some(v)): forged hints, variant v (libzync::host::sig_hint_forged):
    // 0 every verify claimed valid with the wanted parity, 1 the opposite parity, 2 x = r + n (ECDSA), 3 mixed (forge
    // every other verify, honest otherwise: the batch must still fail as a whole).
    let sig: Option<Option<u8>> = args.iter().position(|a| a == "--sig").map(|k| match args.get(k + 1).map(|s| s.as_str()) {
        Some("honest") => None, Some("forged") => Some(0), Some("forged-odd") => Some(1), Some("forged-rn") => Some(2),
        Some("forged-mixed") => Some(3), x => panic!("--sig honest|forged|forged-odd|forged-rn|forged-mixed, got {x:?}") });
    let cases: Vec<Case> = serde_json::from_slice(&std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}")))
        .expect("cases JSON");
    let unhex = |h: &str| -> Vec<u8> { (0..h.len()).step_by(2).map(|j| u8::from_str_radix(&h[j..j + 2], 16).unwrap()).collect() };
    // A guest abort: Core's EvalScript catches a thrown exception (script number over 4 bytes or non-minimal, the only
    // throws in script code) and fails the script with UNKNOWN_ERROR; the guest has no unwinding, so the throw aborts
    // it. Both reject: agreement only where Core rejects (script_tests: UNKNOWN_ERROR; tx tests: invalid).
    const ABORT: i32 = i32::MIN;
    const UNKNOWN_ERROR: i32 = -2;
    // The guest image, decoded once: each execution from the ELF re-decoded it, ~7 s a run, which was the whole cost of
    // a batch rerun case by case.
    let image = risc0_binfmt::ProgramBinary::decode(METHOD_ELF).and_then(|b| b.to_image()).expect("guest image");
    // One guest execution of mode 10: (codes, cycles, the guest's log).
    let exec = |chunk: &[Case], mode: u32, hints: &[Vec<u8>]| -> Option<(Vec<i32>, u64, String)> {
        let mut log = Vec::new();
        let s = {
            let mut b = ExecutorEnv::builder();
            b.stdout(&mut log);
            b.write(&10u32).unwrap();
            b.write(&(chunk.len() as u32)).unwrap();
            for (k, c) in chunk.iter().enumerate() {
                let h = hints.get(k).cloned().unwrap_or_default();
                b.write(&(c.kind, unhex(&c.tx), unhex(&c.prevouts), c.input, c.flags, mode, h)).unwrap();
            }
            risc0_zkvm::ExecutorImpl::new(b.build().unwrap(), image.clone()).ok()?.run().ok()?
        };
        let got: Vec<i32> = s.journal.as_ref().expect("mode 10 journal").decode().expect("mode 10 journal");
        assert_eq!(got.len(), chunk.len());
        Some((got, s.user_cycles, String::from_utf8_lossy(&log).into_owned()))
    };
    // A chunk through the selected path: plain, or collect -> hints -> hinted run.
    let run = |chunk: &[Case]| -> Option<(Vec<i32>, u64)> {
        let Some(forged) = sig else { return exec(chunk, 0, &[]).map(|(g, c, _)| (g, c)) };
        let (_, c0, log) = exec(chunk, 1, &[])?;
        let mut per: Vec<String> = vec![String::new(); chunk.len()];
        let mut cur: Option<usize> = None;
        for l in log.lines() {
            if let Some(at) = l.find("CVCASE ") { cur = l[at + 7..].trim().parse().ok(); continue; }
            if let (Some(k), true) = (cur, l.contains(libzync::wire::BATCHREC_TAG)) { per[k].push_str(l); per[k].push('\n'); }
        }
        let hints: Vec<Vec<u8>> = per.iter().map(|t| libzync::host::sig_records(t).iter().enumerate()
            .flat_map(|(j, r)| match forged {
                None => libzync::host::sig_hint(r),
                Some(3) if j % 2 == 1 => libzync::host::sig_hint(r),
                Some(3) => libzync::host::sig_hint_forged(r, 0),
                Some(v) => libzync::host::sig_hint_forged(r, v),
            }).collect()).collect();
        let (g, c1, _) = exec(chunk, 2, &hints)?;
        Some((g, c0 + c1))
    };
    let (mut agree, mut bad, mut aborts, mut broken, mut cycles) = (0usize, 0usize, 0usize, 0usize, 0u64);
    // A batch that aborts is split in halves until each abort is alone, so one throwing case does not hide the others
    // (rerunning it case by case cost a run per case, and nearly every batch of Core's vectors holds a throw).
    fn solve<C>(chunk: &[C], run: &dyn Fn(&[C]) -> Option<(Vec<i32>, u64)>, cycles: &mut u64, abort: i32) -> Vec<i32> {
        match run(chunk) {
            Some((g, c)) => { *cycles += c; g }
            None if chunk.len() == 1 => vec![abort],
            None => {
                let (l, r) = chunk.split_at(chunk.len() / 2);
                let mut g = solve(l, run, cycles, abort);
                g.extend(solve(r, run, cycles, abort));
                g
            }
        }
    }
    for chunk in cases.chunks(batch) {
        let got: Vec<i32> = solve(chunk, &run, &mut cycles, ABORT);
        for (c, rc) in chunk.iter().zip(got) {
            let core_valid = match (c.code, c.valid) { (Some(code), _) => code == 1, (None, Some(v)) => v,
                                                       (None, None) => panic!("{}: no expectation", c.label) };
            let ok = if matches!(sig, Some(Some(_))) {
                // forged hints: soundness only. Nothing Core rejects may pass.
                if core_valid && rc != 1 { broken += 1; }
                core_valid || rc != 1
            } else {
                match (rc, c.code, c.valid) {
                    // honest hints must never make the batch fail, whatever Core says of the case
                    (-2000, _, _) => false,
                    (ABORT, Some(code), _) => code == UNKNOWN_ERROR,
                    (ABORT, None, v) => v == Some(false),
                    (_, Some(code), _) => rc == code,
                    (_, None, Some(true)) => rc == 1,
                    (_, None, _) => rc != 1 && rc != -1000,
                }
            };
            if ok && rc == ABORT { aborts += 1 }
            if ok { agree += 1 } else {
                bad += 1;
                println!("MISMATCH {}: guest {}, want {}", c.label, if rc == ABORT { "ABORT".into() } else { rc.to_string() },
                         c.code.map(|x| x.to_string()).unwrap_or_else(|| if c.valid == Some(true) { "valid".into() } else { "invalid".into() }));
            }
        }
        eprintln!("  {} / {} cases, {} mismatches", agree + bad, cases.len(), bad);
    }
    let mode = match sig { None => "libsecp".to_string(), Some(None) => "batched, honest hints".to_string(),
                           Some(Some(v)) => format!("batched, FORGED hints (variant {v})") };
    println!("CORE_VECTORS [{mode}] {agree} agree ({aborts} of them by a guest abort where Core catches a throw), {bad} differ, of {} ({} cycles){}",
             cases.len(), cycles, if matches!(sig, Some(Some(_))) { format!("; {broken} Core-valid cases failed under forged hints (liveness only)") } else { String::new() });
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
