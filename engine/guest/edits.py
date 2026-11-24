#!/usr/bin/env python3
"""edits.py <assembled guest main.rs> <assembled libzync/src/coinlog.rs>: the ONLY source edits the engine makes to Hazync's guest (README.md).
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
# 5) SHA-256 in OpenVM's native layout: Core and libsecp call zkvm_sha256_impl directly (core-0201 / secp-0202, laid
#    out by base.sh) and so does the coin log (edit 7). The engine defines no sys_sha_compress (RISC Zero's syscall,
#    state byte-swapped); OpenVM's std PAL keeps its own unreachable!() stub of that name, which nothing calls.
s += """
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

# 7) libzync's coin log (libzync/src/coinlog.rs) on OpenVM's SHA-256 in its native layout. The product calls RISC Zero's
#    sys_sha_compress with pre-swapped IVs (IV_NODE_W / IV_SPENT_W) and gets the digest back as its in-memory bytes (W).
#    Here the IV is the native state (IV_NODE / IV_SPENT as they are), the block is L || R as raw bytes (W is exactly
#    that), and the native output state is swapped once into W (the digest's big-endian bytes). Same bytes out.
p = sys.argv[2]; s = open(p).read()
rep('''/// The IVs as the accelerator's state words (the state's big-endian bytes in memory order).
#[cfg(target_os = "zkvm")]
static IV_NODE_W: [[u32; 8]; 64] = swapped(&IV_NODE);
#[cfg(target_os = "zkvm")]
static IV_SPENT_W: [u32; 8] = swapped(&[IV_SPENT])[0];
''', '')
rep('''#[cfg(target_os = "zkvm")]
const fn swapped<''', '''#[cfg(any())]   // ENGINE: unused (native layout, no pre-swapped IVs)
const fn swapped<''')
rep('''#[cfg(target_os = "zkvm")]
extern "C" {
    fn sys_sha_compress(out: *mut [u32; 8], in_state: *const [u32; 8], b1: *const [u32; 8], b2: *const [u32; 8]);
}

#[cfg(target_os = "zkvm")]
#[inline(always)]
fn compress_w(iv: &'static [u32; 8], a: &W, b: &W, out: &mut W) {
    unsafe { sys_sha_compress(out, iv, a, b) };
}''', '''#[cfg(target_os = "zkvm")]
extern "C" {
    fn zkvm_sha256_impl(state: *const u8, input: *const u8, output: *mut u8);
}

#[cfg(target_os = "zkvm")]
#[inline(always)]
fn compress_w(iv: &'static [u32; 8], a: &W, b: &W, out: &mut W) {   // ENGINE: OpenVM's SHA-256, native state layout
    let blk: [W; 2] = [*a, *b];   // L || R as raw bytes, 4-byte aligned
    let mut st = [0u32; 8];
    unsafe { zkvm_sha256_impl(iv.as_ptr() as *const u8, blk.as_ptr() as *const u8, st.as_mut_ptr() as *mut u8) };
    for (o, x) in out.iter_mut().zip(st.iter()) {
        *o = x.swap_bytes();
    }
}''')
rep('''    #[cfg(target_os = "zkvm")]
    compress_w(&IV_NODE_W[l as usize], a, b, out);''', '''    #[cfg(target_os = "zkvm")]
    compress_w(&IV_NODE[l as usize], a, b, out);''')
rep('''    #[cfg(target_os = "zkvm")]
    compress_w(&IV_SPENT_W, c, s, out);''', '''    #[cfg(target_os = "zkvm")]
    compress_w(&IV_SPENT, c, s, out);''')
assert "sys_sha_compress(" not in s
open(p, "w").write(s); print("engine edits applied:", p)
