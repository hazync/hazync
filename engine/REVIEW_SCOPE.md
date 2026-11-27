# What the 4.2 review rounds cover (prepared 27.11.2026)

ROADMAP 4.2 is the user's: several review rounds of the soundness code, in several ways, findings fixed and re-reviewed,
before the image freeze (feedback: many review rounds before any freeze; no Fable review until the user says so). This
file only maps the scope, so rounds can be planned; no review has been run from it.

The engine-specific code on the soundness path is small. The consensus code itself (Bitcoin Core v28's interpreter,
libsecp256k1, libzync) is the product's, reviewed on RISC Zero; what is new is how it runs on OpenVM.

| # | component | files | lines | what a reviewer checks |
|---|---|---|---|---|
| 1 | compat layer: the RISC Zero guest API on OpenVM (env::read / commit / verify, sha) | engine/platform/risc0-zkvm/src/lib.rs, platform/sha2, platform/risc0-crypto | 232 + 84 + 25 | env::verify = a deferral whose (exe, vm, public values) match; commit = SHA-256(journal) into the 32 public values; read order and word layout as risc0's; nothing a host can feed that changes what is verified |
| 2 | guest edits | engine/guest/edits.py, coinlog_ovm.py | 88 + 74 | each edit's anchor and replacement: that the engine guest computes what the product guest computes, except where an edit says why |
| 3 | batch ECDSA and the MSM on OpenVM's secp256k1 extension | engine/guest/batch_ecdsa_ovm.rs, msm_ovm.rs; secp-0201-openvm-batch-liftx.patch | 145 + 215 + 103 | the batch equation (random coefficients, their source), lift_x / parity / reduction, the hook call sites equal to the product's, failure = refusal |
| 4 | SHA-256 on OpenVM's native state layout | core-0201, core-0202, secp-0202 patches | 207 + 35 + 37 | byte order of the state and the finalise-in-place padding: identical digests (Core's vectors ran identical, results/core_vectors) |
| 5 | the C heap (#794) | prover/methods/guest/src/c_heap.rs, engine/guest/build.rs | 191 + 114 | recycling cannot alias live memory; the 512 MiB design limit; behaviour at exhaustion (a refusal, never a wrong value) |
| 6 | node format 2 | libzync/src/coinlog.rs (format switch), the trie | 286 (+ trie) | format 2's node hashing is collision-free with format 1's IVs; every reader follows the proof's profile bit 7 |
| 7 | the handover mode across OpenVM versions | engine/guest/xver_ovm.rs, handover/verifier-guest, the embedded predecessor keys | 87 + 74 | a predecessor accepted only under a pinned key (SHA-256), at the stated height, with its state; the negatives of handover_step2/3 |
| 8 | the verifier | engine/verify-core/src/lib.rs, swiftsync-verify statement_checks, each zAPP's verify | 193 + ... | the key pin, image id = SHA-256(exe ‖ vm) recomputed from the key, public values = SHA-256(journal), the HZE1 framing; journal decoding (engine/journal = risc0's serde, compat-checked) |
| 9 | segpar | engine/host/src/segpar.rs | 285 | prover-side only (its header says why): confirm a wrong or misordered segment proof fails the continuation checks |
| 10 | parameters | results/security_params | - | the 100-bit estimate's inputs; the key binding the parameters |

Total engine-specific soundness code: 1,616 lines (rows 1-9 without the product's own c_heap.rs and coinlog.rs, which add
477; swiftsync-verify and the zAPP readers not counted), plus the patches (382 lines).
engine/journal (1,270 lines) is risc0-zkvm 3.1.0's serde unchanged but for one constant, checked byte-identical
(engine/journal/compat); a reviewer only confirms the diff.

Ways to review, for the user to pick: independent reviewers per row (a fresh reader each, given the code, not our
conclusions); an adversarial round that tries to make a false statement verify (forged key, journal, child proof,
handover predecessor); differential runs (product guest vs engine guest on the same blocks: results/core_vectors,
edge_blocks, rig_engine already do this).
