//! In-guest verification of this image's own proofs (OpenVM deferral, `openvm-verify-stark-guest`).
//!
//! The product's rule (S1): every verified child must be a proof of THIS image. A guest cannot know its own
//! executable commitment, so it takes it as input (`self_exe`, `self_vm`), requires every child to match it, and
//! commits it in its journal; the verifier checks the outer proof's commitment equals the published one AND the
//! journal's copy, which pins every level of the recursion to the real image.
use alloc::vec::Vec;

use openvm_deferral_guest::Commit;
use openvm_verify_stark_guest::verify_stark_unchecked;

/// Index of the verify-stark deferral circuit in the VM config (the only one).
const VERIFY_STARK: u16 = 0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Image {
    pub exe: Commit,
    pub vm: Commit,
}

impl Image {
    pub fn read() -> Self {
        Image { exe: openvm::io::read(), vm: openvm::io::read() }
    }
    pub fn bytes(&self) -> [u8; 64] {
        let mut b = [0u8; 64];
        b[..32].copy_from_slice(&self.exe);
        b[32..].copy_from_slice(&self.vm);
        b
    }
}

/// Verify one child proof of `image` whose journal is `journal`. `input_commit` names the proof among the
/// deferral inputs (host side); it binds nothing by itself — the returned output is what is checked.
pub fn verify_child(image: &Image, input_commit: &Commit, journal: &[u8]) {
    let out = verify_stark_unchecked::<VERIFY_STARK>(input_commit);
    assert!(out.app_exe_commit == image.exe, "child proof is not of this image (exe)");
    assert!(out.app_vm_commit == image.vm, "child proof is not of this image (vm)");
    let want = openvm_sha2::sha256(journal);
    assert!(out.user_public_values.as_slice() == want.as_slice(), "child journal does not match its proof");
}

/// Read a child: its deferral input commitment, then its journal bytes.
pub fn read_child() -> (Commit, Vec<u8>) {
    let c: Commit = openvm::io::read();
    let j = openvm::io::read_vec();
    (c, j)
}
