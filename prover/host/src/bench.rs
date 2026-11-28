//! Self-test commands on the engine: test-merkle, test-locks, core-vectors. (RISC Zero's measurement commands, exec-time,
//! chunk-profile, segment-mem, segment-size and core-vectors-export, went with CUTOVER C8 step 4; the CORE design's
//! check-*, adversarial, regress and bundle tests before them.)

use bitcoin::Transaction;
use bitcoin::consensus::deserialize;
use serde::Deserialize;
use crate::{hex, hx};
use crate::sink::{Sink, Stream};

/// One self-test run (modes 8, 9) on the engine: its journal decoded as `T`.
fn self_test<T: serde::de::DeserializeOwned>(name: &str, st: &Stream) -> T {
    let x = crate::engine_backend::execute_once(name, &st.0).unwrap_or_else(|e| panic!("{name}: {e}"));
    crate::engine_backend::decode_words(&x.journal.unwrap_or_else(|| panic!("{name}: exit {:?}, no journal", x.exit)))
}

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
        let mut b = Stream::default();
        b.write(&9u32);
        b.write(&flat);
        self_test("test_merkle", &b)
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
/// On the ENGINE (HAZYNC_ENGINE + HAZYNC_ENGINE_KEYS = the core image's keys): every execution runs on `hzengine serve`,
/// the same input words RISC Zero's executor took until CUTOVER C8, the guest's log read from the engine's log file
/// (the collect run's CVCASE / batch records); the comparison with Core's expectation is the same code. Work files
/// under HAZYNC_CV_WORK (default ./cv-work).
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
    crate::engine_backend::require();
    let work = std::env::var("HAZYNC_CV_WORK").unwrap_or_else(|_| "cv-work".into());
    println!("core-vectors on the ENGINE: {} with keys {}", std::env::var("HAZYNC_ENGINE").unwrap(), std::env::var("HAZYNC_ENGINE_KEYS").unwrap_or_default());
    // One guest execution of mode 10: (codes, cycles, the guest's log). The engine reports no cycles (0).
    let exec = |chunk: &[Case], mode: u32, hints: &[Vec<u8>]| -> Option<(Vec<i32>, u64, String)> {
        {
            let mut st = Stream::default();
            st.write(&10u32);
            st.write(&(chunk.len() as u32));
            for (k, c) in chunk.iter().enumerate() {
                let h = hints.get(k).cloned().unwrap_or_default();
                st.write(&(c.kind, unhex(&c.tx), unhex(&c.prevouts), c.input, c.flags, mode, h));
            }
            let x = crate::engine_backend::with(&work, |e| e.try_execute("cv", &st.0)).ok()?;
            let j = x.journal?;
            let w: Vec<u32> = j.chunks_exact(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
            let got: Vec<i32> = hazync_journal::serde::from_slice(&w).expect("mode 10 journal");
            assert_eq!(got.len(), chunk.len());
            Some((got, 0, x.log))
        }
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
    let mut b = Stream::default();
    b.write(&8u32);
    b.write(&raw);
    b.write(&input_idx); // input_idx
    b.write(&coin_h);
    b.write(&cb);
    b.write(&coin_mtp);
    b.write(&spend_h);
    b.write(&spend_mtp);
    let rc: i32 = self_test("test_locks", &b);
    let meaning = match rc { 1 => "VALID", -40 => "REJECT immature-coinbase", -41 => "REJECT height-lock-unmet", -42 => "REJECT time-lock-unmet", _ => "?" };
    // display the ACTUAL nSequence the check read (from the real tx bytes, not the synthetic var)
    let disp_seq = deserialize::<Transaction>(&raw).map(|t| t.input[input_idx as usize].sequence.0).unwrap_or(seq);
    println!("LOCKS rc={} ({})  [nSequence={:#010x} coin_mtp={} spend_mtp={} coin_h={} spend_h={} cb={}]",
        rc, meaning, disp_seq, coin_mtp, spend_mtp, coin_h, spend_h, cb);
}
