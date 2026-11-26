//! Proving Hazync's guest (the product guest built for the engine, engine/guest) from a RISC Zero input stream.
//!
//! stdin chunk 0 = control (emit, n, the deferral input commitment of each child proof, in the order the guest's
//! env::verify calls consume them); chunk 1 = the stream. See engine/platform/risc0-zkvm.
use std::path::{Path, PathBuf};

use eyre::{ensure, Result};
use openvm_sdk::StdIn;
use sha2::{Digest, Sha256};

use crate::engine::{read_proof, record_image, write_proof, Engine, Image};

/// The image id the guest's env::verify compares against: sha256(exe || vm), as 8 little-endian words' bytes.
pub fn image_id(img: &Image) -> [u8; 32] {
    let mut pre = Vec::with_capacity(64);
    pre.extend_from_slice(&img.exe_bytes());
    pre.extend_from_slice(&img.vm_bytes());
    Sha256::digest(&pre).into()
}

fn control(emit: bool, commits: &[[u8; 32]]) -> Vec<u8> {
    control_flags(emit as u32, commits)
}

/// flags: bit 0 emit (print the journal), bit 1 dry (no env::verify; can never commit; collect runs only).
fn control_flags(flags: u32, commits: &[[u8; 32]]) -> Vec<u8> {
    let mut c = Vec::with_capacity(8 + 32 * commits.len());
    c.extend_from_slice(&flags.to_le_bytes());
    c.extend_from_slice(&(commits.len() as u32).to_le_bytes());
    for x in commits {
        c.extend_from_slice(x);
    }
    c
}

/// Where the product host's streams carry their own image id (a mode word, then METHOD_ID).
pub const SELF_ID_AT: usize = 4;

/// This image's id into `stream` at byte `at` (32 bytes, the id's 8 words little-endian, as the guest reads them).
fn put_self_id(dir: &Path, stream: &mut [u8], at: usize) -> Result<()> {
    let img = Engine::image(dir)?;
    ensure!(at % 4 == 0 && stream.len() >= at + 32, "no room for a self id at byte {at} of a {}-byte stream", stream.len());
    stream[at..at + 32].copy_from_slice(&image_id(&img));
    Ok(())
}

pub fn prove(dir: &Path, input: &Path, children: &[PathBuf], self_id_at: Option<usize>, out: &Path) -> Result<()> {
    let e = Engine::load(dir)?;
    prove_with(&e, dir, input, children, self_id_at, out)
}

pub fn prove_with(e: &Engine, dir: &Path, input: &Path, children: &[PathBuf], self_id_at: Option<usize>, out: &Path) -> Result<()> {
    prove_with_known(e, dir, input, children, self_id_at, None, out)
}

/// `known`: the journal an execution of this same input committed (skips the emit pass; checked against the proof).
/// The image the child proofs are of: this one (`dir`), or HZ_CHILD_KEYS=<keys dir> for another image on the SAME VM
/// (a zyncAPP verifying a core proof). Children are decoded, checked and attached against that image's baseline; the
/// aggregation key is the VM's, so it is shared. Returns that dir and its baseline.
fn child_keys(dir: &Path, own: &openvm_verify_stark_host::vk::VerificationBaseline)
    -> Result<(std::path::PathBuf, openvm_verify_stark_host::vk::VerificationBaseline)> {
    match std::env::var_os("HZ_CHILD_KEYS") {
        None => Ok((dir.to_path_buf(), own.clone())),
        Some(d) => {
            let d = std::path::PathBuf::from(d);
            let b = Engine::baseline(&d)?;
            eprintln!("children: proofs of the image in {}", d.display());
            Ok((d, b))
        }
    }
}

pub fn prove_with_known(e: &Engine, dir: &Path, input: &Path, children: &[PathBuf], self_id_at: Option<usize>, known: Option<&Path>, out: &Path) -> Result<()> {
    let exe = Engine::exe(dir)?;
    let baseline = Engine::baseline(dir)?;
    let (child_dir, cbase) = child_keys(dir, &baseline)?;
    let mut stream = std::fs::read(input)?;
    let mut proofs = Vec::new();
    for c in children {
        proofs.push(read_proof(e, &cbase, c)?.0);
    }
    let calls = e.child_calls(&cbase, &proofs)?;
    if !calls.is_empty() {
        let img = Engine::image(&child_dir)?;
        for (c, p) in calls.iter().zip(children) {
            ensure!(c.exe == img.exe_bytes() && c.vm == img.vm_bytes(), "{}: not a proof of this image", p.display());
        }
    }
    if let Some(at) = self_id_at {
        // The product host writes its RISC Zero METHOD_ID at bytes 4..36 (mode word, then the id; SELF_ID_AT); a zAPP
        // stream names its own offset (utreexo: 0). The engine's id goes there.
        put_self_id(dir, &mut stream, at)?;
    }
    let commits: Vec<[u8; 32]> = calls.iter().map(|c| c.input_commit).collect();
    let mk = |emit| {
        let mut s = StdIn::default();
        s.write_bytes(&control(emit, &commits));
        s.write_bytes(&stream);
        s
    };
    let (mut emit, mut prv) = (mk(true), mk(false));
    e.attach_children(&cbase, &mut emit, &proofs)?;
    let defs = e.attach_children(&cbase, &mut prv, &proofs)?;
    let known = known.map(std::fs::read).transpose()?;
    let emit = if known.is_some() { None } else { Some(emit) };
    let (proof, journal) = e.prove_known(exe, emit, known, prv, &defs)?;
    if Engine::image(dir).is_err() {
        record_image(dir, e, &proof)?;
    }
    eprintln!("journal sha256 {}", hex::encode(Sha256::digest(&journal)));
    write_proof(out, proof, &journal)
}

/// Execute only (no proof). `children` are verified in-guest as in `prove`. Writes the guest's whole stdout to `log`
/// (a collect run's ECMH/coin-log/trie lines), the journal to `journal_out` when the run committed one. A run that ends
/// in env::exit(code) prints HZEXIT:<code>; that is reported as `exit <code>` and is not an error here.
pub struct ExecOut {
    pub exit: Option<String>,
    pub cost: u64,
    pub instret: u64,
}

pub fn execute(dir: &Path, input: &Path, children: &[PathBuf], self_id_at: Option<usize>, dry: bool, journal_out: Option<&Path>, log: Option<&Path>) -> Result<()> {
    let e = Engine::load(dir)?;
    let o = execute_with(&e, dir, input, children, self_id_at, dry, journal_out, log)?;
    match o.exit {
        None => println!("EXECUTE_OK"),
        Some(c) => println!("EXECUTE_EXIT {c}"),
    }
    Ok(())
}

pub fn execute_with(e: &Engine, dir: &Path, input: &Path, children: &[PathBuf], self_id_at: Option<usize>, dry: bool, journal_out: Option<&Path>, log: Option<&Path>) -> Result<ExecOut> {
    ensure!(!dry || children.is_empty(), "a dry run takes no proofs");
    let exe = Engine::exe(dir)?;
    let baseline = Engine::baseline(dir)?;
    let (_, cbase) = child_keys(dir, &baseline)?;
    let mut stream = std::fs::read(input)?;
    let mut proofs = Vec::new();
    for c in children {
        proofs.push(read_proof(e, &cbase, c)?.0);
    }
    let calls = e.child_calls(&cbase, &proofs)?;
    if let Some(at) = self_id_at {
        put_self_id(dir, &mut stream, at)?;
    }
    let commits: Vec<[u8; 32]> = calls.iter().map(|c| c.input_commit).collect();
    let mut s = StdIn::default();
    s.write_bytes(&control_flags(1 | if dry { 2 } else { 0 }, &commits));
    s.write_bytes(&stream);
    e.attach_children(&cbase, &mut s, &proofs)?;
    let t = std::time::Instant::now();
    if std::env::var("HZ_SEGMENTS").is_ok() {
        // how the prover would segment this run (HZ_SEG_MEM_GB changes it)
        // and what the segments cost: trace cells by circuit, as executed and padded to powers of two
        let (_, segs) = crate::capture::stdout_of(|| e.sdk.execute_metered(exe.clone(), s.clone())).0?;
        let pk = &e.sdk.app_pk().app_vm_pk.vm_pk;
        let w: Vec<u64> = pk.per_air.iter().map(|a| a.vk.params.width.total_width() as u64).collect();
        let it: Vec<u64> = pk.per_air.iter().map(|a| a.vk.symbolic_constraints.interactions.len() as u64).collect();
        let (mut un, mut pad, mut iun, mut ipad) = (0u64, 0u64, 0u64, 0u64);
        for sg in &segs {
            for (i, &h) in sg.trace_heights.iter().enumerate() {
                let h = h as u64;
                let p = if h == 0 { 0 } else { h.next_power_of_two() };
                un += h * w[i];
                pad += p * w[i];
                iun += h * it[i];
                ipad += p * it[i];
            }
        }
        eprintln!(
            "segments: {} | main cells {un} padded {pad} ({:.1}% padding) | interaction rows {iun} padded {ipad} ({:.1}%)",
            segs.len(),
            100.0 * (pad - un) as f64 / pad.max(1) as f64,
            100.0 * (ipad - iun) as f64 / ipad.max(1) as f64
        );
    }
    let (r, out) = crate::capture::stdout_of(|| e.sdk.execute_metered_cost(exe.clone(), s));
    if let Some(p) = log {
        std::fs::write(p, out.lines().filter(|l| !l.starts_with("HZJ:")).collect::<Vec<_>>().join("\n") + "\n")?;
    }
    let exit = out.lines().find_map(|l| l.strip_prefix("HZEXIT:")).map(|c| c.trim().to_string());
    match (r, exit) {
        (Ok((pvs, (cost, instret))), _) => {
            let journal = crate::capture::journal_of(&out);
            ensure!(pvs == Sha256::digest(&journal).as_slice(), "journal does not match the public values");
            eprintln!("execute: {:.1} s, cost {cost}, instructions {instret}, journal {} bytes sha256 {}",
                      t.elapsed().as_secs_f64(), journal.len(), hex::encode(Sha256::digest(&journal)));
            if let Some(p) = journal_out {
                std::fs::write(p, &journal)?;
            }
            Ok(ExecOut { exit: None, cost, instret })
        }
        (Err(_), Some(code)) => {
            eprintln!("execute: {:.1} s, guest exit {code} (no journal)", t.elapsed().as_secs_f64());
            Ok(ExecOut { exit: Some(code), cost: 0, instret: 0 })
        }
        (Err(err), None) => Err(err.into()),
    }
}
