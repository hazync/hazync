# Stage 3: the board and services on the engine (plan, 27.11.2026)

Gate 3 (ROADMAP): a dry run of the whole system on the engine (tip blocks proved live, board claiming, zAPPs served,
web verifier checking), with the old system untouched.

## 3.2 The board: the same three host commands, answered by the engine

The coordinator (`coordinator/server.py`) and its worker (`coordinator/hazync`) drive a host binary through three
commands (prover/host/src/lz_coord.rs): `prove-range-bridge <h>`, `fold-range <l> <r> <out>`, `verify-any <f> [<lo>]
[--check-end]`, plus `libzync-genesis-tip` and `method-id`. A host that answers them for the engine leaves the board,
claims, folds and the frontier unchanged. Surveyed 27.11:

| command | today with HAZYNC_ENGINE set | to do | state |
|---|---|---|---|
| `prove-range-bridge <h>` | `lz_prove_range_cmd` already proves on the engine (2.2: 1..200 through the product host) | write the result as ONE file, HZE1 (what readers take), not `block_<h>.rcpt` | ✅ real CPU run 1..4 (results/board_engine) |
| `fold-range <l> <r> <out>` | `lz_join_cmd` joins on the engine (mode 18), inputs as P.proof.json + P.journal pairs | take and write HZE1: hzengine serve accepts an HZE1 child, and writes HZE1 | ✅ real CPU run 1..4 (results/board_engine) |
| `verify-any` | RISC Zero only (receipt decode, FakeReceipt, METHOD_ID) | under HAZYNC_ENGINE: verify-core `verify_full_bytes` with the pinned key, image id from the keys, then the same RANGE-OK checks (anchor = state before lo, end=board) on the decoded ChainState | ✅ tests + real CPU run (results/board_engine) |
| `libzync-genesis-tip` | link digest of the genesis state | unchanged (the state layout is the same) | unchanged |
| `method-id` | the RISC Zero METHOD_ID | the engine image id (HAZYNC_ENGINE_KEYS) | ✅ real CPU run 1..4 (results/board_engine) |

Then the worker: its image check uses `method-id`; the seg-po2 retry ladder is RISC Zero's (the engine sizes chunks
itself); the pod image carries hzengine and its keys (keys from B2, pinned by SHA-256). The coordinator's config:
the engine image id, and verify-any run with the engine env. Tests: the existing coordinator suite (mocked host), and
a real CPU run of a small range (blocks 1..4: two leaves, a fold, verify-any on each, the board's chain check).

## 3.1 Tip pipeline, 3.3 prep, 3.4 services

- 3.3 first: prep's leaf bundles, trie store and history regenerated from genesis in node format 2 (the engine's), on
  hazync-proof beside the RISC Zero run's, which stays untouched (Gate 3).
  **27.11.2026: running** (hazync-engine-prep / -fetch on hazync-proof, 1..977,800 from genesis; format 2 checked: its state
  at 200 = the engine chain's proven state; 180,000 blocks in the first 3,187 s; engine/results/prep_format2). The board's
  every-block bundles are a separate paced pass once the engine board runs.
- 3.1: the tip controller (coordinator/tip_*.py) runs the engine host (HAZYNC_ENGINE) on rented GPUs; measured end to
  end against FEASIBILITY Q17.
- 3.4: zync-path / zync-serve / the headers follower on the engine chain's history and proofs (their code is engine
  only since the cutover; this is deployment, new units beside the old ones).

## Order

3.2 (code, testable on CPU now) -> 3.3 (prep regenerated) -> dry run of the board on a small range -> 3.1 (GPU, costs:
quoted first) -> 3.4 -> 3.5 / 3.6 with the release. prover/host's RISC Zero backend is removed after the dry run
(CUTOVER C8).
