# R1-2: guest edits and SHA-256 patches (reviewer report, 27.11.2026, as given)

No way found for an invalid block, transaction or state to verify, and no digest that differs from the product
guest's: every SHA-256 byte-order change cancels exactly, every edit is equivalent to what it replaces. Findings low
(robustness, documentation). Method: read the scoped files, product main.rs, agg_lite.rs, swiftsync.rs,
libzync/src/coinlog.rs, the platform shims and OpenVM v2.1.0's sha2 (05f3b73); laid out the base tree (Core v28.0 +
0001/0002/0009/0010/0101, then core-0201, core-0202; secp-0201, secp-0202 on pristine libsecp256k1 v0.5.1: all applied
cleanly) and read the resulting sha256.cpp and hash_impl.h. Nothing in the repo modified.

1. LOW (verified). Edit 4's anchor is not checked for uniqueness (engine/guest/edits.py:51-52): s.index("\nfn main() {")
   then the first "\n}\n" after it, without the count assert the other edits have. Lands correctly today (main
   2395-2455, no column-0 } inside, every mode returns into the match, so finalize() always runs). Risk: a future
   column-0 } inside main or a second fn main puts finalize in the wrong place.
2. LOW (verified). env::exit differs (main.rs:2359, the engine's exit()): RISC Zero halts with a receipt, the engine
   panics (no proof). Affects only provability (collect runs are dry anyway), and fails safe.
3. LOW. coinlog_ovm.py:47's comment says `out` "may overlap the block"; Rust's &mut W and &[W;2] cannot alias. Code
   correct, comment wrong.
4. Relied on, not verified (out of scope): OpenVM's verifier accepts only a run that ended successfully with exit code 0
   (every "panic = no proof" argument, and OpenVM's sqrt looping on a bad hint, depend on it); batch_ecdsa_ovm.rs and
   xver_ovm.rs (edits 2 and 1 point to them) not reviewed.

Checked and sound: OpenVM's native SHA-256 layout (zkvm_sha256_impl takes / returns 8 native LE u32 words, standard IV,
reads 64 raw block bytes, in place as OpenVM's own sha2 calls it; Core's and libsecp's s[8] already in that layout, so
dropping the swaps is right; the product's swap-in/swap-out gives the same values; unaligned chunks staged into
alignas(4) buffers); hz_d64 (pad block [62]=0x02 = 512 bits; second block BE digest, 0x80, [62]=0x01; output BE; input
read fully before out is written, so SHA256D64's in-place merkle use is safe); core-0202 in-place Finalize (n < 56 one
block, n >= 56 two, bytes advances as before; length classes 0, 55, 56, 63, 64, multi-block; bit count taken first);
code relying on the old layout (only sha256.cpp calls sys_sha_compress; no hard-coded midstates; HashWriter / CSHA256
mid copy whole objects; TransformD64Wrapper and the generic path native; libsecp's tagged midstates native words;
Core's bundled secp256k1 not compiled; no IV_NODE_W / swapped / sys_sha_compress left); the coin log (edit 7: product
swap(state with swapped IV) = native words = format 2; engine native state from native IV, block = L||R as stored,
= h_of; = the host's compress_w format 2; &IV_NODE[l] 'static, tested); edit 10 (swiftsync x_for: be_to_limbs(BE)[k] =
state[7-k] = short_digest_limbs; n = 36, ctr LE; host test 0..55 vs RustCrypto); edits 8, 9 (read_slice_ref = read_slice
bytes through the same take(), copies when unaligned); edit 1 (stub makes `carried` false: a lineage pinning a non-zero
RISC Zero verifier fails to compile, verify panics; SAME_ZKVM needs an OpenVM proof hashing to that id: fails closed);
edit 3 (hints read then dropped; hz_fe_sqrt checks its own input < p; OpenVM's Sqrt checks by squaring, proves
non-residue via sqrt^2 == a.non_qr, zero a square without a hint: a prover cannot force a valid key's lift to fail,
which would flip a legacy CHECKSIG NOT); the platform sha2 shim (finalize and padding mirror core-0202, host-tested at
0/1/3/55/56/63/64/65/129; Block align(4)); every other anchor asserted unique.
