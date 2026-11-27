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
