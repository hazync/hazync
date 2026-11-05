//! Distributed segment proving: seg-serve / seg-connect / seg-coordinate(-tree) / seg-work / seg-join /
//! seg-distribute / seg-prove-one, the framed wire with its hello and tags, reconnects, timeouts and
//! the join-tree planning.

use methods::{METHOD_ELF, METHOD_ID};
use risc0_zkvm::{ExecutorEnv, ProverOpts};
use crate::{build_full, hex, prove_segment_resilient, read_bridge_bundle, seg_po2, write_range_env};
use crate::chunks::{chunk_block_hash, chunk_bounds, nchunks_env, read_chunk_receipts, write_chunk_inputs};
use crate::lz::{execute_chunks_parallel, hint_threads, libzync_agg_prep, libzync_agg_tail,
    write_aggregate_env};

// ── where seg-serve listens (hazync#365, and the multi-unit ergonomics behind it) ─────────────
//
// ⛔ ONE function, used by BOTH the log line and the bind. They used to be two independent
// env::var calls that agreed only by coincidence: change one and the process prints an address it is
// not listening on, which is the worst possible lie for someone debugging a worker that will not
// attach.
//
// Loopback stays the default. This wire is UNAUTHENTICATED -- seg-connect opens a bare TcpStream and
// HAZYNC_WORKER_ID is a log label, not an identity -- so anyone who can reach the port can take work
// and stall the run. Binding 0.0.0.0 must stay a deliberate act.
//
// HAZYNC_SEG_REMOTE=1 is that deliberate act, spelled as intent rather than as a socket detail: it
// says "workers are on other machines", which is the thing an operator actually knows. HAZYNC_BIND
// still wins if set, for tunnels and private interfaces.
pub(crate) fn seg_bind_addr() -> String {
    if let Ok(b) = std::env::var("HAZYNC_BIND") {
        return b;
    }
    if std::env::var("HAZYNC_SEG_REMOTE").ok().as_deref() == Some("1") {
        return "0.0.0.0".into();
    }
    "127.0.0.1".into()
}

/// True when nothing widened the bind, so workers on other machines cannot attach.
pub(crate) fn seg_bind_is_loopback() -> bool {
    seg_bind_addr() == "127.0.0.1"
}

// ── the push transport's job deadline (hazync#365) ───────────────────────────────────────────
//
// A worker that accepts pushed work and then goes QUIET — without disconnecting — used to stall a run
// for ever. Two failures were already handled: a bad receipt is requeued explicitly, and a dropped
// connection returns what it owed. A peer that simply stops answering is neither, so nothing reclaimed
// its jobs and the join tree waited on them until the run was killed by hand. This is not
// malice-specific: a box that hard-freezes or loses power without the TCP connection tearing down
// produces the identical symptom.
//
// ⛔ THE DEADLINE IS MEASURED DISPATCH-TO-RETURN, NOT ON COMPUTE. Measured on the first mode-6 run
// (docs/history/BENCH_MODE6_3xRTX4090_2026-09-17.md): a join costs 0.38 s of GPU (p50, n=2,352) and
// 46.7 s of round trip. A window fitted to compute would be ~7 s and would kill every healthy join.
// The coordinator cannot see compute anyway; round trip is what "this worker has stopped answering"
// actually means, and it is already recorded for the rtt_ms telemetry.
//
// ⛔ AND IT IS PER-WORKER, because one global number cannot serve a fleet. On that same run, two
// HEALTHY workers differed 7.4x at p90:
//
//     pod 1 (same host)   n=1594  p50 45.9s  p90  50.3s  max  57.2s
//     pod 3               n=438   p50 63.0s  p90 102.9s  max 128.9s
//     pod 2 (tunnelled)   n=321   p50 79.6s  p90 374.1s  max 393.6s
//
// A window tight enough to reclaim pod 1's work quickly would have dropped pod 2 repeatedly during a
// run that completed perfectly. Replaying that run's 2,353 round trips against candidate windows:
// 600 s would have killed NOTHING, 300 s would have killed 59 healthy jobs (2.51%), 120 s would have
// killed 111 (4.72%). Hence the floor.
//
// The adaptive term only ever WIDENS the window: a slow-link worker earns more time, a fast one is
// never given less than the floor. The ceiling stops a worker that has been slow all run from growing
// its own deadline without bound, which is how a genuinely hung peer would otherwise hide.
//
// ⚠ THE FLOOR IS COUPLED TO hazync#367, NOT A PHYSICAL CONSTANT. Today `seg-connect` has no reconnect:
// it exits on link close, so a false positive costs that worker for the REST OF THE RUN. That is why
// the floor is conservative. Once a worker rejoins by itself, a false positive costs one requeue and a
// few seconds, and 300 s becomes reasonable. Retune here, not by guessing.
//
// 600 s is deliberately the SAME floor the worker CLI already uses for "too long without hearing
// anything" (HAZYNC_STALL_MIN). Reusing that judgement beats inventing a second one.
pub(crate) fn job_timeout_floor_s() -> f64 {
    std::env::var("HAZYNC_JOB_TIMEOUT_FLOOR").ok().and_then(|s| s.parse().ok()).unwrap_or(600.0)
}
pub(crate) fn job_timeout_mult() -> f64 {
    std::env::var("HAZYNC_JOB_TIMEOUT_MULT").ok().and_then(|s| s.parse().ok()).unwrap_or(8.0)
}
pub(crate) fn job_timeout_ceiling_s() -> f64 {
    std::env::var("HAZYNC_JOB_TIMEOUT_CEILING").ok().and_then(|s| s.parse().ok()).unwrap_or(1800.0)
}
// Samples needed before the adaptive term is trusted. Below this the floor governs: a window fitted to
// three observations is noise, and the first jobs of a run are the least representative (cold CUDA, a
// cold link, and the queue still filling).
pub(crate) fn job_timeout_min_samples() -> usize {
    std::env::var("HAZYNC_JOB_TIMEOUT_SAMPLES").ok().and_then(|s| s.parse().ok()).unwrap_or(16)
}

// hazync#367: how long to wait with NO worker connected while work is still outstanding.
//
// MEASURED 2026-09-17, on the run that validated #365. After the deadline dropped a quiet peer and
// requeued its 4 segments, the coordinator stayed alive and listening with nobody connected --
// serve=1, listening=1, the log frozen at the breach line -- through t+30 s, t+60 s and t+90 s, and
// was still holding the port when the run was abandoned.
//
// With an operator watching that is defensible: `seg-connect` has no reconnect, so a replacement
// worker is started by hand and the requeued work is there waiting for it. Unattended it is a silent
// hang, which is precisely what #367 warns about -- "an operator driving three shells notices a
// stall; a one-command run does not". #365 made a QUIET WORKER fail fast; a run that loses EVERY
// worker still waited for ever.
//
// 600 s is the same floor #365 uses and the same the worker CLI uses for HAZYNC_STALL_MIN. Reusing
// that judgement beats inventing a third one, and it is comfortably long enough to start a worker by
// hand after noticing.
pub(crate) fn no_peers_timeout_s() -> f64 {
    std::env::var("HAZYNC_NO_PEERS_TIMEOUT").ok().and_then(|s| s.parse().ok()).unwrap_or(600.0)
}

/// Whether the run should give up: nobody is connected, work is stranded, and it has been that way
/// for longer than the timeout.
///
/// Pure so it can be tested without a socket, exactly like `peer_job_timeout_s`. Each condition is
/// load-bearing and the middle one is the dangerous one to omit:
///
/// - `live_peers > 0` -- someone is proving. Never give up on a run that is making progress.
/// - `outstanding > 0` -- something is actually stranded. ⛔ A run whose segments are all back and
///   whose join tree is being assembled locally legitimately has NO peers; giving up there would
///   destroy a finished proof seconds before it is written out.
/// - `quiet_s > timeout_s` -- past the grace. A worker may be restarting, and a run that has just
///   lost its only peer has not yet failed.
pub(crate) fn no_peers_breached(live_peers: usize, outstanding: usize, quiet_s: f64, timeout_s: f64) -> bool {
    live_peers == 0 && outstanding > 0 && quiet_s > timeout_s
}

/// This peer's deadline, in seconds, from its own observed round trips.
///
/// Pure so it can be tested without a socket: `rtts` is that peer's completed round trips in seconds,
/// in any order. Returns `max(floor, mult * p90)` clamped to `ceiling`, or the floor while there are
/// fewer than `min_samples`.
pub(crate) fn peer_job_timeout_s(rtts: &[f64], floor: f64, mult: f64, ceiling: f64, min_samples: usize) -> f64 {
    if rtts.len() < min_samples {
        return floor;
    }
    let mut v: Vec<f64> = rtts.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    // p90 by index, matching how the run above was summarised.
    let p90 = v[((v.len() as f64) * 0.9) as usize % v.len()];
    (mult * p90).max(floor).min(ceiling)
}

// SEGMENT DISTRIBUTION, phase 0: prove a chunk with every segment routed through a wire.
//
// Block latency is chunk_work/N + aggregate and the aggregate does not divide, so measured near-tip
// numbers put the floor near 18 minutes at ANY card count. Distributing SEGMENTS is the only route
// under ten, because it moves parallelism below the level recursion charges for.
//
// This proves the decomposition is sound before any network exists. Each segment is serialised,
// deserialised, and proved from the deserialised copy; each receipt is serialised and deserialised
// again. If a segment or a receipt cannot survive that round trip, nothing distributed can work, and
// this is where it shows -- cheaply, on one machine, with no protocol to debug.
//
// Assembly is NOT reimplemented here. `assemble_from_segment_receipts` is the same code
// `prove_session` runs after its own loop, so the two paths cannot drift. The step that made this
// worth doing carefully is the journal/assumption merge into the LAST segment's claim: miss it and
// you get a receipt that fails its own verify with nothing pointing at why.
pub(crate) fn seg_distribute_cmd() {
    use risc0_zkvm::{ExecutorImpl, VerifierContext, Segment, SegmentReceipt};
    use risc0_zkvm::sha::Digestible;
    use std::time::Instant;

    let idx: usize = std::env::var("HAZYNC_CHUNK").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
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
    let session = ExecutorImpl::from_elf(b.build().unwrap(), METHOD_ELF).unwrap().run()
        .unwrap_or_else(|e| panic!("{}", crate::lz::guest_failure("guest execution", &e)));
    let exec_s = t_exec.elapsed().as_secs_f64();
    let total = session.segments.len();

    println!("=== segment-distributed chunk prove — block {} chunk {} po2 {} ===", w.height, idx, seg_po2());
    println!("  execution {exec_s:.1} s   segments {total}");

    let opts = ProverOpts::succinct();
    let server = risc0_zkvm::get_prover_server(&opts).expect("prover server");
    let ctx = VerifierContext::default();

    let mut receipts: Vec<SegmentReceipt> = Vec::with_capacity(total);
    let (mut wire_out, mut wire_back) = (0usize, 0usize);
    let t_prove = Instant::now();
    for (i, sref) in session.segments.iter().enumerate() {
        let seg = sref.resolve().expect("resolve");

        // OUT: coordinator -> worker.
        let bytes = bincode::serialize(&seg).expect("serialize segment");
        wire_out += bytes.len();
        let seg_wire: Segment = bincode::deserialize(&bytes).expect("deserialize segment");

        // The worker's entire job. Nothing here needs the session, the ELF, or the witness.
        let sr = prove_segment_resilient(&server, &ctx, &seg_wire, &format!("segment {i}"));

        // BACK: worker -> coordinator. Verified on arrival, because a worker is untrusted: a receipt
        // is self-verifying, so a bad worker can only fail to produce one, never forge one.
        let rb = bincode::serialize(&sr).expect("serialize receipt");
        wire_back += rb.len();
        let sr_wire: SegmentReceipt = bincode::deserialize(&rb).expect("deserialize receipt");
        sr_wire.verify_integrity_with_context(&ctx).expect("returned receipt failed verify");

        receipts.push(sr_wire);
        if i % 50 == 0 || i + 1 == total {
            println!("    segment {}/{}  {:.0}s elapsed", i + 1, total, t_prove.elapsed().as_secs_f64());
        }
    }
    let prove_s = t_prove.elapsed().as_secs_f64();

    let t_asm = Instant::now();
    let info = server.assemble_from_segment_receipts(&ctx, &session, receipts)
        .expect("assemble from distributed receipts");
    let asm_s = t_asm.elapsed().as_secs_f64();

    info.receipt.verify(METHOD_ID).expect("DISTRIBUTED RECEIPT FAILED verify against METHOD_ID");

    println!();
    println!("  execution   {exec_s:8.1} s");
    println!("  proving     {prove_s:8.1} s   ({total} segments, each through a bincode round trip)");
    println!("  assembly    {asm_s:8.1} s   (lift + join + succinct, coordinator-side)");
    println!("  TOTAL       {:8.1} s", exec_s + prove_s + asm_s);
    println!();
    println!("  wire out    {:8.2} MB   ({:.3} MB/segment)", wire_out as f64/1e6, wire_out as f64/total as f64/1e6);
    println!("  wire back   {:8.2} MB   ({:.3} MB/segment)", wire_back as f64/1e6, wire_back as f64/total as f64/1e6);
    println!();
    if let Ok(out) = std::env::var("HAZYNC_SEG_OUT") {
        std::fs::write(&out, bincode::serialize(&info.receipt).expect("serialize")).expect("write receipt");
        println!("  saved {out}");
    }
    println!(">>> DISTRIBUTED RECEIPT VERIFIED against METHOD_ID.");
    println!("    journal {} bytes, digest {}", info.receipt.journal.bytes.len(), hex(info.receipt.journal.digest().as_bytes()));
    println!();
    println!("  Every segment crossed a wire and every receipt was verified on arrival. What is NOT");
    println!("  proved here: that this is FASTER. One machine proved them in sequence. The point is");
    println!("  that the work is now in units a worker can take, and the journal digest above is what");
    println!("  a monolithic prove of the same chunk must produce.");
}

// SEGMENT DISTRIBUTION, phase 1/2: a work directory that separate processes can share.
//
// The segment coordinator cannot hand off its Session -- it holds `Box<dyn SegmentRef>` and does not
// serialise -- and it does not need to. Assembly needs the session (journal, assumptions, claim), so
// the segment coordinator executes, publishes segments, and assembles. ONLY segment proving leaves the
// process, which is the 76% worth moving.
//
// Claiming is an O_EXCL create of claim_NNNN. That is atomic on a local filesystem and on NFS, needs
// no lock server, and a worker that dies simply leaves a claim that a sweeper can expire. Receipts
// are written to a .tmp and renamed, so the segment coordinator never sees a half-written file -- it polls
// for existence and a rename is atomic.
pub(crate) fn seg_workdir() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HAZYNC_WORKDIR").unwrap_or_else(|_| "/tmp/hazync-segwork".into()))
}

// Worker: claim segments from the directory, prove them, write receipts. Knows nothing about the
// block, the guest, or the session -- it needs only the segment in front of it.
pub(crate) fn seg_work_cmd() {
    use risc0_zkvm::{VerifierContext, Segment};
    use std::time::Instant;
    let dir = seg_workdir();
    let id = std::env::var("HAZYNC_WORKER_ID").unwrap_or_else(|_| "w0".into());
    let opts = ProverOpts::succinct();
    let server = risc0_zkvm::get_prover_server(&opts).expect("prover server");
    let ctx = VerifierContext::default();

    let count: usize = loop {
        if let Ok(s) = std::fs::read_to_string(dir.join("MANIFEST")) {
            if let Ok(n) = s.trim().parse() { break n; }
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    };

    let mut done = 0usize;
    let t0 = Instant::now();
    for i in 0..count {
        let claim = dir.join(format!("claim_{i:04}"));
        // create_new is the whole mutual exclusion: exactly one worker wins each segment.
        if std::fs::OpenOptions::new().write(true).create_new(true).open(&claim).is_err() { continue; }
        let seg_path = dir.join(format!("seg_{i:04}.bin"));
        let bytes = match std::fs::read(&seg_path) { Ok(b) => b, Err(e) => { eprintln!("[{id}] seg {i} unreadable: {e}"); continue; } };
        let seg: Segment = bincode::deserialize(&bytes).expect("deserialize segment");
        let t = Instant::now();
        let sr = prove_segment_resilient(&server, &ctx, &seg, &format!("segment {i}"));

        // STEP 2. Lift here rather than on the segment coordinator. Lifts are per-segment and wholly
        // independent, and they are the largest term that does not divide: 886.7 s of 3081 s on
        // CPU, 679.3 s of 1167 s on GPU. Every worker doing its own removes all of it at once.
        //
        // The LAST segment is the exception and has to stay behind. The session journal digest and
        // assumption set are merged into its claim before it is lifted, and a worker has neither the
        // session nor any way to get it. So the last worker returns an unlifted SegmentReceipt and
        // the segment coordinator finishes that one itself.
        // ⚠ THIS FLAG DOES NOT APPLY TO THE TIP FLEET, AND ITS 58% -> 2.1% IS ALREADY BANKED.
        // It is read here (seg-work) and in seg_coordinate_tree_cmd -- both MANUAL commands. Every
        // automated run uses the push path, seg_connect_cmd, where the lift is UNCONDITIONAL: every
        // segment but the last is lifted on the worker that proved it, gated only by NOLIFT_TAG.
        // Measured 2026-09-21 over 12 runs on block 741000: worker task kinds were
        // `segment:34, segment_nolift:1` in EVERY arm including the baseline, with and without this
        // variable set. Setting it changes nothing on a tip run.
        // ⇒ Do not read SEGDIST_TASKS.md's table as a lever still waiting to be pulled (hazync#252).
        let lift_here = std::env::var("HAZYNC_WORKER_LIFTS").ok().as_deref() == Some("1") && i + 1 < count;
        if lift_here {
            let lifted = server.lift(&sr).expect("lift");
            let tmp = dir.join(format!("lift_{i:04}.tmp"));
            std::fs::write(&tmp, bincode::serialize(&lifted).expect("serialize lift")).expect("write lift");
            std::fs::rename(&tmp, dir.join(format!("lift_{i:04}.bin"))).expect("rename lift");
        } else {
            let tmp = dir.join(format!("rcpt_{i:04}.tmp"));
            std::fs::write(&tmp, bincode::serialize(&sr).expect("serialize receipt")).expect("write receipt");
            std::fs::rename(&tmp, dir.join(format!("rcpt_{i:04}.bin"))).expect("rename receipt");
        }
        done += 1;
        println!("[{id}] segment {i} {} in {:.1}s", if lift_here {"proved+lifted"} else {"proved"}, t.elapsed().as_secs_f64());
    }
    println!("[{id}] DONE {done} segments in {:.1}s", t0.elapsed().as_secs_f64());
}

// Coordinator: execute, publish, wait, assemble, verify.
pub(crate) fn seg_coordinate_cmd() {
    use risc0_zkvm::{ExecutorImpl, VerifierContext, SegmentReceipt};
    use risc0_zkvm::sha::Digestible;
    use std::time::Instant;

    let dir = seg_workdir();
    std::fs::create_dir_all(&dir).expect("create workdir");
    for e in std::fs::read_dir(&dir).unwrap().flatten() { let _ = std::fs::remove_file(e.path()); }

    let idx: usize = std::env::var("HAZYNC_CHUNK").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
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
    let session = ExecutorImpl::from_elf(b.build().unwrap(), METHOD_ELF).unwrap().run()
        .unwrap_or_else(|e| panic!("{}", crate::lz::guest_failure("guest execution", &e)));
    let exec_s = t_exec.elapsed().as_secs_f64();
    let total = session.segments.len();

    let t_pub = Instant::now();
    let mut wire_out = 0usize;
    for (i, sref) in session.segments.iter().enumerate() {
        let seg = sref.resolve().expect("resolve");
        let bytes = bincode::serialize(&seg).expect("serialize segment");
        wire_out += bytes.len();
        std::fs::write(dir.join(format!("seg_{i:04}.bin")), &bytes).expect("write segment");
    }
    // MANIFEST last: it is the signal that every segment is on disk, so a worker that sees it can
    // trust any index below the count.
    std::fs::write(dir.join("MANIFEST"), format!("{total}\n")).expect("write manifest");
    let pub_s = t_pub.elapsed().as_secs_f64();

    println!("=== coordinator — block {} chunk {} po2 {} ===", w.height, idx, seg_po2());
    println!("  execution {exec_s:.1} s   published {total} segments in {pub_s:.1} s ({:.2} MB)", wire_out as f64/1e6);
    println!("  workdir {}", dir.display());
    println!("  waiting for {total} receipts...");

    let t_wait = Instant::now();
    let mut last = 0usize;
    loop {
        let have = (0..total).filter(|i| dir.join(format!("rcpt_{i:04}.bin")).exists()).count();
        if have != last { println!("    {have}/{total} receipts  {:.0}s", t_wait.elapsed().as_secs_f64()); last = have; }
        if have == total { break; }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    let wait_s = t_wait.elapsed().as_secs_f64();

    let ctx = VerifierContext::default();
    let mut receipts: Vec<SegmentReceipt> = Vec::with_capacity(total);
    let mut wire_back = 0usize;
    for i in 0..total {
        let bytes = std::fs::read(dir.join(format!("rcpt_{i:04}.bin"))).expect("read receipt");
        wire_back += bytes.len();
        let sr: SegmentReceipt = bincode::deserialize(&bytes).expect("deserialize receipt");
        // Workers are untrusted. A receipt is self-verifying, so this is the entire defence.
        sr.verify_integrity_with_context(&ctx).expect("worker receipt failed verify");
        receipts.push(sr);
    }

    let opts = ProverOpts::succinct();
    let server = risc0_zkvm::get_prover_server(&opts).expect("prover server");
    let t_asm = Instant::now();
    let info = server.assemble_from_segment_receipts(&ctx, &session, receipts).expect("assemble");
    let asm_s = t_asm.elapsed().as_secs_f64();
    info.receipt.verify(METHOD_ID).expect("DISTRIBUTED RECEIPT FAILED verify");

    println!();
    println!("  execution        {exec_s:8.1} s");
    println!("  publish          {pub_s:8.1} s");
    println!("  worker wall      {wait_s:8.1} s   <- this is what more workers shrink");
    println!("  assembly         {asm_s:8.1} s   <- coordinator-side, does not distribute yet");
    println!("  TOTAL            {:8.1} s", exec_s + pub_s + wait_s + asm_s);
    println!("  wire out/back    {:.2} / {:.2} MB", wire_out as f64/1e6, wire_back as f64/1e6);
    println!();
    println!(">>> DISTRIBUTED RECEIPT VERIFIED against METHOD_ID");
    println!("    journal digest {}", hex(info.receipt.journal.digest().as_bytes()));
}

// SEGMENT DISTRIBUTION step 3: run the join tree as distributed work items.
//
// The join tree is what caps a distributed prover. Segment proving divides across workers; assembly
// did not, because risc0's fold was strictly linear -- lift, join into an accumulator, lift, join --
// so every join depended on the one before it. With the fold rebalanced into a tree (see the
// vendored crate), the joins at a given level are independent of each other, and independent work
// is work a worker can take.
//
// Level l holds ceil(n_l / 2) joins over n_l receipts. Each is published, claimed with the same
// O_EXCL create the segment workers use, and collected before the next level starts. The barrier
// per level is unavoidable -- level l+1 consumes level l's output -- but the DEPTH is log2(N), so
// at 44 segments that is 6 barriers rather than 43 sequential steps.
//
// An odd receipt at the end of a level carries forward untouched. It keeps its position, so
// adjacency is preserved and the final claim is unchanged.
pub(crate) fn seg_join_cmd() {
    use risc0_zkvm::{SuccinctReceipt, ReceiptClaim};
    use std::time::Instant;
    let dir = seg_workdir();
    let id = std::env::var("HAZYNC_WORKER_ID").unwrap_or_else(|_| "j0".into());
    let opts = ProverOpts::succinct();
    let server = risc0_zkvm::get_prover_server(&opts).expect("prover server");
    let mut done = 0usize;
    let t0 = Instant::now();

    // Follow the segment coordinator level by level. JOINLEVEL names the level currently open for claiming;
    // JOINDONE appearing ends the run.
    let mut level = 0usize;
    loop {
        if dir.join("JOINDONE").exists() { break; }
        let lv = match std::fs::read_to_string(dir.join("JOINLEVEL")) {
            Ok(s) => s.trim().split(',').map(|x| x.to_string()).collect::<Vec<_>>(),
            Err(_) => { std::thread::sleep(std::time::Duration::from_millis(200)); continue; }
        };
        if lv.len() != 2 { std::thread::sleep(std::time::Duration::from_millis(200)); continue; }
        let (cur, npairs): (usize, usize) = (lv[0].parse().unwrap_or(0), lv[1].parse().unwrap_or(0));
        if cur < level { std::thread::sleep(std::time::Duration::from_millis(200)); continue; }
        level = cur;

        for p in 0..npairs {
            let claim = dir.join(format!("jclaim_{level}_{p:04}"));
            if std::fs::OpenOptions::new().write(true).create_new(true).open(&claim).is_err() { continue; }
            let a_path = dir.join(format!("jin_{level}_{:04}.bin", p * 2));
            let b_path = dir.join(format!("jin_{level}_{:04}.bin", p * 2 + 1));
            let (ab, bb) = match (std::fs::read(&a_path), std::fs::read(&b_path)) {
                (Ok(a), Ok(b)) => (a, b),
                _ => { eprintln!("[{id}] level {level} pair {p}: inputs missing"); continue; }
            };
            let a: SuccinctReceipt<ReceiptClaim> = bincode::deserialize(&ab).expect("deserialize a");
            let b: SuccinctReceipt<ReceiptClaim> = bincode::deserialize(&bb).expect("deserialize b");
            let t = Instant::now();
            // join asserts a.post == b.pre. A worker that returns a join of the wrong two receipts
            // cannot produce something that survives this, which is why untrusted joins are safe.
            let j = server.join(&a, &b).expect("join");
            let tmp = dir.join(format!("jout_{level}_{p:04}.tmp"));
            std::fs::write(&tmp, bincode::serialize(&j).expect("serialize join")).expect("write join");
            std::fs::rename(&tmp, dir.join(format!("jout_{level}_{p:04}.bin"))).expect("rename join");
            done += 1;
            println!("[{id}] level {level} pair {p} joined in {:.1}s", t.elapsed().as_secs_f64());
        }
        level += 1;
    }
    println!("[{id}] JOIN DONE {done} joins in {:.1}s", t0.elapsed().as_secs_f64());
}

// Segment coordinator for step 3: prove segments (workers), lift them, then drive the join tree as
// distributed levels, then assemble. HAZYNC_JOIN_DISTRIBUTED=1 selects this over the in-process
// assembly, so both paths stay reachable and comparable.
pub(crate) fn seg_coordinate_tree_cmd() {
    use risc0_zkvm::{ExecutorImpl, VerifierContext, SegmentReceipt, SuccinctReceipt, ReceiptClaim};
    use risc0_zkvm::sha::Digestible;
    use std::time::Instant;

    let dir = seg_workdir();
    std::fs::create_dir_all(&dir).expect("create workdir");
    for e in std::fs::read_dir(&dir).unwrap().flatten() { let _ = std::fs::remove_file(e.path()); }

    let idx: usize = std::env::var("HAZYNC_CHUNK").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
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
    let session = ExecutorImpl::from_elf(b.build().unwrap(), METHOD_ELF).unwrap().run()
        .unwrap_or_else(|e| panic!("{}", crate::lz::guest_failure("guest execution", &e)));
    let exec_s = t_exec.elapsed().as_secs_f64();
    let total = session.segments.len();

    for (i, sref) in session.segments.iter().enumerate() {
        let seg = sref.resolve().expect("resolve");
        std::fs::write(dir.join(format!("seg_{i:04}.bin")), bincode::serialize(&seg).expect("ser")).expect("write");
    }
    std::fs::write(dir.join("MANIFEST"), format!("{total}\n")).expect("manifest");
    println!("=== segment coordinator (distributed join tree) — block {} chunk {} po2 {} ===", w.height, idx, seg_po2());
    println!("  execution {exec_s:.1} s   {total} segments published");

    let worker_lifts = std::env::var("HAZYNC_WORKER_LIFTS").ok().as_deref() == Some("1");

    let t_wait = Instant::now();
    loop {
        // With worker lifts, segments 0..N-2 arrive as lift_ files and only the last as rcpt_.
        let have = (0..total).filter(|i| {
            if worker_lifts && i + 1 < total { dir.join(format!("lift_{i:04}.bin")).exists() }
            else { dir.join(format!("rcpt_{i:04}.bin")).exists() }
        }).count();
        if have == total { break; }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    let prove_s = t_wait.elapsed().as_secs_f64();

    let ctx = VerifierContext::default();
    let mut receipts: Vec<SegmentReceipt> = Vec::new();
    let first_rcpt = if worker_lifts { total - 1 } else { 0 };
    for i in first_rcpt..total {
        let sr: SegmentReceipt = bincode::deserialize(&std::fs::read(dir.join(format!("rcpt_{i:04}.bin"))).expect("read")).expect("de");
        sr.verify_integrity_with_context(&ctx).expect("worker receipt failed verify");
        receipts.push(sr);
    }

    let opts = ProverOpts::succinct();
    let server = risc0_zkvm::get_prover_server(&opts).expect("prover server");

    // With HAZYNC_WORKER_LIFTS the workers have already lifted every segment but the last, so all
    // that remains here is the last one -- merge the session output into its claim, then lift it.
    // prepare_lifts merges into the LAST element of what it is given, so handing it a one-element
    // vector does exactly that and nothing else.
    let t_lift = Instant::now();
    let lifted = if worker_lifts {
        let mut v: Vec<SuccinctReceipt<ReceiptClaim>> = Vec::with_capacity(total);
        for i in 0..total - 1 {
            let r: SuccinctReceipt<ReceiptClaim> =
                bincode::deserialize(&std::fs::read(dir.join(format!("lift_{i:04}.bin"))).expect("read lift")).expect("de lift");
            r.verify_integrity_with_context(&ctx).expect("worker lift failed verify");
            v.push(r);
        }
        let last = receipts.pop().expect("last segment receipt");
        let mut tail = server.prepare_lifts(&ctx, &session, vec![last]).expect("merge+lift last");
        v.append(&mut tail);
        v
    } else {
        server.prepare_lifts(&ctx, &session, receipts).expect("prepare lifts")
    };
    let lift_s = t_lift.elapsed().as_secs_f64();
    println!("  lifted {} receipts in {:.1} s{}", lifted.len(), lift_s,
        if worker_lifts { "  (all but the last done by workers)" } else { "" });

    // Drive the tree. Publish a level, wait for its joins, feed the outputs in as the next level.
    let t_join = Instant::now();
    let mut level_recs: Vec<SuccinctReceipt<ReceiptClaim>> = lifted;
    let mut level = 0usize;
    while level_recs.len() > 1 {
        for (i, r) in level_recs.iter().enumerate() {
            std::fs::write(dir.join(format!("jin_{level}_{i:04}.bin")), bincode::serialize(r).expect("ser")).expect("write");
        }
        let npairs = level_recs.len() / 2;                 // odd tail carries, not joined
        let odd = level_recs.len() % 2 == 1;
        std::fs::write(dir.join("JOINLEVEL"), format!("{level},{npairs}\n")).expect("level");
        println!("    level {level}: {} receipts -> {npairs} joins{}", level_recs.len(), if odd {" (+1 carried)"} else {""});
        loop {
            let have = (0..npairs).filter(|p| dir.join(format!("jout_{level}_{p:04}.bin")).exists()).count();
            if have == npairs { break; }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        let mut next: Vec<SuccinctReceipt<ReceiptClaim>> = Vec::with_capacity(npairs + 1);
        for p in 0..npairs {
            let r: SuccinctReceipt<ReceiptClaim> = bincode::deserialize(&std::fs::read(dir.join(format!("jout_{level}_{p:04}.bin"))).expect("read")).expect("de");
            r.verify_integrity_with_context(&ctx).expect("joined receipt failed verify");
            next.push(r);
        }
        if odd { next.push(level_recs.pop().expect("odd tail")); }
        level_recs = next;
        level += 1;
    }
    std::fs::write(dir.join("JOINDONE"), b"1").expect("done");
    let join_s = t_join.elapsed().as_secs_f64();

    let joined = level_recs.pop().expect("one receipt remains");
    let t_asm = Instant::now();
    let info = server.assemble_from_joined(&ctx, &session, joined).expect("assemble from joined");
    let asm_s = t_asm.elapsed().as_secs_f64();
    info.receipt.verify(METHOD_ID).expect("DISTRIBUTED-TREE RECEIPT FAILED verify");

    println!();
    println!("  execution      {exec_s:8.1} s");
    println!("  segment prove  {prove_s:8.1} s   (workers)");
    println!("  lift           {lift_s:8.1} s   (segment coordinator; step 2 moves this to workers)");
    println!("  join tree      {join_s:8.1} s   ({level} levels, distributed)");
    println!("  resolve+build  {asm_s:8.1} s");
    println!();
    println!(">>> DISTRIBUTED-TREE RECEIPT VERIFIED against METHOD_ID");
    println!("    digest {}", hex(info.receipt.journal.digest().as_bytes()));
}

// Prove exactly one segment, from a file, to a file. The whole job of a remote worker.
//
// It takes no session, no ELF, no METHOD_ID and no block -- a Segment carries everything the
// prover needs. That is why a worker on another machine, built by a different toolchain against a
// guest with a different image id, can prove segments for a session it knows nothing about and
// return receipts that verify there. It is also why workers can be untrusted: the receipt is
// self-verifying and the segment coordinator checks it on arrival.
pub(crate) fn seg_prove_one_cmd() {
    use risc0_zkvm::{VerifierContext, Segment};
    use std::time::Instant;
    let inp = std::env::var("HAZYNC_SEGFILE").expect("HAZYNC_SEGFILE");
    let out = std::env::var("HAZYNC_RCPTFILE").expect("HAZYNC_RCPTFILE");
    let opts = ProverOpts::succinct();
    let server = risc0_zkvm::get_prover_server(&opts).expect("prover server");
    let ctx = VerifierContext::default();
    let seg: Segment = bincode::deserialize(&std::fs::read(&inp).expect("read segment")).expect("deserialize segment");
    let t = Instant::now();
    let sr = prove_segment_resilient(&server, &ctx, &seg, &inp);
    sr.verify_integrity_with_context(&ctx).expect("own receipt failed verify");
    std::fs::write(&out, bincode::serialize(&sr).expect("serialize receipt")).expect("write receipt");
    println!("proved one segment in {:.1}s -> {out}", t.elapsed().as_secs_f64());
}

// PUSH TRANSPORT (hazync#151). The segment coordinator sends work; it does not wait to be asked.
//
// The pull worker cost three SSH connections per segment -- claim, fetch, return -- at roughly
// 150 ms of setup each, against a segment that proves in 470 ms on an L40S at po2 18. A second
// matched GPU therefore added NOTHING: box 2 took 63% of the segments and the run got no faster,
// because it spent more time on round trips than on proving. Bandwidth was never the constraint;
// a segment is 0.06 MB. Connection setup and latency were.
//
// Pushing removes all three. The segment coordinator already holds the work list, so there is nothing to
// claim; one connection stays open for the whole session; and the pipelining comes free from TCP
// rather than from threads in the worker. The segment coordinator writes segment N+1 into the socket while
// the worker is still proving N, so the worker's next read returns from a buffer that already
// filled. At depth 4 that is 240 KB in flight, which no socket notices.
//
// Frame: [u32 index][u32 len][bytes]. Index 0xFFFF_FFFF means no more work.
pub(crate) const SEG_EOF: u32 = 0xFFFF_FFFF;
// A join job rather than a segment. The index space is otherwise segment indices, so the top bit is
// free and one connection can carry both kinds of work without a second protocol or a second socket.
// Body is [u32 len_a][a][u32 len_b][b] -- the two lifted receipts to join.
pub(crate) const JOIN_TAG: u32 = 0x8000_0000;
// A resolve job (hazync#153). Bit 30, so it cannot collide with JOIN_TAG's bit 31 or with a segment
// index -- a chunk of 4,207 segments uses fourteen bits, and the aggregate is smaller still.
// Body is the same [len][a][len][b] pair as a join: the conditional receipt, then the assumption.
//
// Resolves are a CHAIN, not a tree: each consumes the previous conditional, so there is exactly one
// outstanding at a time and this wins by moving the work off the segment coordinator rather than by
// parallelising it. That is a smaller prize than the join tree and it is the whole of the remaining
// undivided term, which is why it is worth having anyway.
pub(crate) const RESOLVE_TAG: u32 = 0x4000_0000;
// PROVE BUT DO NOT LIFT (hazync#157). Bit 29, clear of JOIN_TAG's bit 31 and of any segment index.
//
// The last segment cannot be lifted by a worker: the session journal digest and the assumption set
// are merged into its CLAIM first, and a worker has neither. The push design therefore withheld it
// and proved it on the segment coordinator -- but the merge is the only step that needs the session.
// PROVING never did. The pull design already had this right: the worker proved it and returned the
// unlifted receipt.
//
// So the last segment goes out like any other, tagged, and the worker returns a SegmentReceipt
// instead of a SuccinctReceipt. The segment coordinator merges and lifts, which is ~3.4x less work than
// proving and lifting -- and it lands in the undivided term every card in the fleet waits on.
pub(crate) const NOLIFT_TAG: u32 = 0x2000_0000;
// LIFT THIS RECEIPT (hazync#161). Bit 28, clear of the three tags above and of any segment index.
//
// The last segment's claim must have the session journal digest and assumption set merged into it
// before it is lifted, and only the coordinator has the session. But the LIFT itself needs no
// session -- it is an ordinary recursion proof over a receipt. Bundling them in prepare_lifts is the
// only reason a segment coordinator owns a GPU at all, and it is why a CUDA build whose device has
// gone away aborts at the final step after a fully successful distributed run.
//
// So the coordinator merges (claim surgery, no prover) and sends the merged SegmentReceipt out as a
// lift job. Body is the bare receipt, not a pair. With this the coordinator holds no prover.
pub(crate) const LIFT_TAG: u32 = 0x1000_0000;

// I AM THIS CARD (hazync#570). Bit 27, clear of every tag above and of any segment index. Body is
// HAZYNC_WORKER_ID as UTF-8; the worker sends it as its first frame on connect AND on every
// reconnect, because the aggregate sees a reconnect as a brand-new connection.
//
// ⛔ WHY IT HAD TO BE ON THE WIRE. The aggregate's `[rtt]` line identified a worker by `peer`, which
// is the address `accept()` returned — i.e. the EGRESS IP. Several cards on one pod, and several pods
// behind one NAT, share it: up to eleven cards came back as one `peer` on the 2026-09-21 fleet. So
// "which card has the slow tail?" was unanswerable from the logs, which is #570, and #550 (group
// cards into banks by their measured tail) cannot be built on a label that is not per-card.
//
// The ephemeral source port does distinguish the CONNECTIONS, but nothing maps a port back to a card,
// and a reconnect silently changes it. Only the worker knows which card it is, so only the worker can
// say. Sending it costs one small frame per connection.
//
// ⚠ BOTH ENDS SHIP IN ONE BINARY. A fleet run launches the aggregate and every worker from the same
// pinned HOST_RELEASE, so the two sides always match. The aggregate is written to tolerate a worker
// that never says hello (it reports `card=?`), which covers an old worker against a new aggregate.
// The reverse — a new worker against an OLD aggregate — would have its hello read as a receipt, so do
// not mix releases within a run.
pub(crate) const HELLO_TAG: u32 = 0x0800_0000;

/// The card name in a hello body, or `None` if it carries nothing usable.
///
/// Pure so it can be tested without a socket, exactly like `peer_job_timeout_s`. This is the ONLY
/// value on the wire a worker chooses freely, and it lands in log lines that `tools/live` and the
/// tail analysis in #550 parse, so it is bounded and filtered to a character set that cannot forge a
/// field: no whitespace (which would split a key=value line), and nothing that is not
/// `[A-Za-z0-9._-]`. A worker that sends nothing usable stays anonymous rather than being trusted.
pub(crate) fn hello_card_name(body: &[u8]) -> Option<String> {
    let name: String = String::from_utf8_lossy(body)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
        .take(40)
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

#[cfg(test)]
mod hello_tests {
    use super::{hello_card_name, HELLO_TAG, JOIN_TAG, LIFT_TAG, NOLIFT_TAG, RESOLVE_TAG, SEG_EOF};

    #[test]
    fn a_normal_worker_id_survives_intact() {
        assert_eq!(hello_card_name(b"hz-smoke-11").as_deref(), Some("hz-smoke-11"));
        assert_eq!(hello_card_name(b"w0").as_deref(), Some("w0"));
        assert_eq!(hello_card_name(b"hz_board.5-gpu2").as_deref(), Some("hz_board.5-gpu2"));
    }

    #[test]
    fn nothing_usable_stays_anonymous() {
        // The aggregate prints card=? for each of these rather than inventing a name.
        assert_eq!(hello_card_name(b""), None);
        assert_eq!(hello_card_name(b"   \n\t "), None, "whitespace alone is not a name");
        assert_eq!(hello_card_name(b"!!!@@@"), None, "punctuation alone is not a name");
    }

    #[test]
    fn a_name_cannot_forge_another_field() {
        // ⛔ THE POINT OF FILTERING. The name is printed into a key=value line that tools parse. A
        // worker claiming to be `a peer=1.2.3.4 rtt_ms=0.1` must not be able to inject either the
        // separator or a second key, or one card could rewrite another's measurements in the log.
        let got = hello_card_name(b"a peer=9.9.9.9 rtt_ms=0.1").unwrap();
        assert!(!got.contains(' '), "no whitespace survives: {got:?}");
        assert!(!got.contains('='), "no separator survives: {got:?}");
        assert_eq!(got, "apeer9.9.9.9rtt_ms0.1");
        assert!(hello_card_name(b"x\ny").unwrap() == "xy", "a newline cannot start a fake line");
    }

    #[test]
    fn a_long_name_is_bounded() {
        let got = hello_card_name(&vec![b'a'; 10_000]).unwrap();
        assert_eq!(got.len(), 40, "a worker must not be able to print an unbounded line");
    }

    #[test]
    fn invalid_utf8_does_not_panic() {
        // from_utf8_lossy substitutes U+FFFD, which the filter then drops.
        assert_eq!(hello_card_name(&[0xff, 0xfe, 0x80]), None);
        assert_eq!(hello_card_name(&[b'h', 0xff, b'z']).as_deref(), Some("hz"));
    }

    #[test]
    fn the_hello_tag_collides_with_nothing() {
        // ⛔ A tag that overlapped a routing bit would send the hello down the job path, and one that
        // overlapped a segment index would shadow a real segment. Bit 27, nothing else near it.
        for other in [JOIN_TAG, RESOLVE_TAG, NOLIFT_TAG, LIFT_TAG] {
            assert_eq!(HELLO_TAG & other, 0, "HELLO_TAG overlaps {other:#x}");
        }
        assert_ne!(HELLO_TAG, SEG_EOF);
        assert_eq!(HELLO_TAG & (JOIN_TAG | RESOLVE_TAG | LIFT_TAG), 0,
                   "the reader's routing test must not match a hello");
        // And it is above any plausible segment index: a block would need 134M segments to reach it.
        assert!(HELLO_TAG > 100_000_000);
    }
}

pub(crate) fn pack_pair(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(8 + a.len() + b.len());
    v.extend_from_slice(&(a.len() as u32).to_le_bytes());
    v.extend_from_slice(a);
    v.extend_from_slice(&(b.len() as u32).to_le_bytes());
    v.extend_from_slice(b);
    v
}

pub(crate) fn unpack_pair(body: &[u8]) -> Option<(&[u8], &[u8])> {
    if body.len() < 8 { return None; }
    let la = u32::from_le_bytes(body[0..4].try_into().ok()?) as usize;
    if body.len() < 4 + la + 4 { return None; }
    let lb = u32::from_le_bytes(body[4 + la..8 + la].try_into().ok()?) as usize;
    if body.len() < 8 + la + lb { return None; }
    Some((&body[4..4 + la], &body[8 + la..8 + la + lb]))
}

#[cfg(test)]
mod participant_count_tests {
    //! Counting PARTICIPANTS: distinct cards, never connections (hazync#567 follow-up).
    //!
    //! ⛔ MEASURED, tip hour 4, 2026-09-29. The summary printed "81 card(s) at peak" for a fleet of
    //! **38**. The count was the peak of `live_peers`, which increments per CONNECTION: a worker is
    //! re-armed for every block, and a reconnect can overlap its predecessor's reader thread, which
    //! has not yet reached the decrement at the end. Roughly 2x inflation.
    //!
    //! ⛔ AND IT WAS NOT COSMETIC. It fed `exec_bound_cards`, so the crossover printed 133 cards for
    //! 969,118 where the honest figure is ~62 — an operator reading it would conclude the executor was
    //! nowhere near binding when the fleet was already 60% of the way there.
    //!
    //! The fix is possible only because of #570: a card now says its own name, so reconnects collapse.
    use std::collections::HashSet;

    /// What the fix does: distinct names, plus connections that never identified themselves.
    fn participants(events: &[Option<&str>]) -> (usize, usize) {
        let mut named: HashSet<&str> = HashSet::new();
        let mut unnamed = 0usize;
        for e in events {
            match e {
                Some(n) => { named.insert(n); }
                None => unnamed += 1,
            }
        }
        (named.len(), unnamed)
    }

    /// What it used to do: one per connection, whatever it was.
    fn connections(events: &[Option<&str>]) -> usize { events.len() }

    /// Hour 4's shape: 38 cards, each connecting once per block over 21 blocks.
    fn hour4() -> Vec<Option<&'static str>> {
        const NAMES: [&str; 38] = ["w1","w2","w3","w4","w5","w6","w7","w8","w9","w10","w11","w12",
            "w13","w14","w15","w16","w17","w18","w19","w20","w21","w22","w23","w24","w25","w26","w27",
            "w28","w29","w30","w31","w32","w33","w34","w35","w36","w37","w38"];
        let mut v = Vec::new();
        for _block in 0..21 { for n in NAMES { v.push(Some(n)); } }
        v
    }

    #[test]
    fn a_card_that_reconnects_is_still_one_card() {
        let (named, unnamed) = participants(&hour4());
        assert_eq!((named, unnamed), (38, 0), "798 connections from 38 cards is 38 cards");
        // ⛔ THE CONTROL: the old rule, on the same events.
        assert_eq!(connections(&hour4()), 798);
        assert!(connections(&hour4()) > named * 20,
                "the old rule inflates by the number of blocks — which is how 38 read as 81");
    }

    #[test]
    fn an_unidentified_connection_is_reported_separately_not_merged() {
        // A pre-#570 worker cannot be de-duplicated. Counting all of them as ONE card would understate
        // the fleet exactly as badly as counting each connection overstated it.
        let ev = vec![Some("w1"), Some("w1"), None, None, Some("w2")];
        assert_eq!(participants(&ev), (2, 2), "2 named, 2 unidentified — not 3, and not 5");
    }

    #[test]
    fn the_crossover_moves_with_the_honest_count() {
        // 969,118: segment phase 258.6 s, executor window 157.7 s.
        let p = super::BlockPhases { prologue_s: 0.0, exec_s: 157.7, segment_s: 258.6, assembly_s: 38.4 };
        let honest = p.exec_bound_cards(38).unwrap();
        let inflated = p.exec_bound_cards(81).unwrap();
        assert!((honest - 62.3).abs() < 1.0, "honest crossover ~62 cards, got {honest}");
        assert!((inflated - 132.8).abs() < 1.0, "the inflated count printed ~133, got {inflated}");
        assert!(inflated > honest * 2.0,
                "⛔ a 2x miscount doubles the crossover, which is the number an operator acts on");
    }

    #[test]
    fn degenerate_cases() {
        assert_eq!(participants(&[]), (0, 0));
        assert_eq!(participants(&[None]), (0, 1), "an unnamed connection is not a named card");
    }
}

#[cfg(test)]
mod hello_wire_tests {
    //! The hello over a REAL socket (hazync#570).
    //!
    //! ⛔ WHY A SOCKET AND NOT A UNIT TEST. #570 put a new frame on the wire between the aggregate and
    //! every worker, and `mod hello_tests` only checks the tag arithmetic and the name filter — neither
    //! touches `write_frame`/`read_frame`. The failure that would actually hurt is FRAMING DESYNC: if the
    //! hello's length prefix were wrong by a byte, every subsequent frame on that connection would
    //! misalign, and the symptom would be a worker returning garbage receipts rather than anything
    //! naming a handshake. A tip run would fail in a way that looks like a bad card.
    //!
    //! So these drive the real functions across a real TCP connection, in the real order: hello first,
    //! then work. Nothing here needs a prover or a GPU, which is the point — it is the part of #570 that
    //! could be tested before the release and was not.
    use super::{hello_card_name, read_frame, write_frame, HELLO_TAG, JOIN_TAG, LIFT_TAG,
                RESOLVE_TAG, SEG_EOF};
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};

    /// A connected pair on the loopback. Port 0 so concurrent tests cannot collide.
    ///
    /// ⛔ BOTH ENDS CARRY A READ TIMEOUT, and that is not decoration. A test here that misaligns the
    /// stream on purpose can make `read_frame` take a garbage length (0x77000000 in one case), try to
    /// allocate ~2 GB and then block in `read_exact` for ever. That hung the suite for 12 minutes at
    /// 0% CPU with no output — and a hanging test reports nothing, so it is worse than a failing one.
    /// With a timeout the same mistake fails in 5 seconds and says which read it was.
    fn pair() -> (TcpStream, TcpStream) {
        let l = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let addr = l.local_addr().expect("addr");
        let client = TcpStream::connect(addr).expect("connect");
        let (server, _) = l.accept().expect("accept");
        for s in [&client, &server] {
            s.set_read_timeout(Some(std::time::Duration::from_secs(5))).expect("read timeout");
        }
        (client, server)
    }

    #[test]
    fn a_hello_then_work_arrives_in_order_and_intact() {
        let (mut w, mut agg) = pair();
        // The worker's side, exactly as seg-connect does it: hello on connect, then it answers work.
        write_frame(&mut w, HELLO_TAG, b"hz-smoke-11").expect("hello");
        write_frame(&mut w, JOIN_TAG | 0x0003_0001, b"a-join-receipt").expect("join reply");

        let (t1, b1) = read_frame(&mut agg).expect("read hello");
        assert_eq!(t1, HELLO_TAG, "the first frame is the hello");
        assert_eq!(hello_card_name(&b1).as_deref(), Some("hz-smoke-11"));

        // ⛔ THE DESYNC CHECK. If the hello's framing were wrong this tag and body would be shredded.
        let (t2, b2) = read_frame(&mut agg).expect("read join");
        assert_eq!(t2, JOIN_TAG | 0x0003_0001, "the NEXT frame is undamaged: {t2:#x}");
        assert_eq!(b2, b"a-join-receipt", "and so is its body");
    }

    #[test]
    fn the_aggregates_routing_test_skips_a_hello() {
        // This is the branch the reader takes: HELLO_TAG must not match the job routing mask, or it
        // would fall through to the SEGMENT path and pop a receipt that was never sent (#161's shape).
        assert_eq!(HELLO_TAG & (JOIN_TAG | RESOLVE_TAG | LIFT_TAG), 0);
        assert_ne!(HELLO_TAG, SEG_EOF, "and it must not be mistaken for end-of-stream");
    }

    #[test]
    fn an_empty_hello_body_still_frames_correctly() {
        // A worker with HAZYNC_WORKER_ID unset to empty sends a zero-length body. It must not be read
        // as SEG_EOF (which carries no length at all) and must not eat the next frame's header.
        let (mut w, mut agg) = pair();
        write_frame(&mut w, HELLO_TAG, b"").expect("empty hello");
        write_frame(&mut w, 7u32, b"segment-0").expect("segment");
        let (t1, b1) = read_frame(&mut agg).expect("read");
        assert_eq!((t1, b1.len()), (HELLO_TAG, 0));
        assert!(hello_card_name(&b1).is_none(), "and it leaves the card anonymous");
        let (t2, b2) = read_frame(&mut agg).expect("read next");
        assert_eq!((t2, &b2[..]), (7u32, &b"segment-0"[..]), "the next frame survived");
    }

    #[test]
    fn a_long_name_crosses_the_wire_whole_and_is_bounded_on_arrival() {
        // The sender does not truncate; the RECEIVER bounds it. Both halves matter: a 10 KB name must
        // not desync the stream, and must not print 10 KB into the log either.
        let (mut w, mut agg) = pair();
        let long = vec![b'a'; 10_000];
        write_frame(&mut w, HELLO_TAG, &long).expect("long hello");
        write_frame(&mut w, 1u32, b"after").expect("after");
        let (t, b) = read_frame(&mut agg).expect("read");
        assert_eq!(t, HELLO_TAG);
        assert_eq!(b.len(), 10_000, "the wire carried it whole");
        assert_eq!(hello_card_name(&b).unwrap().len(), 40, "and the parser bounds it to 40");
        assert_eq!(read_frame(&mut agg).expect("after").0, 1u32, "stream still aligned");
    }

    #[test]
    fn a_worker_that_never_says_hello_is_not_broken_by_this() {
        // Backward compatibility, over the socket: a pre-#570 worker's first frame is a work reply.
        // The aggregate must read it as such, not wait for a handshake that never comes.
        let (mut w, mut agg) = pair();
        write_frame(&mut w, LIFT_TAG, b"lifted").expect("lift reply");
        let (t, b) = read_frame(&mut agg).expect("read");
        assert_eq!(t, LIFT_TAG, "an old worker's first frame routes normally");
        assert_eq!(b, b"lifted");
    }

    #[test]
    fn control_a_mis_framed_hello_really_does_desync_the_stream() {
        // ⛔⛔ THE CONTROL FOR THIS WHOLE MODULE. Every test above passes today, which on its own says
        // nothing: they would also pass if `read_frame` ignored lengths entirely. So break the framing
        // deliberately — declare 11 bytes and send 10 — and prove the next frame comes back WRONG.
        // If this test ever starts failing, the desync checks above have stopped being able to detect
        // the thing they exist for.
        let (mut w, mut agg) = pair();
        let mut buf = Vec::new();
        buf.extend_from_slice(&HELLO_TAG.to_le_bytes());
        buf.extend_from_slice(&(11u32).to_le_bytes());       // claims 11
        buf.extend_from_slice(b"hz-smoke-1");                // sends 10
        buf.extend_from_slice(&JOIN_TAG.to_le_bytes());      // the next frame's header
        buf.extend_from_slice(&(4u32).to_le_bytes());
        buf.extend_from_slice(b"work");
        w.write_all(&buf).expect("write");
        w.flush().expect("flush");

        let (t1, b1) = read_frame(&mut agg).expect("read hello");
        assert_eq!(t1, HELLO_TAG);
        // It swallowed the first byte of the NEXT frame's tag to reach its declared 11.
        assert_eq!(b1.len(), 11, "the over-read happened: 10 sent, 11 consumed");
        assert_eq!(b1[10], 0x00, "and the stolen byte is JOIN_TAG's low LE byte");
        // ⭐ AND THE NAME SURVIVES IT, which is worth knowing rather than asserting away: the stolen
        // byte is a NUL, and `hello_card_name`'s filter drops anything outside [A-Za-z0-9._-]. So the
        // sanitiser happens to make the NAME robust to this corruption — which is exactly why the name
        // is NOT a usable desync signal, and the following TAG is.
        assert_eq!(hello_card_name(&b1).as_deref(), Some("hz-smoke-1"));

        // ⛔⛔ DO NOT CALL read_frame AGAIN HERE. I did, and it hung the whole suite for 12 minutes at
        // 0% CPU. After the over-read the stream is misaligned, so read_frame took `00 00 80 04` as the
        // tag and then `00 00 00 77` as the LENGTH — 0x77000000, ~2 GB — and sat in read_exact waiting
        // for bytes that will never arrive, having first tried to allocate it. On an 8 GB box that is a
        // memory hazard as well as a hang, and a hanging test reports nothing at all.
        //
        // Read the next four bytes directly instead: deterministic, no allocation, and it asserts the
        // same thing more precisely.
        let mut next_tag = [0u8; 4];
        agg.set_read_timeout(Some(std::time::Duration::from_secs(5))).expect("timeout");
        std::io::Read::read_exact(&mut agg, &mut next_tag).expect("four more bytes");
        assert_ne!(u32::from_le_bytes(next_tag), JOIN_TAG,
                   "⛔ a mis-framed hello MUST shift the following tag — it came back intact as {:#x}, \
which would mean these tests cannot detect a framing bug at all",
                   u32::from_le_bytes(next_tag));
        // Precisely: the tag is shifted left by the one stolen byte.
        assert_eq!(next_tag, [0x00, 0x00, 0x80, 0x04],
                   "the stream is shifted by exactly one byte, as the over-read implies");
    }

    #[test]
    fn a_hello_split_across_writes_still_reassembles() {
        // ⚠ TCP is a STREAM: nothing guarantees one write_frame arrives as one read. read_frame uses
        // read_exact, so this must hold — but "must" is why it is tested rather than assumed.
        let (mut w, mut agg) = pair();
        let mut buf = Vec::new();
        buf.extend_from_slice(&HELLO_TAG.to_le_bytes());
        buf.extend_from_slice(&(11u32).to_le_bytes());
        buf.extend_from_slice(b"hz-smoke-11");
        let (head, tail) = buf.split_at(6);            // mid-length-prefix, the nastiest split
        w.write_all(head).expect("head");
        w.flush().expect("flush");
        std::thread::sleep(std::time::Duration::from_millis(50));
        w.write_all(tail).expect("tail");
        w.flush().expect("flush");
        let (t, b) = read_frame(&mut agg).expect("read");
        assert_eq!(t, HELLO_TAG);
        assert_eq!(hello_card_name(&b).as_deref(), Some("hz-smoke-11"));
    }
}

pub(crate) fn write_frame(s: &mut std::net::TcpStream, idx: u32, body: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    s.write_all(&idx.to_le_bytes())?;
    s.write_all(&(body.len() as u32).to_le_bytes())?;
    s.write_all(body)?;
    s.flush()
}

pub(crate) fn read_frame(s: &mut std::net::TcpStream) -> std::io::Result<(u32, Vec<u8>)> {
    use std::io::Read;
    let mut i = [0u8; 4];
    s.read_exact(&mut i)?;
    let idx = u32::from_le_bytes(i);
    if idx == SEG_EOF { return Ok((idx, Vec::new())); }
    let mut l = [0u8; 4];
    s.read_exact(&mut l)?;
    let mut body = vec![0u8; u32::from_le_bytes(l) as usize];
    s.read_exact(&mut body)?;
    Ok((idx, body))
}

// Worker: connect, then read-prove-write forever. It holds no work list, does no claiming and
// makes no decisions -- the segment coordinator drives. That also makes it simpler than the pull worker.
// ── seg-connect reconnect policy (hazync#367 follow-up) ──────────────────────────────────────
//
// WHY THIS EXISTS. `seg-connect` used to exit on link close: a worker that dropped was gone for the
// rest of the run and somebody restarted it by hand. That is defensible with an operator watching and
// a silent loss otherwise, and it is why the #365 deadline floor is 600 s rather than 300 -- a false
// positive cost that worker permanently, so the window had to be generous.
//
// ⛔ THE SERVER SIDE ALREADY MAKES THIS SAFE, which is the whole reason a bare retry is correct:
//   - a dropped connection returns what it owed, and those segments go back on the queue;
//   - work is tracked by TAG, not by position, so a returning worker takes fresh tasks;
//   - #365's deadline reclaims a peer that goes quiet.
// So a reconnecting worker cannot double-prove. It simply asks for more work.
//
// The policy is split out from the loop deliberately: `cuda` is a non-default feature and CI builds
// `host` with ExternalProver, so `get_prover_server()` -- called before the connect -- means no CI test
// can drive seg-connect end to end. These functions need neither a socket nor a GPU.
//
// Modelled on prove_segment_resilient one level down: bounded, retry ONLY what is known transient,
// and stay loud. A silent reconnect would hide a flapping link, and link quality is exactly what a
// cross-machine fleet needs to see.

/// Seconds to wait before reconnect attempt `n` (1-based): 1, 2, 4, 8, 16, capped at 30.
pub(crate) fn seg_reconnect_backoff_s(attempt: u32) -> u64 {
    let raw = 1u64 << attempt.saturating_sub(1).min(20);
    raw.min(30)
}

/// Whether to keep trying, given how long we have been failing.
///
/// ⛔ CAPPED ON ELAPSED TIME, NOT ATTEMPT COUNT. Backoff means attempts and wall-clock diverge
/// wildly, and what actually matters is "how long has this card been earning nothing". 600 s is the
/// same floor HAZYNC_STALL_MIN and the #365 deadline already use -- reusing that judgement beats
/// inventing a fourth one.
/// ⛔ SPLIT IN TWO SO THE DECISION CAN BE TESTED WITHOUT THE PROCESS ENVIRONMENT (hazync#473).
/// `should_retry` used to read the variable itself, so the only way to test it was for a test to
/// `set_var` -- and libtest runs tests as THREADS IN ONE PROCESS. Two tests mutating one global,
/// each reading it immediately after, is a data race: measured at 125 failures in 1000 runs with the
/// box under load and 0 in 300 on an idle one, which is precisely why it only ever appeared on a
/// busy CI runner and never locally.
pub(crate) fn seg_reconnect_retry_within(elapsed_s: f64, cap_s: f64) -> bool {
    cap_s > 0.0 && elapsed_s < cap_s
}

/// How an operator's value becomes a cap. Anything unparseable falls back to the default rather than
/// disabling reconnect: a typo in a deploy script must not silently turn the feature off.
pub(crate) fn seg_reconnect_cap_from(raw: Option<&str>) -> f64 {
    raw.and_then(|s| s.parse::<f64>().ok()).unwrap_or(600.0)
}

pub(crate) fn seg_reconnect_should_retry(elapsed_s: f64) -> bool {
    let cap = seg_reconnect_cap_from(std::env::var("HAZYNC_RECONNECT_MAX_S").ok().as_deref());
    seg_reconnect_retry_within(elapsed_s, cap)
}

/// A link failure is transient and worth reconnecting for. Anything else is not.
///
/// ⛔ DELIBERATELY NARROW. A deserialize failure, a prove failure or a malformed frame will fail
/// identically on a fresh connection, and retrying those burns a card to say the same thing three
/// times. Only the transport is retried.
pub(crate) fn seg_reconnect_is_link_failure(kind: &str) -> bool {
    matches!(kind, "link closed" | "send failed" | "connect")
}

/// Reconnect after a transport failure. `true` to resume the task loop, `false` to give up.
///
/// Resets `t_down` on success so the cap always measures the CURRENT outage, and resets
/// `last_send_ms` so the idle-latency signal (#252) is not polluted by the outage itself. `done` and
/// the run clock are deliberately NOT reset: they are per-run accounting, and a reconnect does not
/// start a new run.
pub(crate) fn seg_reconnect(id: &str, addr: &str, s: &mut std::net::TcpStream,
                 t_down: &mut std::time::Instant, attempt: &mut u32,
                 last_send_ms: &mut u128) -> bool {
    loop {
        if !seg_reconnect_should_retry(t_down.elapsed().as_secs_f64()) {
            println!("[reconnect] [{id}] giving up after {:.0}s down", t_down.elapsed().as_secs_f64());
            return false;
        }
        *attempt += 1;
        let wait = seg_reconnect_backoff_s(*attempt);
        println!("[reconnect] [{id}] attempt {attempt}, retrying {addr} in {wait}s");
        std::thread::sleep(std::time::Duration::from_secs(wait));
        match std::net::TcpStream::connect(addr) {
            Ok(ns) => {
                ns.set_nodelay(true).ok();
                *s = ns;
                *t_down = std::time::Instant::now();
                *attempt = 0;
                *last_send_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
                // #570: a reconnect is a NEW connection to the aggregate, with a new ephemeral port
                // and no memory of who we are, so the hello has to go again. Best-effort: if this
                // write fails the link is already gone and the next send will find that out.
                write_frame(s, HELLO_TAG, id.as_bytes()).ok();
                println!("[reconnect] [{id}] reconnected to {addr}");
                return true;
            }
            Err(e) => println!("[reconnect] [{id}] still down: {e}"),
        }
    }
}

#[cfg(test)]
mod seg_reconnect_tests {
    use super::{seg_reconnect_backoff_s, seg_reconnect_cap_from, seg_reconnect_is_link_failure,
                seg_reconnect_retry_within};

    /// Backoff must GROW and must STOP growing. Unbounded doubling means a worker that has been down
    /// nine minutes sleeps another eight before noticing the coordinator came back.
    #[test]
    fn backoff_doubles_then_caps_at_30s() {
        assert_eq!(seg_reconnect_backoff_s(1), 1);
        assert_eq!(seg_reconnect_backoff_s(2), 2);
        assert_eq!(seg_reconnect_backoff_s(3), 4);
        assert_eq!(seg_reconnect_backoff_s(5), 16);
        assert_eq!(seg_reconnect_backoff_s(6), 30, "must cap, not reach 32");
        assert_eq!(seg_reconnect_backoff_s(40), 30, "a large attempt must not overflow the shift");
        assert_eq!(seg_reconnect_backoff_s(0), 1, "attempt 0 must not underflow to a huge shift");
    }

    /// ⛔ THE NARROWNESS IS THE POINT. A deserialize or prove failure fails identically on a fresh
    /// connection; retrying it burns a card to say the same thing again. Only transport is retried.
    #[test]
    fn only_transport_failures_reconnect() {
        assert!(seg_reconnect_is_link_failure("link closed"));
        assert!(seg_reconnect_is_link_failure("send failed"));
        assert!(seg_reconnect_is_link_failure("connect"));
        assert!(!seg_reconnect_is_link_failure("deserialize segment"),
                "a malformed frame must not be retried");
        assert!(!seg_reconnect_is_link_failure("prove"), "a prove failure must not be retried");
        assert!(!seg_reconnect_is_link_failure(""), "an unknown kind must not be retried");
    }

    /// The cap is on elapsed time, because backoff makes attempt count and wall-clock diverge.
    ///
    /// ⛔ NOT ONE `set_var` IN THIS MODULE (hazync#473). These tests used to install the cap in the
    /// process environment, which libtest shares across every test THREAD; two of them writing it
    /// and reading it back raced, and the loser saw the other's value. The cap is an argument now,
    /// so the decision is tested directly and the parsing is tested separately.
    #[test]
    fn retry_until_the_elapsed_cap() {
        let cap = seg_reconnect_cap_from(None);
        assert_eq!(cap, 600.0, "an unset variable means the documented default");
        assert!(seg_reconnect_retry_within(0.0, cap));
        assert!(seg_reconnect_retry_within(599.0, cap));
        assert!(!seg_reconnect_retry_within(600.0, cap), "at the cap it stops");
        assert!(!seg_reconnect_retry_within(6000.0, cap));
    }

    /// An operator must be able to turn it off. 0 means "behave as before": never reconnect.
    #[test]
    fn zero_disables_reconnect_entirely() {
        assert!(!seg_reconnect_retry_within(0.0, seg_reconnect_cap_from(Some("0"))),
                "0 must mean never, not an immediate cap breach");
        let five = seg_reconnect_cap_from(Some("5"));
        assert!(seg_reconnect_retry_within(1.0, five));
        assert!(!seg_reconnect_retry_within(9.0, five));
    }

    /// ⚠ A TYPO MUST NOT SILENTLY DISABLE RECONNECT. `HAZYNC_RECONNECT_MAX_S=six hundred` in a deploy
    /// script should fall back to the default, not to 0 — off is a decision, not a parse error.
    #[test]
    fn an_unparseable_cap_falls_back_rather_than_disabling() {
        for raw in ["", "six hundred", "600s", "--"] {
            assert_eq!(seg_reconnect_cap_from(Some(raw)), 600.0, "{raw:?} must fall back");
        }
        assert_eq!(seg_reconnect_cap_from(Some("0")), 0.0, "an explicit 0 is still honoured");
        assert_eq!(seg_reconnect_cap_from(Some("12.5")), 12.5, "and a real value is parsed");
    }
}

pub(crate) fn seg_connect_cmd(addr: &str) {
    use risc0_zkvm::{VerifierContext, Segment, SegmentReceipt, SuccinctReceipt, ReceiptClaim};
    use std::time::Instant;
    let id = std::env::var("HAZYNC_WORKER_ID").unwrap_or_else(|_| "push1".into());
    let opts = ProverOpts::succinct();
    let server = risc0_zkvm::get_prover_server(&opts).expect("prover server");
    let ctx = VerifierContext::default();
    // The connection is the ONLY thing rebuilt on a reconnect. `server`, `ctx`, `opts` and `id` are
    // built above and never touched by the socket -- and the prover server costs seconds to create,
    // so re-entering here rather than re-running the command keeps a reconnect cheap.
    // ⛔ `t_down` is reset every time we are CONNECTED, so the cap measures the current outage, not
    // the age of the worker. Measuring since start meant a worker that ran happily for an hour and
    // then dropped was already past its budget and would refuse to reconnect -- a reconnect that
    // never reconnects, which is worse than none because it looks implemented.
    let mut t_down = Instant::now();
    let mut attempt: u32 = 0;
    let mut s = loop {
        match std::net::TcpStream::connect(addr) {
            Ok(s) => break s,
            Err(e) => {
                attempt += 1;
                if !seg_reconnect_is_link_failure("connect")
                    || !seg_reconnect_should_retry(t_down.elapsed().as_secs_f64()) {
                    panic!("connect {addr}: {e}");
                }
                let wait = seg_reconnect_backoff_s(attempt);
                println!("[reconnect] connect {addr} failed ({e}); attempt {attempt}, retrying in {wait}s");
                std::thread::sleep(std::time::Duration::from_secs(wait));
            }
        }
    };
    s.set_nodelay(true).ok();   // these are small frames; Nagle would add 40 ms for nothing
    // hazync#254: every line leads with epoch ms and every task gets one line. The old output printed
    // every 25th task and carried no absolute time, so a fleet's aggregate timing could not be
    // reconstructed from its workers (#235 had to wrap every worker's stdout in a timestamping shell
    // loop). wait_s is how long this worker sat idle waiting for work -- the latency signal #252 needs.
    // HAZYNC_SEG_QUIET=1 restores the old sparse, undated output.
    let quiet = std::env::var("HAZYNC_SEG_QUIET").is_ok();
    let now_ms = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let say = |line: String| if quiet { println!("{line}") } else { println!("{} {line}", now_ms()) };
    // #570: name ourselves before any work is exchanged, so every [rtt] line the aggregate prints for
    // this connection can be attributed to THIS card rather than to a shared egress IP.
    write_frame(&mut s, HELLO_TAG, id.as_bytes()).ok();
    say(format!("[{id}] connected to {addr}"));

    let (mut done, t0) = (0usize, Instant::now());
    let mut last_send_ms = now_ms();
    let task_line = |kind: &str, detail: String, recv_ms: u128, wait_ms: u128, compute_s: f64, bytes_in: usize, bytes_out: usize, done: usize| {
        say(format!("[{id}] task kind={kind} {detail} recv_ms={recv_ms} send_ms={} wait_s={:.3} compute_s={compute_s:.3} bytes_in={bytes_in} bytes_out={bytes_out} done={done}",
                 now_ms(), wait_ms as f64 / 1000.0));
    };
    // `'run` wraps the task loop so a dropped link reconnects and resumes instead of exiting. The
    // server has already requeued whatever this worker owed, so resuming means asking for fresh
    // work -- never re-proving something already returned.
    'run: loop {
        let (idx, body) = match read_frame(&mut s) {
            Ok(v) => v,
            Err(e) => {
                say(format!("[{id}] link closed: {e}"));
                // ⛔ NOT an error: SEG_EOF is how the coordinator says there is no more work, and it
                // arrives as a clean frame, not a link failure. Only transport failures land here.
                match seg_reconnect(&id, addr, &mut s, &mut t_down, &mut attempt, &mut last_send_ms) {
                    true => continue 'run,
                    false => break 'run,
                }
            }
        };
        let recv_ms = now_ms();
        let wait_ms = recv_ms.saturating_sub(last_send_ms);
        if idx == SEG_EOF { say(format!("[{id}] no more work")); break; }
        let t = Instant::now();

        // A lift job carries a single SegmentReceipt whose claim the coordinator has already merged
        // the session output into. Lifting needs no session, so it is ordinary work.
        if idx & LIFT_TAG != 0 {
            let sr: SegmentReceipt = bincode::deserialize(&body).expect("deserialize segment receipt");
            let lifted = server.lift(&sr).expect("lift");
            let out = bincode::serialize(&lifted).expect("serialize lift");
            let compute_s = t.elapsed().as_secs_f64();
            if let Err(e) = write_frame(&mut s, idx, &out) {
            say(format!("[{id}] send failed: {e}"));
            // This task's result is lost, and that is fine: the server requeued it the moment the
            // connection dropped. Reconnect and take fresh work rather than re-proving it here.
            if seg_reconnect(&id, addr, &mut s, &mut t_down, &mut attempt, &mut last_send_ms) { continue 'run; }
            break 'run;
        }
            done += 1;
            last_send_ms = now_ms();
            if quiet { say(format!("[{id}] lifted the merged last segment in {:.2}s", t.elapsed().as_secs_f64())); }
            else { task_line("lift", "idx=last".into(), recv_ms, wait_ms, compute_s, body.len(), out.len(), done); }
            continue;
        }

        // A resolve job carries a conditional receipt and one assumption to discharge against it.
        // Tested BEFORE the join branch: both bodies are pairs, and the only thing telling them
        // apart is which tag bit is set, so the order of these two tests is the discriminator.
        //
        // Like join, this is safe to hand to an untrusted worker. resolve() checks the assumption
        // against the conditional receipt's own assumption list, so a worker that returns a resolve
        // of something else produces a receipt that fails at the next step or at the final verify.
        if idx & RESOLVE_TAG != 0 {
            let (cb, ab) = unpack_pair(&body).expect("malformed resolve pair");
            let cond: SuccinctReceipt<ReceiptClaim> = bincode::deserialize(cb).expect("deserialize conditional");
            let assum: SuccinctReceipt<risc0_zkvm::Unknown> = bincode::deserialize(ab).expect("deserialize assumption");
            let r = server.resolve(&cond, &assum).expect("resolve");
            let out = bincode::serialize(&r).expect("serialize resolve");
            let compute_s = t.elapsed().as_secs_f64();
            if let Err(e) = write_frame(&mut s, idx, &out) {
            say(format!("[{id}] send failed: {e}"));
            // This task's result is lost, and that is fine: the server requeued it the moment the
            // connection dropped. Reconnect and take fresh work rather than re-proving it here.
            if seg_reconnect(&id, addr, &mut s, &mut t_down, &mut attempt, &mut last_send_ms) { continue 'run; }
            break 'run;
        }
            done += 1;
            last_send_ms = now_ms();
            if quiet { say(format!("[{id}] resolve {} in {:.2}s ({done} done)", idx & !RESOLVE_TAG, t.elapsed().as_secs_f64())); }
            else { task_line("resolve", format!("k={}", idx & !RESOLVE_TAG), recv_ms, wait_ms, compute_s, body.len(), out.len(), done); }
            continue;
        }

        // A join job carries two lifted receipts instead of a segment. Same connection, same loop.
        // join() asserts a.post == b.pre, so a worker cannot return a join of the wrong two receipts
        // and have it survive the next level -- untrusted joins are safe for the same reason
        // untrusted proving is.
        if idx & JOIN_TAG != 0 {
            let (ab, bb) = unpack_pair(&body).expect("malformed join pair");
            let a: SuccinctReceipt<ReceiptClaim> = bincode::deserialize(ab).expect("deserialize a");
            let b: SuccinctReceipt<ReceiptClaim> = bincode::deserialize(bb).expect("deserialize b");
            let j = server.join(&a, &b).expect("join");
            let out = bincode::serialize(&j).expect("serialize join");
            let compute_s = t.elapsed().as_secs_f64();
            if let Err(e) = write_frame(&mut s, idx, &out) {
            say(format!("[{id}] send failed: {e}"));
            // This task's result is lost, and that is fine: the server requeued it the moment the
            // connection dropped. Reconnect and take fresh work rather than re-proving it here.
            if seg_reconnect(&id, addr, &mut s, &mut t_down, &mut attempt, &mut last_send_ms) { continue 'run; }
            break 'run;
        }
            done += 1;
            last_send_ms = now_ms();
            if quiet {
                if done % 25 == 0 { say(format!("[{id}] join {} in {:.2}s ({done} done)", idx & !JOIN_TAG, t.elapsed().as_secs_f64())); }
            } else {
                // seg-serve packs a join as JOIN_TAG | (level << 16) | position
                let v = idx & !JOIN_TAG;
                task_line("join", format!("level={} pos={}", v >> 16, v & 0xFFFF), recv_ms, wait_ms, compute_s, body.len(), out.len(), done);
            }
            continue;
        }

        let seg: Segment = bincode::deserialize(&body).expect("deserialize segment");
        let sr = prove_segment_resilient(&server, &ctx, &seg, &format!("segment {}", idx & !NOLIFT_TAG));

        // NOLIFT: prove and stop. This is the LAST segment, and lifting it needs the session journal
        // digest and assumption set merged into its claim first -- which the segment coordinator has and a
        // worker does not. Proving never needed the session, so it is done here like any other
        // segment and only the merge stays central (hazync#157).
        let out = if idx & NOLIFT_TAG != 0 {
            say(format!("[{id}] segment {} proved, returned UNLIFTED (last)", idx & !NOLIFT_TAG));
            bincode::serialize(&sr).expect("serialize segment receipt")
        } else {
            let lifted = server.lift(&sr).expect("lift");
            bincode::serialize(&lifted).expect("serialize lift")
        };
        let compute_s = t.elapsed().as_secs_f64();
        if let Err(e) = write_frame(&mut s, idx, &out) {
            say(format!("[{id}] send failed: {e}"));
            // This task's result is lost, and that is fine: the server requeued it the moment the
            // connection dropped. Reconnect and take fresh work rather than re-proving it here.
            if seg_reconnect(&id, addr, &mut s, &mut t_down, &mut attempt, &mut last_send_ms) { continue 'run; }
            break 'run;
        }
        done += 1;
        last_send_ms = now_ms();
        if quiet {
            if done % 25 == 0 || done < 3 {
                say(format!("[{id}] segment {idx} in {:.2}s ({done} done, {:.1}s elapsed)", t.elapsed().as_secs_f64(), t0.elapsed().as_secs_f64()));
            }
        } else {
            let kind = if idx & NOLIFT_TAG != 0 { "segment_nolift" } else { "segment" };
            task_line(kind, format!("idx={}", idx & !NOLIFT_TAG), recv_ms, wait_ms, compute_s, body.len(), out.len(), done);
        }
    }
    say(format!("[{id}] PUSH DONE {done} segments in {:.1}s", t0.elapsed().as_secs_f64()));
}

// Coordinator with a push transport. Executes, then serves segments to whoever connects, keeping
// several in flight per worker so the network never becomes the worker's critical path.
//
// One thread per connection, sharing a work queue behind a mutex. Each thread writes up to
// PUSH_DEPTH segments before reading the first receipt back; because the worker proves serially,
// receipts return in the order they were sent, so a VecDeque is enough to match them up.
//
// If a worker dies its in-flight segments go back on the queue and another worker takes them. That
// is the same reassignment the pull design got from expiring a stale claim, without needing claims.
/// Width of every level of the join tree, given the leaf count -- `widths[0]` is the leaf count and
/// the last entry is always 1.
///
/// The tree's shape is a function of the leaf count ALONE, which is what lets a join be published
/// the moment its two children exist rather than when its whole level does. Level `l+1` position `p`
/// joins `(l, 2p)` and `(l, 2p+1)`; when a level is odd its LAST node carries forward untouched to
/// position `npairs` of the next level, keeping its position so adjacency holds.
///
/// Extracted so the equivalence test below exercises the shipped function rather than a copy of it.
/// hazync#252 lever 1: should the resolve chain run on the aggregate rather than the fleet?
///
/// ⭐ ON BY DEFAULT since 2026-09-21, on measurement. `HAZYNC_RESOLVE_LOCAL=0` restores pushing the
/// chain to workers.
///
/// The chain is SERIAL by construction -- each step consumes the previous step's conditional
/// receipt -- so distributing it buys no parallelism whatsoever and pays a full round trip per step,
/// carrying a receipt out and back. Measured live on block 741000 (12 runs, 3x A40):
///
///     resolve   rtt p50 902.2 ms   compute p50 431.0 ms   =>  transport 471.2 ms   52%
///
/// and confirmed structurally: with the lever on, `resolve` tasks disappear from the distributed
/// set entirely (`dist={join:34, lift:1}` against `{join:34, lift:1, resolve:3}` for the default).
///
/// ⚠ The saving is small in absolute terms -- ~1.4 s on this block -- but it is strictly positive
/// and has no counterweight: local costs compute alone, remote costs compute PLUS the trip. Unlike
/// lever 2 there is no serial-versus-parallel trade, because the chain was never parallel.
pub(crate) fn resolve_runs_locally(v: Option<&str>) -> bool {
    // Absent means ON. "0" is the only way off -- an unset variable must not read as disabled.
    v.map(|x| x != "0").unwrap_or(true)
}

/// hazync#252 lever 2: should this join level be proved here rather than handed to a worker?
///
/// Narrow levels only. A level of `npairs` joins can occupy at most `npairs` workers, so once the
/// tree is narrower than the fleet, distributing buys no parallelism -- and still pays the round
/// trip, which is 77.8% of a join (measured 2026-09-21: 1596.0 ms rtt, 354.5 ms compute).
///
/// ⛔ `max` is a ceiling on the level's WIDTH, so this is `<=`, never `>=`. Inverted, it would keep
/// the WIDE bottom levels local -- serialising the hundreds of joins that are the only part of the
/// tree wide enough to use the fleet, which is far slower than shipping every one of them out.
/// `max == 0` disables the lever entirely and is the default.
pub(crate) fn join_stays_local(npairs: usize, max: usize) -> bool {
    max > 0 && npairs <= max
}

pub(crate) fn join_tree_widths(n: usize) -> Vec<usize> {
    let mut widths = vec![n];
    let mut m = n;
    while m > 1 {
        let npairs = m / 2;
        let odd = m % 2 == 1;
        m = npairs + if odd { 1 } else { 0 };
        widths.push(m);
    }
    widths
}

/// One block's phases, with the OVERLAP made explicit (hazync#567).
///
/// ⛔ WHAT THIS EXISTS FOR. The summary used to print `TOTAL = execution + worker wall + assembly`,
/// and those windows OVERLAP: the executor publishes each segment as it is produced (one-segment lag),
/// so connected workers are already proving while it runs — the comment at the executor call says so
/// in as many words, "that overlap is the entire point of the change". The timers were started 100
/// lines apart in the same scope and both ran through it, so adding them counted execution twice.
///
/// It was not a cosmetic error. It inflated the reported cost of every tip block and produced a
/// "serial floor" that included a phase which is not serial, and the conclusion drawn from it —
/// that a 8,924-segment block needs ~102 cards — was wrong by a factor of nearly three.
///
/// 📏 The two tables in `docs/history/TIP_HOUR_3_2026-09-28.md` disagreed by exactly this amount, from
/// two independent measurement paths, and nobody noticed:
///
///     969,018   session wall  691.7 s   phase sum  885.6 s   excess 193.9 s   execution 205.8 s
///     969,019   session wall 1027.9 s   phase sum 1299.0 s   excess 271.1 s   execution 277.3 s
///
/// The excess is execution minus the prologue, on both blocks. That is the signature.
///
/// ⇒ **The serial floor is `prologue + assembly`, NOT `execution + assembly`.** Execution is hidden
/// behind the segment phase until the fleet is big enough to outrun the executor, and
/// `exec_bound_cards` says where that is.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BlockPhases {
    /// Setup before the first segment can be published: bind, allocate, spawn. Genuinely serial.
    prologue_s: f64,
    /// The executor's own window. ⚠ CONTAINED IN `segment_s` — never add it to anything.
    exec_s: f64,
    /// First segment published → last segment returned. Production and proving, overlapped.
    segment_s: f64,
    /// Last segment + join tree + resolves, on the aggregate alone. Genuinely serial.
    assembly_s: f64,
}

impl BlockPhases {
    /// Wall time for the block. The phases that do NOT overlap, so this is the honest total.
    #[cfg(test)]
    fn total_s(&self) -> f64 { self.prologue_s + self.segment_s + self.assembly_s }

    /// The part no number of cards can shrink.
    fn serial_floor_s(&self) -> f64 { self.prologue_s + self.assembly_s }

    /// What the old summary printed. Kept ONLY so a test can assert we never print it again.
    ///
    /// ⚠ All four fields, because the old `execution` column was itself `prologue + executor` — its
    /// timer started before the bind. Summing those columns therefore added the prologue once and the
    /// executor once, on top of a segment phase that already contained both.
    #[cfg(test)]
    fn naive_sum_s(&self) -> f64 {
        self.prologue_s + self.exec_s + self.segment_s + self.assembly_s
    }

    /// How much the naive sum over-counted: exactly the executor window, which ran inside the
    /// segment phase and was then added to it.
    #[cfg(test)]
    fn overcount_s(&self) -> f64 { self.naive_sum_s() - self.total_s() }

    /// Fleet size at which EXECUTION becomes the binding constraint rather than proving.
    ///
    /// Below this the executor is hidden and cards still buy time; at and above it the segment phase
    /// cannot go below `exec_s` however many cards join, so #567's executor work is the only lever
    /// left. `None` when there is nothing to divide.
    fn exec_bound_cards(&self, cards: usize) -> Option<f64> {
        if cards == 0 || self.exec_s <= 0.0 { return None; }
        Some(self.segment_s * cards as f64 / self.exec_s)
    }
}

#[cfg(test)]
mod block_phases_tests {
    use super::BlockPhases;

    // 969,019 as measured on tip hour 3: the block that produced the "~102 cards" claim.
    //
    // 📏 The two figures the run recorded, from independent paths:
    //   session wall              1027.9 s   (the coordinator's own clock)
    //   OLD reported execution     277.3 s   (the aggregate's `execution` line)
    //   OLD phase sum             1299.0 s   ⇒ excess over the wall = 271.1 s
    //
    // ⭐ The excess (271.1) is LESS than the reported execution (277.3) by 6.2 s, and that gap is
    // itself evidence: the old `execution` timer started ~6 s before the first segment could be
    // published, so it carried the prologue. Splitting them gives prologue 6.2 s and an executor
    // window of 271.1 s — and the executor window is then exactly the over-count, which is the
    // property `overcount_s` asserts.
    const OLD_REPORTED_EXEC: f64 = 277.3;
    const OBSERVED_EXCESS: f64 = 271.1;

    fn b969019() -> BlockPhases {
        let prologue_s = OLD_REPORTED_EXEC - OBSERVED_EXCESS;          // 6.2
        BlockPhases {
            prologue_s,
            exec_s: OBSERVED_EXCESS,                                   // the executor alone
            // What actually elapsed between the first segment going out and the last coming back.
            segment_s: 1027.9 - 161.9 - prologue_s,
            assembly_s: 161.9,
        }
    }

    #[test]
    fn the_total_matches_the_session_wall_not_the_old_sum() {
        let p = b969019();
        assert!((p.total_s() - 1027.9).abs() < 0.05,
                "total {} must be the session wall", p.total_s());
        // ⛔ THE BUG, REPRODUCED: the old arithmetic returns a number 271 s longer than the block took.
        // The old summary printed 1299.0 s for this block. Reproduce it exactly.
        assert!((p.naive_sum_s() - 1299.0).abs() < 0.05,
                "naive sum {} must reproduce the 1299.0 s the run actually printed", p.naive_sum_s());
        assert!(p.naive_sum_s() > p.total_s() + 200.0,
                "the old sum must be visibly wrong, or this test proves nothing");
        assert!((p.overcount_s() - p.exec_s).abs() < 0.001,
                "the over-count IS the executor window: {} vs {}", p.overcount_s(), p.exec_s);
    }

    #[test]
    fn the_serial_floor_excludes_execution() {
        let p = b969019();
        assert!((p.serial_floor_s() - 168.1).abs() < 0.01, "floor {}", p.serial_floor_s());
        // 27% of a 600 s gate, not the 73% that "execution + assembly" produced.
        assert!(p.serial_floor_s() / 600.0 < 0.30, "floor is {:.0}% of the gate",
                p.serial_floor_s() / 6.0);
        assert!(p.exec_s + p.assembly_s > 430.0,
                "and the old floor really was ~73% — the control for the line above");
    }

    #[test]
    fn cards_needed_is_far_below_the_hundred_the_old_floor_implied() {
        let p = b969019();
        // Parallel work in card-seconds, from the fleet that actually ran it.
        let card_s = p.segment_s * 19.0;
        let budget = 600.0 - p.serial_floor_s();
        let needed = (card_s / budget).ceil();
        assert_eq!(needed, 38.0, "needed {needed} cards");
        assert!(needed < 50.0, "the ~102-card conclusion came from the double-counted floor");
    }

    #[test]
    fn execution_becomes_binding_only_on_a_large_fleet() {
        let p = b969019();
        let n = p.exec_bound_cards(19).unwrap();
        // 859.8 s of segment phase on 19 cards vs a 271.1 s executor ⇒ ~60 cards.
        assert!((n - 60.3).abs() < 0.5, "crossover at {n} cards");
        // ⇒ 38 cards is still worker-bound, so the fleet lever works there. Past ~59 it does not.
        assert!(n > 38.0, "if the crossover were below 38, cards could not reach the gate at all");
    }

    #[test]
    fn a_fleet_that_outruns_the_executor_reports_no_further_gain() {
        // Segment phase already down to the executor's window: more cards cannot help.
        let p = BlockPhases { prologue_s: 6.2, exec_s: 271.1, segment_s: 271.1, assembly_s: 161.9 };
        assert!((p.exec_bound_cards(60).unwrap() - 60.0).abs() < 0.01);
        assert!(p.total_s() < 600.0, "and it does make the gate: {:.0}s", p.total_s());
    }

    #[test]
    fn degenerate_inputs_do_not_panic() {
        let z = BlockPhases { prologue_s: 0.0, exec_s: 0.0, segment_s: 0.0, assembly_s: 0.0 };
        assert_eq!(z.total_s(), 0.0);
        assert_eq!(z.overcount_s(), 0.0);
        assert!(z.exec_bound_cards(10).is_none(), "no executor window ⇒ no crossover to report");
        assert!(b969019().exec_bound_cards(0).is_none(), "no cards ⇒ nothing to divide");
    }
}

pub(crate) fn seg_serve_cmd() {
    use risc0_zkvm::{ExecutorImpl, VerifierContext, SegmentReceipt, SuccinctReceipt, ReceiptClaim};
    use risc0_zkvm::sha::Digestible;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    let port: u16 = std::env::var("HAZYNC_PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(9110);
    let depth: usize = std::env::var("HAZYNC_PUSH_DEPTH").ok().and_then(|s| s.parse().ok()).unwrap_or(4);
    let idx: usize = std::env::var("HAZYNC_CHUNK").ok().and_then(|s| s.parse().ok()).unwrap_or(0);

    // HAZYNC_AGG=1 serves the mode-5 AGGREGATE instead of a mode-4 chunk (hazync#153).
    //
    // This was the gap that stopped a whole BLOCK from distributing. Segments and joins already
    // divided, but only for a chunk: seg-serve wrote `4u32` unconditionally and never called
    // add_assumption, so the aggregate -- the thing that folds the chunk receipts into the block
    // proof -- could not be served at all. Everything downstream is unchanged, because an aggregate
    // session produces ordinary segments like any other.
    // HAZYNC_RANGE=<n> serves the mode-6 BRIDGE RANGE — the board's own per-block proof (#361).
    //
    // The same gap as #153, one mode along. `prove-range-bridge` builds its session from
    // bundle_<n>.json and proves it in ONE local process, so a board block got exactly one card
    // however large it was, while the fixture path (modes 4/5) could spread a block over 27. The two
    // produce different artifacts — the coordinator verifies a KIND_RANGE receipt — so the fast path
    // was unsubmittable and the submittable path was single-card. Serving mode 6 here closes that:
    // everything downstream is untouched, because a range session produces ordinary segments too.
    //
    // ⛔ The input stream is written by the SAME helper `prove-range-bridge` uses, and must be: the
    // guest reads a fixed order, so a divergence desynchronises its stream rather than failing loudly.
    let agg = std::env::var("HAZYNC_AGG").is_ok();
    let range_n: Option<u32> = std::env::var("HAZYNC_RANGE").ok().and_then(|s| s.parse().ok());
    // libzync HAZYNC_AGG_EARLY=1: start the aggregate BEFORE the chunk receipts exist. Its execution only
    // needs each chunk's claim (journal + image id), which this box gets by executing the chunks itself, in
    // parallel, in seconds. The receipts are needed only by the resolves at the very end, so they are read
    // there -- meanwhile the aggregate's segments are proved alongside the chunks (GPU_RESULTS.md step 1).
    let agg_early = agg && std::env::var("HAZYNC_AGG_EARLY").as_deref() == Ok("1");
    let mut agg_nchunks = 0usize;
    let mut b = ExecutorEnv::builder();
    // The only witness field the rest of this function needs. A bundle carries it too, so mode 6 never
    // builds a BlockWitness — there is no HAZYNC_BLOCK fixture for a board block, which is the whole
    // reason `build_full()` cannot serve this path.
    let height: u32;
    if let Some(n) = range_n {
        let bd = read_bridge_bundle(n);
        height = bd.height;
        println!("=== segment coordinator (push): RANGE [{n}..{n}] from bridge bundle, po2 {} ===",
                 seg_po2());
        write_range_env(&mut b, &bd);
    } else {
        let (anchor, mut w) = build_full();
        height = w.height;
        if agg {
            let bounds = chunk_bounds(&w, nchunks_env());
            agg_nchunks = bounds.len();
            let receipts: Vec<risc0_zkvm::Receipt> = if agg_early {
                let t = std::time::Instant::now();
                let r: Vec<_> = execute_chunks_parallel(&w, &bounds, hint_threads()).into_iter().map(|(r, _)| r).collect();
                println!("  AGG_EARLY: {} chunk claims from local execution in {:.1} s; receipts are awaited at resolve",
                         r.len(), t.elapsed().as_secs_f64());
                r
            } else { read_chunk_receipts(bounds.len()) };
            println!("=== segment coordinator (push): AGGREGATE of {} chunk receipts, block {} po2 {} ===",
                     receipts.len(), w.height, seg_po2());
            // libzync: the same aggregate stream as agg-chunks, or the guest desyncs.
            libzync_agg_prep(&mut w);
            write_aggregate_env(&mut b, &receipts, &anchor, &w);
            libzync_agg_tail(&mut b, &w);
        } else {
            let bounds = chunk_bounds(&w, nchunks_env());
            let (lo, hi) = *bounds.get(idx).expect("chunk index");
            b.segment_limit_po2(seg_po2());
            b.write(&4u32).unwrap();
            b.write(&w.height).unwrap();
            b.write(&chunk_block_hash(&w.header)).unwrap();
            write_chunk_inputs(&mut b, &w, lo, hi);
        }
    }

    // hazync#235 / hazync#207: STREAM segments as the executor produces them.
    //
    // This was `run()` followed by a loop serialising every segment, and BOTH finished before the
    // listener below was bound. Every worker idled through the whole execute AND the whole bulk
    // serialisation, which is why the aggregate saturated at exactly N=2 however many workers were
    // added (1.81x at N=2, then flat to N=4): a 16-chunk join tree is 8 wide, so width was never
    // the limit -- the serial prologue was.
    //
    // The executor now runs on its OWN thread and publishes each segment as it appears, so the
    // listener binds immediately and a worker can prove segment 0 while segment 50 is still being
    // produced.
    //
    // ⛔ THE LAST SEGMENT IS NEVER STREAMED. The session journal and assumption set merge into its
    // claim before it is lifted, so it is tagged NOLIFT and proven here -- and which one is last is
    // not knowable until execution ends. Segments are therefore published with a ONE-SEGMENT LAG:
    // on producing segment i we publish i-1. The final one is published only once the executor has
    // returned, by which point it is known to be last, so no worker can ever receive it.
    // hazync#567: the WHOLE block window. TOTAL is now MEASURED here rather than summed from phases
    // that overlap — see BlockPhases for what the sum got wrong and by how much.
    let t_block = Instant::now();

    // Each segment's bytes sit behind their own Arc, so a sender clones a pointer under the lock
    // instead of ~0.77 MB while other threads wait on it.
    let wire: Arc<Mutex<Vec<Arc<Vec<u8>>>>> = Arc::new(Mutex::new(Vec::new()));
    let queue: Arc<Mutex<VecDeque<usize>>> = Arc::new(Mutex::new(VecDeque::new()));
    let out: Arc<Mutex<Vec<Option<SuccinctReceipt<ReceiptClaim>>>>> = Arc::new(Mutex::new(Vec::new()));
    // usize::MAX until the executor returns. Nothing may assume a total before then.
    let total_known = Arc::new(std::sync::atomic::AtomicUsize::new(usize::MAX));


    // hazync#163: bound the in-flight window by BYTES, not by frame count.
    //
    // `depth` alone is meaningless because HAZYNC_SEG_PO2 changes segment size underneath it: at
    // po2 18 a segment is small enough that depth 16 fits in the socket buffers, at po2 22 it is
    // ~0.77 MB and the same depth puts 12.3 MB in flight, which does not. The old failure was a
    // write/write deadlock -- silent, unbounded, and indistinguishable from a slow run.
    //
    // The reader/writer split below removes that deadlock outright; this clamp keeps the knob
    // MEANINGFUL, so a depth that is safe on one po2 does not silently become absurd on another.
    // It announces itself: a cap applied in silence reads as "your setting was honoured".
    let depth = {
        // 64 MB, not 4 MB. Measured cross-box on block 962000 at po2 22: segments average
        // 288.7 MB / 376 = 0.77 MB, but the LARGEST is 3.98 MB. Clamping by max() against a 4 MB
        // budget therefore yielded cap = 1 -- no pipelining at all, one segment in flight, and a
        // measured 10.7% throughput loss against the unclamped control (360/375 at 1535 s vs
        // 1387 s). A guard against a hang that has never reproduced must not cost that.
        //
        // This bound is now belt-and-braces: the reader/writer split above removes the deadlock
        // class outright, so the budget only exists to stop a pathological depth from queueing
        // absurd amounts. Sized so the default depth stays fully pipelined at po2 22.
        let budget: usize = std::env::var("HAZYNC_PUSH_BYTES").ok().and_then(|s| s.parse().ok())
            .unwrap_or(64 * 1024 * 1024);
        // hazync#235: under streaming there is no complete set to take a max over -- the clamp now
        // runs before most segments exist. Size it from the segment limit instead, which is the
        // bound the executor is actually working to: a po2-N segment cannot exceed 2^N cycles, and
        // 4 MB at po2 22 is what the largest measured segment (3.98 MB) came to. Over-estimating
        // is the safe direction: it only ever lowers the cap.
        let maxseg = (1usize << seg_po2().min(22)) / 1024 * 4;
        let maxseg = maxseg.max(1);
        let cap = (budget / maxseg).max(1);
        if depth > cap {
            println!("  push depth {depth} -> {cap}: {maxseg} B/segment against a {budget} B budget \
(HAZYNC_PUSH_BYTES). Raise the budget, not the depth.");
            cap
        } else { depth }
    };

    if !agg && range_n.is_none() {
        println!("=== segment coordinator (push) — block {} chunk {} po2 {} ===", height, idx, seg_po2());
    }
    println!("  streaming segments as they are produced (hazync#235), depth {depth}");
    println!("  listening on {}:{port}", seg_bind_addr());
    if seg_bind_is_loopback() {
        // The old hint was one clause on this line and scrolled past during startup; the consequence
        // only surfaced much later as a worker that silently never attached.
        println!("  ⚠ LOOPBACK ONLY — workers on OTHER machines cannot attach to this run.");
        println!("      multi-unit: set HAZYNC_SEG_REMOTE=1   (or HAZYNC_BIND=<addr> for a tunnel)");
        println!("      multi-card on this box needs nothing — 127.0.0.1 is correct for local workers");
    } else {
        println!("  ⚠ {}:{port} is OPEN and UNAUTHENTICATED for the life of this run — anyone who can",
                 seg_bind_addr());
        println!("      reach it can take work and stall the run. Prefer a tunnel on a public host.");
    }

    // Segments 0..total-1 go to workers. The LAST one is deliberately withheld: the session journal
    // and assumption set are merged into its claim before it is lifted, and a worker has no session,
    // so the segment coordinator proves that one itself. Handing it out anyway is what made the wait below
    // spin -- workers returned all `total` segments while the loop tested for exactly `total - 1`.
    // ALL segments, including the last (hazync#157). It is tagged NOLIFT at write time rather than
    // being stored tagged, so `wire[i]` stays a plain index.
    // `queue` and `out` are created above, before the executor starts, and GROW as segments arrive.
    // The last segment comes back as a SegmentReceipt, not a SuccinctReceipt, so it needs its own
    // slot -- the `out` vector is typed for lifted receipts.
    let last_out: Arc<Mutex<Option<SegmentReceipt>>> = Arc::new(Mutex::new(None));
    // Join work, fed one tree level at a time. Threads take from here once segments run out, so a
    // connection stays open across both phases instead of the fleet disbanding after proving and
    // leaving the segment coordinator to fold alone -- which is what left assembly flat at 373 s while
    // segment proving scaled 1.96x.
    let jobs: Arc<Mutex<VecDeque<(u32, Vec<u8>)>>> = Arc::new(Mutex::new(VecDeque::new()));
    let jout: Arc<Mutex<std::collections::HashMap<u32, SuccinctReceipt<ReceiptClaim>>>> =
        Arc::new(Mutex::new(std::collections::HashMap::new()));
    let alldone = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // hazync#365, the other half: DO NOT expose an unauthenticated listener by default.
    //
    // There is no authentication on this wire — `seg-connect` opens a bare TcpStream and
    // HAZYNC_WORKER_ID is a log label, not an identity — so anyone who can reach this port can attach
    // as a worker. They cannot corrupt the proof (every returned receipt is verified, and the assembly
    // is verified against METHOD_ID) but they can consume work and, before the deadline above, stall
    // the run outright.
    //
    // Binding 0.0.0.0 unconditionally meant a pod on a public IP was open to the internet for the life
    // of the run. Loopback by default makes reaching it a deliberate act — a tunnel, a private network,
    // or an explicit HAZYNC_BIND=0.0.0.0 — rather than the automatic consequence of starting a prove.
    // The first mode-6 validation run tunnelled every worker for exactly this reason.
    let bind_addr = seg_bind_addr();
    let listener = std::net::TcpListener::bind((bind_addr.as_str(), port))
        .unwrap_or_else(|e| panic!("bind {bind_addr}:{port}: {e}"));
    listener.set_nonblocking(true).ok();

    // #567: everything before this point is genuinely serial — no segment can have been published
    // yet, so no worker can be busy. This, plus assembly, is the real floor.
    let prologue_s = t_block.elapsed().as_secs_f64();
    let t_work = Instant::now();

    // NO thread::scope HERE, and that is the fix for a deadlock I introduced. The scope's implicit
    // join waited for the connection threads; the connection threads waited for join work; and the
    // join work was only published after the scope returned. A circular wait, and it hung a run for
    // 35 minutes with the GPU idle.
    //
    // The connection threads touch only Arc state -- the queues, the outputs, the serialised
    // segments -- and never the prover or the session, so they do not need to borrow anything and
    // can be plain detached threads. The main thread then stays free to drive the tree WHILE they
    // are still alive, which is the whole point.
    let acc_alldone = alldone.clone();
    // hazync#367: how many workers are connected RIGHT NOW. Incremented on accept, decremented where
    // each connection thread returns what it owed, so it counts live provers rather than sockets ever
    // seen. The acceptor is the only reader, which is why the "how long has it been zero" clock below
    // is a plain local -- no second shared structure, no mutex.
    let live_peers = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let acc_live = live_peers.clone();
    // #567: the summary is printed after the fold, by which point workers have dropped — reporting
    // `live_peers` there would divide the segment phase by however many happened to still be
    // attached, which on a finished run can be one or none.
    //
    // ⛔⛔ AND IT MUST COUNT CARDS, NOT CONNECTIONS. The first version took the peak of `live_peers`,
    // which is incremented per CONNECTION. Measured on tip hour 4, 2026-09-29: a 38-card fleet
    // reported "81 card(s) at peak", because a worker is re-armed for every block and a reconnect can
    // overlap its predecessor's thread — the old connection has not yet reached the decrement at the
    // end of its reader. Roughly 2x inflation, and it fed straight into the crossover figure (printed
    // 133 cards for 969,118 where the real number is ~62).
    //
    // ⇒ Count DISTINCT card names, which #570's hello makes available. A connection that never says
    // hello cannot be de-duplicated, so it is tallied separately and reported as such rather than
    // silently counted as a card.
    let cards_seen: Arc<Mutex<std::collections::HashSet<String>>> =
        Arc::new(Mutex::new(std::collections::HashSet::new()));
    let unnamed_conns = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let acc_seen = cards_seen.clone();
    let acc_unnamed = unnamed_conns.clone();
    let (aq, ao, aw, aj, ajo, alo, atk) =
        (queue.clone(), out.clone(), wire.clone(), jobs.clone(), jout.clone(), last_out.clone(),
         total_known.clone());
    // Detached on purpose -- see the deadlock note above; nothing joins this handle.
    let _acceptor = std::thread::spawn(move || {
        let mut handles = Vec::new();
        // hazync#367: when the peer count last fell to zero with work still outstanding. `None` while
        // anyone is connected, or while nothing is stranded. Local because the WouldBlock arm below is
        // the only reader; see the note there.
        let mut quiet_since: Option<Instant> = None;
        while !acc_alldone.load(std::sync::atomic::Ordering::Relaxed) {
            match listener.accept() {
                Ok((s, peer)) => {
                    println!("  worker connected from {peer}");
                    // hazync#367: counted here and decremented at the very end of this thread, below.
                    // Incrementing and decrementing in one place keeps them impossible to desynchronise
                    // -- a leaked count would make the no-peers check believe a worker is still there
                    // and never fire, which is the failure this is meant to remove.
                    acc_live.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    // Counted here; the hello below moves it into `cards_seen` by name.
                    acc_unnamed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let live = acc_live.clone();
                    // ⚠ Cloned PER CONNECTION, like every other Arc here. Referencing `acc_seen`
                    // inside the closure moves it on the first accept and the second one will not
                    // compile — which is the borrow checker catching the same "built once before the
                    // loop" mistake that #540 hit three times at runtime.
                    let (conn_seen, conn_unnamed) = (acc_seen.clone(), acc_unnamed.clone());
                    let (queue, out, wire, jobs, jout, alldone, last_out, total_known) =
                        (aq.clone(), ao.clone(), aw.clone(), aj.clone(), ajo.clone(), acc_alldone.clone(),
                         alo.clone(), atk.clone());
                    handles.push(std::thread::spawn(move || {
                        // hazync#163: one READER thread and one WRITER thread per connection.
                        //
                        // The old shape was a single thread that topped up to `depth` frames before
                        // it could reach any read_frame, so depth*segment_bytes went out with
                        // nothing draining the return path. write_frame is write_all on a BLOCKING
                        // socket and no write timeout is ever set, so once the buffers filled both
                        // ends blocked in write_all for ever. Splitting the directions across two
                        // threads on cloned handles removes the class, rather than tuning around it.
                        //
                        // No new correlation scheme is needed: receipts are matched by the RETURNED
                        // TAG, not by position, so `inflight` is only a bound and a requeue list.
                        s.set_nodelay(true).ok();
                        let mut ws = s;
                        let mut rs = match ws.try_clone() { Ok(r) => r, Err(e) => {
                            println!("  cannot split connection: {e}"); return; } };

                        // Sent-but-unanswered work, so a dropped connection returns exactly what it owed.
                        //
                        // hazync#365: each entry carries WHEN it was written, so a peer that accepts work
                        // and then goes quiet can be timed out. Segments need this as much as joins do —
                        // arguably more, since a segment is ~2.8 s of compute and a worker sitting on one
                        // silently is the common case — and `injobs` already carried an Instant while this
                        // did not, so only half the transport could be timed.
                        let inflight: Arc<Mutex<VecDeque<(usize, Instant)>>> = Arc::new(Mutex::new(VecDeque::new()));
                        // This peer's completed round trips, for its own adaptive deadline. Kept here (not
                        // shared across peers) because the whole point is that peers differ: on the run this
                        // was sized from, two healthy workers were 7.4x apart at p90.
                        let peer_rtts: Arc<Mutex<Vec<f64>>> = Arc::new(Mutex::new(Vec::new()));
                        // #570: who this connection says it is. Written once by the reader when the
                        // hello arrives, read by the reader's [rtt] line and by the writer's timeout
                        // line. Empty until then, and it stays empty for a worker too old to say.
                        let card: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
                        // hazync#252: the send INSTANT rides with the job. Assembly's cost was
                        // attributed to geography by inference — run 4 measured 128.7 s where compute
                        // alone predicts ~17 s — and the issue says so in as many words: "that part is
                        // inference, not measured (no RTTs were recorded)". Every join and resolve is
                        // a round trip carrying receipts, so the round trip is the thing to time.
                        //
                        // Timed on the SERVER's clock at both ends, which is what makes it trustworthy:
                        // the worker already logs its own recv/send (#254/#267), but comparing those to
                        // ours needs the two clocks to agree, and across rented boxes in several
                        // countries they do not. round_trip - compute needs no such assumption.
                        let injobs: Arc<Mutex<VecDeque<(u32, Vec<u8>, Instant)>>> = Arc::new(Mutex::new(VecDeque::new()));
                        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

                        let reader = {
                            // #365: peer_rtts is written by the reader (it times each completed round
                            // trip) and read by the writer (it sizes that peer's deadline), so it is
                            // cloned here like inflight/injobs rather than moved into the closure.
                            let (inflight, injobs, stop, peer_rtts, card) =
                                (inflight.clone(), injobs.clone(), stop.clone(), peer_rtts.clone(),
                                 card.clone());
                            let (seen, unnamed) = (conn_seen.clone(), conn_unnamed.clone());
                            let (out, jout, last_out, queue, jobs) =
                                (out.clone(), jout.clone(), last_out.clone(), queue.clone(), jobs.clone());
                            std::thread::spawn(move || {
                                // Its own VerifierContext: it holds Rc, so it is not Sync.
                                let ctx = &VerifierContext::default();
                                loop {
                                    let (i, body) = match read_frame(&mut rs) { Ok(v) => v, Err(_) => break };
                                    // #570: the hello, before any routing. It is not a job reply, so
                                    // it must never reach the tag match below -- an unrecognised tag
                                    // falls through to the SEGMENT path, which would pop a receipt
                                    // that was never sent and index `out` by a tag bit.
                                    if i == HELLO_TAG {
                                        if let Some(name) = hello_card_name(&body) {
                                            println!("  worker at {peer} is card {name}");
                                            // ⚠ A set, so a RECONNECT from the same card is not a
                                            // second card — which is the whole bug being fixed.
                                            seen.lock().unwrap().insert(name.clone());
                                            unnamed.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                                            *card.lock().unwrap() = name;
                                        }
                                        continue;
                                    }
                                    // EVERY job reply lands in jout, keyed by the tag it came
                                    // back under -- joins, resolves AND the merged lift (#161).
                                    //
                                    // Omitting LIFT_TAG here was fatal: the coordinator pushes the
                                    // merged last segment as jobs.push_back((LIFT_TAG, body)) and
                                    // then waits on jout.remove(&LIFT_TAG), so the reply has to be
                                    // routed here. Falling through instead reached the segment path,
                                    // where `plain = i & !NOLIFT_TAG` leaves bit 28 set and indexes
                                    // out[0x10000000] -- panicking at the LAST step of every
                                    // distributed aggregate:
                                    //
                                    //   index out of bounds: the len is 376 but the index is 268435456
                                    //
                                    // The pre-split code could not have this bug: it read each job
                                    // reply inline and inserted under whatever tag returned. Making
                                    // the routing explicit is what created the chance to miss one.
                                    if i & (JOIN_TAG | RESOLVE_TAG | LIFT_TAG) != 0 {
                                        // The worker proves serially and answers in order, so the
                                        // front of injobs is the job this reply belongs to.
                                        let owed = { injobs.lock().unwrap().pop_front() };
                                        // One line per job, timed on the SERVER's clock at both
                                        // ends. The worker's own line for the same task carries
                                        // compute_s (#254/#267), and compute_s is a DURATION, not a
                                        // timestamp — so rtt_ms - compute_s*1000 is transport and
                                        // queueing without the two clocks ever having to agree.
                                        // That subtraction is the number #252 has been reasoning
                                        // about and has never measured.
                                        if let Some((_, ref sent_body, at)) = owed {
                                            let kind = if i & RESOLVE_TAG != 0 { "resolve" }
                                                       else if i & LIFT_TAG != 0 { "lift" } else { "join" };
                                            // #570: card= is the worker's own name; peer= stays
                                            // because it is still what routes, and the two together
                                            // are what show several cards sharing one egress IP.
                                            let who = card.lock().unwrap();
                                            let who = if who.is_empty() { "?" } else { who.as_str() };
                                            println!("  [rtt] card={who} peer={peer} kind={kind} tag={i:#x} \
rtt_ms={:.1} bytes_out={} bytes_in={}",
                                                     at.elapsed().as_secs_f64() * 1000.0,
                                                     sent_body.len(), body.len());
                                        }
                                        match bincode::deserialize::<SuccinctReceipt<ReceiptClaim>>(&body) {
                                            Ok(r) if r.verify_integrity_with_context(ctx).is_ok() => {
                                                jout.lock().unwrap().insert(i, r);
                                            }
                                            _ => {
                                                let (kind, n) = if i & RESOLVE_TAG != 0 {
                                                    ("resolve", i & !RESOLVE_TAG)
                                                } else { ("join", i & !JOIN_TAG) };
                                                println!("  worker returned a bad {kind} for {n}");
                                                // MUST requeue explicitly: it has been popped off
                                                // injobs, so the teardown requeue will not cover it.
                                                // Dropping it would lose a join for ever and the tree
                                                // would wait on it until the run died.
                                                if let Some((t, b, _)) = owed { jobs.lock().unwrap().push_back((t, b)); }
                                            }
                                        }
                                        continue;
                                    }
                                    let plain = (i & !NOLIFT_TAG) as usize;
                                    // #365: record the round trip before dropping the entry — this peer's
                                    // own distribution is what sizes its deadline.
                                    {
                                        let mut fl = inflight.lock().unwrap();
                                        if let Some(pos) = fl.iter().position(|&(x, _)| x == plain) {
                                            let (_, at) = fl[pos];
                                            peer_rtts.lock().unwrap().push(at.elapsed().as_secs_f64());
                                            fl.remove(pos);
                                        }
                                    }
                                    if i & NOLIFT_TAG != 0 {
                                        match bincode::deserialize::<SegmentReceipt>(&body) {
                                            Ok(sr) if sr.verify_integrity_with_context(ctx).is_ok() => {
                                                *last_out.lock().unwrap() = Some(sr);
                                            }
                                            _ => {
                                                println!("  worker returned an invalid LAST segment receipt");
                                                queue.lock().unwrap().push_back(plain);
                                            }
                                        }
                                        continue;
                                    }
                                    match bincode::deserialize::<SuccinctReceipt<ReceiptClaim>>(&body) {
                                        Ok(r) if r.verify_integrity_with_context(ctx).is_ok() => {
                                            out.lock().unwrap()[plain] = Some(r);
                                        }
                                        _ => {
                                            println!("  worker returned an invalid receipt for {i}");
                                            queue.lock().unwrap().push_back(plain);
                                        }
                                    }
                                }
                                stop.store(true, std::sync::atomic::Ordering::Relaxed);
                            })
                        };

                        // Writer. It can block in write_all without consequence now, because the
                        // reader is draining the other direction on its own thread.
                        while !stop.load(std::sync::atomic::Ordering::Relaxed)
                              && !alldone.load(std::sync::atomic::Ordering::Relaxed) {
                            // hazync#365: has this peer gone quiet while holding work?
                            //
                            // No new thread: this loop already wakes every 20 ms when idle and already
                            // holds every structure the check needs.
                            //
                            // ⛔ ON BREACH THE WHOLE PEER IS DROPPED, never one job. The reader pops
                            // `injobs` FRONT-first because "the worker proves serially and answers in
                            // order" — so requeueing a single job while the connection stayed open would
                            // let a late reply pop the WRONG `owed` entry and mis-attribute every later
                            // rtt_ms. Tripping `stop` runs the existing teardown below, which shuts the
                            // socket, joins the reader, and returns BOTH tracks to their queues. One
                            // unwind path, already correct, rather than a second one to keep in step.
                            {
                                let win = peer_job_timeout_s(&peer_rtts.lock().unwrap(), job_timeout_floor_s(),
                                                             job_timeout_mult(), job_timeout_ceiling_s(),
                                                             job_timeout_min_samples());
                                let oldest_seg = inflight.lock().unwrap().front().map(|&(i, at)| (i, at.elapsed().as_secs_f64()));
                                let oldest_job = injobs.lock().unwrap().front().map(|(t, _, at)| (*t, at.elapsed().as_secs_f64()));
                                let overdue = match (oldest_seg, oldest_job) {
                                    (Some((i, a)), _) if a > win => Some((format!("segment {i}"), a)),
                                    (_, Some((t, a))) if a > win => Some((format!("job {t:#x}"), a)),
                                    _ => None,
                                };
                                if let Some((what, age)) = overdue {
                                    // Loud, for the same reason the push-depth clamp is loud: a peer
                                    // dropped in silence reads as "the run is just slow".
                                    let (nseg, njob) = (inflight.lock().unwrap().len(), injobs.lock().unwrap().len());
                                    // #570: name the card being dropped. "peer 1.2.3.4 was dropped"
                                    // is useless when eleven cards share that address — it reads as
                                    // if the whole pod went, and nothing says which GPU to replace.
                                    let who = { let g = card.lock().unwrap();
                                                if g.is_empty() { "?".to_string() } else { g.clone() } };
                                    println!("  ⛔ card={who} peer={peer} has not answered {what} for {age:.0}s \
(window {win:.0}s from its own {} round trips, HAZYNC_JOB_TIMEOUT_*). Dropping it and requeueing \
{nseg} segment(s) + {njob} job(s) — hazync#365.",
                                             peer_rtts.lock().unwrap().len());
                                    stop.store(true, std::sync::atomic::Ordering::Relaxed);
                                    break;
                                }
                            }
                            let room = { depth.saturating_sub(inflight.lock().unwrap().len()) };
                            let mut wrote = false;
                            for _ in 0..room {
                                let next = { queue.lock().unwrap().pop_front() };
                                match next {
                                    Some(i) => {
                                        // hazync#235: `total` is unknown until the executor returns, so the
                                        // NOLIFT test reads the atomic. It is only ever true for the segment
                                        // published AFTER execution ended (the one-segment lag guarantees
                                        // that), so a worker cannot be handed the last segment early.
                                        let known = total_known.load(std::sync::atomic::Ordering::SeqCst);
                                        let tag = if known != usize::MAX && i == known - 1 {
                                            i as u32 | NOLIFT_TAG
                                        } else { i as u32 };
                                        // Clone the Arc under the lock, not the ~0.77 MB behind it.
                                        let frame = { wire.lock().unwrap()[i].clone() };
                                        // #365: the send instant rides with the entry, so a silent peer
                                        // can be timed out. Recorded BEFORE the write, as before.
                                        inflight.lock().unwrap().push_back((i, Instant::now()));
                                        if write_frame(&mut ws, tag, &frame).is_err() {
                                            inflight.lock().unwrap().retain(|&(x, _)| x != i);
                                            queue.lock().unwrap().push_front(i);
                                            stop.store(true, std::sync::atomic::Ordering::Relaxed);
                                            break;
                                        }
                                        wrote = true;
                                    }
                                    None => break,
                                }
                            }
                            if !stop.load(std::sync::atomic::Ordering::Relaxed) {
                                let job = { jobs.lock().unwrap().pop_front() };
                                if let Some((tag, body)) = job {
                                    injobs.lock().unwrap().push_back((tag, body.clone(), Instant::now()));
                                    if write_frame(&mut ws, tag, &body).is_err() {
                                        injobs.lock().unwrap().pop_back();
                                        jobs.lock().unwrap().push_front((tag, body));
                                        stop.store(true, std::sync::atomic::Ordering::Relaxed);
                                    } else { wrote = true; }
                                }
                            }
                            if !wrote { std::thread::sleep(std::time::Duration::from_millis(20)); }
                        }
                        if alldone.load(std::sync::atomic::Ordering::Relaxed) {
                            let _ = write_frame(&mut ws, SEG_EOF, &[]);
                        }

                        // Break the reader out of its blocking read, then return what was owed.
                        let _ = ws.shutdown(std::net::Shutdown::Both);
                        let _ = reader.join();
                        {
                            let mut q = queue.lock().unwrap();
                            // `queue` is indices only — drop the #365 send instant on the way back.
                            for (i, _) in inflight.lock().unwrap().drain(..) { q.push_front(i); }
                        }
                        {
                            let mut j = jobs.lock().unwrap();
                            for (t, b, _) in injobs.lock().unwrap().drain(..) { j.push_front((t, b)); }
                        }
                        // hazync#367: the matching decrement for the fetch_add on accept. It belongs
                        // HERE, after the owed work is back on the queues -- so by the time the count
                        // can reach zero, everything this peer held is visible as outstanding work
                        // rather than briefly invisible to the check.
                        live.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                    }));
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // hazync#367: nobody connected, work stranded, and it has stayed that way.
                    //
                    // This rides the existing 50 ms wake rather than adding a thread, the same way
                    // #365's deadline rides the writer's 20 ms idle poll. `quiet_since` is a plain
                    // local because this loop is its only reader -- no second shared clock.
                    let n = acc_live.load(std::sync::atomic::Ordering::Relaxed);
                    let outstanding = aq.lock().unwrap().len() + aj.lock().unwrap().len();
                    if n > 0 || outstanding == 0 {
                        quiet_since = None;      // someone is here, or nothing is stranded
                    } else {
                        let since = *quiet_since.get_or_insert_with(Instant::now);
                        let win = no_peers_timeout_s();
                        if no_peers_breached(n, outstanding, since.elapsed().as_secs_f64(), win) {
                            // Loud, for the same reason #365's drop is loud: a coordinator holding a
                            // port in silence reads as "the run is just slow".
                            println!("  ⛔ no worker has been connected for {:.0}s and {outstanding} \
piece(s) of work are stranded (window {win:.0}s, HAZYNC_NO_PEERS_TIMEOUT). Giving up — hazync#367.",
                                     since.elapsed().as_secs_f64());
                            println!("     seg-connect has no reconnect: a worker that dropped will not \
come back by itself. Start one against this coordinator, or re-run.");
                            acc_alldone.store(true, std::sync::atomic::Ordering::Relaxed);
                            std::process::exit(1);
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(e) => { println!("  accept failed: {e}"); break; }
            }
        }
        for h in handles { let _ = h.join(); }
    });

    // hazync#235: collect the executor. It has been producing segments this whole time, with any
    // connected worker already proving them — that overlap is the entire point of the change. From
    // here on `total` is final, and everything below needs the session (journal + assumptions).
    //
    // ⚠ Joining here, AFTER the acceptor is detached and serving, is load-bearing. Join before the
    // listener binds and the streaming is decorative: workers still arrive to a finished execute.
    // ⚠ ExecutorImpl is NOT Send (it holds a `dyn SessionEvents`), so the executor cannot be moved
    // to a thread. It does not need to be: the acceptor above is already detached and serving, so
    // running the executor HERE — after the bind — gives the same overlap. Workers connect and prove
    // segment 0 while this call is still producing segment 50.
    // #567: time the EXECUTOR itself, not the executor plus the setup before it. The old timer
    // started 100 lines earlier, so `execution` also carried the prologue — which is why the
    // over-count came to slightly less than the reported execution figure.
    let t_exec = Instant::now();
    let session = {
        let mut pending: Option<usize> = None;   // the one-segment lag; see the note above
        ExecutorImpl::from_elf(b.build().unwrap(), METHOD_ELF).unwrap()
            .run_with_callback(|segment| {
                let bytes = bincode::serialize(&segment).expect("serialize");
                let idx = {
                    let mut wl = wire.lock().unwrap();
                    wl.push(Arc::new(bytes));
                    out.lock().unwrap().push(None);
                    wl.len() - 1
                };
                if let Some(prev) = pending.replace(idx) {
                    queue.lock().unwrap().push_back(prev);
                }
                // The bytes are already captured above; the ref itself is not needed again.
                Ok(Box::new(risc0_zkvm::NullSegmentRef))
            })
            .expect("execute")
    };
    let exec_s = t_exec.elapsed().as_secs_f64();
    let total = wire.lock().unwrap().len();
    total_known.store(total, std::sync::atomic::Ordering::SeqCst);
    // The held-back segment is the last one. Safe to publish now that that is established.
    if total > 0 { queue.lock().unwrap().push_back(total - 1); }
    let bytes: usize = wire.lock().unwrap().iter().map(|v| v.len()).sum();
    println!("  execution {exec_s:.1} s   {total} segments, {:.1} MB (streamed)", bytes as f64 / 1e6);

    // Wait for every segment to come back before lifting. The threads stay alive throughout and
    // pick up join work as the tree below publishes it.
    // Report progress while segments come in. Without this the segment coordinator prints its header and
    // then nothing for the whole segment phase, which is both the #145 complaint again and, more
    // immediately, a run that any no-output watchdog will kill for looking wedged while it is
    // working perfectly well. It already killed one.
    let mut nextmark = 0usize;
    let step = (total / 20).max(1);
    loop {
        let have = { out.lock().unwrap().iter().filter(|r| r.is_some()).count() };
        if have >= nextmark {
            let el = t_work.elapsed().as_secs_f64();
            let eta = if have > 0 { el / have as f64 * (total - 1 - have) as f64 } else { 0.0 };
            println!("    {have}/{} segments  {el:.0}s elapsed, ~{eta:.0}s left", total - 1);
            nextmark = have + step;
        }
        // Both conditions: total-1 lifted receipts AND the last segment's unlifted receipt. Testing
        // only the first would fall through with the last segment still in flight -- the same class
        // of bug as the earlier `== total - 1` spin, in the opposite direction.
        let have_last = { last_out.lock().unwrap().is_some() };
        if have >= total - 1 && have_last { break; }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let work_s = t_work.elapsed().as_secs_f64();

    // Every segment but the last arrives lifted. The last cannot -- the session journal and
    // assumptions merge into its claim before lifting and a worker has no session -- so it is
    // proved here. In this mode the segment coordinator keeps that one segment for itself.
    let opts = ProverOpts::succinct();
    let server = risc0_zkvm::get_prover_server(&opts).expect("prover server");
    let t_asm = Instant::now();
    let mut lifted: Vec<SuccinctReceipt<ReceiptClaim>> = Vec::with_capacity(total);
    {
        let o = out.lock().unwrap();
        for i in 0..total - 1 { lifted.push(o[i].clone().expect("missing lift")); }
    }
    // The last segment was PROVED by a worker (hazync#157). All that is left here is the part that
    // genuinely needs the session: merging the journal digest and assumption set into its claim, and
    // lifting the result. prepare_lifts merges into the LAST element of what it is given, so a
    // one-element vector does exactly that and nothing else.
    // MERGE HERE, LIFT ON A WORKER (hazync#161). The merge is claim surgery and needs the session;
    // the lift is a recursion proof and does not. Splitting them means the segment coordinator holds
    // no prover at all -- so a missing or dead GPU cannot abort a run at its final step, and a
    // coordinator can be a machine with no card.
    let mut last_rcpt: SegmentReceipt = last_out.lock().unwrap().take().expect("last segment receipt");
    server.merge_session_output(&session, &mut last_rcpt).expect("merge session output");
    {
        let body = bincode::serialize(&last_rcpt).expect("ser merged last");
        jobs.lock().unwrap().push_back((LIFT_TAG, body));
    }
    let tail_lift = loop {
        let got = { jout.lock().unwrap().remove(&LIFT_TAG) };
        if let Some(r) = got { break r; }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    lifted.push(tail_lift);

    // DISTRIBUTE THE JOIN TREE, PIPELINED. Publish a join the moment its two children exist,
    // rather than when its whole level does.
    //
    // The old driver was level-synchronous: it queued a level, waited for every join in it, then
    // queued the next. That barrier is NOT inherent -- level l+1 position q consumes exactly two
    // nodes, (l, 2q) and (l, 2q+1), so it is ready as soon as those two are, regardless of how the
    // rest of level l is doing. Waiting for the level wastes every card that finishes early, and
    // the waste grows with the fleet: the tree narrows towards the root, so join-tree efficiency
    // runs 93% at 2 cards, 68% at 8, 54% at 16 and 40% at 32. That is why the two-card aggregate
    // test looked healthy -- N=2 is the one regime where this barrier is invisible.
    //
    // ORDERING IS UNCHANGED, and that is the point. The tree shape is a function of the leaf count
    // alone, so it is precomputed here: position p pairs with p^1, and the EVEN position is always
    // the left operand. Joins chain claims (join asserts a.post == b.pre) so they do not commute --
    // only the SCHEDULE changes, never which two receipts meet or in which order.
    let widths = join_tree_widths(lifted.len());
    let depth = widths.len() - 1; // number of join levels; 0 when there is a single leaf

    let mut ready: Vec<Vec<Option<SuccinctReceipt<ReceiptClaim>>>> =
        widths.iter().map(|w| vec![None; *w]).collect();
    for (i, r) in lifted.into_iter().enumerate() {
        ready[0][i] = Some(r);
    }
    let mut published: Vec<Vec<bool>> = widths.iter().map(|w| vec![false; w / 2]).collect();

    // hazync#252 lever 2: prove NARROW levels here instead of distributing them.
    //
    // MEASURED, not assumed (2026-09-21, 3 A40s, 2 remote workers, 38 [rtt] samples):
    //     join     p50 rtt 1596.0 ms   compute 354.5 ms   => TRANSPORT 1241.5 ms   77.8%
    // More than three quarters of a join is the round trip. Near the root the tree is narrower than
    // the fleet, so distributing buys no parallelism at all and still pays that trip: the widths run
    // 37, 19, 10, 5, 3, 2, 1, and each of those levels waits on the one below.
    //
    // ⚠ OFF BY DEFAULT. (Lever 1 shipped off too and was turned ON on 2026-09-21 once measured;
    // this one is measured at ~0.4-1.6 s and stays off pending a fleet run.) This changes where
    // a join runs, never which two receipts meet or in which order -- `server.join(&a, &b)` is the
    // same call a worker makes on the same pair -- but it is the prover's core and it earns its
    // default the way lever 1 did: by being measured in the wild first.
    let join_local_max: usize = std::env::var("HAZYNC_JOIN_LOCAL_MAX")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    if join_local_max > 0 {
        println!("    joining levels of <= {join_local_max} pair(s) locally (HAZYNC_JOIN_LOCAL_MAX)");
    }
    let mut local_joins = 0usize;
    let mut local_join_s = 0.0f64;

    println!("    join tree: {} leaves, {depth} levels, widths {:?}", widths[0], widths);
    let njoins: usize = widths.iter().take(depth).map(|w| w / 2).sum();
    let mut harvested = 0usize;

    while depth > 0 && ready[depth][0].is_none() {
        // Publish every join whose two children have arrived and which has not been queued yet.
        // ⛔ NOTHING IS PROVED WHILE THE JOBS LOCK IS HELD. A local join takes ~350 ms, and every
        // connection thread takes work through this same mutex -- proving inside the lock would
        // stall the whole fleet for the duration of each one. Decide here, prove below.
        let mut local_todo: Vec<(usize, usize, SuccinctReceipt<ReceiptClaim>,
                                 SuccinctReceipt<ReceiptClaim>)> = Vec::new();
        {
            let mut q = jobs.lock().unwrap();
            for l in 0..depth {
                let npairs = widths[l] / 2;
                let keep_local = join_stays_local(npairs, join_local_max);
                for p in 0..npairs {
                    if published[l][p] {
                        continue;
                    }
                    let (a, b) = (&ready[l][p * 2], &ready[l][p * 2 + 1]);
                    if let (Some(a), Some(b)) = (a, b) {
                        if keep_local {
                            local_todo.push((l, p, a.clone(), b.clone()));
                        } else {
                            let ab = bincode::serialize(a).expect("ser a");
                            let bb = bincode::serialize(b).expect("ser b");
                            q.push_back((JOIN_TAG | ((l as u32) << 16) | p as u32, pack_pair(&ab, &bb)));
                        }
                        published[l][p] = true;
                    }
                }
            }
        }

        for (l, p, a, b) in local_todo {
            let t = std::time::Instant::now();
            let j = server.join(&a, &b).expect("local join");
            let took = t.elapsed().as_secs_f64();
            local_join_s += took;
            local_joins += 1;
            ready[l + 1][p] = Some(j);
            harvested += 1;
            println!("    [local] join level {l} pos {p} in {took:.2}s ({harvested}/{njoins})");
        }

        // An odd receipt carries forward untouched, keeping its position so adjacency holds. It is
        // ready the instant its source node is -- it waits on no join at all.
        for l in 0..depth {
            if widths[l] % 2 == 1 {
                let npairs = widths[l] / 2;
                if ready[l + 1][npairs].is_none() {
                    if let Some(r) = ready[l][widths[l] - 1].clone() {
                        ready[l + 1][npairs] = Some(r);
                    }
                }
            }
        }

        // Harvest whatever the workers have returned, keyed by tag rather than by count -- results
        // from different levels are now in flight at the same time.
        {
            let mut m = jout.lock().unwrap();
            if !m.is_empty() {
                for l in 0..depth {
                    let npairs = widths[l] / 2;
                    for p in 0..npairs {
                        if ready[l + 1][p].is_some() {
                            continue;
                        }
                        if let Some(r) = m.remove(&(JOIN_TAG | ((l as u32) << 16) | p as u32)) {
                            ready[l + 1][p] = Some(r);
                            harvested += 1;
                            if harvested % 25 == 0 || harvested == njoins {
                                println!("    joins {harvested}/{njoins}");
                            }
                        }
                    }
                }
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    let mut level: Vec<SuccinctReceipt<ReceiptClaim>> =
        vec![ready[depth][0].take().expect("root receipt")];

    // DISTRIBUTE THE RESOLVES (hazync#153). This is the last term that stayed on the segment coordinator.
    //
    // A mode-4 chunk has no assumptions, so this loop does nothing and costs nothing. A mode-5
    // aggregate has one per chunk receipt, at roughly 11.35 M cycles each -- about 175 s for
    // sixteen, all of it previously undivided while every card but one sat idle.
    //
    // They are a CHAIN: each resolve consumes the conditional the last one produced, so exactly one
    // is outstanding at a time and this does not parallelise. It wins by moving the work off the
    // coordinator, which is the whole point when the segment coordinator is the machine everything else is
    // waiting on. Honest about the size of the prize: sixteen sequential steps, not log2(16).
    let mut conditional = level.pop().expect("one left");
    let assumptions = if agg_early {
        // The session holds placeholder (fake) assumptions; resolve with the REAL chunk receipts, in the
        // same order the guest verified them (chunk order). Wait for the files: the chunk fleet may still
        // be finishing. Each one is verified against METHOD_ID on the way in (read_chunk_receipts).
        let dir = std::env::var("HAZYNC_RECEIPTS").unwrap_or_else(|_| ".".into());
        let t_wait = Instant::now();
        while (0..agg_nchunks).any(|i| !std::path::Path::new(&format!("{dir}/chunk_{i}.hzk")).exists()) {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        println!("    AGG_EARLY: all {agg_nchunks} chunk receipts present after {:.1} s of waiting",
                 t_wait.elapsed().as_secs_f64());
        // Resolve order is the SESSION's assumption order, which need not be chunk order: match each
        // session assumption to the real receipt with the same claim digest. A miss is a loud failure.
        use risc0_zkvm::sha::Digestible;
        let real = read_chunk_receipts(agg_nchunks);
        let digests: Vec<_> = real.iter().map(|r| r.claim().expect("chunk receipt claim").digest()).collect();
        let order: Vec<usize> = session.assumptions.iter().map(|(a, _)| {
            digests.iter().position(|d| *d == a.claim)
                .unwrap_or_else(|| panic!("no chunk receipt has the claim of aggregate assumption {}", a.claim))
        }).collect();
        println!("    AGG_EARLY: resolve order (chunk index per session assumption) {order:?}");
        order.into_iter().map(|k| match real[k].inner.clone() {
            risc0_zkvm::InnerReceipt::Succinct(sr) => sr.into_unknown(),
            risc0_zkvm::InnerReceipt::Composite(c) =>
                server.composite_to_succinct(&c).expect("composite -> succinct").into_unknown(),
            _ => panic!("chunk receipt is neither succinct nor composite"),
        }).collect::<Vec<_>>()
    } else {
        server.session_assumptions_succinct(&session).expect("session assumptions")
    };
    let nres = assumptions.len();
    // hazync#252 lever 1: HAZYNC_RESOLVE_LOCAL=1 runs the chain HERE instead. Distributing it bought
    // nothing but moving work off this box -- it is a chain, one step outstanding at a time -- and
    // each step is a full round trip carrying a conditional receipt out and back. On a fleet spread
    // across countries (milestone run 4) that is paid 27 times on the critical path, while this
    // box's GPU sits idle: the join tree is done and the last segment was lifted long ago.
    // OPT-IN, because it is only a win when this box has a GPU: a CPU-only segment coordinator would
    // trade 27 round trips for 27 CPU resolves. Default stays the measured behaviour until a GPU run
    // settles it (#252).
    if local_joins > 0 {
        // Report BOTH numbers. A local join's own compute time is the honest figure to compare
        // against a worker's `compute_s`; the saving is the transport it did not pay, which shows up
        // as a shorter critical path overall, not in this line.
        println!("    {local_joins} of {njoins} joins proved locally, {:.2}s total ({:.2}s each)",
                 local_join_s, local_join_s / local_joins as f64);
    }

    let res_local = resolve_runs_locally(std::env::var("HAZYNC_RESOLVE_LOCAL").ok().as_deref());
    let t_res = Instant::now();
    for (k, a) in assumptions.iter().enumerate() {
        if res_local {
            conditional = server.resolve(&conditional, a).expect("resolve (local)");
            println!("    resolve {}/{nres}  {:.0}s elapsed (local)", k + 1, t_res.elapsed().as_secs_f64());
            continue;
        }
        let tag = RESOLVE_TAG | k as u32;
        let cb = bincode::serialize(&conditional).expect("ser conditional");
        let ab = bincode::serialize(a).expect("ser assumption");
        jobs.lock().unwrap().push_back((tag, pack_pair(&cb, &ab)));
        // Wait for this one before publishing the next: the next job's INPUT is this job's output,
        // so queueing them together would hand out work built on a conditional that does not exist
        // yet. The barrier is inherent to a chain, not an implementation shortcut.
        loop {
            let got = { jout.lock().unwrap().remove(&tag) };
            if let Some(r) = got { conditional = r; break; }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        println!("    resolve {}/{nres}  {:.0}s elapsed", k + 1, t_res.elapsed().as_secs_f64());
    }
    let res_s = t_res.elapsed().as_secs_f64();

    alldone.store(true, std::sync::atomic::Ordering::Relaxed);
    let info = server.assemble_from_resolved(&session, conditional).expect("assemble");
    let asm_s = t_asm.elapsed().as_secs_f64();
    info.receipt.verify(METHOD_ID).expect("PUSH RECEIPT FAILED verify");

    // hazync#567: these phases are NOT a column to add up. `execution` runs INSIDE the segment
    // phase, so the old `TOTAL = execution + worker wall + assembly` counted it twice and inflated
    // every block by the executor's window. TOTAL is measured now; the layout says which nest.
    let phases = BlockPhases { prologue_s, exec_s, segment_s: work_s, assembly_s: asm_s };
    println!();
    println!("  prologue       {prologue_s:8.1} s   <- serial: bind + allocate + spawn");
    println!("  segment phase  {work_s:8.1} s   <- {} segments produced AND proved, overlapped", total - 1);
    println!("    of which execution {exec_s:6.1} s   <- INSIDE the line above, not additional");
    println!("  assembly       {asm_s:8.1} s   <- serial: last segment + join tree + resolves");
    if nres > 0 {
        println!("    of which resolves {res_s:6.1} s   <- {nres} {}", if res_local { "run locally (the default)" } else { "pushed to workers (HAZYNC_RESOLVE_LOCAL=0)" });
    }
    println!("  TOTAL          {:8.1} s   <- MEASURED, not the sum of the above", t_block.elapsed().as_secs_f64());
    println!("  serial floor   {:8.1} s   <- prologue + assembly; no fleet size reduces it",
             phases.serial_floor_s());
    // ⚠ DISTINCT CARDS, not connections — see the note at `cards_seen`.
    let named = cards_seen.lock().unwrap().len();
    let unnamed = unnamed_conns.load(std::sync::atomic::Ordering::Relaxed);
    if let Some(n) = phases.exec_bound_cards(named + unnamed) {
        let extra = if unnamed > 0 { format!(" + {unnamed} unidentified connection(s)") }
                    else { String::new() };
        println!("  {named} card(s) by name{extra}; execution becomes the binding constraint at \
~{n:.0}, and below that cards still buy time — hazync#567");
    }
    println!();
    println!(">>> PUSH-TRANSPORT RECEIPT VERIFIED against METHOD_ID");
    println!("    digest {}", hex(info.receipt.journal.digest().as_bytes()));

    // ⛔ PERSIST IT. This receipt is the entire point of the run and it used to exist only in this
    // process's memory: `seg-serve` verified it, printed a digest, and exited. The 2026-09-10
    // milestone (block 966,256, 26 cards, 485 s) therefore ended with a digest in a log and no
    // artifact -- and the coordinator was a rented pod that was torn down minutes later. Hours of
    // fleet time produced something nobody can hand to a third party.
    //
    // `prove-chunk` has always honoured HAZYNC_OUT; the aggregate simply never did. Same convention
    // here, with a default so the file exists even when nobody sets it.
    //
    // ⛔ Mode 6 defaults to the name `prove-range-bridge` writes, so a board block proved across N
    // cards lands as the same artifact one card would have produced and submits unchanged (#361).
    let out = std::env::var("HAZYNC_OUT").unwrap_or_else(|_| match range_n {
        Some(n) => format!("range_{n}.hzk"),
        None => "aggregate_receipt.bin".into(),
    });
    match bincode::serialize(&info.receipt) {
        Ok(bytes) => match std::fs::write(&out, &bytes) {
            Ok(()) => println!("    receipt written to {out} ({} bytes)", bytes.len()),
            // Loud, and a non-zero exit: a run that verified but could not save is not a success.
            Err(e) => { eprintln!("⛔ RECEIPT VERIFIED BUT NOT SAVED: {out}: {e}"); std::process::exit(3); }
        },
        Err(e) => { eprintln!("⛔ RECEIPT VERIFIED BUT NOT SERIALIZABLE: {e}"); std::process::exit(3); }
    }
}

#[cfg(test)]
mod join_tree_tests {
    use super::{join_tree_widths, join_stays_local, resolve_runs_locally, JOIN_TAG,
                peer_job_timeout_s, no_peers_breached};

    /// A join tree over symbolic nodes, so two schedules can be compared for structural identity.
    #[derive(Clone, PartialEq, Eq, Debug)]
    enum Node { Leaf(usize), Join(Box<Node>, Box<Node>) }

    /// The OLD level-synchronous driver, transcribed literally -- including `pop()` taking the LAST
    /// element as the odd carry. This is the behaviour the pipelined version must not change.
    fn old_tree(n: usize) -> Node {
        let mut level: Vec<Node> = (0..n).map(Node::Leaf).collect();
        while level.len() > 1 {
            let npairs = level.len() / 2;
            let odd = level.len() % 2 == 1;
            let mut next: Vec<Node> = Vec::with_capacity(npairs + 1);
            for p in 0..npairs {
                next.push(Node::Join(Box::new(level[p * 2].clone()), Box::new(level[p * 2 + 1].clone())));
            }
            if odd { next.push(level.pop().expect("odd tail")); }
            level = next;
        }
        level.pop().expect("root")
    }

    /// The NEW schedule, built from the SHIPPED `join_tree_widths`.
    fn new_tree(n: usize) -> Node {
        let widths = join_tree_widths(n);
        let depth = widths.len() - 1;
        let mut ready: Vec<Vec<Option<Node>>> = widths.iter().map(|w| vec![None; *w]).collect();
        for i in 0..n { ready[0][i] = Some(Node::Leaf(i)); }
        for l in 0..depth {
            let npairs = widths[l] / 2;
            for p in 0..npairs {
                let a = ready[l][p * 2].clone().expect("left child");
                let b = ready[l][p * 2 + 1].clone().expect("right child");
                ready[l + 1][p] = Some(Node::Join(Box::new(a), Box::new(b)));
            }
            if widths[l] % 2 == 1 {
                ready[l + 1][npairs] = ready[l][widths[l] - 1].clone();
            }
        }
        ready[depth][0].clone().expect("root")
    }

    /// hazync#252 lever 1 is ON by default as of 2026-09-21. The danger in a defaulted-on flag is
    /// the UNSET case: `unwrap_or(false)` would silently disable it for every run that never sets
    /// the variable, which is all of them -- and the only visible symptom would be the old, slower
    /// behaviour, which looks like nothing at all.
    #[test]
    fn resolve_is_local_unless_explicitly_disabled() {
        // ⛔ THE CASE THAT MATTERS: nothing set at all.
        assert!(resolve_runs_locally(None), "unset must mean ON, or the default never applies");

        // "0" is the one and only way off.
        assert!(!resolve_runs_locally(Some("0")), "=0 must push the chain back to workers");

        // Everything else is on, including the historical "=1" that used to be the only way ON.
        for v in ["1", "", "true", "yes", "01", "00", " 0", "0 "] {
            assert!(resolve_runs_locally(Some(v)),
                    "{v:?} must not read as disabled -- only an exact \"0\" disables");
        }

        // ⛔ POSITIVE CONTROL: the OLD rule must fail the unset case, or this test would pass
        // just as happily on the bug it exists to catch.
        let old_rule = |v: Option<&str>| v.map(|x| x == "1").unwrap_or(false);
        assert!(!old_rule(None), "control: the old rule did default to OFF");
        assert!(old_rule(None) != resolve_runs_locally(None),
                "control did not fail: the new rule is indistinguishable from the old one");
    }

    /// hazync#252 lever 2 moves joins from a worker to this process. It must be invisible to the
    /// PROOF: the same pairs, in the same order, whoever runs them. Build the tree with every
    /// possible local/remote split and demand the root be identical to the pre-pipelining tree.
    ///
    /// ⛔ This models BOTH PATHS as `seg_serve_cmd` runs them -- local pairs proved straight after
    /// the lock is dropped, remote pairs packed into a `JOIN_TAG | (level << 16) | pos` frame,
    /// queued, and written back from that tag on harvest. A test that built one tree and merely
    /// CALLED the predicate could not fail; the tag round trip is where an indexing bug lives.
    #[test]
    fn local_join_split_builds_an_identical_tree() {
        for n in 1..=64usize {
            let want = old_tree(n);
            let widths = join_tree_widths(n);
            // 0 = the shipped default (nothing local); widths[0] = every level local; and every
            // ceiling in between, so no value of HAZYNC_JOIN_LOCAL_MAX is left untried.
            for max in 0..=widths[0] {
                let depth = widths.len() - 1;
                let mut ready: Vec<Vec<Option<Node>>> =
                    widths.iter().map(|w| vec![None; *w]).collect();
                for i in 0..n { ready[0][i] = Some(Node::Leaf(i)); }
                let mut n_local = 0usize;
                for l in 0..depth {
                    let npairs = widths[l] / 2;
                    // The partition, from the SHIPPED predicate -- a copy of the rule here would
                    // only prove the copy right.
                    let keep_local = join_stays_local(npairs, max);
                    let mut local_todo: Vec<(usize, usize, Node, Node)> = Vec::new();
                    let mut remote_q: Vec<(u32, Node, Node)> = Vec::new();
                    for p in 0..npairs {
                        let a = ready[l][p * 2].clone().expect("left child");
                        let b = ready[l][p * 2 + 1].clone().expect("right child");
                        if keep_local {
                            local_todo.push((l, p, a, b));
                        } else {
                            remote_q.push((JOIN_TAG | ((l as u32) << 16) | p as u32, a, b));
                        }
                    }
                    // local path: proved here, written straight into the level above
                    for (l, p, a, b) in local_todo {
                        ready[l + 1][p] = Some(Node::Join(Box::new(a), Box::new(b)));
                        n_local += 1;
                    }
                    // remote path: a worker answers out of order, so harvest by the TAG alone
                    remote_q.reverse();
                    for (tag, a, b) in remote_q {
                        let lvl = ((tag & !JOIN_TAG) >> 16) as usize;
                        let pos = (tag & 0xffff) as usize;
                        ready[lvl + 1][pos] = Some(Node::Join(Box::new(a), Box::new(b)));
                    }
                    if widths[l] % 2 == 1 {
                        ready[l + 1][npairs] = ready[l][widths[l] - 1].clone();
                    }
                }
                assert_eq!(ready[depth][0].clone().expect("root"), want,
                           "n={n} max={max}: the local/remote split changed the tree");
                // ⛔ and the split must actually have HAPPENED -- otherwise every iteration above
                // is the same all-remote tree and the loop over `max` proves nothing.
                let njoins: usize = (0..depth).map(|l| widths[l] / 2).sum();
                if max == 0 {
                    assert_eq!(n_local, 0, "n={n}: the default took joins locally");
                } else if max >= widths[0] {
                    assert_eq!(n_local, njoins, "n={n} max={max}: not every join was local");
                }
            }
        }
    }

    /// The lever must take the NARROW TOP and nothing else. Inverted (`>=`) it would keep the wide
    /// bottom levels -- serialising the only part of the tree that can use the fleet.
    #[test]
    fn local_join_takes_the_narrow_top_only() {
        // 512 leaves: pair counts run 256, 128, 64, 32, 16, 8, 4, 2, 1.
        let widths = join_tree_widths(512);
        let depth = widths.len() - 1;
        let pairs: Vec<usize> = (0..depth).map(|l| widths[l] / 2).collect();

        // Default: the lever is OFF and not one join is taken.
        assert!(pairs.iter().all(|&np| !join_stays_local(np, 0)), "max=0 must take nothing");

        // A taken set is correct when it is a contiguous run ending at the ROOT level, and no
        // level in it is wider than the ceiling.
        let top_anchored_and_narrow = |taken: &[usize], max: usize| -> bool {
            !taken.is_empty()
                && *taken.last().expect("taken") == depth - 1
                && taken.windows(2).all(|w| w[1] == w[0] + 1)
                && taken.iter().all(|&l| pairs[l] <= max)
        };

        for max in 1..=8usize {
            let taken: Vec<usize> =
                (0..depth).filter(|&l| join_stays_local(pairs[l], max)).collect();
            assert!(top_anchored_and_narrow(&taken, max),
                    "max={max} took {taken:?}, which is not the narrow top");

            // ⛔ POSITIVE CONTROL: the inverted rule takes the WIDE BOTTOM and must be rejected --
            // otherwise this test would pass just as happily on the bug it exists to catch.
            let inverted: Vec<usize> = (0..depth).filter(|&l| pairs[l] >= max).collect();
            assert!(!top_anchored_and_narrow(&inverted, max),
                    "control did not fail: `>=` took {inverted:?} and passed at max={max}");
        }

        // Monotonic: raising the ceiling never takes FEWER joins.
        let count = |max: usize| (0..depth).filter(|&l| join_stays_local(pairs[l], max)).count();
        for max in 1..8usize {
            assert!(count(max + 1) >= count(max), "raising max to {} took fewer levels", max + 1);
        }
    }

    /// Pipelining may change WHEN a join runs; it must never change WHICH two receipts meet, or in
    /// which order. `join` asserts `a.post == b.pre`, so joins chain claims and do not commute -- a
    /// carry-indexing error yields receipts that fail to verify, not merely a slower tree.
    #[test]
    fn pipelined_schedule_builds_an_identical_tree() {
        // 1..=2048 covers every parity pattern: powers of two, one either side, and the long runs
        // of odd levels that exercise repeated carries.
        for n in 1..=2048usize {
            assert_eq!(old_tree(n), new_tree(n), "join tree differs at n={n}");
        }
        // Real sizes seen in the fleet: a 116-segment aggregate, a 501-input chunk, 1,684 segments,
        // and block 962,000's full input count.
        for n in [116usize, 501, 1684, 8006] {
            assert_eq!(old_tree(n), new_tree(n), "join tree differs at n={n}");
        }
    }

    /// hazync#365. The deadline is sized from a peer's OWN round trips, and the numbers below are the
    /// real per-peer distributions from the first mode-6 run
    /// (docs/history/BENCH_MODE6_3xRTX4090_2026-09-17.md). Every one of those jobs RETURNED, so any
    /// window that would have killed them is a false positive on a healthy run.
    #[test]
    fn peer_deadline_never_clips_a_healthy_worker() {
        let (floor, mult, ceiling, min_n) = (600.0, 8.0, 1800.0, 16usize);

        // Too few samples: the floor governs. A window fitted to three observations is noise, and the
        // first jobs of a run are the least representative (cold CUDA, cold link, queue still filling).
        for n in 0..min_n {
            let few: Vec<f64> = std::iter::repeat(1.0).take(n).collect();
            assert_eq!(peer_job_timeout_s(&few, floor, mult, ceiling, min_n), floor,
                       "below min_samples the floor must govern, n={n}");
        }

        // pod 1, same host as the coordinator: p90 50.3 s -> 8x = 402 s, under the floor, so the floor
        // wins. A fast local worker must never get a TIGHTER deadline than a slow remote one.
        let pod1: Vec<f64> = (0..100).map(|i| 40.0 + (i as f64) * 0.15).collect();   // p90 ~53 s
        assert_eq!(peer_job_timeout_s(&pod1, floor, mult, ceiling, min_n), floor,
                   "a fast peer gets the floor, not a tighter window");

        // pod 2, tunnelled over a domestic uplink: p90 374.1 s, max 393.6 s -- all healthy. 8x p90
        // exceeds the ceiling, so it clamps. The clamp must still be comfortably ABOVE its worst
        // observed round trip, or the run this was measured from would have killed it.
        let pod2: Vec<f64> = (0..100).map(|i| if i < 90 { 79.0 + i as f64 } else { 374.0 + (i - 90) as f64 }).collect();
        let w2 = peer_job_timeout_s(&pod2, floor, mult, ceiling, min_n);
        assert_eq!(w2, ceiling, "a very slow peer clamps at the ceiling");
        assert!(w2 > 393.6, "the ceiling must exceed pod 2's worst HEALTHY round trip (393.6 s)");

        // pod 3, in between: p90 102.9 s -> 823 s, above the floor and below the ceiling, so the
        // adaptive term is what governs. This is the case the multiplier exists for.
        let pod3: Vec<f64> = (0..100).map(|i| 60.0 + (i as f64) * 0.5).collect();     // p90 ~105 s
        let w3 = peer_job_timeout_s(&pod3, floor, mult, ceiling, min_n);
        assert!(w3 > floor && w3 < ceiling, "a middling peer is governed by its own p90, got {w3}");

        // The adaptive term may only ever WIDEN. Slower peer, never a smaller window.
        assert!(peer_job_timeout_s(&pod3, floor, mult, ceiling, min_n)
                >= peer_job_timeout_s(&pod1, floor, mult, ceiling, min_n),
                "a slower peer must not get a tighter deadline than a faster one");

        // And nothing, however pathological, escapes the ceiling.
        let absurd: Vec<f64> = std::iter::repeat(100_000.0).take(50).collect();
        assert_eq!(peer_job_timeout_s(&absurd, floor, mult, ceiling, min_n), ceiling,
                   "a peer that has been slow all run cannot grow its own deadline without bound");
    }

    #[test]
    fn widths_terminate_at_one_and_shrink_monotonically() {
        for n in 1..=1024usize {
            let w = join_tree_widths(n);
            assert_eq!(w[0], n, "first width must be the leaf count");
            assert_eq!(*w.last().unwrap(), 1, "tree must terminate at a single root, n={n}");
            for pair in w.windows(2) {
                assert!(pair[1] < pair[0], "width must strictly shrink, n={n}: {pair:?}");
            }
        }
    }

    /// hazync#367. MEASURED 2026-09-17 on the run that validated #365: after the deadline dropped a
    /// quiet peer and requeued its 4 segments, `seg-serve` stayed alive and listening with nobody
    /// connected through t+30 s, t+60 s and t+90 s, and was still holding the port when the run was
    /// abandoned. These are the conditions under which giving up is right -- and, more importantly,
    /// the ones under which it would be WRONG.
    #[test]
    fn no_peers_timeout_fires_only_when_work_is_stranded() {
        let win = 600.0;

        // A connected worker is proving. However long the run takes and however deep the queue, a
        // coordinator with peers has not failed.
        for quiet in [0.0, 600.0, 86_400.0] {
            assert!(!no_peers_breached(1, 12, quiet, win), "a live peer must never trip it, quiet={quiet}");
            assert!(!no_peers_breached(3, 1, quiet, win), "three live peers must never trip it, quiet={quiet}");
        }

        // ⛔ THE DANGEROUS ONE. Once every segment is back and the join tree is being assembled
        // locally there are legitimately NO peers and nothing outstanding. Firing here would kill the
        // run seconds before it writes out a finished proof -- destroying hours of fleet time at the
        // moment of success.
        for quiet in [0.0, 600.0, 86_400.0] {
            assert!(!no_peers_breached(0, 0, quiet, win),
                    "no peers but nothing stranded is a FINISHING run, not a failure (quiet={quiet})");
        }

        // The measured case: 4 segments requeued, nobody connected. At the 90 s actually observed it
        // must NOT fire -- that is one hand-started worker away from finishing -- and past the window
        // it must, because `seg-connect` has no reconnect and nobody is coming back by themselves.
        assert!(!no_peers_breached(0, 4, 90.0, win), "90 s of quiet is well inside the 600 s grace");
        assert!(!no_peers_breached(0, 4, 600.0, win), "exactly at the window is not yet past it");
        assert!(no_peers_breached(0, 4, 600.1, win), "past the window with work stranded, give up");
        assert!(no_peers_breached(0, 1, 3600.0, win), "a single stranded job is still stranded");

        // The knob must actually move the boundary, or it is decoration.
        assert!(no_peers_breached(0, 4, 61.0, 60.0), "a tightened window fires sooner");
        assert!(!no_peers_breached(0, 4, 1000.0, 3600.0), "a widened window waits longer");
    }
}
