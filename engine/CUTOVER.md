# Cutover: RISC Zero out of Hazync (27.11.2026)

The user cleared it on 27.11.2026: "Feel free to change from risc0 whenever you want. No one is using the board
anyway." The RISC Zero chain stopped on 26.11 (ROADMAP 5.4); its proofs stay checkable with v0.27.1's verifiers, the
last RISC Zero release. This branch becomes engine only. A RELEASE of it is still discussed with the user first.

## Order

Each step is one commit (or a few), with every suite and `test.sh` passing at its end.

| # | step | done when |
|---|---|---|
| C1 | `engine/journal` (crate `hazync-journal`): the journal word encoding the guests commit (RISC Zero's serde, Apache-2.0, as engine/platform keeps it), `Journal`, a 32-byte id type. No RISC Zero crate in it. | its tests: encodings of every statement type byte-identical to risc0-zkvm 3.1.0's, checked once against it |
| C2 | swiftsync-verify engine only: the `engine` feature goes (always on); the RISC Zero front, receipt kinds, FakeReceipt / dev-mode checks, Groth16 and earlier-image logic go; risc0-zkvm and bincode leave its graph | suites pass on engine fixtures; the RISC Zero receipt tests are deleted, not skipped |
| C3 | Image records: one record per network holds the network's parameters, with NO image until the Stage 4 freeze (VERIFIER.md: a development image must not become every default caller's image). A reader gives `--image` and `--vk`; without them the verifier says no engine image is published yet | CLI and library tests |
| C4 | Every zAPP verifier (assume, deposit, headers, hints, liveness, statement, utreexo, zeroconf, corecheck) the same: engine only, `--vk` where it takes a proof, risc0-zkvm gone | each suite and `test.sh` |
| C5 | WASM modules engine only (`build.sh` without `--engine`), sizes measured, pins renewed (two-machine check as before) | the pins in each README |
| C6 | Host and engine renames (AUDIT.md item 1): `r0.rs`, "a RISC Zero input stream", `--self-id` | hzengine and host tests |
| C7 | Format 1 goes: `--node-format`, `take_node_format_flag`, `set_node_format(1)` callers (only the RISC Zero chain's history used it) | liveness and path tests on format 2 |
| C8 | The RISC Zero prover. Surveyed 27.11: the engine guest is ASSEMBLED from prover/methods/guest, libzync and coinbase-smt sources (engine/guest/assemble.sh), the zAPP engine guests likewise from prover/zyncapps/methods/*, so those sources stay; their RISC Zero manifests and build glue go. prover/host (13,855 lines, risc0 in 11 of its files) is also the engine's driver (prep, `engine-export`, engine_backend.rs): its RISC Zero backend goes with Stage 3 (board and tip controller on the engine), not as a deletion here. vendor/risc0-*, history/verifier* (the RISC Zero chain's readers; v0.27.1 keeps them released) go once nothing builds against them | a build of everything left |

Not here: the site (its wasm and text change with the release, 6.2/6.3, wording approved by the user), the
board's services (6.4), image records for the engine chain (Stage 4 freeze).

## Log

- 27.11.2026 C1 done: `engine/journal` (hazync-journal). Its serde is risc0-zkvm 3.1.0's file for file (diff: WORD_SIZE
  as a constant). `compat/` (cargo run --release): 2,000 random values of a struct using every serde type the encoding
  has, plus a real ChainState and the id word layout, encode and decode byte-identical to risc0-zkvm 3.1.0.
- 27.11.2026 C2 done: swiftsync-verify engine only (no risc0-zkvm, no bincode; hazync-journal for the encoding), its
  CLI, C ABI (swiftsync-verify-ffi) and the wasm crate's sources. vectors/ rebuilt on the engine proof of mainnet
  1..200 (make_vectors.py; 16/16), test.sh 19/19, C ABI test.c 18/18 and test_engine.c 7/7.
- 27.11.2026 C3 done: one record per network, no image id or key until the Stage 4 freeze; every surface takes the image
  id and its key (CLI: pinned as given; C ABI and WASM: with its pin).
- 27.11.2026 C4 done: every zAPP verifier engine only, risc0-zkvm out of every graph (cargo tree, all features). Each
  module's test.sh passes up to its wasm steps (build, README pin, .mjs glue), which are C5's. The regtest harness's
  OK-verdict scripts (test-regtest.sh of assume, deposit, liveness, statement) are deleted with its RISC Zero
  receipts, until the harness is proved on the engine.
- 27.11.2026 C5 done: every WASM module engine only (engine/scripts/wasm_ovm.sh; one core module; engine-graph locks).
  Eight modules built on the laptop (twice) and in a fresh clone of ba9ec0b8 on hazync-coord, published image,
  REQUIRE_CLEAN + REQUIRE_PUBLISHED: equal; READMEs pinned. Core 423,774 bytes (1,241,166 with RISC Zero). Node tests
  ported to engine proofs, all pass (counts in VERIFIER.md). Not done: the site and deploy.sh (with the release),
  swiftsync-verify-wasm/zapp/check.mjs (needs the RISC Zero board's live server), the page in a browser.
- 27.11.2026 C7 done: `--node-format` goes from zync-path, zync-serve and zapp-liveness (and `take_node_format_flag`):
  producers build paths in node format 2, the thread default. Readers still take each proof's format from its profile
  (`use_format_of`), so liveness's format-1 test chains (the regtest harness's host data) still check. swiftsync-verify
  lib 22 passed + test.sh rc 0, liveness test.sh rc 0 (wasm 30/30), the core wasm rebuilt: still 1f6ebed9 (the helper
  sat at the end of lib.rs). Also 27.11: prover/host's RISC Zero receipt tests read tests/data/risc0_vectors (101
  passed); zyncapp builds against the engine-only verifiers.
