#!/usr/bin/env python3
"""edits.py <assembled guest main.rs> <assembled libzync/src/coinlog.rs>: the ONLY source edits the engine makes to Hazync's guest (README.md).
Each asserts its anchor occurs exactly once, so a product change that moves one fails here, by name."""
import sys
p = sys.argv[1]; s = open(p).read()
def rep(old, new):
    global s
    assert s.count(old) == 1, ("anchor not unique / missing", old[:70], s.count(old))
    s = s.replace(old, new)
# 1) Cross-version handover. The product's src/xver checks an OLDER RISC Zero's receipt; an engine chain has no RISC Zero
#    predecessor. With the engine feature `ovm-xver` (an image that pins an engine predecessor of ANOTHER OpenVM
#    version, engine/HANDOVER.md step 2) xver is the engine's own (xver_ovm.rs: the predecessor's proof checked by
#    OpenVM's verifier under its pinned key). Without it, the stub: the path is unreachable, `carried` says no.
#    ⛔ The switch is on ONE line and lines 2-5 are the old stub's, byte for byte: a panic's location (file, line,
#    column) is in the image, so any line added here would move every later line of main.rs and every default image id.
rep("mod xver;", """#[cfg(feature = "ovm-xver")] #[path = "xver_ovm.rs"] mod xver; #[cfg(not(feature = "ovm-xver"))] mod xver {   // ENGINE: no RISC Zero predecessor; cross-version handover is unreachable
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
assert s.count("\nfn main() {") == 1, ("edit 4: `fn main() {` not unique / missing", s.count("\nfn main() {"))
i = s.index("\nfn main() {"); j = s.index("\n}\n", i)
# j must END main: the first column-0 `}` after it (review R1-2 #1). A column-0 `}` inside main would put finalize()
# mid-body, so what follows j must be a new item, a blank line or the end of the file, never indented code.
after = s[j + 3:].lstrip("\n")
assert after == "" or not after[0].isspace(), ("edit 4: main's closing brace is not the end of main", after[:70])
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
    let Some(x) = k256::Secp256k1Coord::from_be_bytes(a) else { panic!("hz_fe_sqrt: a non-canonical input (the caller normalises; review R2-4 #2)") };
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

# 7) libzync's coin log on OpenVM's SHA-256 in its native layout: engine/guest/coinlog_ovm.py (shared with the
#    zyncAPP guests' assemblers, so every includer of coinlog.rs gets the same edit).
import os, subprocess
subprocess.run([sys.executable, os.path.join(os.path.dirname(os.path.abspath(__file__)), "coinlog_ovm.py"), sys.argv[2]], check=True)

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
