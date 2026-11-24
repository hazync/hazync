//! M1 self-test: leaf proofs and a node verifying two of them in-guest.
use std::path::Path;

use eyre::{ensure, Result};
use openvm_sdk::StdIn;

use crate::engine::{read_proof, record_image, write_proof, Engine};

const MODE_LEAF: u32 = 100;
const MODE_NODE: u32 = 101;

fn stdin_head(mode: u32, emit: bool) -> StdIn {
    let mut s = StdIn::default();
    s.write(&mode);
    s.write(&(emit as u32));
    s
}

pub fn leaf(dir: &Path, value: u64, out: &Path) -> Result<()> {
    let e = Engine::load(dir)?;
    let exe = Engine::exe(dir)?;
    let baseline = Engine::baseline(dir)?;
    let mk = |emit| {
        let mut s = stdin_head(MODE_LEAF, emit);
        s.write(&value);
        s
    };
    let (mut emit, mut prove) = (mk(true), mk(false));
    let defs = e.attach_children(&baseline, &mut prove, &[])?;
    e.attach_children(&baseline, &mut emit, &[])?;
    let (proof, journal) = e.prove(exe, emit, prove, &defs)?;
    if Engine::image(dir).is_err() {
        record_image(dir, &e, &proof)?;
    }
    write_proof(out, proof, &journal)
}

/// Negative controls (execute only; the guest must refuse): `tamper` = "journal" flips a byte of the left child's
/// journal, "image" flips a byte of the image the node is told it is. The host's own pre-checks are skipped.
pub fn negative(dir: &Path, left: &Path, right: &Path, tamper: &str) -> Result<()> {
    let e = Engine::load(dir)?;
    let exe = Engine::exe(dir)?;
    let baseline = Engine::baseline(dir)?;
    let img = Engine::image(dir)?;
    let (lp, mut lj) = read_proof(&e, &baseline, left)?;
    let (rp, rj) = read_proof(&e, &baseline, right)?;
    let mut exe_c = img.exe_bytes();
    match tamper {
        "journal" => lj[70] ^= 1,
        "image" => exe_c[5] ^= 1,
        _ => eyre::bail!("tamper: journal | image"),
    }
    let children = vec![lp, rp];
    let calls = e.child_calls(&baseline, &children)?;
    let mut s = stdin_head(MODE_NODE, false);
    s.write(&exe_c);
    s.write(&img.vm_bytes());
    s.write(&calls[0].input_commit);
    s.write_bytes(&lj);
    s.write(&calls[1].input_commit);
    s.write_bytes(&rj);
    e.attach_children(&baseline, &mut s, &children)?;
    match e.sdk.execute(exe, s) {
        Ok(_) => eyre::bail!("NEGATIVE CONTROL FAILED: the guest accepted a tampered {tamper}"),
        Err(err) => {
            eprintln!("negative control ({tamper}): refused as it must be: {err}");
            Ok(())
        }
    }
}

pub fn node(dir: &Path, left: &Path, right: &Path, out: &Path) -> Result<()> {
    let e = Engine::load(dir)?;
    let exe = Engine::exe(dir)?;
    let baseline = Engine::baseline(dir)?;
    let img = Engine::image(dir)?;
    let (lp, lj) = read_proof(&e, &baseline, left)?;
    let (rp, rj) = read_proof(&e, &baseline, right)?;
    let children = vec![lp, rp];
    let calls = e.child_calls(&baseline, &children)?;
    for c in &calls {
        ensure!(c.exe == img.exe_bytes() && c.vm == img.vm_bytes(), "child is not of this image");
    }
    let mk = |emit| {
        let mut s = stdin_head(MODE_NODE, emit);
        s.write(&img.exe_bytes());
        s.write(&img.vm_bytes());
        s.write(&calls[0].input_commit);
        s.write_bytes(&lj);
        s.write(&calls[1].input_commit);
        s.write_bytes(&rj);
        s
    };
    let (mut emit, mut prove) = (mk(true), mk(false));
    e.attach_children(&baseline, &mut emit, &children)?;
    let defs = e.attach_children(&baseline, &mut prove, &children)?;
    let (proof, journal) = e.prove(exe, emit, prove, &defs)?;
    write_proof(out, proof, &journal)
}
