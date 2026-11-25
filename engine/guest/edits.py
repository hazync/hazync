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
# 8) Large word arrays read once are LENT out of the input buffer (env::read_slice_ref) instead of copied into a fresh
#    zeroed Vec: the trie witness (976,379: 5.2 MB of a 6.5 MB stream) and the aggregate's chunk journals (2.3 MB).
#    Same bytes, one copy fewer, and that much less memory touched (memory hashing was 25% of a trie part's cells).
for m in ("let (nw, nm, ni, np) = (h[0] as usize, h[1] as usize, h[2] as usize, h[3] as usize);",
          "let (d, nw, nm, ni, np) = (h[0], h[1] as usize, h[2] as usize, h[3] as usize, h[4] as usize);",
          "let (nw, nm, ni, np, nparts) = (h[0] as usize, h[1] as usize, h[2] as usize, h[3] as usize, h[4] as usize);"):
    rep(m + "\n    let mut wit = vec![0u32; nw];\n    env::read_slice(&mut wit);",
        m + "\n    let wit: &'static [u32] = env::read_slice_ref(nw);   // ENGINE: lent, not copied")
rep("let mut lite_journals: Vec<Vec<u32>> = Vec::with_capacity(k as usize);",
    "let mut lite_journals: Vec<&'static [u32]> = Vec::with_capacity(k as usize);   // ENGINE: lent journals")
rep("""            let mut words = vec![0u32; (n / 4) as usize];
            env::read_slice(&mut words);
            env::verify(self_id, &words).expect("chunk proof invalid");""",
    """            let words: &'static [u32] = env::read_slice_ref((n / 4) as usize);   // ENGINE: lent, not copied
            env::verify(self_id, words).expect("chunk proof invalid");""")
rep("{ lite_journals.push(words); continue; }", "{ lite_journals.push(words.leak()); continue; }")
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
    unsafe { compress_blk(iv, &blk, out) }
}

/// One compression of a contiguous L || R into `out` as the digest's bytes. The native state's words are stored byte
/// by byte, most significant first: 3 shifts + 4 byte stores a word, where a word swap costs ~10 ALU instructions
/// (RV32IM has no rev8) plus the store. Volatile so the compiler does not merge them back into a swapped word.
#[cfg(target_os = "zkvm")]
#[inline(always)]
unsafe fn compress_blk(iv: &'static [u32; 8], blk: &[W; 2], out: &mut W) {
    let mut st = core::mem::MaybeUninit::<[u32; 8]>::uninit();
    zkvm_sha256_impl(iv.as_ptr() as *const u8, blk.as_ptr() as *const u8, st.as_mut_ptr() as *mut u8);
    let s = st.assume_init_ref();
    let o = out.as_mut_ptr() as *mut u8;
    for i in 0..8 {
        let (x, p) = (s[i], o.add(4 * i));
        core::ptr::write_volatile(p, (x >> 24) as u8);
        core::ptr::write_volatile(p.add(1), (x >> 16) as u8);
        core::ptr::write_volatile(p.add(2), (x >> 8) as u8);
        core::ptr::write_volatile(p.add(3), x as u8);
    }
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

# 9) agg_lite::validate_block_lite takes the lent journals (edit 8) as slices.
import os
p = os.path.join(os.path.dirname(sys.argv[1]), "agg_lite.rs"); s = open(p).read()
rep("journals: &[Vec<u32>],", "journals: &[&[u32]],   // ENGINE: lent journals (edits.py 8)")
open(p, "w").write(s); print("engine edits applied:", p)

# 10) swiftsync::x_for: SHA-256(leaf ‖ ctr) is one block, padded in place on OpenVM's compression; the x coordinate's
#     limbs are the native state words in reverse (sha2 shim's short_digest_limbs, host-tested against RustCrypto), where
#     the product byte-swapped the digest into bytes and be_to_limbs swapped it back. Same x.
p = os.path.join(os.path.dirname(sys.argv[1]), "swiftsync.rs"); s = open(p).read()
rep('''    let mut m = [0u8; 36];
    m[..32].copy_from_slice(leaf);
    m[32..].copy_from_slice(&ctr.to_le_bytes());
    let h: [u8; 32] = Sha256::digest(m).into();
    // x >= p has probability ~2^-224; refusing it keeps the map single-valued.
    Fq::from_bigint(BigInt(be_to_limbs(&h))).expect("ECMH: hash >= p")''', '''    // ENGINE (edits.py 10): leaf ‖ ctr (little-endian) in a word block, the digest's limbs straight from the state
    let mut b = [0u32; 16];
    unsafe { core::ptr::copy_nonoverlapping(leaf.as_ptr(), b.as_mut_ptr() as *mut u8, 32) };
    b[8] = ctr;   // its little-endian bytes: the target is little-endian (risc0-crypto shim asserts it)
    // x >= p has probability ~2^-224; refusing it keeps the map single-valued.
    Fq::from_bigint(BigInt(sha2::short_digest_limbs(&mut b, 36))).expect("ECMH: hash >= p")''')
open(p, "w").write(s); print("engine edits applied:", p)
