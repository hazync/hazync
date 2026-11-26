# OpenVM engine: roadmap to deployment (26.11.2026)

The path from today's branch (`engine/openvm`) to Hazync running on the engine. **Six stages, each with a gate**: a stage
starts only when the previous gate's evidence is committed (a results/ entry or a log), never on "should work".
**[U]** marks a decision or action that is the user's. Status: ✅ done, ◐ in progress, ☐ not started.

Order (user 26.11): **gates first** (Stage 1 before more porting).

Ground rules (the user's): no release without discussing its contents; many review rounds, in different ways, before
any image freeze; only measured numbers; merged is not deployed (every stage ends deployed AND checked).

---

## Stage 1: correctness gates (the engine proves what RISC Zero proves, and refuses what it refuses)

| # | task | status | evidence |
|---|---|---|---|
| 1.1 | Guest memory: #794 C-heap recycling on the engine | ✅ | results/c_heap (+1% cost, journals identical) |
| 1.2 | tap_small (block 127's one input) PROVED with the C heap at 2^29 | ✅ (and tap_full) | results/q7_attack_blocks |
| 1.3 | light_only proved with the C heap ✅; mem_k1140 proved at 2^29 (2x PRO 6000, 1,484 s, 16 chunks) ✅ | ✅ | results/q7_attack_blocks |
| 1.4 | Node format 2 (trie / coin log) adopted, both provers agree | ✅ | results/node_format2 |
| 1.5 | **Final-image re-run** (format 2 + C heap): Core's vectors ✅ (identical), the rig ✅ 97/97 (tap_heap_794 AGREE), edge blocks ✅ 10/10, mem_k1140 ✅ proved (16 chunks, 2x PRO 6000, 1,484 s; end state = RISC Zero's except the image id); 962,000 negatives ✅ (all 12, NEGATIVES_ENGINE_OK) | ✅ | results/core_vectors, rig_engine, edge_blocks, q7_attack_blocks, negatives_962000 |
| 1.6 | Block 2,016 | ✅ identical (its file is prover/testdata/libzync/edge/block_2016.json) | results/edge_blocks |
| 1.7 | **[U]** OpenVM audit coverage: is v2.1.0's post-audit diff reviewed; do the 1.x ECC/algebra audits hold for 2.x. **User 26.11: later (park until Stage 4)**; gate 1 passes without it, gate 4 needs it | ☐ | FEASIBILITY Q1 (diff sizes measured) |
| 1.8 | 512 MiB stated as a design limit: the guest's worst-case memory per chunk measured with the C heap: **62.9 MiB worst (mem_k1140)**, tap_small 4.6 MiB; the docs statement to adopt | ✅ (ENGINE.md "Guest memory: a design limit") | results/guest_memory |

**Gate 1:** every row ✅, the final image's evidence committed, the audit answer recorded.

## Stage 2: the code

| # | task | status | evidence |
|---|---|---|---|
| 2.1 | Core guest on the engine: chunks, trie, aggregate, recursion, segpar, memory levers | ✅ | ENGINE.md, results/* |
| 2.2 | Engine chain from genesis through the product host | ✅ (1..200) | results/zapp_hints |
| 2.3 | zAPP guests: headers ✅ (whole chain, 22.4x), hints ✅ end to end, utreexo base ✅ PROVED on the engine, utreexo-tip ✅ proved on it, a step on a step too (journals = RISC Zero's except the ids); `--stream` in every zAPP host ✅ (RISC Zero inputs unchanged, checked); zeroconf ☐ (never finished on RISC Zero either: needs a fixture, results/zapp_utreexo) | ◐ | results/zapp_headers, zapp_hints, zapp_utreexo |
| 2.4 | Verifier: swiftsync-verify / corecheck accept ENGINE proofs (OpenVM's verifier + image + journal checks, as engine/wasm-verify's verify_full), native and WASM | ☐ | wasm probe: engine/wasm-verify |
| 2.5 | Verifier-side zAPPs on engine proofs: assume, deposit, liveness, statement (they rest on 2.4) | ☐ | |
| 2.6 | Handover across OpenVM versions: step 1 ✅ (verifier in-guest, 15.6B); step 2 the pin + handover mode in the aggregate; step 3 a rehearsal across two OpenVM tags with negative controls | ◐ | engine/HANDOVER.md, results/handover_step1 |
| 2.7 | hzengine as a product component: CLI surface, errors, logs; host flags the engine path left RISC Zero-shaped (rig text, chunk overrides: fixed 26.11) audited | ☐ | |
| 2.8 | Small cards (2^26, 4 GB): an EXPERIMENTAL build option, documented as unable to prove some adversarial blocks | ☐ | results/gpu_a40_memory |
| 2.9 | Windows: rebuild with the current engine; prove on Xesktop (PRO 6000), then the 1050 Ti laptop (small-card option) | ☐ later (user: not yet important) | Q12 |

**Gate 2:** every zAPP and verifier passes its own test suite on engine proofs; handover rehearsal passes with its
negative controls.

## Stage 3: infrastructure (everything that touches proofs moves)

| # | task | status |
|---|---|---|
| 3.1 | Tip pipeline on the engine: detection -> hints -> prove -> verified, live, timed end to end (FEASIBILITY Q17) | ☐ |
| 3.2 | Board / coordinator (coordinator/server.py, sponsor_bot.py, run-workers.sh, spine loop): engine workers, the engine's proof format, the pod image (CUDA 12.9, glibc 2.35 host build) | ☐ |
| 3.3 | Prep: leaf bundles, the trie store and history in node format 2 (regenerated from genesis) | ☐ |
| 3.4 | Path services (zync-path, zync-serve) and the headers follower on engine proofs | ☐ |
| 3.5 | Web: hazync.org verifier page with the engine WASM (verify_full), the headers page | ☐ |
| 3.6 | Release tooling: package-release.sh, release gates (CI, GPU gate on a real Linux GPU incl. Blackwell, and Windows when 2.9 lands), check-deployment / check-deployed-verifier for the new artefacts | ☐ |
| 3.7 | RunPod scripts: GraphQL -> REST v2 before Jan 2027 (existing note) | ☐ |

**Gate 3:** a dry run of the whole system on the engine (tip blocks proved live, board claiming, zAPPs served, web
verifier checking), with the old system untouched.

## Stage 4: freeze

| # | task |
|---|---|
| 4.1 | Reproducible container build of every engine image (core, each zAPP, handover verifier); ids pinned like reproduce/LIBZYNC_IDS; two machines agree |
| 4.2 | **[U]** Review rounds of the soundness code, in several ways: the compat layer (engine/platform), guest edits (edits.py, coinlog_ovm.py), the C heap, node format 2, segpar, the handover mode; findings fixed and re-reviewed |
| 4.3 | The OpenVM patch set minimised; MSVC, WASM, nozstd and nologging patches offered upstream |
| 4.4 | Security parameters and versions recorded (OpenVM / stark-backend tags, 100-bit params) |

**Gate 4:** an image id the user approves for the re-prove; nothing on the soundness path changes after it.

## Stage 5: re-prove from genesis

| # | task |
|---|---|
| 5.1 | Cost estimate refined: ~50 more blocks across eras and transaction types; the automatic chunk sizing checked on each kind (user, 26.11). **Found 26.11: the cost model misses heavy scripts by 17,000-24,000x (tap_small, tap_full) and splits mem_k1140 60x unevenly** (results/guest_memory) |
| 5.2 | **[U]** Budget and schedule (cards, board, sponsors) |
| 5.3 | The re-prove, with checkpoints and the spine; progress public |
| 5.4 | **User 26.11: STOP the RISC Zero chain now**: it freezes at its current tip; all proving budget goes to the engine. Its proofs stay verifiable; read-only services stay up until cutover |

**Gate 5:** the engine chain at the tip, verified by the released verifiers.

## Stage 6: release and cutover

| # | task |
|---|---|
| 6.1 | **[U]** Release contents discussed; release notes in the user's voice |
| 6.2 | Release (CI + GPU gates), deploy, check deployed (sweep with no name filter) |
| 6.3 | Cutover: board, path services, web, docs point at the engine chain |
| 6.4 | The RISC Zero chain's services retired (its proofs stay verifiable with its released verifiers) |
| 6.5 | Delving / paper updates (the user's plan) |
