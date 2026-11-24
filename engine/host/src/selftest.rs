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
