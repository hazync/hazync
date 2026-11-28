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

**Gate 3, the board half: done on a small range (27.11.2026, results/board_dryrun).** The real coordinator and worker
on hazync-coord's CPU, mainnet 1..8, configured for the engine by environment only (no coordinator code change): 8
leaves claimed, proved and re-verified by the board (414-432 s each), 7 folds (271 s each), the spine to [1..8]
genesis-anchored, check-proofs 15/15; a tampered journal, a tampered proof, another image's proof and a misplaced
range refused, the real proof accepted. Found and fixed: a forged proof was reported as another build (99c0572c).
Left: GPUs and the board's scale (pod image, 3.3's data), an engine-to-engine handover on the board.

## 3.1 Tip pipeline, 3.3 prep, 3.4 services

- 3.3 first: prep's leaf bundles, trie store and history regenerated from genesis in node format 2 (the engine's), on
  hazync-proof beside the RISC Zero run's, which stays untouched (Gate 3).
  **27.11.2026: running** (hazync-engine-prep / -fetch on hazync-proof, 1..977,800 from genesis; format 2 checked: its state
  at 200 = the engine chain's proven state; 180,000 blocks in the first 3,187 s; engine/results/prep_format2). The board's
  every-block bundles are a separate paced pass once the engine board runs.
- 3.1: the tip controller (coordinator/tip_*.py) runs the engine host (HAZYNC_ENGINE) on rented GPUs; measured end to
  end against FEASIBILITY Q17.
  **28.11.2026: the code, tested on simulated pods (no pod rented).** The RISC Zero tip path spreads one block's
  segments over a fleet of single-GPU cards (tip_smoke / tip_runner / tip_fleet, seg-serve + seg-connect). The engine
  proves a whole block on the GPUs of ONE machine (engine_backend.rs, HAZYNC_ENGINE_GPUS), so its tip path is
  `coordinator/tip_engine.py`: one pod booted by sponsor_bot's engine boot (signed release, CPU host + hzengine CUDA +
  core keys, CUDA 12.9, SHA_OK / KEYS_OK / CUDART_OK / GPU_OK + the engine image id), fed leaf_<h>.bin bundles in
  height order from --tip-from (never skipping: the 968,984 / 968,986 gap), each proved with `prove-range-bridge`,
  checked on the pod (`verify-any`) and again HERE under the release key's pin (HAZYNC_ENGINE_VK / _PIN), logs and
  proofs harvested before the release, the pod released by the run's unique prefix, a signal (SIGUSR1 included) a
  teardown. test_tip_engine.py (22 checks; its control fails 11) and fold_rent.py selftest (engine pods: engine.env,
  the CPU host as driver, the CPU hzengine sha-verified for --compare) are in adversarial.yml.

  **The GPU measurement (FEASIBILITY Q17 end to end), when 3.3's prep reaches the tip (it is at ~377,000 on
  28.11.2026). Not rented until then.**
  - What it times, per block, from the run's ledger (tip_stage legs) and the bridge's own log: block seen by the node
    -> bundle written by the prep box (detect + hints + bundle) -> bundle synced -> pushed to the pod (`push`) ->
    proved on the pod (`prove`, the host's own clock) -> fetched and verified here under the pin (`verify`). The
    end-to-end number is seen-by-node to verified-here; the 10-minute gate is against that.
  - The card: 4x RTX PRO 6000 Blackwell Server Edition, secure, the configuration measured on the engine (969,122
    in 33.9 s, 976,379 in 38.6 s, prove-block; results/gpu_blackwell_4x_969122). The run goes through
    `prove-range-bridge` (the product path), not `hzengine prove-block`: the first run also measures that difference.
  - Cost, quoted from measured numbers only (`tip_engine.py plan`): the pod at the measured $6.76/hr bills by the
    clock; at the tip each block is waited for (Bitcoin's 600 s target interval), so 6 tip blocks ~$8.45 (15 min setup
    + 6 x 600 s); a backlog of 6 bundles already there ~$2.12. --minutes and --budget-usd bound it whatever happens.
    The user's OK and a quoted cost come first (RunPod rules: cheapest listing of the right SKU, poll every 2-3 min).
  - Before the run: the release (or a release candidate) with hazync-keys-core.tar.gz and the engine CUDA build in a
    signed manifest (sponsor_bot's boot refuses anything else), HAZYNC_ENGINE_VK + _PIN here, an engine coordinator
    for /api/meta (the engine image id), and the bundle sync from the prep box.
- 3.4: zync-path / zync-serve / the headers follower on the engine chain's history and proofs (their code is engine
  only since the cutover; this is deployment, new units beside the old ones).

## Order

3.2 (code, testable on CPU now) -> 3.3 (prep regenerated) -> dry run of the board on a small range -> 3.1 (GPU, costs:
quoted first) -> 3.4 -> 3.5 / 3.6 with the release. prover/host's RISC Zero backend is removed after the dry run
(CUTOVER C8).
