#!/usr/bin/env python3
"""coinlog_ovm.py <coinlog.rs>: edits.py's edit 7, on its own so every guest that includes libzync's coinlog.rs (the core
guest through edits.py, each zyncAPP guest through its assembler) gets the SAME engine edit. Anchored like edits.py."""
import sys
def rep(old, new):
    global s
    assert s.count(old) == 1, ("anchor not unique / missing", old[:70], s.count(old))
    s = s.replace(old, new)
# 7) libzync's coin log (libzync/src/coinlog.rs) on OpenVM's SHA-256 in its native layout. The product calls RISC Zero's
#    sys_sha_compress with pre-swapped IVs (IV_NODE_W / IV_SPENT_W) and gets the digest back as its in-memory bytes (W).
#    Here the IV is the native state (IV_NODE / IV_SPENT as they are), the block is L || R as raw bytes (W is exactly
#    that), and the native output state IS W (node format 2, COINLOG.md §2: the state words as they are). The product's
#    RISC Zero path swaps its accelerator's output back to the same words, so both give the same hash.
p = sys.argv[1]; s = open(p).read()
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
    let mut t = [0u32; 8];
    unsafe { sys_sha_compress(&mut t, iv, a, b) };
    for (o, x) in out.iter_mut().zip(t.iter()) {
        *o = x.swap_bytes();
    }
}''', '''#[cfg(target_os = "zkvm")]
extern "C" {
    fn zkvm_sha256_impl(state: *const u8, input: *const u8, output: *mut u8);
}

#[cfg(target_os = "zkvm")]
#[inline(always)]
fn compress_w(iv: &'static [u32; 8], a: &W, b: &W, out: &mut W) {   // ENGINE: OpenVM's SHA-256, native state layout
    let blk: [W; 2] = [*a, *b];   // L || R as raw bytes, 4-byte aligned
    unsafe { compress_blk(iv, &blk, out) }
}

/// One compression of a contiguous L || R into `out`: node format 2 (COINLOG.md §2) is the native state words as they
/// are (format 1 stored each word's bytes reversed). Through a local state (Rust's &mut cannot alias the block).
#[cfg(target_os = "zkvm")]
#[inline(always)]
unsafe fn compress_blk(iv: &'static [u32; 8], blk: &[W; 2], out: &mut W) {
    let mut st = [0u32; 8];
    zkvm_sha256_impl(iv.as_ptr() as *const u8, blk.as_ptr() as *const u8, st.as_mut_ptr() as *mut u8);
    *out = st;
}''')
rep('''    #[cfg(target_os = "zkvm")]
    compress_w(&IV_NODE_W[l as usize], a, b, out);''', '''    #[cfg(target_os = "zkvm")]
    compress_w(&IV_NODE[l as usize], a, b, out);''')
rep('''    #[cfg(target_os = "zkvm")]
    compress_w(&IV_SPENT_W, c, s, out);''', '''    #[cfg(target_os = "zkvm")]
    compress_w(&IV_SPENT, c, s, out);''')
rep('''pub fn parent_blk(l: u32, blk: &[W; 2], out: &mut W) {
    parent_w(l, &blk[0], &blk[1], out)
}''', '''pub fn parent_blk(l: u32, blk: &[W; 2], out: &mut W) {
    #[cfg(target_os = "zkvm")]
    unsafe { compress_blk(&IV_NODE[l as usize], blk, out) }   // ENGINE: the pair is the block, read in place
    #[cfg(not(target_os = "zkvm"))]
    parent_w(l, &blk[0], &blk[1], out)
}''')
assert "sys_sha_compress(" not in s
open(p, "w").write(s); print("engine edits applied:", p)

