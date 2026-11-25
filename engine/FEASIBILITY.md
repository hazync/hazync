# OpenVM engine: feasibility questions (25.11.2026)

The questions to answer before deciding whether the engine replaces RISC Zero. Each answer is marked MEASURED,
ESTIMATED or UNKNOWN and cites a file or log for every number. Answers go under each question; the list is the user's.

  A. Trust and soundness
  1. Exactly which OpenVM version, commit and proving system does the engine use, and which audits cover that exact version? Include the auditor, the date,
     the scope, and any open findings. Which parts are unaudited (GPU prover, recursion, SHA and bigint extensions, the STARK-to-SNARK wrapper)?

     **Answer (25.11.2026; sources: zkSecurity report https://reports.zksecurity.xyz/reports/openvm/, releases
     https://github.com/openvm-org/openvm/releases, blog https://blog.openvm.dev/2.0-production, issue #3196):**
     - MEASURED (the build): engine/host/Cargo.toml and engine/guest pin OpenVM **v2.1.0**, tag commit 05f3b73e
       (released 04.11.2026), stark-backend v2.1.0; proof system SWIRL (BabyBear/Poseidon2 STARK), 100-bit params
       (app_params_with_100_bits_security / hook_params_with_100_bits_security in engine.rs).
     - The audit: zkSecurity, "Audit of OpenVM 2.0" for Axiom, report dated 09.06.2026, in four phases. Phase 2
       (from 18.05.2026, openvm commit f01c912) is continuations and deferral, and names **crates/continuations (incl.
       the deferral framework), extensions/deferral, crates/verify and guest-libs/verify-stark/{circuit,guest}**: the
       in-guest verification our aggregate relies on IS in scope. Phase 1: SWIRL/stark-backend/recursion (openvm
       c89d052d). Phase 3: the Halo2 static verifier. Phase 4: SHA-2/Keccak and memory-adapter changes (f01c912 ->
       0110850). Deferral findings: 4 High (#00 padded aggregation roots read as hook IO leaves; #01/#02 host verifier
       deferral Merkle checks; #03 counter overflow), 2 Medium, Low/Info items; by the client's responses all
       addressed in PRs #2815-#2866 (#00 "partially fixes" #2815, domain separation #2857); #10 (output_len an
       unauthenticated allocation hint) answered with docs, no circuit binding.
     - Gaps (UNKNOWN / not covered):
       - **The audited commits are not the release we run.** v2.0.0 (15a7ab6, 10.07.2026) followed the audit; v2.1.0
         changed "recursion and continuation circuit interactions and constraints for formal verification" and
         nothing says v2.1.0 was re-audited. -> the diff f01c912..05f3b73e in the audited directories is unreviewed
         by a third party as far as public material shows.
       - **secp256k1 / modular / ECC extensions: not in the v2.0 audit** (only k256/p256 manifests). v2.1.0 shipped
         security advisories in exactly these (GHSA k256/p256 canonicality, ECC decompression) plus RV32IM abort
         transpilation and std main. Earlier OpenVM 1.x audits (Cantina, credited for v1.7.0) are where the ECC /
         algebra / RV32IM circuits were reviewed; whether those circuits changed in 2.x is to be checked.
       - **GPU/CUDA prover: not mentioned.** Prover-side only (a wrong GPU trace fails verification, it cannot make a
         false statement verify), so a liveness risk, not soundness.
       - **SDK (crates/sdk): not in Phase 2 scope**; open issue #3196 (reused deferral-aware StarkProver: later proofs
         fail verification, DeferralInitialRootMismatch; open, no maintainer reply) is there. Liveness, not
         soundness. The engine does not hit it: Sdk::prove builds a fresh prover per call, and segpar builds a fresh
         instance and its initial tree right after reset_state.
     - Verdict so far: the soundness-critical recursion/deferral path is third-party audited at a pre-release commit;
       the release we run has post-audit circuit changes and fresh ECC advisories. Not a deal-breaker; it is the
       first thing to raise with the OpenVM team (is v2.1.0's diff reviewed? are the 1.x ECC/algebra audits still
       valid for 2.x?).
  2. What security level do the proof parameters give, in bits? Does that rely on any conjectures, and how does it compare with risc0's 3.1.0 settings?
  3. Is OpenVM still actively maintained? Give its release pace, how it discloses security issues, and who maintains it.
  4. The compatibility layer that runs risc0's guest API on OpenVM is our own code on the soundness path. List every place where it could accept something
     risc0 would refuse, and what tests cover each one.
  5. How is the image id bound? What exactly does "exe commitment + vm" commit to? Can two different guests or configurations share an id? Can a proof from
     one configuration verify under another?

     **Answer (25.11.2026, from the code; MEASURED where marked):**
     - image id = SHA-256(app_exe_commit ‖ app_vm_commit) (engine/host/src/r0.rs `image_id`; the guest computes the
       same in engine/platform/risc0-zkvm `env::verify`).
     - app_exe_commit (OpenVM `compute_exe_commit`): Poseidon2 Merkle hash of the program's code commitment, the
       initial memory root and the start pc. Any change to the guest's code or initial data changes it.
     - app_vm_commit: Poseidon2 hash of the verifier's key-commit components (verify-stark `output_raw_from_proof`:
       `vk_commit_components(verifier_pvs)`): the commitments to the app VM's verifying key and the aggregation
       layers'. A different VM configuration (extensions, moduli, system params) means different AIRs, a different
       key and so a different vm commit.
     - `env::verify(image_id, journal)` in the guest: verify_stark on the next supplied proof, then asserts
       SHA-256(exe ‖ vm) == image_id ("the proof is not of this image") and the proof's public values ==
       SHA-256(journal) ("journal does not match the proof"). MEASURED: negative controls in the M1 tests (wrong image,
       wrong journal) are refused.
     - Two guests / configurations sharing an id: only by a SHA-256 or Poseidon2 collision. A proof from another
       configuration carries that configuration's vm commit, so it fails the image check.
     - Caveat (ESTIMATED, to confirm): the segment cap (segmentation_max_memory) is prover-side and not in any key
       or commitment (engine.rs Engine::load says so), so proofs made at different caps verify under the same id;
       that is intended. Whether OpenVM's own `VerificationBaseline` (the outer verifier's pin) can admit a proof of
       a different aggregation key is answered by the baseline pinning agg_vk; to be checked against crates/verify.
  6. List every difference between the engine guest and the product guest (CHANGES.md), and explain why each one cannot change a verdict or committed state.

     **Answer (25.11.2026; the full list is engine/CHANGES.md and engine/guest/edits.py, each edit anchored to exactly
     one place so a product change that moves it fails assembly by name):**
     | difference | why it cannot change a verdict or committed state | evidence |
     |---|---|---|
     | edit 1: cross-version handover stubbed (panics) | an engine chain starts at genesis; the path is unreachable and panics (no proof) if reached | code |
     | edit 2 + batch_ecdsa_ovm.rs + msm_ovm.rs: batch signature check on OpenVM's secp256k1 | same public items, hint layout, transcript and batch equation; a call without a valid hint is verified directly | msm host test vs openvm's msm with negative controls; journals |
     | edit 3: pubkey lift via libsecp's sqrt hook (hinted, checked by squaring) | the hint is checked; a wrong root fails the square check | journals |
     | edit 4: journal hash -> the proof's public values once at the end | the binding the outer verifier and env::verify check (Q5) | M1 negative controls |
     | edits 5/7 + core-0201/0202 + secp-0202 + sha2 shim: SHA-256 on OpenVM's native layout, padding in place | same bytes in, same digest out | sha2-shim-test vs RustCrypto (all lengths 0-299, splits, unaligned; negative control fails); journals |
     | edits 8/9: large inputs lent, not copied | read-only views of the same words | journals |
     | edit 10: x_for on one padded block | same digest, limbs read without swaps | short_digest_limbs host test (0..55 bytes, negative control); aggregate journal identical |
     | platform (engine/platform): RISC Zero guest API on OpenVM, env::verify = verify-stark deferral | Q4 lists where it could accept more than RISC Zero | M1 tests, journals |
     | shared libzync changes on this branch (coin log update_one, trie hash pairs, KIND_TRIE_PARTS) | same hashes in the same order; parts = the combine's audited-by-test logic run in the aggregate | libzync tests (76) incl. against the old recursion, negative controls fail |
     - MEASURED (acceptance): every chunk, trie and part journal byte-identical to RISC Zero's on 976,379, 962,000,
       741,000, 364,292 (17/17), 130,000; each aggregate identical except its 32-byte image id.
     - UNKNOWN (rejection): identical journals on VALID blocks show the engine accepts as RISC Zero does, not that it
       refuses what RISC Zero refuses. That needs Q9 (Core's 22,666 vectors and the 97 rig cases) run on the engine.

  B. Completeness: can every block Core accepts still be proved?
  7. Do the #794 attack blocks prove on the engine: the memory block, the single tapscript input, and the costly SIGHASH_NONE block at a small scale? What
  allocator does the guest use, and does it ever free memory?
  8. Does it prove an old block with many inputs (364,292), plus a few of the edge blocks from blocks.sh?
  9. Do all 22,666 Core test vectors and the 97 rig cases pass with verdicts identical to risc0's?
  10. What are the limits on memory, cycles per segment and input size, and where do they come from?

  C. Where it has to run
  11. Browser and phone verifier: can a proof be checked in WASM in a browser? How large is the proof and verifier, how long does a check take on a phone,
  and what does the verifier trust?
  12. Windows: does the GPU prover build and prove on Windows with a native CUDA toolchain? What's the minimum driver version?
  13. Cards: which GPUs are supported (oldest architecture, minimum memory)? Do consumer cards like a 3060 or 4090 work, and how fast?
  14. Is there a CPU-only prover, and how fast is it?
  15. Reproducible ids: do two clean builds on different machines give the same image id? Can that be pinned the way reproduce/LIBZYNC_IDS is pinned today?

  D. Cost and speed
  16. What is the whole-chain cost, from genesis to the tip, in GPU-hours and dollars? Use measured per-block costs across eras, and say which blocks the
  estimate rests on.
  17. How do the tip-block times compare (detect to verified) with risc0's current 191 s on 4× PRO 6000?
  18. What do the three levers still on the table buy, measured: fewer and larger chunks, CPU/GPU overlap, and native SHA? Does any of them change a
  commitment or format?
  19. How does it scale across several GPUs for one block?

  E. Migration
  20. What has to be rebuilt? Cover the zAPPs (headers, hints, utreexo, deposit, assume, liveness, statement), the verifiers, the board and coordinator, the
  worker and the Windows app, the release tooling and the site pins.
  21. Which commitments and formats can stay byte-identical, so existing tools and zAPP logic carry over?
  22. What's the plan for re-proving from genesis, and does the existing risc0 chain proof have any role in it?
  23. Does the engine support composition, so a zAPP can verify a core proof inside its own proof as it does today?
  24. What does it cost to maintain over time? Do we follow upstream OpenVM releases, and what happens to image ids when we do?

  F. The deciding question
  25. Which of the above, if it came back badly, would rule the switch out, and which are just work?
