# The first engine release: a proposal for 6.1 (27.11.2026)

A draft to go through with the user (feedback: no release without discussing its contents). Nothing here is decided;
the scripts are unchanged until it is.

## Two ways to ship

| | A. verifiers and provers now, chain proving in public | B. after Gate 5 (the engine chain at the tip) |
|---|---|---|
| what ships | engine-only verifiers (CLI, wasm, every zAPP), the engine prover (hzengine + host), the board on the engine | the same, plus the published proof of the whole chain |
| a reader's verdict on the day | DEMO on the core chain until the re-prove passes Core's minimum work and the checkpoint; OK for the header chain (proved) and deposits bound to it | OK |
| what it needs first | Gate 3 (dry run), Gate 4 (frozen images, review rounds, the user's image approval) | also Gate 5 (the re-prove: budget 5.2, the user's) |

Both need Gate 4: a release publishes image ids, and nothing on the soundness path may change after that.

## Assets (today's list, scripts/release-assets.txt, and what changes)

| asset | now | engine release |
|---|---|---|
| hazync-host-x86_64-linux-gnu, -cuda | the product host with the RISC Zero guest embedded | the product host as the engine's driver (prep, export, board commands); RISC Zero backend gone (CUTOVER C8) |
| (new) hazync-engine-x86_64-linux-gnu, -cuda | - | hzengine, the OpenVM prover (CPU; CUDA 12.9) |
| (new) hazync-engine-keys-<image>.tar | - | per published image: agg.vk, baseline.json, openvm.toml, sdk.pk, guest.vmexe (keys-ch today: 1.6 MB pk, 96 KB vk, 31.9 MB vmexe), from reproduce/engine-ids.sh, so anyone can rebuild and compare |
| hazync-verify-x86_64-linux-gnu, hazync-verify.wasm, .js | the libzync verifier (RISC Zero) | engine only (C2-C5); the wasm 1,241,166 -> 423,774 bytes |
| (new) the zAPP CLIs and wasm modules | built from source | to decide: ship them, or keep "build from source" |
| hazync-host-x86_64-windows-cuda/-cpu.zip, the installer | Windows | needs 2.9 (Windows on the engine; parked by the user): left out, or shipped as before for the old chain? |
| hazync-worker, coordinator, run-workers | proof party | engine-aware worker (3.2) |
| hazync-verify-aarch64 (sign job) | built in CI | engine verifier for aarch64 (CI minutes: ask) |

Published with the assets: the image records (id, key pin, profile, anchor, checkpoint) in swiftsync-verify's IMAGES
and each zAPP's, the wasm shas on the site and in deploy.sh, and engine/results/reproduce_engine's commands for a
reader to reproduce the ids.

## Gates (3.6)

- CI gate as today (scripts/release-ci-gate.sh), plus the engine suites (every zapps/*/test.sh, swiftsync-verify,
  the C ABIs). CI minutes cost money: the user approves the runs.
- GPU gate (feedback: the exact binaries prove real blocks on a real Linux GPU, Blackwell included, before a
  release): the release's hzengine CUDA build proves a block and the header chain on an RTX PRO 6000 (the 268.0 s
  reference) and on one Ada/Ampere card (the A6000's 722.0 s). scripts/release-gpu-gate*.sh need the engine path.
- Windows (feedback: a Windows card too): only if Windows ships (2.9).
- check-deployment / check-deployed-verifier: the new artefacts and the site's wasm sha.

## For the user to decide

1. A or B.
2. Windows in or out of the first engine release.
3. zAPP CLIs and modules as release assets, or source only.
4. The version number (the RISC Zero line ended at v0.27.1).
