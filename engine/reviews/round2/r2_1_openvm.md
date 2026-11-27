# R2-1: OpenVM's own guarantees (reviewer report, 27.11.2026, as given)

Read OpenVM 05f3b73 and stark-backend a6b4761; changed nothing. All four guarantees hold in OpenVM's source, given
the pinned key the host path already depends on. No soundness gap in what OpenVM checks.

1. Exit code 0 only (high). Aggregation circuit crates/continuations/src/circuit/inner/vm_pvs/air.rs: :134-140
   is_terminate boolean, forces is_last and exit_code == 0 (a non-zero exit or panic cannot be aggregated); :143-148
   non-terminating segments must carry exit code 42 (DEFAULT_SUSPEND_EXIT_CODE, crates/vm/src/system/connector/mod.rs:34);
   :151-157 segments chain (final_pc / final_root = next initial_pc / initial_root); :316-340 output public values =
   first segment's initial state, last segment's exit code and is_terminate. Host crates/verify/src/lib.rs: :121-125
   rejects unless exit_code == Success and is_terminate == 1 (a prefix cut at a segment boundary fails); :103-120 pins
   program_commit, initial_root, initial_pc to the baseline; :129 internal_flag == 2; :207-245 recursion depth bound and
   cached commit. Deferred child guest-libs/verify-stark/circuit/src/verifier/air.rs: :132-134 asserts in-circuit
   exit_code == Success and is_terminate == 1; :140-158 internal_flag == 2, depth range-checked. Hazync: finalize asserts
   !dry (engine/platform/risc0-zkvm/src/lib.rs:212); a run skipping finalize reveals all-zero public values, never
   sha256(journal).
2. What the image id binds (medium-high on vm, high on exe). app_exe_commit = compute_exe_commit(program_commit,
   initial_state, initial_pc) (crates/verify/src/vk.rs:65-73; in-circuit verifier/air.rs:290-348); program_commit = the
   ProgramAir trace's cached commit (vm_pvs/air.rs:276-286); the initial memory includes the deferral-circuit commits
   (the transpiler seeds each def_idx's input accumulator with def_circuit_commit,
   extensions/deferral/transpiler/src/lib.rs:108-124), so which deferral circuits a guest accepts is in the exe commit.
   app_vm_commit = hash_slice over app_vk, leaf_vk, internal_for_leaf_vk, each (cached_commit, vk_pre_hash)
   (verifier/air.rs:263-283; Hazync's image_of matches); vk_pre_hash = hash of the full postcard MultiStarkVerifyingKey0
   (params, every per-AIR key, trace-height constraints: stark-backend keygen/mod.rs:198-216), so every extension's AIRs
   are in it; the app cached_commit from the constraint DAG and interactions (crates/recursion/src/batch_constraint/
   expr_eval/symbolic_expression/trace.rs:421). Two app keys share a vm commit only via a Poseidon2 collision. NOT in the
   image id: internal_recursive_vk_commit, expected_def_hook_commit, memory_dimensions, num_user_pvs, the outer mvk. Guest
   path: constants of the verify-stark circuit (circuit/src/lib.rs:48-56, verifier/air.rs:240-261, 483-489), fixed by the
   parent's def_circuit_commit: sound. Host path: verify_full_pinned with a key matched only by image id lets an
   attacker swap them (the known pin issue); verify_full_bytes (SHA-256 pin) is the right entry point; make sure no
   caller reaches verify_full_pinned with a key from input.
3. What a guest verify proves about the child (medium-high). A real STARK under the universal internal-recursive key
   (pre_hash constant, verifier/air.rs:240-250; cached commit internal_for_leaf's at depth 1 or the internal-recursive
   constant, :216-261); exit 0 with is_terminate; exe, vm and user public values checked in the Merkle opening of
   final_root (:167-173, UserPvsInMemoryAir); output = a sponge hash output_commit, the OUTPUT opcode constrains the bytes
   written to memory to it (extensions/deferral/circuit/src/output/air.rs:67-68, 194-213); input_commit = the
   compression of the child verifier's final Fiat-Shamir state (:375-382). The host cannot point a deferral at a proof
   that does not exist: host input/output commits only enter the deferral address-space accumulators (call/air.rs:161ff);
   the final proof needs deferral_flag == 2 with def_hook_commit == expected and Merkle paths tying initial/final
   acc_hash into initial/final_root (crates/verify/src/lib.rs:259-313, deferral.rs:61-136), or flag 0 with zero
   accumulators and the address space unchanged; the hook's onion chain starts at def_circuit_commit and folds each
   verified deferral's (input, output) in order (continuations/.../deferral/hook/onion/air.rs:85-104): a fabricated,
   missing or reordered deferral cannot match; node_idx == 0 and depth enforced. A child that itself used deferrals is
   constrained the same way (verifier/air.rs:479-523). Not checked line by line: the hook verifier sub-AIR's
   derivation of def_circuit_commit, and the count bus.
4. Same-version handover (high). p.image from LINEAGE in source (main.rs:2265-2268); env::verify recomputes
   SHA-256(exe || vm) from the circuit-constrained output, compares to p.image, checks sha256(prev_journal) vs the public
   values (risc0-zkvm/src/lib.rs:165-176): binds program, initial state, deferral set, app key, exit 0, journal. Liveness
   only: the predecessor must share this image's verify-stark internal-recursive key and hook commit (same agg params),
   else a valid predecessor is rejected, never a forgery accepted.
Other: s.commits from the host, every one must be consumed (finalize :214); bench-noverify domain-separated and a
different exe commit; Hazync's poseidon2_hash_slice and le_bytes must stay identical to OpenVM's (extension.rs:55-56):
a mismatch only falsely rejects.
