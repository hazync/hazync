# The board on the engine: every path, ready or not (ROADMAP 3.2, 28.11.2026)

Start: Gate 3's dry run (engine/results/board_dryrun, 27.11.2026). The real coordinator and a real worker proved, folded
and anchored mainnet 1..8 on the engine on hazync-coord's CPU, configured by environment only. This file lists every
path of the board, the worker, the sponsor bot and the spine, and whether it is ready for the engine. Line numbers are
at engine/openvm, 28.11.2026.

How the engine is selected: the product host (`HAZYNC_HOST`) is the engine's driver. `HAZYNC_ENGINE` (hzengine) +
`HAZYNC_ENGINE_KEYS` (the image's key dir) make it prove on the engine; `HAZYNC_ENGINE_VK` + `HAZYNC_ENGINE_VK_PIN` make
it verify HZE1 under the pinned key and report the engine image as its `method-id` (prover/host/src/engine_backend.rs
l.34-41, l.342-366). Nothing in the Python names a prover: every proof operation is a host command.

## Ready (measured in the dry run, or by the tests below)

| path | where | how it works on the engine | evidence |
|---|---|---|---|
| submit verification (a range) | coordinator/server.py l.1267-1308 | `host verify-any <file> <lo> --check-end`: HZE1 under the pinned key; refusal classes from lz_coord.rs `engine_refusal` | board_dryrun: 8 leaves + 7 folds accepted; refusals table |
| spine verification | server.py l.1381-1417 | `host verify-any <spine> 1 --check-end` | board_dryrun: spine [1..8] |
| /api/meta method_id | server.py l.4699-4710 | `host method-id` = the engine image (from HAZYNC_ENGINE_VK) | board_dryrun: c7fe0eb8... |
| genesis tip | the unit's GENESIS_TIP | same value as the libzync board (same state layout) | board_dryrun |
| worker: prove a block | coordinator/hazync l.1021-1070 (`prove-range-bridge`) | the host proves through hzengine | board_dryrun: blocks 1..8 |
| worker: segment ladder | hazync l.598-622 | `seg-po2` answers `engine`: one attempt, no ladder | test_worker_engine.py 1-3 |
| worker: fold | hazync l.1994-2069 (`fold-range`) | engine join, mode 18 | board_dryrun: 7 folds |
| worker: spine | hazync l.2102-2170 (`fold-range` on a libzync board) | engine join from [1..N] | board_dryrun: 4 steps |
| spine loop | coordinator/hazync-spine-loop.sh | runs `hazync-worker spine`; env passes through | (no engine-specific code) |
| spine offsite verifier | deploy/libzync/hazync-lz-verify-spine.sh | `host verify-any <spine> 1`, env passes through | (the unit needs the VK env: see below) |
| check-proofs | coordinator/check-proofs.py | hashes and verify records | board_dryrun: 15 of 15 |

## Closed here (28.11.2026)

| gap | fix | test |
|---|---|---|
| run-workers.sh did not check the engine's settings: a missing key file failed each block in turn after a claim; a relative HAZYNC_ENGINE path broke in the workers' directories (the 2026-11-12 HAZYNC_HOST bug) | engine preflight: HAZYNC_ENGINE executable and made absolute, HAZYNC_ENGINE_KEYS required with sdk.pk, agg.vk, openvm.toml, guest.vmexe, baseline.json | shellcheck -S warning; test_worker_device.py (the launcher's smoke paths) passes |
| the worker treated an engine box that cannot prove at all (hzengine cannot load libcudart.so.12; no key directory) as an ordinary failure: claim, fail, claim the next | both join the box-fatal list: EX_CONFIG on the first attempt, run-workers stops the loop | test_worker_engine.py 4 (control: fails) |
| the sponsor bot could only boot RISC Zero CUDA pods (hazync-host-...-cuda) | HAZYNC_SPONSOR_ENGINE=1: the release's CPU host + hzengine CUDA + hazync-keys-core.tar.gz from the signed manifest; keys unpacked; the CUDA 12.9 runtime installed if missing; engine.env sourced by the smoke and every work list; boot requires SHA_OK, KEYS_OK, CUDART_OK, GPU_OK and the engine image id | test_sponsor_engine.py (a simulated pod; control: fails) |
| no coordinator unit for the engine board | deploy/engine/hazync-engine-coordinator.service: the libzync unit's settings on port 8920, its own dirs, the engine prep's leaves, the VK and pin from /etc/hazync-engine/board.env. NOT installed | - |

## Open: needs a GPU, the box, or a decision

| gap | why it is open | needs |
|---|---|---|
| hzengine's Linux CUDA release binary and what it links | no reproducible release build exists yet (engine/RELEASE_TOOLING.md); OpenVM's cuda-builder links `cudart` dynamically (stark-backend crates/cuda-builder/src/lib.rs l.42), so the binary needs libcudart.so.12 from CUDA 12.9 on the pod: the boot installs it. `ldd` of the real binary is not yet measured | the release build (4.1 export), then `ldd` on a pod |
| the CUDA archs of the release build | the fleet cards: sm_86 (A40, A6000, 3090), sm_89 (4090, L40S), sm_120 (Blackwell); a card whose SASS is absent needs PTX JIT and a newer driver | the release build's CUDA_ARCHS; the GPU gate |
| the engine pod boot on a real pod | the cuda-cudart-12-9 package path and the keyring fallback are tested on a simulated pod only | one cheap pod (A40/A6000) running boot.sh against a staged release |
| the board at scale | 3.3's prep data (the whole chain in format 2) is at about block 377,000; the board's every-block leaf bundles are a separate paced pass | 3.3 done, then a GPU dry run (Gate 3) |
| an engine-to-engine handover on the board | `verify-any` takes a zero lineage only (board_dryrun) | a published second image |
| fold_rent.py, the tip controller (tip_*.py) | CLOSED 28.11.2026 for the engine: fold_rent.py folds through the CPU host + hzengine under engine.env on an engine pod (its --compare swaps in the CPU hzengine, sha-verified); the engine tip path is coordinator/tip_engine.py (ONE multi-GPU pod, sponsor_bot's engine boot; test_tip_engine.py); tip_driver.kill_provers(engine=True) stops hzengine + the driver host. tip_smoke / tip_runner / tip_fleet stay the RISC Zero multi-card path (seg-serve), unchanged | 3.1: the live GPU run (STAGE3.md 3.1 plan) |
| the offsite spine units (hazync-lz-offsite-spine*.service) and check units | call the host's verify-any: on the engine board they need the same HAZYNC_ENGINE_VK / _PIN as the coordinator | engine copies of those units when the board moves (deploy) |
| the sponsor pod image | runpod/pytorch:2.4.0-py3.11-cuda12.4.1-devel-ubuntu22.04 (sponsor_bot.py IMAGE) stays; the CUDA 12.9 runtime is added at boot. A CUDA 12.9 image (runpod/pytorch 2.2.0-cu1290-...-ubuntu2404 tags exist, 24.11.2026) would save the install but is untested | a decision after the real-pod boot |
