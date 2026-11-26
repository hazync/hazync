# OpenVM engine: feasibility questions (25.11.2026)

The questions to answer before deciding whether the engine replaces RISC Zero. Each answer is marked MEASURED,
ESTIMATED or UNKNOWN and cites a file or log for every number. Answers go under each question; the list is the user's.

## Status at 26.11.2026 (morning)

| | question | status | in one line |
|---|---|---|---|
| Q1 | audits | OPEN | recursion/deferral audited at a pre-release commit; post-audit diffs measured (thousands of lines); ECC/algebra not in the 2.0 audit: ask OpenVM |
| Q2-Q6 | params, maintenance, compat layer, image id, guest edits | answered | see each |
| **Q7** | **#794 attack blocks** | **BAD as is** | **512 MiB guest memory vs RISC Zero's 3 GiB: tap_small not provable; needs a larger address space or a reclaiming allocator** |
| Q8 | old + edge blocks | GOOD | 364,292 proved; 9/10 edge blocks identical (2,016 not run) |
| Q9 | Core vectors + rig | GOOD so far | 22,666 vectors identical; rig 55/55 agree, 41 to run |
| Q10 | limits | answered | 512 MiB address space is the binding one (Q7) |
| Q11 | browser verifier | GOOD | full check (STARK + image + journal) in Node and Chromium, 25-39 ms; phone not tried |
| Q12 | Windows | builds | CI exe (sm_61..120); proving on a Windows card not tried |
| Q13 | cards | GOOD | 5 card types; full block in 3.8-3.9 GB at 2^26 (test image only, Q7) |
| Q14 | CPU prover | yes | slow (hours per modern block) |
| Q15 | reproducible ids | answered | one machine, two setups |
| Q16 | whole-chain cost | ESTIMATE | ~$16-37k depending on card and fixed cost |
| Q17 | tip time | GOOD | 969,122: 33.9 s proving on 4x PRO 6000 (RISC Zero 191 s end to end) |
| Q18-Q19 | levers, scaling | measured | segpar -25% on the aggregate |
| Q20 | migration | started | header zAPP ported and proved |
| Q21-Q24 | formats, genesis plan, composition, upkeep | answered | see each |

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
     - MEASURED (26.11.2026, `git diff` in a full clone of openvm-org/openvm; line counts are a proxy for review
       effort, not a finding): the post-audit change in the Phase 2 directories, audited f01c912 (18.05.2026) -> our
       05f3b73e, non-test code: crates/continuations 54 files +1,596/-480 (AIR files +694/-206), crates/recursion 110
       files +1,398/-708 (AIR +310/-134), extensions/deferral +314/-181 (AIR +46/-30), crates/verify +340/-129,
       guest-libs/verify-stark +304/-111 (AIR +65/-27). Phase 4 (0110850 -> 05f3b73e): extensions/sha2 +30/-12,
       keccak256 +18/-1, crates/vm/src/system/memory +626/-18. From the 1.x audits (v1.7.0) to v2.1.0: rv32im circuit
       87 files +1,262/-1,111, mostly execution/AOT code, with constraint changes in adapters/loadstore.rs (+15/-23)
       and hintstore/mod.rs (+57/-11); algebra circuit +1,021/-1,580; ecc circuit +585/-537; crates/circuits/
       primitives 68 files +3,144/-974; mod-builder -2,740 net. So "are the 1.x ECC/algebra audits valid for 2.x" is a
       real question: the code moved.
     - Verdict so far: the soundness-critical recursion/deferral path is third-party audited at a pre-release commit;
       the release we run has post-audit circuit changes and fresh ECC advisories. Not a deal-breaker; it is the
       first thing to raise with the OpenVM team (is v2.1.0's diff reviewed? are the 1.x ECC/algebra audits still
       valid for 2.x?).
  2. What security level do the proof parameters give, in bits? Does that rely on any conjectures, and how does it compare with risc0's 3.1.0 settings?

     **Answer (25.11.2026, from stark-backend v2.1.0 crates/stark-sdk/src/config/mod.rs; the engine uses these
     functions, engine.rs):**
     - Target: **100 bits of PROVEN round-by-round security** (the file's own words), BabyBear base field with a
       degree-4 extension, Poseidon2 hashing, WHIR polynomial commitments, LogUp grinding calibrated to the PCS list size.
     - Proximity regime per layer: app and leaf `UniqueDecoding` (no list decoding at all); internal
       `ListDecoding { m: 2 }`; root and hook (deferral) `ListDecoding { m: 1 }`. List decoding with a radius set by m
       stays within the Johnson bound, which is proven for Reed-Solomon codes: no proximity-gap conjecture is used.
     - Assumptions that remain: Poseidon2 over BabyBear as a random oracle / collision-resistant hash (Fiat-Shamir),
       and SHA-256 for our image id and journal binding.
     - RISC Zero for comparison (sources: risc0-zkp 3.0.1 docs.rs lib.rs: "50 FRI queries is sufficient to achieve our
       security target of 97 bits (conjectured security)"; risc0 README: "98 bits of conjectured security"): RISC
       Zero's level is CONJECTURED (FRI, toy-problem conjecture); no proven figure found. ePrint 2024/1161 shows
       FRI's conjectured levels can sit well above what is provable. To re-check against risc0 3.1.0's own
       soundness calculator before quoting externally.
     - So on parameters alone the engine is at least as strong: 100 bits proven against ~97-98 conjectured.
  3. Is OpenVM still actively maintained? Give its release pace, how it discloses security issues, and who maintains it.

     **Answer (25.11.2026; source https://github.com/openvm-org/openvm/releases, https://blog.openvm.dev/2.0-production):**
     - Maintainer: Axiom (openvm-org); the 2.0 audit was commissioned by Axiom.
     - Release pace (MEASURED from the releases page): v2.0.0 10.07.2026, v2.0.1 17.07.2026, v2.0.2 14.08.2026,
       v2.1.0 04.11.2026; 1.x before that (Cantina-audited v1.7.0). Active.
     - Disclosure: GitHub security advisories (GHSA) published with releases; v2.1.0 lists five (std main, RV32IM abort
       transpilation, k256/p256 canonicality, ECC decompression). Versioning: patch releases keep the verifying key
       (OpenVM AGENTS.md / VERSIONING.md), so a patch upgrade should not move our image ids; a minor (2.1 -> 2.2) may.
     - Responsiveness: issue #3196 (SDK, opened 08.11.2026) had no maintainer reply when read on 25.11.2026: one data
       point, not a pattern.
     - UNKNOWN: a security contact / bug bounty, and support commitments for 2.x.
  4. The compatibility layer that runs risc0's guest API on OpenVM is our own code on the soundness path. List every place where it could accept something
     risc0 would refuse, and what tests cover each one.

     **Answer (25.11.2026, from engine/platform/risc0-zkvm/src/lib.rs, 232 lines; every point below is in that file):**
     | place | could it accept more than RISC Zero? | covered by |
     |---|---|---|
     | `env::verify` | No, if OpenVM's deferral is sound: verify_stark on the next supplied input commitment (the commitment is a hash of the child proof's transcript, checked in-circuit), then asserts SHA-256(exe ‖ vm) == the image id and the 32 public-value bytes == SHA-256(journal). "unchecked" in `verify_stark_unchecked` only means it returns the output for the caller to compare, which the shim does. Audit Low #10 (output_len an unauthenticated hint) cannot widen this: a shorter or padded output fails the exact 32-byte comparison. | M1 negative controls (wrong image, wrong journal refused); every block's aggregate |
     | proofs supplied but not used | No: `finalize` asserts every supplied commitment was verified | code |
     | dry runs (flag bit 1: every env::verify skipped) | No: `finalize` refuses a dry run ("a dry run cannot commit a journal"), and a dry run must take no proofs. Collect runs end in env::exit before any commit. | code |
     | `bench-noverify` feature (env::verify always accepts) | Only in a metering build, whose image id is its own; a chain pins the image id, so its proofs cannot stand for the real image's | build feature; ids differ (b13 vs hz13 etc.) |
     | input reads past the end | No: asserts ("input exhausted") | code |
     | serde word format | Vendored from risc0-zkvm 3.1.0 unchanged | journals identical on every block |
     | `read_slice_ref` (lent words) | Same bytes as read_slice | journals |
     | `env::exit(code)` | Panics after printing the code: no journal, no proof (RISC Zero: a non-zero exit, no receipt claim the chain accepts) | code |
     | `env::cycle_count` returns 0 | Used only in log lines (batch_ecdsa timing, TXCYC); no verdict depends on it | grep of the guest |
     | control chunk (flags, commitment list) | Unknown flags refused; length checked; the commitments are only names of proofs that must verify | code |
     - The soundness this rests on that is NOT ours: OpenVM's deferral / verify-stark circuits (Q1: audited at
       f01c912, post-audit changes in v2.1.0) and the outer verifier.
     - Not yet tested directly: a hostile host supplying a commitment of a proof of ANOTHER image with a matching
       journal (must fail the image check), and one of this image with a different journal (must fail the journal
       check). The M1 negative controls cover both at the CPU level; a GPU-era re-run of them is cheap and worth doing.
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

     **Answer (26.11.2026): RUN on the engine (results/q7_attack_blocks). Not at parity: OpenVM's 512 MiB guest
     memory is a sixth of RISC Zero's 3 GiB.** One A40, regtest images at 2^26 and 2^29:
     - The #794 re-split works on the engine (OpenVM's "Maximum memory exceeded" is recognised).
     - At 2^29: light_only PROVED, end state identical to RISC Zero's except the image id; tap_small's single big input runs out of memory (RISC Zero proves it); tap_full
       fails on both, as before; mem_k1140 splits into 36 pieces of up to 41.2B cost each (more than a whole modern
       block), 768-778 s per piece on an A40, provable in principle but stopped (~2.6 h).
     - At 2^26 (the 4 GB-card image): light_only's aggregate runs out of memory and tap_small fails too: too small
       for adversarial blocks.
     - So completeness on the worst single input needs a larger OpenVM address space (soundness above 29 bits
       UNKNOWN: ask OpenVM) or a reclaiming allocator (an image change). This is the main open gate found tonight.
     Earlier notes (25.11.2026):
     - Allocator: OpenVM's bump allocator (openvm-platform heap; never frees). The guest REFUSES an allocator that
       reuses memory (main.rs: verify_input.cpp's per-transaction caches and Core's legacy sighash cache are keyed
       by address; review 14.11). A measurement-only image with size-class reuse made chunks 4% MORE expensive
       (results/profile_chunk, hz21/hz22), so reuse buys nothing here anyway.
     - Guest memory 512 MiB (Q10). The #794 MEM input (worst.py: 201 x OP_CHECKSIG over a 9.8 kB scriptCode, which
       Core copies per call) costs ~2 MB of never-freed heap per input (ESTIMATED from the construction), so ~200
       such inputs fit in one chunk's memory. The product's answer is host-side (#795, merged): a chunk that runs out
       of memory is re-split; mem_k1140 (regtest block 126) then proves in 7 chunks on RISC Zero (dev mode,
       /root/oomfix/runs/mem_k1140.log). The engine needs the same re-split in its own chunking (engine-export /
       prove-block cut chunks by cost, not memory): TO DO.
     - FOUND (25.11.2026): #795 (874454fc, on main, NOT yet on engine/openvm) detects a guest out of memory by RISC
       Zero's exact panic text ("Out of memory! You have been using the default bump allocator ...", lz.rs
       `is_guest_oom`). OpenVM's allocator says "ERROR: Maximum memory exceeded, program terminating." and exits 1
       (openvm-platform heap/bump.rs). On the engine the re-split would never trigger: `is_guest_oom` must also
       recognise OpenVM's message (as reported by hzengine) when the branch takes #795. Small, but required.
     - Test plan: regtest engine guest (Cargo feature `regtest` exists in engine/guest/Cargo.toml.in), product host
       built HAZYNC_NETWORK=regtest, block JSON from worst.py's regtest node (mkblock.py), engine-export, then execute
       every chunk on the engine (CPU) and compare journals; the single-tapscript and SIGHASH_NONE cases likewise.
  8. Does it prove an old block with many inputs (364,292), plus a few of the edge blocks from blocks.sh?

     **Answer (26.11.2026): 364,292 PROVED on GPU; 10 era blocks MEASURED on CPU; blocks.sh edge blocks NOT YET.**
     - 364,292 (5,569 inputs, legacy sighash over one huge transaction): 16 chunks + trie + aggregate on ONE RTX PRO
       6000 Blackwell, 201 s wall, final proof verified, every chunk and trie journal IDENTICAL to RISC Zero's
       (results/gpu_blackwell_364292). The default cost model mis-prices its legacy sighash (chunks 12.49B falling to
       1.39B); the fitted model (HAZYNC_COST_MODEL=fit) balances them (7.28B .. 5.66B), journals again identical.
     - Blocks 100,000, 200,000, 250,000, 300,000, 400,000 ... 900,000: every chunk and trie journal identical on the
       engine (CPU, results/era_costs); 130,000, 140,000, 741,000, 962,000, 965,500, 976,379 likewise (exports on the
       build box; 46 chunk/trie inputs of 976379t, 364292t, 962000t, 965500 also in a 64 MiB address space, below).
     - blocks.sh's edge blocks (26.11.2026, results/edge_blocks): 9 of 10 run with every chunk and trie journal
       identical to RISC Zero's and the aggregate identical except its 32-byte image id: both BIP30 repeats (91,842,
       91,880), 185,956, the BIP34/66/65 activations (227,931, 363,725, 388,381), 364,292, and the retarget +
       CSV / segwit activations (419,328, 481,824). 2,016 not run (its prepared block file is not in the repo).
     - The #794 attack blocks (regtest) are Q7.
  9. Do all 22,666 Core test vectors and the 97 rig cases pass with verdicts identical to risc0's?

     **Answer (25.11.2026): Core vectors MEASURED, all identical; rig cases NOT YET.** All 22,666 of Core's script and
     transaction vectors through mode 10 on both: 21,987 cases with a verdict (176 streams) IDENTICAL, 0 differing;
     the 679 cases RISC Zero's guest aborts on are refused by the engine too (0 accepted), and no engine-only abort
     (results/core_vectors). The 97 rig cases (real Core + regtest guest on crafted blocks) need a regtest engine
     build, like Q7.
     - **Rig, first run on the engine (26.11.2026, results/rig_engine): 55 of 96 cases ran, all 55 AGREE with Core
       31.1** (real proofs; 232-block base chain proved on the engine), read with rig.py's own reason table plus the
       guest's logs. Two host issues found and fixed (the guest's text missing from a failed execute; an explicit
       HAZYNC_CHUNKS overridden on the engine), which stopped the run at split_tx; the remaining 41 (signatures,
       taproot, multisig, CLTV/CSV, P2SH, BIP30 repeats) are the next run.
  10. What are the limits on memory, cycles per segment and input size, and where do they come from?

     **Answer (25.11.2026, from OpenVM v2.1.0's source and the engine; MEASURED where marked):**
     - Guest memory: 2^29 bytes = **512 MiB** address space (openvm-platform memory.rs MEM_BITS = 29; stack top
       0x00200400, text 0x00200800, heap from the end of data up to 512 MiB). Input, heap and stack share it. The
       guest's allocator is a bump allocator that never frees, on purpose (the address-keyed caches need it; reuse
       measured +4% cost anyway, results/profile_chunk). MEASURED heap use, 976,379: chunks ~8 MB, trie ~12 MB,
       aggregate ~31 MB (mostly capacity never touched) — far below 512 MiB. An adversarial memory block (#794) is
       the case to test (Q7).
     - Per segment: the metered executor cuts a segment before its traces exceed 2^24 rows in any chip
       (max_trace_height_bits = MAX_APP_LOG_STACKED_HEIGHT = 24) or the memory estimate exceeds
       segmentation_max_memory (OpenVM default 15 GiB; the engine sets it per card: memory / provers - 4, at most 24
       GiB; 32 GiB made the prover refuse its own trace). Segments are unlimited in number: a long proof is just more
       segments (MEASURED: the trie of 976,379 = 21 segments).
     - Input size: no protocol limit beyond guest memory (the input stream is held in memory once, 4-byte aligned).
       MEASURED largest so far: the trie witness of 976,379, 5.2 MB of a 6.5 MB stream; the aggregate's stream 4.4 MB.
     - Compared with RISC Zero: RISC Zero's segments are capped by segment_limit_po2 (cycles); OpenVM's by trace rows
       and memory per chip, so "cycles per segment" has no single number here.

  C. Where it has to run
  11. Browser and phone verifier: can a proof be checked in WASM in a browser? How large is the proof and verifier, how long does a check take on a phone,
  and what does the verifier trust?

     **Answer so far (25.11.2026):**
     - MEASURED: the final proof (VmStarkProof, versioned JSON) is ~642 KB (prof13/agg.proof.proof.json 642,402
       bytes); `hzengine verify` checks it natively in 0.01-0.05 s (build box, Blackwell pods). The verifier trusts
       the aggregation verifying key (agg.vk) and the baseline (baseline.json) pinned with the image, plus the image id.
     - MEASURED (wasm32-unknown-unknown build of a crate using openvm-verify-stark-host v2.1.0): **does not compile
       as shipped.** After the usual getrandom JS backend, it stops in `abi_stable` (-> `libloading`, native dynamic
       loading), a HARD dependency of openvm-circuit (crates/vm/src/arch/execution_mode/metered/memory_ctx.rs uses
       abi_stable's RVec), which the host verifier pulls in. zstd (C) is next in line.
     - Options: (a) a small patch making abi_stable optional for a verify-only build (feature-gate the metered
       executor) — upstream PR or a fork, then measure size/time in a browser; (b) OpenVM's STARK -> Halo2/KZG wrapper
       (root prover, EVM proof): a SNARK proof is small and its verifier runs in WASM, at the cost of a wrapping step
       per proof and a trusted setup (KZG); (c) a verifier of our own over the proof format (most work, most risk).
     - **MEASURED (later the same day): option (a) works.** With `abi_stable` removed from openvm-circuit (one type,
       engine/wasm-verify/openvm-abi-stable.patch), OpenVM's own verifier builds for wasm32 and, in Node on a laptop,
       ACCEPTS a real engine proof (block 130,000's aggregate) in 24-45 ms and REJECTS it with one encoded byte flipped.
       Sizes: wasm 709 KB, verifying key 96 KB, proof 229 KB (zstd). engine/wasm-verify/README.md.
     - **MEASURED (26.11.2026): the full check, in Node and in a browser.** `verify_full(vk, proof, journal, image_id)`
       adds the image check (image id derived from the vk's baseline alone, equal to the engine's image.json) and the
       journal check (public values == SHA-256(journal)) to the STARK check. On the real proof: valid accepted in
       25-39 ms (Node 24, laptop); one journal bit flipped, another image id, a flipped proof byte, a short id and a
       truncated vk each rejected with their own code; the same codes in Chromium 156 (headless shell; its timings are
       virtual, so not quoted). wasm 736,681 bytes. engine/wasm-verify/README.md.
     - Still to do: a phone run, and the abi_stable patch upstream (or carried as our one fork line). UNKNOWN: phone
       time (expect a small multiple of the laptop's).
  12. Windows: does the GPU prover build and prove on Windows with a native CUDA toolchain? What's the minimum driver version?

     **Answer (26.11.2026): BUILDS (CI), proving on a real Windows card NOT YET.**
     - The CUDA hzengine.exe builds with MSVC 14.44 + CUDA 12.9.1 on windows-2022: run 45660566592, 28 min, one
       artifact for sm_61, 70, 75, 80, 86, 89 and 120 (Pascal through Blackwell). It is packaged, not run: it imports
       nvcuda.dll, which the GPU-less runner lacks (exit 127).
     - OpenVM v2.1.0 does not build with MSVC as shipped. engine/ci/openvm-msvc.patch + stark-backend-msvc.patch
       (applied by engine/ci/openvm-patches.sh) carry same-values portability edits, each an upstream candidate:
       C++20 designated initializers, glibc's `uint`, GCC's __builtin_clz/ctz (engine/ci/msvc_builtins.h), C99
       compound literals, GCC-only warning flags, zero-length S-box arrays (same column layout), MSVC's traditional
       preprocessor and __VA_ARGS__, the GNU `?:` operator, CUB scans over Fp with an explicit operator, and an nvlink
       temp-file race (NVCC_THREADS=1). hzengine's own Unix-only code uses SetStdHandle on Windows (71ae6d49).
     - The exe predates the 25.11 night's memory work (app VM release, aggregation tree, MiB segment cap): a new
       build is needed before the Windows test (Xesktop RTX PRO 6000, then the GTX 1050 Ti laptop).
     - Minimum driver (NVIDIA, CUDA 12.9 Update 1 release notes, Toolkit Driver Version table,
       https://docs.nvidia.com/cuda/archive/12.9.1/cuda-toolkit-release-notes/index.html): Windows >= 576.57, Linux
       >= 575.57.08. CUDA's minor-version compatibility lets 12.x run on Windows >= 528.33 / Linux >= 525.60.13 with
       limitations (https://docs.nvidia.com/deploy/cuda-compatibility/latest/minor-version-compatibility.html); the
       exe embeds compiled code for each architecture, but an older driver is NOT TRIED.
  13. Cards: which GPUs are supported (oldest architecture, minimum memory)? Do consumer cards like a 3060 or 4090 work, and how fast?

     **Answer (26.11.2026): sm_61 (Pascal) and newer BUILD; MEASURED on five card types; 4 GB cards close.**
     - Architectures: OpenVM's only Volta+ instruction is `__match_any_sync` (histogram.cuh, a de-duplication of
       atomic adds). Below sm_70 each thread adds its own count, the same totals (124ff7b2, in openvm-msvc.patch), so
       Pascal (GTX 10xx) builds; Pascal proving is NOT YET RUN (the GTX 1050 Ti laptop is the test).
     - MEASURED, full block 976,379, every proof verified, journals identical: RTX PRO 6000 Blackwell 65 s (one card),
       42.6-48 s (four); A40 132 s (engine defaults, 3 provers); RTX 4090 (consumer, 24 GB) 330-357 s one prover at
       2 GiB segments; L4 604 s; RTX A6000. Consumer cards work.
     - Minimum memory (results/gpu_a40_memory): one prover, full block, nvidia-smi peak, from 6.7 GB at OpenVM
       defaults down to **3,943 MiB** (1.5 GiB segments, 584 s on a 4090) or **3,765 MiB** (1 GiB, 1,619 s), with:
       a 2^26 guest address space (keygen; every heavy chunk/trie input fits, journals identical), a proving-time
       aggregation tree of 2/2, the app VM's 1.2 GiB device key released before aggregation, and HZ_SEG_MEM_MB.
       ~400 MiB of each figure is the 4090's CUDA context (SM-count dependent; a 1050 Ti's is not measured). So 6 GB
       cards fit with margin; 4 GB cards are borderline until measured on one.
  14. Is there a CPU-only prover, and how fast is it?

     **Answer (26.11.2026): YES, the same engine without the cuda feature; MEASURED on the 16-core build box.**
     - hzengine built without `--features cuda` proves on CPU with the same keys and the same proofs (verified by the
       same verifier). Block 130,000 (1 chunk + trie + aggregate): 373 s on 16 cores (results/gpu_a40_memory, CPU check
       of f38eb2fa); 1,286 s with 2 leaf workers and 1 GiB segments (results/segpar_cpu).
     - A modern full block on CPU is not measured end to end; from the era costs (~25.8B for 976,379) and 130,000's
       rate it would be hours per block on 16 cores: a fallback and a verifier/test tool, not a way to keep up.
  15. Reproducible ids: do two clean builds on different machines give the same image id? Can that be pinned the way reproduce/LIBZYNC_IDS is pinned today?

     **Answer (25.11.2026): MEASURED on one machine, two setups.**
     - Before (d4893fac-era host): the same commit built twice on the build box, once at another path with another
       CARGO_HOME, gave DIFFERENT exe commitments (a1112967... vs 532c4d1b...; vm commitment equal). Strings in the
       program named /root/.rustup (std sources), the CARGO_HOME (git deps), the C/C++ base tree (Core's __FILE__),
       the assembled tree and the engine kit.
     - Fix d1190c2a: `--remap-path-prefix` for CARGO_HOME, RUSTUP_HOME, the assembled tree and the engine kit
       (engine.rs `remap_flags`), `-ffile-prefix-map` for the base and the tree (engine/guest/build.rs).
     - After: the two builds are **byte-identical** (guest.vmexe cmp equal), exe e635105e3b569a54...,
       vm 00e00077450a5a56... (repro2.out on the build box).
     - Not yet tested: a second machine. Remaining inputs that must match: the Rust nightly (nightly-2026-02-28, from
       OpenVM's toolchain pin), the riscv32 gcc/g++ 13.2 (~/.risc0/cpp; its own baked-in source paths are constant
       strings like /home/flaub/src, the same in every copy), the base tree's sources (base.sh: pinned Core v28.0 and
       libsecp v0.5.1 plus the patches) and OpenVM v2.1.0. A container build (as the product's reproduce/ does) pins
       all of these; with it, the ids can be pinned the same way reproduce/LIBZYNC_IDS is.

  D. Cost and speed
  16. What is the whole-chain cost, from genesis to the tip, in GPU-hours and dollars? Use measured per-block costs across eras, and say which blocks the
  estimate rests on.

     **Answer (25.11.2026): costs MEASURED on 11 blocks, the total ESTIMATED.**
     - MEASURED (engine/results/era_costs: CPU-metered cost, chunks + trie + aggregate, every journal identical to
       RISC Zero's): 100,000: 0.11B; 200,000: 4.10B; 250,000: 1.57B; 300,000: 2.44B; 400,000: 17.2B; 500,000: 20.5B;
       600,000: 15.8B; 700,000: 27.2B; 800,000: 25.8B; 900,000: 15.8B; plus 976,379: 25.8B. (364,292, an outlier:
       ~113B.)
     - ESTIMATE: costs interpolated between those heights (one block per height: noisy), converted at 976,379's
       measured rate on ONE RTX PRO 6000 Blackwell (65 s for 25.8B, ~2.5 s per B), plus a fixed cost per block for the
       three proofs' overheads, F (not measured on Blackwell for small blocks; 5-15 s assumed):
       | F | Blackwell GPU-hours | at $2.69/h (secure) | on A40s (2.46x slower, $0.59/h) |
       |---|---|---|---|
       | 5 s | ~10,900 | ~$29k | ~$16k |
       | 10 s | ~12,300 | ~$33k | ~$18k |
       | 15 s | ~13,600 | ~$37k | ~$20k |
       By era (F = 10 s): blocks 1-200k ~710 GPU-h, 200k-400k ~1,400, 400k-977k ~10,100 (83%). The fixed part is ~22%.
     - So the modern chain dominates; multi-block proofs for small blocks would save at most the early fixed costs
       (~6% of the total); per-block cost work on modern blocks is the bigger lever. To firm up: more blocks per era,
       and F measured on a small block on Blackwell. MEASURED 26.11.2026: 130,000 (10 inputs) proves in 7.2 s on 4x PRO
       6000 (leaves 3.6 + aggregate 3.5; results/gpu_blackwell_4x_969122), so F is under 10 s on 4 cards; one card
       not measured.
  17. How do the tip-block times compare (detect to verified) with risc0's current 191 s on 4× PRO 6000?

     **Answer so far (25.11.2026): proving MEASURED, detect-to-verified NOT YET.**
     - prove-block (all leaves + aggregate + final verify) for 976,379 on 4x RTX PRO 6000 Blackwell: 42.6-48 s
       (results/gpu_blackwell_hz24, gpu_blackwell_4x_hz26); 65 s on ONE card.
     - Not included and not yet measured on the engine path: block detection, the host's preparation (hints:
       48-50 s CPU for 364,292 on the build box; the export's RISC Zero executions are a test harness and would not run
       in production), and moving proofs. The product's 191 s includes all of these, so the fair comparison needs the
       engine wired into the tip pipeline (HAZYNC_ENGINE backend exists: engine_backend.rs) and timed end to end.
     - ESTIMATE (26.11.2026, from measured parts; not an end-to-end run): RISC Zero's 191 s is block 969,122 on 4x RTX
       PRO 6000, detect to verified receipt, of which the host's hints were 26.5 s on the pod (libzync 9c2123e
       rehearsal). Hints are the same code on either prover (976,379: 56 s on the 16-core build box). The engine
       proves 976,379 in 42.6-48 s on 4x PRO 6000. So the engine's detect-to-verified would be about 26.5 + 43-48 s
       plus detection and staging, roughly 70-110 s against 191 s.
     - MEASURED like for like (26.11.2026, results/gpu_blackwell_4x_969122): 969,122 itself on 4x PRO 6000, prove-block
       **33.9 s** (leaves 20.6 + aggregate 13.0), final proof verified; its hints 35 s on the build box (26.5 s on
       RISC Zero's pod); every chunk/trie journal identical. So hints + proving is ~60-70 s against RISC Zero's 191 s
       end to end. Still to measure: the engine in the live tip pipeline (detection, staging), on a live window.
  18. What do the three levers still on the table buy, measured: fewer and larger chunks, CPU/GPU overlap, and native SHA? Does any of them change a
  commitment or format?

     **Answer (26.11.2026): all three MEASURED; none changes a commitment or format.** (The trie's new format, a
     fourth lever, does.)
     - Fewer, larger chunks (host-side layout; the guest and journals are the same, the aggregate takes any count):
       976,379 on one A6000, 16 chunks 289 s vs 5 cost-sized chunks 240 s, 221 s with a 24 GiB segment cap
       (results/gpu_a6000_tuning). Small blocks gain most: 130,000 56 s (16 chunks) vs 19 s (1); 741,000 110 s vs 54 s
       (results/gpu_a6000_multiblock). Each proof has ~5 s of fixed cost (results/gpu_a40_block976379).
     - CPU/GPU overlap (prover-side): 2 provers sharing one A6000, 289 s -> 245 s, GPU use 48% -> 59%
       (gpu_a6000_tuning); the engine's default is 3 provers on 40 GiB+ cards (A40 976,379: 204 s at 74%,
       gpu_a40_defaults; 132 s on 26.11's keys and code). Segment-parallel proving across GPUs (segpar) is the
       multi-card form (Q19).
     - Native SHA (guest code: SHA-256 in OpenVM's own state layout instead of RISC Zero's byte-swapped compress;
       same digests, journals byte-identical): -22% on the trie (ENGINE.md, "What native means"). Merged; in every
       engine image since keys-hz4. It changes the image id (guest code), not a commitment format.
     - Not among the three, and a format change: the trie in its new format (node hashes take child state words
       directly), a further -22% (27.2 s vs 36.1 s, ENGINE.md item 5). It changes the trie commitment, so it needs a
       decision; with the restart from genesis (decision 25.11.2026) there is no old chain to stay compatible with.
  19. How does it scale across several GPUs for one block?

     **Answer (25.11.2026, MEASURED, 976,379 unless noted; results/gpu_blackwell_*):**
     | cards (RTX PRO 6000 Blackwell) | whole trie | notes |
     |---|---|---|
     | 1 | 65 s | leaves 43 s + aggregate 18.5 s |
     | 2 | 51 s (split trie) | |
     | 4 | 42.6-48 s | leaves 25-28 s + aggregate 16-17 s |
     364,292: 201 s on 1 card, 76 s on 4. Scaling flattens because the aggregate (16-19 s) runs alone at the end.
     segpar (the aggregate's segments over all cards) cut it 16.8 -> 14.3 s on 4 cards; reusing the leaf workers for
     it (37a02971) removes the startup cost that made the first try a net loss. RE-MEASURED 26.11.2026, same pod
     (results/gpu_blackwell_4x_969122): 976,379 aggregate 19.4 s without segpar -> 14.5 s with it (-25%), block
     42.4 -> 38.6 s; 969,122 33.9 s.

  E. Migration
  20. What has to be rebuilt? Cover the zAPPs (headers, hints, utreexo, deposit, assume, liveness, statement), the verifiers, the board and coordinator, the
  worker and the Windows app, the release tooling and the site pins.

     **Answer (25.11.2026, inventory; ESTIMATED effort):**
     - Core guest: done (the engine guest is the product source + engine/guest edits + engine/platform).
     - zAPP guests (zapps/: assume, corecheck, deposit, headers, hints, liveness, statement, utreexo): each a guest
       to build on engine/platform the same way, and its env::verify of a core proof re-pointed at the engine image.
       Their LOGIC stays (Q21: the core journals are byte-identical).
     - Verifiers: native (hzengine verify exists), WASM (probe works, Q11: needs the image/journal checks, the patch
       upstreamed or carried), the site's verifier page, swiftsync-verify / zapp verifiers that pin image ids.
     - Host side: product host already has the engine backend (HAZYNC_ENGINE: libzync-prove-range / join / verify
       through hzengine serve); the board, coordinator and worker would run hzengine (CUDA) instead of the RISC Zero
       prover; Windows app + Windows CUDA build: NOT TRIED (Q12).
     - Release tooling: reproducible build container for the engine guest (Q15), id pins (reproduce/LIBZYNC_IDS ->
       engine ids), GPU test matrix (Linux A40/A6000/Blackwell measured; Windows, consumer cards not).
  21. Which commitments and formats can stay byte-identical, so existing tools and zAPP logic carry over?

     **Answer (MEASURED):** every chunk, trie and part journal, and every aggregate journal except its 32-byte image
     id, is byte-identical to RISC Zero's on every block run (976,379, 962,000, 741,000, 364,292, 130,000, the genesis
     chain): the chain state, coin log roots, trie roots, ECMH set hash, txlog and the update-list digests are the same
     commitments. What changes: the image id(s), the proof format (OpenVM VmStarkProof, ~229 KB compressed vs a RISC
     Zero receipt), and the verifier. So zAPP logic over the journals carries over; anything that pins an image id or
     parses a receipt changes.
  22. What's the plan for re-proving from genesis, and does the existing risc0 chain proof have any role in it?

     **Answer (25.11.2026; options, ESTIMATED):**
     - Re-prove from genesis on the engine: the engine starts at genesis already (handover stubbed, edit 1;
       results/cpu_chain_genesis). Cost: Q16; the early chain is dominated by per-proof fixed cost, so multi-block
       proofs for small blocks are the lever (not built).
     - Use the RISC Zero chain proof (a handover): OpenVM has a pairing extension (BN254, guest-libs/pairing), so an
       engine guest could verify a RISC Zero receipt COMPRESSED TO GROTH16 (RISC Zero's own STARK-to-SNARK step) and
       continue from its chain state at height H. That keeps the existing proof's value and avoids re-proving
       1..H. Needs: the Groth16 verifier in the engine guest (BN254 pairing on OpenVM's extension), RISC Zero's
       verifying key pinned, and a review. Not built.
       (The trust then includes RISC Zero's Groth16 setup and prover for 1..H.)
  23. Does the engine support composition, so a zAPP can verify a core proof inside its own proof as it does today?

     **Answer (25.11.2026): YES for the mechanism, MEASURED on CPU. 26.11.2026: the first zAPP guest PORTED (headers).**
     - The header zyncAPP (engine/zapps/headers: its own source + two anchored edits) executes 0..40,319 from genesis
       with the end hash, chainwork and MMR peaks equal to Bitcoin Core's node, and proves + verifies 0..4,031 on CPU
       (results/zapp_headers). The other zAPPs (hints, utreexo, deposit, assume, liveness, statement) follow the same
       recipe; each is its own image.
     - `env::verify(image, journal)` works on the engine as on RISC Zero (Q4/Q5): the aggregate verifies every chunk
       and trie proof in-guest on every block; block 2 verified block 1's proof in-guest from genesis, and a
       `libzync-join` of blocks 2 + 3 verified both and checked adjacency; `libzync-verify` accepted all
       (results/cpu_chain_genesis). Negative controls: a tampered child journal and a child of another image are
       refused in-guest (results/NEGATIVE.md).
     - A zAPP guest (headers, hints, utreexo, ...) needs building on the engine platform (same approach as the core
       guest: its source + engine/platform), then the same env::verify of a core proof. Not done yet (Q20).
  24. What does it cost to maintain over time? Do we follow upstream OpenVM releases, and what happens to image ids when we do?

     **Answer (25.11.2026):**
     - Our own code on top of OpenVM: engine/platform (RISC Zero API shim, sha2/crypto shims, ~500 lines), the guest
       edits (edits.py, each anchored so a product change fails loudly), C patches (core-0201/0202, secp-0201/0202),
       msm_ovm.rs/batch_ecdsa_ovm.rs, hzengine (host). Plus, for WASM, one line patched out of openvm-circuit.
     - Upstream: OpenVM patch releases keep the verifying key (their VERSIONING), so a patch upgrade should keep our
       image ids; a minor release (2.1 -> 2.2) may change keys and ids. An id change needs a handover between engine
       images: verifying the old image's proofs in the new guest works if the old aggregation key can still be
       verified by the new verify-stark (UNKNOWN across OpenVM minors; to test at the next release).

  F. The deciding question
  25. Which of the above, if it came back badly, would rule the switch out, and which are just work?

     **Draft (25.11.2026), to settle with the user:**
     - Could rule it out: Q1 (if the v2.1.0 changes to the deferral/recursion path or the ECC extensions turned out
       unreviewed AND the OpenVM team will not cover them), Q9 (any verdict differing from RISC Zero / Core on the
       test vectors), Q7 (a Core-valid block the engine cannot prove at all).
       So far: Q1 partly open (audit at a pre-release commit; ECC not in the 2.0 audit); Q9 Core vectors ALL IDENTICAL (rig cases to do); Q7 untested.
       **26.11.2026:** Q9: the rig's first 55 cases all agree with Core 31.1 (41 to run). Q7 CAME BACK BADLY as the
       engine stands: tap_small (a Core-valid regtest block RISC Zero proves) cannot be proved at OpenVM's 512 MiB
       (one input runs out of memory; RISC Zero's guest has 3 GiB). It is a deal-breaker unless one of two fixes
       works: a larger OpenVM address space (soundness above 2^29 to be confirmed by OpenVM) or a reclaiming
       allocator in the guest (an image change; also what tap_full needs on RISC Zero). Q1 diff sizes measured.
     - Just work: Q11 (WASM: the probe works), Q15 (ids: fixed on one machine, container next), Q12/Q13 (Windows,
       consumer cards), Q16-Q19 (cost/speed: measured strong), Q20-Q24 (migration).
