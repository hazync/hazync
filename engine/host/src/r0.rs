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
    let mut c = Vec::with_capacity(8 + 32 * commits.len());
    c.extend_from_slice(&(emit as u32).to_le_bytes());
    c.extend_from_slice(&(commits.len() as u32).to_le_bytes());
    for x in commits {
        c.extend_from_slice(x);
    }
    c
}

pub fn prove(dir: &Path, input: &Path, children: &[PathBuf], set_self_id: bool, out: &Path) -> Result<()> {
    let e = Engine::load(dir)?;
    let exe = Engine::exe(dir)?;
    let baseline = Engine::baseline(dir)?;
    let mut stream = std::fs::read(input)?;
    let mut proofs = Vec::new();
    for c in children {
        proofs.push(read_proof(&e, &baseline, c)?.0);
    }
    let calls = e.child_calls(&baseline, &proofs)?;
    if !calls.is_empty() {
        let img = Engine::image(dir)?;
        for (c, p) in calls.iter().zip(children) {
            ensure!(c.exe == img.exe_bytes() && c.vm == img.vm_bytes(), "{}: not a proof of this image", p.display());
        }
    }
    if set_self_id {
        // The product host writes its RISC Zero METHOD_ID at bytes 4..36 (mode word, then the id): the engine's goes there.
        let img = Engine::image(dir)?;
        ensure!(stream.len() >= 36, "stream too short for a self id");
        stream[4..36].copy_from_slice(&image_id(&img));
    }
    let commits: Vec<[u8; 32]> = calls.iter().map(|c| c.input_commit).collect();
    let mk = |emit| {
        let mut s = StdIn::default();
        s.write_bytes(&control(emit, &commits));
        s.write_bytes(&stream);
        s
    };
    let (mut emit, mut prv) = (mk(true), mk(false));
    e.attach_children(&baseline, &mut emit, &proofs)?;
    let defs = e.attach_children(&baseline, &mut prv, &proofs)?;
    let (proof, journal) = e.prove(exe, emit, prv, &defs)?;
    if Engine::image(dir).is_err() {
        record_image(dir, &e, &proof)?;
    }
    eprintln!("journal sha256 {}", hex::encode(Sha256::digest(&journal)));
    write_proof(out, proof, &journal)
}

/// Execute only (no proof): the journal and the guest's metered cost, for checks and measurement.
pub fn execute(dir: &Path, input: &Path, journal_out: Option<&Path>) -> Result<()> {
    let e = Engine::load(dir)?;
    let exe = Engine::exe(dir)?;
    let stream = std::fs::read(input)?;
    let mut s = StdIn::default();
    s.write_bytes(&control(true, &[]));
    s.write_bytes(&stream);
    s.deferrals = vec![Default::default()];
    let t = std::time::Instant::now();
    let (r, out) = crate::capture::stdout_of(|| e.sdk.execute_metered_cost(exe.clone(), s));
    let (pvs, (cost, instret)) = r?;
    let journal = crate::capture::journal_of(&out);
    ensure!(pvs == Sha256::digest(&journal).as_slice(), "journal does not match the public values");
    for l in out.lines().filter(|l| !l.starts_with("HZJ:")).take(40) {
        eprintln!("  guest: {l}");
    }
    eprintln!("execute: {:.1} s, cost {cost}, instructions {instret}, journal {} bytes sha256 {}",
              t.elapsed().as_secs_f64(), journal.len(), hex::encode(Sha256::digest(&journal)));
    if let Some(p) = journal_out {
        std::fs::write(p, &journal)?;
    }
    Ok(())
}
