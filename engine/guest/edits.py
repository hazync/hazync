#!/usr/bin/env python3
"""edits.py <assembled guest main.rs>: the ONLY source edits the engine makes to Hazync's guest (README.md).
Each asserts its anchor occurs exactly once, so a product change that moves one fails here, by name."""
import sys
p = sys.argv[1]; s = open(p).read()
def rep(old, new):
    global s
    assert s.count(old) == 1, ("anchor not unique / missing", old[:70], s.count(old))
    s = s.replace(old, new)
# 1) Cross-version handover (vendored old RISC Zero recursion circuits): an engine chain starts at genesis and has no
#    RISC Zero predecessor, so this path must be unreachable. The stub panics if called; `carried` says no.
rep("mod xver;", """mod xver {   // ENGINE: no RISC Zero predecessor; cross-version handover is unreachable
    pub const fn carried(_v: &[u8; 32]) -> bool { false }
    pub mod accel { pub fn suite() {} }
    pub fn verify<A, B, C, D, E>(_: A, _: B, _: C, _: D, _: E) -> Result<(), &'static str> { panic!("engine: no cross-version handover") }
}""")
# 2) Batch signature check on OpenVM's secp256k1 extension (same public items, hint layout, transcript, equation).
rep('#[cfg(feature = "batch-ecdsa")]\nmod batch_ecdsa;', '#[cfg(feature = "batch-ecdsa")]\n#[path = "batch_ecdsa_ovm.rs"]\nmod batch_ecdsa;')
# 3) risc0's hinted pubkey lift (patch 0013) is replaced by libsecp's sqrt hook (HAZYNC_LIFTX_OVM); the chunk path
#    calls the module outside its feature gate, so an inert one stands in.
rep('mod liftx_hint;', 'mod liftx_hint;\n#[cfg(not(feature = "liftx-hint"))]\nmod liftx_hint {   // ENGINE: inert (libsecp sqrt hook instead)\n    pub fn install(_: alloc::vec::Vec<([u8; 32], [u8; 32])>) {}\n    pub fn stats() -> (u32, u32) { (0, 0) }\n}')
# 4) The journal's hash becomes the proof's public values, once, at the end of main.
i = s.index("\nfn main() {"); j = s.index("\n}\n", i)
s = s[:j] + "\n    risc0_zkvm::guest::env::finalize();" + s[j:]
# 5) RISC Zero's sys_sha_compress (Core patches 0002/0009/0010/0101 and libzync's coin log call it) on OpenVM's SHA-256
#    extension. OpenVM's std PAL defines the same symbol as an unreachable!() stub; this definition is linked first
#    (--allow-multiple-definition in build.rs).
s += """
extern "C" { fn zkvm_sha256_impl(state: *const u8, input: *const u8, output: *mut u8); }
/// ENGINE: RISC Zero's sys_sha_compress contract (state words byte-swapped from native, block as two 32-byte halves).
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
/// ENGINE: libsecp's sqrt hook (HAZYNC_LIFTX_OVM, secp-0201): square root mod p on OpenVM's modular extension (hinted,
/// checked by squaring in the extension). 1 = a square (out set), 0 = not a square.
#[no_mangle]
pub unsafe extern "C" fn hz_fe_sqrt(a_be: *const u8, out_be: *mut u8) -> i32 {
    use openvm_algebra_guest::{IntMod, Sqrt};
    let a = core::slice::from_raw_parts(a_be, 32);
    let Some(x) = k256::Secp256k1Coord::from_be_bytes(a) else { return 0 };
    match x.sqrt() { Some(y) => { core::slice::from_raw_parts_mut(out_be, 32).copy_from_slice(&y.to_be_bytes()); 1 } None => 0 }
}
openvm::init!();
"""
open(p, "w").write(s); print("engine edits applied:", p)
