//! One block, end to end, in one process: every chunk, the trie update, then the aggregate verifying all of them
//! in-guest (env::verify -> deferred STARK verification). Inputs are the product host's `engine-export <dir>`.
//! Each journal is compared with RISC Zero's for the same stream (identical except the aggregate's image ids).
use std::path::{Path, PathBuf};
use std::time::Instant;

use eyre::{ensure, Result};
use openvm_sdk::{Sdk, StdIn};
use sha2::{Digest, Sha256};

use crate::engine::{record_image, write_proof, Engine};
use crate::r0::image_id;

fn control(emit: bool, commits: &[[u8; 32]]) -> Vec<u8> {
    let mut c = Vec::new();
    c.extend_from_slice(&(emit as u32).to_le_bytes());
    c.extend_from_slice(&(commits.len() as u32).to_le_bytes());
    for x in commits {
        c.extend_from_slice(x);
    }
    c
}

/// A `serve` process on one GPU for the aggregate's segments (segpar), started early so its keys are loaded by the
/// time the aggregate needs it. Share 1, as the aggregate's own process: every side must cut the same segments.
pub struct Worker {
    io: std::sync::Mutex<(std::process::ChildStdin, std::io::BufReader<std::process::ChildStdout>, bool)>,
    child: std::sync::Mutex<std::process::Child>,
}
impl Worker {
    pub fn spawn(dir: &Path, dev: &str) -> Result<Worker> {
        use std::process::{Command, Stdio};
        let mut ch = Command::new(std::env::current_exe()?)
            .args(["serve", "--dir"]).arg(dir)
            .env("HZ_GPU_SHARE", "1").env("CUDA_VISIBLE_DEVICES", dev)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn()?;
        let (i, o) = (ch.stdin.take().unwrap(), std::io::BufReader::new(ch.stdout.take().unwrap()));
        Ok(Worker { io: std::sync::Mutex::new((i, o, false)), child: std::sync::Mutex::new(ch) })
    }
    /// Segments lo..hi of the program on the serialized stdin at `input`, their proofs to `out`; returns how many
    /// segments the worker cut in all.
    pub fn prove_segments(&self, input: &Path, lo: usize, hi: usize, out: &Path) -> Result<usize> {
        use std::io::{BufRead, Write};
        let mut g = self.io.lock().unwrap();
        let (stdin, rd, ready) = &mut *g;
        if !*ready {
            let mut l = String::new();
            rd.read_line(&mut l)?;
            ensure!(l.contains("\"ready\":true"), "segment worker did not start: {l}");
            *ready = true;
        }
        let req = serde_json::json!({"op": "prove_segments", "input": input, "lo": lo, "hi": hi, "out": out});
        writeln!(stdin, "{req}")?;
        stdin.flush()?;
        let mut r = String::new();
        rd.read_line(&mut r)?;
        let v: serde_json::Value = serde_json::from_str(&r).map_err(|err| eyre::eyre!("segment worker reply {r:?}: {err}"))?;
        ensure!(v["ok"] == true, "segment worker: {r}");
        Ok(v["segments"].as_u64().unwrap_or(0) as usize)
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let mut c = self.child.lock().unwrap();
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// Segment workers for the aggregate: one per GPU other than this process's first (HZ_SEGPAR=0: none).
fn segment_workers(dir: &Path) -> Vec<Worker> {
    if std::env::var("HZ_SEGPAR").is_ok_and(|v| v == "0") { return vec![]; }
    let gpus = crate::engine::gpu_count();
    let devs: Vec<String> = match std::env::var("CUDA_VISIBLE_DEVICES") {
        Ok(v) => v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
        Err(_) => (0..gpus).map(|g| g.to_string()).collect(),
    };
    devs.iter().skip(1).filter_map(|d| Worker::spawn(dir, d).map_err(|err| eprintln!("segpar: worker on {d}: {err}")).ok()).collect()
}

/// One leaf-level proof for `leaves_parallel`: `children` (names of earlier jobs, verified in-guest in that order) and
/// `self_id` as the serve request's; it starts once every job in `deps` is proved.
struct Job { name: String, input: PathBuf, children: Vec<String>, self_id: bool, deps: Vec<usize> }

/// The chunk and trie proofs on `workers` `hzengine serve` processes at once, spread over the visible GPUs, so one
/// proof's host-side work overlaps another's GPU work; the aggregate then runs here, alone on GPU 0 (its segment cap is
/// this process's: share 1). Each worker takes the first job whose dependencies are done (a split trie's combine
/// waits for its parts and goes before the chunks still queued).
fn leaves_parallel(dir: &Path, out: &Path, jobs: &[Job], workers: usize, report: &mut String) -> Result<()> {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};
    let me = std::env::current_exe()?;
    // 0 queued, 1 running, 2 done
    let state = std::sync::Mutex::new(vec![0u8; jobs.len()]);
    let gpus = crate::engine::gpu_count();
    // provers per card: workers spread round-robin, so a card holds ceil(workers / gpus) of them
    let share = workers.div_ceil(gpus);
    // device names as this process sees them (a CUDA_VISIBLE_DEVICES subset keeps its own ids)
    let devs: Vec<String> = match std::env::var("CUDA_VISIBLE_DEVICES") {
        Ok(v) => v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
        Err(_) => (0..gpus).map(|g| g.to_string()).collect(),
    };
    let lines = std::sync::Mutex::new(Vec::<(usize, String)>::new());
    let err = std::sync::Mutex::new(None::<String>);
    let t0 = Instant::now();
    std::thread::scope(|sc| {
        for k in 0..workers {
            let (me, state, lines, err, devs) = (&me, &state, &lines, &err, &devs);
            sc.spawn(move || {
                let mut cmd = Command::new(me);
                cmd.args(["serve", "--dir"]).arg(dir).env("HZ_GPU_SHARE", share.to_string());
                if gpus > 1 {
                    cmd.env("CUDA_VISIBLE_DEVICES", &devs[k % devs.len()]);
                }
                let mut ch = match cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).spawn() {
                    Ok(c) => c, Err(e) => { *err.lock().unwrap() = Some(format!("spawn serve: {e}")); return; }
                };
                let mut stdin = ch.stdin.take().unwrap();
                let mut rd = BufReader::new(ch.stdout.take().unwrap());
                let mut l = String::new();
                rd.read_line(&mut l).ok();
                loop {
                    if err.lock().unwrap().is_some() { break; }
                    let pick = {
                        let mut st = state.lock().unwrap();
                        if st.iter().all(|&x| x != 0) { break; }
                        let i = (0..jobs.len()).find(|&i| st[i] == 0 && jobs[i].deps.iter().all(|&d| st[d] == 2));
                        if let Some(i) = i { st[i] = 1; }
                        i
                    };
                    let Some(i) = pick else { std::thread::sleep(std::time::Duration::from_millis(100)); continue };
                    let j = &jobs[i];
                    let t = Instant::now();
                    let children: Vec<PathBuf> = j.children.iter().map(|c| out.join(c)).collect();
                    let req = serde_json::json!({"op": "prove", "input": j.input, "children": children, "self_id": j.self_id, "out": out.join(&j.name)});
                    let _ = writeln!(stdin, "{req}").and_then(|_| stdin.flush());
                    let mut r = String::new();
                    rd.read_line(&mut r).ok();
                    if !r.contains("\"ok\":true") { *err.lock().unwrap() = Some(format!("{}: {r}", j.name)); break; }
                    let journal = std::fs::read(crate::engine::with_ext(&out.join(&j.name), "journal")).unwrap_or_default();
                    let r0 = std::fs::read(j.input.with_extension("r0journal")).ok();
                    let same = r0.map(|r| r == journal);
                    if same == Some(false) { *err.lock().unwrap() = Some(format!("{}: journal differs from RISC Zero's", j.name)); }
                    state.lock().unwrap()[i] = 2;
                    lines.lock().unwrap().push((i, format!("{}: proved {:.1} s (worker {k}, done at {:.1} s), journal {} bytes sha256 {} r0 {}\n",
                        j.name, t.elapsed().as_secs_f64(), t0.elapsed().as_secs_f64(), journal.len(), &hex::encode(Sha256::digest(&journal))[..16],
                        match same { Some(true) => "IDENTICAL", Some(false) => "DIFFERENT", None => "n/a" })));
                }
                drop(stdin);
                let _ = ch.wait();
            });
        }
    });
    if let Some(e) = err.into_inner().unwrap() { eyre::bail!("parallel leaves: {e}"); }
    let mut ls = lines.into_inner().unwrap();
    ls.sort();
    for (_, l) in ls { eprint!("{l}"); report.push_str(&l); }
    Ok(())
}

pub fn prove_block(dir: &Path, export: &Path, out: &Path, verify_each: bool, workers: usize) -> Result<()> {
    std::fs::create_dir_all(out)?;
    let e = Engine::load(dir)?;
    let exe = Engine::exe(dir)?;
    let baseline = Engine::baseline(dir)?;
    let n: usize = std::fs::read_to_string(export.join("chunks"))?.trim().parse()?;
    let t_all = Instant::now();
    let mut report = String::new();
    let mut leaf = |name: String, input: PathBuf, report: &mut String| -> Result<(openvm_verify_stark_host::VmStarkProof, Vec<u8>)> {
        let stream = std::fs::read(&input)?;
        let mk = |emit| {
            let mut s = StdIn::default();
            s.write_bytes(&control(emit, &[]));
            s.write_bytes(&stream);
            s.deferrals = vec![Default::default()];
            s
        };
        let t = Instant::now();
        let (proof, journal) = e.prove(exe.clone(), mk(true), mk(false), &[Default::default()])?;
        let secs = t.elapsed().as_secs_f64();
        let r0 = std::fs::read(input.with_extension("r0journal")).ok();
        let same = r0.as_ref().map(|r| *r == journal);
        let line = format!("{name}: proved {secs:.1} s, journal {} bytes sha256 {} r0 {}\n", journal.len(),
                           &hex::encode(Sha256::digest(&journal))[..16],
                           match same { Some(true) => "IDENTICAL", Some(false) => "DIFFERENT", None => "n/a" });
        eprint!("{line}");
        report.push_str(&line);
        ensure!(same != Some(false), "{name}: journal differs from RISC Zero's");
        if verify_each {
            Sdk::verify_proof(e.agg_vk.as_ref().clone(), baseline.clone(), &proof)?;
        }
        write_proof(&out.join(&name), proof.clone(), &journal)?;
        Ok((proof, journal))
    };
    // the aggregate's segment workers, loading their keys while the leaves prove
    let segw = segment_workers(dir);
    let mut children = Vec::new();
    // a split trie update (engine-export with HAZYNC_TRIE_SPLIT): parts, then a combine verifying them, in place of trie
    let parts: usize = match std::fs::read_to_string(export.join("trie_parts")) {
        Ok(t) => t.trim().parse()?,
        Err(_) => 0,
    };
    let mut jobs: Vec<Job> = Vec::new();
    // A split update goes to the aggregate as its parts (ttree::KIND_TRIE_PARTS): no combine proof, the parts prove
    // beside the chunks. HZ_TRIE_COMBINE=1: the combine proof (mode 20) as before.
    let combine = parts > 0 && std::env::var("HZ_TRIE_COMBINE").is_ok_and(|v| v == "1");
    let pn: Vec<String> = (0..parts).map(|i| format!("trie_part_{i:02}")).collect();
    let trie_names: Vec<String> = if parts > 0 {
        for p in &pn {
            jobs.push(Job { name: p.clone(), input: export.join(format!("{p}.in")), children: vec![], self_id: false, deps: vec![] });
        }
        if combine {
            jobs.push(Job { name: "trie_combine".into(), input: export.join("trie_combine.in"), children: pn.clone(), self_id: true, deps: (0..parts).collect() });
            vec!["trie_combine".into()]
        } else {
            pn.clone()
        }
    } else {
        // the whole update first: it is the largest leaf (976,379: 6.3B against ~3.4B per chunk), so starting it
        // first keeps it off the end of the leaf phase
        jobs.push(Job { name: "trie".into(), input: export.join("trie.in"), children: vec![], self_id: false, deps: vec![] });
        vec!["trie".into()]
    };
    for i in 0..n {
        jobs.push(Job { name: format!("chunk_{i:02}"), input: export.join(format!("chunk_{i:02}.in")), children: vec![], self_id: false, deps: vec![] });
    }
    let workers = workers.min(jobs.len()); // no more provers than leaf proofs (a 1-chunk block has 2)
    if workers > 1 || parts > 0 {
        leaves_parallel(dir, out, &jobs, workers, &mut report)?;
        let names: Vec<String> = (0..n).map(|i| format!("chunk_{i:02}")).chain(trie_names.iter().cloned()).collect();
        for nm in names {
            children.push(crate::engine::read_proof(&e, &baseline, &out.join(nm))?.0);
        }
    } else {
        for i in 0..n {
            let (p, _) = leaf(format!("chunk_{i:02}"), export.join(format!("chunk_{i:02}.in")), &mut report)?;
            if i == 0 && Engine::image(dir).is_err() {
                record_image(dir, &e, &p)?;
            }
            children.push(p);
        }
        let (tp, _) = leaf("trie".into(), export.join("trie.in"), &mut report)?;
        children.push(tp);
    }
    let t_leaves = t_all.elapsed().as_secs_f64();

    // the aggregate: this image's id where the product host wrote its METHOD_ID, then every child verified in-guest
    let img = Engine::image(dir)?;
    let mut stream = std::fs::read(export.join("agg.in"))?;
    let r0id = std::fs::read(export.join("method_id"))?;
    ensure!(stream[4..36] == r0id[..], "agg.in: METHOD_ID not at bytes 4..36");
    stream[4..36].copy_from_slice(&image_id(&img));
    if parts > 0 && !combine {
        // The aggregate reads the trie journal last ([word count] + words): the export wrote the whole update's (RISC
        // Zero's mode 17); the parts payload goes there (ttree::KIND_TRIE_PARTS): the combine's witness and order from
        // its export input, then each part's journal as this engine proved it (each identical to RISC Zero's).
        let tj = std::fs::read(export.join("trie.r0journal"))?;
        let tail = 4 + tj.len();
        ensure!(stream.len() >= tail && stream[stream.len() - tail..stream.len() - tj.len()] == ((tj.len() / 4) as u32).to_le_bytes()
                && stream[stream.len() - tj.len()..] == tj[..], "agg.in does not end with the trie journal");
        let words = |b: &[u8]| -> Vec<u32> { b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect() };
        // trie_combine.in (prover/host trie_split): mode 20, image id (8 words), [nw, nm, ni, np, nparts], witness,
        // marks x24, inserts x27, order, part journals
        let ci = words(&std::fs::read(export.join("trie_combine.in"))?);
        ensure!(ci.len() >= 14 && ci[0] == 20, "trie_combine.in: not a combine stream");
        let (nw, nm, ni, np, nparts) = (ci[9] as usize, ci[10] as usize, ci[11] as usize, ci[12] as usize, ci[13] as usize);
        let (ws, ps) = (14, 14 + nw + 24 * nm + 27 * ni);
        ensure!(nparts == parts && ci.len() == ps + np + 34 * nparts, "trie_combine.in: layout");
        let mut pj = Vec::with_capacity(parts);
        for p in &pn {
            let j = words(&std::fs::read(out.join(format!("{p}.journal")))?);
            ensure!(j.len() == 34, "{p}: not a part journal");
            pj.push(<[u32; 34]>::try_from(&j[..]).unwrap());
        }
        let mut payload = vec![0xC4A1_0021u32, nw as u32, np as u32, parts as u32];   // ttree::KIND_TRIE_PARTS
        payload.extend_from_slice(&ci[ws..ws + nw]);
        payload.extend_from_slice(&ci[ps..ps + np]);
        for j in &pj { payload.extend_from_slice(j); }
        stream.truncate(stream.len() - tail);
        stream.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        for w in &payload { stream.extend_from_slice(&w.to_le_bytes()); }
        let line = format!("trie split: {parts} parts verified by the aggregate (no combine proof), payload {} words\n", payload.len());
        eprint!("{line}");
        report.push_str(&line);
    }
    if combine {
        // The aggregate reads the trie proof's journal last ([word count] + words): the export wrote the whole update's
        // (RISC Zero's mode 17); the combine's goes there. Same old root, new root and list digest, then this image's id.
        let tj = std::fs::read(export.join("trie.r0journal"))?;
        let cj = std::fs::read(out.join("trie_combine.journal"))?;
        let tail = 4 + tj.len();
        ensure!(stream.len() >= tail && stream[stream.len() - tail..stream.len() - tj.len()] == ((tj.len() / 4) as u32).to_le_bytes()
                && stream[stream.len() - tj.len()..] == tj[..], "agg.in does not end with the trie journal");
        ensure!(cj.len() == tj.len() + 32 && cj[4..tj.len()] == tj[4..] && cj[tj.len()..] == image_id(&img)[..],
                "the combine's journal is not the whole update's (old root, new root, list) with this image's id");
        stream.truncate(stream.len() - tail);
        stream.extend_from_slice(&((cj.len() / 4) as u32).to_le_bytes());
        stream.extend_from_slice(&cj);
        let line = format!("trie split: {parts} parts + combine; combine journal = RISC Zero's whole update (old root, new root, list) + this image's id\n");
        eprint!("{line}");
        report.push_str(&line);
    }
    let calls = e.child_calls(&baseline, &children)?;
    for c in &calls {
        ensure!(c.exe == img.exe_bytes() && c.vm == img.vm_bytes(), "a child is not of this image");
    }
    let commits: Vec<[u8; 32]> = calls.iter().map(|c| c.input_commit).collect();
    let mk = |emit| {
        let mut s = StdIn::default();
        s.write_bytes(&control(emit, &commits));
        s.write_bytes(&stream);
        s
    };
    let (mut emit, mut prv) = (mk(true), mk(false));
    e.attach_children(&baseline, &mut emit, &children)?;
    let defs = e.attach_children(&baseline, &mut prv, &children)?;
    let t = Instant::now();
    let (proof, journal) = if segw.is_empty() {
        e.prove(exe, emit, prv, &defs)?
    } else {
        let line = format!("aggregate: segments spread over this process and {} segment workers (segpar)\n", segw.len());
        eprint!("{line}");
        report.push_str(&line);
        e.prove_split(exe, emit, prv, &defs, &out.join("agg.stdin"), &segw)?
    };
    drop(segw);
    let t_agg = t.elapsed().as_secs_f64();
    let t = Instant::now();
    Sdk::verify_proof(e.agg_vk.as_ref().clone(), baseline.clone(), &proof)?;
    let t_ver = t.elapsed().as_secs_f64();
    let r0 = std::fs::read(export.join("agg.r0journal")).ok();
    let diff = r0.as_ref().map(|r| (r.len() == journal.len(), r.iter().zip(&journal).filter(|(a, b)| a != b).count()));
    let line = format!(
        "aggregate ({} children verified in-guest): proved {t_agg:.1} s, journal {} bytes sha256 {}; vs RISC Zero: {:?} (same length, differing bytes)\n\
         block: leaves {t_leaves:.1} s + aggregate {t_agg:.1} s = {:.1} s; final proof verified in {t_ver:.2} s\n",
        calls.len(), journal.len(), hex::encode(Sha256::digest(&journal)), diff, t_all.elapsed().as_secs_f64());
    eprint!("{line}");
    report.push_str(&line);
    write_proof(&out.join("block"), proof, &journal)?;
    std::fs::write(out.join("REPORT.txt"), report)?;
    Ok(())
}
