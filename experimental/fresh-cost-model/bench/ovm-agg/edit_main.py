#!/usr/bin/env python3
"""The only edits to Hazync's guest main.rs for the OpenVM port (see README.md)."""
import sys
p = sys.argv[1]; s = open(p).read()
def rep(old, new, n=1):
    global s
    assert s.count(old) == n, (old[:60], s.count(old)); s = s.replace(old, new)
# 1) cross-version handover verifier (old RISC Zero recursion circuits): not ported; mainnet's coin-log lineage is empty.
i = s.index("mod xver;")
rep("mod xver;", """mod xver {   // OpenVM port: cross-version handover NOT PORTED (mainnet coin-log lineage is empty, so never called)
    pub const fn carried(_v: &[u8; 32]) -> bool { false }
    pub mod accel { pub fn suite() {} }
    pub fn verify<A, B, C, D, E>(_: A, _: B, _: C, _: D, _: E) -> Result<(), &'static str> { panic!("xver not ported to OpenVM") }
}""")
# 2) batch_ecdsa: the OpenVM implementation (same public items)
rep('#[cfg(feature = "batch-ecdsa")]\nmod batch_ecdsa;', '#[cfg(feature = "batch-ecdsa")]\n#[path = "batch_ecdsa_ovm.rs"]\nmod batch_ecdsa;')
# 2b) liftx_hint (risc0's hinted pubkey lift, patch 0013): the chunk path calls it outside its feature gate. On OpenVM the
#     libsecp sqrt hook (HAZYNC_LIFTX_OVM) replaces it, so the module is inert here.
rep('mod liftx_hint;', 'mod liftx_hint;\n#[cfg(not(feature = "liftx-hint"))]\nmod liftx_hint {   // OpenVM port: inert (libsecp sqrt hook instead)\n    pub fn install(_: alloc::vec::Vec<([u8; 32], [u8; 32])>) {}\n    pub fn stats() -> (u32, u32) { (0, 0) }\n}')
# 3) reveal the journal hash at the end of main (the host prints the same "journal sha256" for RISC Zero)
i = s.index("\nfn main() {"); j = s.index("\n}\n", i)
s = s[:j] + "\n    risc0_zkvm::guest::env::finalize_journal();" + s[j:]
# 4) risc0's sys_sha_compress (Core's patches 0002/0009/0010 and libzync's coinlog call it) on OpenVM's SHA-256
#    extension. OpenVM's std PAL defines the same symbol as an unreachable!() stub; this crate's definition is linked
#    first (--allow-multiple-definition), as in bench/ovm-core.
s += """
extern "C" { fn zkvm_sha256_impl(state: *const u8, input: *const u8, output: *mut u8); }
/// OpenVM port: RISC Zero's sys_sha_compress contract (state words byte-swapped from native, block as two halves).
#[no_mangle]
pub unsafe extern "C" fn sys_sha_compress(out: *mut u32, inp: *const u32, b1: *const u32, b2: *const u32) {
    let mut st = [0u32; 8]; for i in 0..8 { st[i] = (*inp.add(i)).swap_bytes(); }
    let mut blk = [0u8; 64];
    core::ptr::copy_nonoverlapping(b1 as *const u8, blk.as_mut_ptr(), 32);
    core::ptr::copy_nonoverlapping(b2 as *const u8, blk.as_mut_ptr().add(32), 32);
    let mut o = [0u32; 8];
    zkvm_sha256_impl(st.as_ptr() as *const u8, blk.as_ptr(), o.as_mut_ptr() as *mut u8);
    for i in 0..8 { *out.add(i) = o[i].swap_bytes(); }
}
"""
open(p, "w").write(s); print("main.rs edited")
