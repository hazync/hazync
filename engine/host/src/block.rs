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

/// `workers` > 1: the chunk and trie proofs run on that many `hzengine serve` processes at once (sharing the GPU),
/// so one proof's host-side work overlaps another's GPU work; the aggregate then runs here, alone on the GPU (its
/// segment cap is this process's: share 1).
fn leaves_parallel(dir: &Path, export: &Path, out: &Path, n: usize, workers: usize, report: &mut String) -> Result<()> {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};
    let me = std::env::current_exe()?;
    let mut jobs: Vec<(String, PathBuf)> = (0..n).map(|i| (format!("chunk_{i:02}"), export.join(format!("chunk_{i:02}.in")))).collect();
    jobs.push(("trie".into(), export.join("trie.in")));
    let next = std::sync::atomic::AtomicUsize::new(0);
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
    std::thread::scope(|sc| {
        for k in 0..workers {
            let (me, next, lines, err, jobs, devs) = (&me, &next, &lines, &err, &jobs, &devs);
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
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((name, input)) = jobs.get(i) else { break };
                    let t = Instant::now();
                    let req = serde_json::json!({"op": "prove", "input": input, "children": [], "self_id": false, "out": out.join(name)});
                    let _ = writeln!(stdin, "{req}").and_then(|_| stdin.flush());
                    let mut r = String::new();
                    rd.read_line(&mut r).ok();
                    if !r.contains("\"ok\":true") { *err.lock().unwrap() = Some(format!("{name}: {r}")); break; }
                    let journal = std::fs::read(crate::engine::with_ext(&out.join(name), "journal")).unwrap_or_default();
                    let r0 = std::fs::read(input.with_extension("r0journal")).ok();
                    let same = r0.map(|r| r == journal);
                    if same == Some(false) { *err.lock().unwrap() = Some(format!("{name}: journal differs from RISC Zero's")); }
                    lines.lock().unwrap().push((i, format!("{name}: proved {:.1} s (worker), journal {} bytes sha256 {} r0 {}\n",
                        t.elapsed().as_secs_f64(), journal.len(), &hex::encode(Sha256::digest(&journal))[..16],
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
    let mut children = Vec::new();
    let workers = workers.min(n + 1); // no more provers than leaf proofs (a 1-chunk block has 2)
    if workers > 1 {
        leaves_parallel(dir, export, out, n, workers, &mut report)?;
        let names: Vec<String> = (0..n).map(|i| format!("chunk_{i:02}")).chain(["trie".to_string()]).collect();
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
    let (proof, journal) = e.prove(exe, emit, prv, &defs)?;
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
