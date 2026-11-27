# Docs sweep for the engine (ROADMAP 6.6)

The user, 27.11.2026: every doc, the paper and the web pages are updated for the engine. Inventory made 27.11.2026:
files that mention RISC Zero, risc0, receipts, Groth16, the old image ids or v0.2x (count = matching lines).

**Left as written, on purpose:** dated records (docs/history/, docs/RELEASE_NOTES_*, CHANGELOG's released entries,
engine/results/, engine/FEASIBILITY.md, engine/CHANGES.md). They say what was true then; a record rewritten later is
no record. 81 files of the 145.

## A. What a reader runs: the verifiers and zAPPs (can be done now; the code is engine only since the cutover)

| file | lines | state |
|---|---|---|
| docs/libzync/UTREEXO_ZAPP.md | 88 | ☐ |
| docs/libzync/ROADMAP.md | 52 | ☐ |
| docs/libzync/LIBZYNC.md | 45 | ☐ |
| docs/libzync/DEPOSIT_ZAPP.md | 42 | ☐ |
| docs/libzync/ZAPP_TESTING.md | 40 | ☐ |
| docs/libzync/REORG.md | 32 | ☐ |
| docs/libzync/LIVENESS_ZAPP.md | 26 | ☐ |
| docs/SOUNDNESS.md | 25 | ☐ |
| docs/libzync/STATEMENT_ZAPP.md | 23 | ☐ |
| docs/libzync/HINTS_ZAPP.md | 22 | ☐ |
| docs/libzync/HANDOVER.md | 21 | ☐ |
| docs/libzync/ZYNCAPPS.md | 18 | ☐ |
| docs/README.md | 18 | ☐ |
| SECURITY.md | 18 | ☐ |
| docs/libzync/ASSUME_ZAPPS.md | 16 | ☐ |
| docs/HANDOVER.md | 16 | ☐ |
| THIRD_PARTY_NOTICES.md | 16 | ☐ |
| README.md | 16 | ☐ |
| docs/libzync/UNDO.md | 14 | ☐ |
| docs/libzync/SWIFTSYNC_PROOFS.md | 14 | ☐ |
| swiftsync-verify/README.md | 11 | ✅ |
| docs/EXTERNAL_REVIEW.md | 8 | ☐ |
| docs/libzync/PROOF_BUNDLE.md | 7 | ☐ |
| docs/EXPLAINER.md | 7 | ☐ |
| docs/libzync/ZAPP_FOLLOWER.md | 6 | ☐ |
| docs/libzync/CONSENSUS_RULES.md | 6 | ☐ |
| swiftsync-verify-wasm/README.md | 5 | ✅ |
| .claude/skills/benchmark-infographic/SKILL.md | 4 | ☐ |
| zapps/utreexo/wasm/README.md | 3 | ✅ |
| zapps/utreexo/README.md | 3 | ✅ |
| zapps/statement/wasm/README.md | 3 | ✅ |
| zapps/liveness/wasm/README.md | 3 | ✅ |
| swiftsync-verify/vectors/README.md | 3 | ✅ |
| docs/libzync/COINLOG.md | 3 | ☐ |
| zapps/hints/wasm/README.md | 2 | ✅ |
| zapps/headers/wasm/README.md | 2 | ✅ |
| zapps/deposit/wasm/README.md | 2 | ✅ |
| zapps/assume/wasm/README.md | 1 | ✅ |
| swiftsync-verify-ffi/README.md | 1 | ✅ |
| docs/libzync/PREP_MEMORY.md | 1 | ☐ |

## B. Operations: board, coordinator, build, release, prover (after Stage 3, which changes what they describe)

| file | lines | state |
|---|---|---|
| docs/BUILD_AND_RELEASE.md | 77 | ☐ |
| docs/OPERATIONS.md | 68 | ☐ |
| coordinator/deploy/libzync/RESET_v026.md | 63 | ☐ |
| coordinator/deploy/RUNBOOK.md | 40 | ☐ |
| CONTRIBUTING.md | 39 | ☐ |
| scripts/zapp-regtest/README.md | 18 | ☐ |
| coordinator/README.md | 18 | ☐ |
| coordinator/deploy/libzync/SWITCH.md | 17 | ☐ |
| tools/win-gui/README.md | 15 | ☐ |
| docs/COORDINATOR_REFERENCE.md | 15 | ☐ |
| coordinator/deploy/headers/README.md | 13 | ☐ |
| coordinator/deploy/utreexo/README.md | 11 | ☐ |
| docs/COORDINATOR.md | 9 | ☐ |
| prover/README.md | 8 | ☐ |
| docs/RUN_YOUR_OWN_COORDINATOR.md | 6 | ☐ |
| coordinator/deploy/headers-api/README.md | 6 | ☐ |
| tools/milestone/README.md | 5 | ☐ |
| prover/host/tests/data/risc0_vectors/README.md | 2 | ☐ |
| coordinator/SEAM_FINDINGS.md | 2 | ☐ |
| .github/pull_request_template.md | 2 | ☐ |
| .github/ISSUE_TEMPLATE/bug_report.md | 2 | ☐ |
| prover/testdata/libzync/README.md | 1 | ☐ |
| coordinator/deploy/dropins/README.md | 1 | ☐ |
| .github/ISSUE_TEMPLATE/prover_board_problem.md | 1 | ☐ |

## C. hazync.org (hazync-web): 21 pages + 404

Public text: drafted on a branch, wording approved by the user, deployed with the release (6.2/6.3). Pages with the
most RISC Zero / receipt mentions: zapps/utreexo 6, how-it-works 5, contribute 5, zapps/liveness 4, zapps/deposit 4,
devs 4. Beyond the words: every number (proof times, sizes, chain progress) and the verify page's module and inputs
(image id, key file, pin).

## D. The paper (private: ~/libzync-gpu/paper/DRAFT.md, never in the repo)

DRAFT.md (1,492 lines, 46 lines on RISC Zero / risc0, none on OpenVM). Its measurements move to the engine's
(engine/results/*, only measured numbers, no proving timeframe), and it describes Hazync as it is (no RISC Zero to
OpenVM history in the abstract or introduction; at most a later development section). Persona reviews after.
