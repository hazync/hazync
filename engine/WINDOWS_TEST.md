# Windows on the engine: the Xesktop session's checklist (ROADMAP 2.9 b/c; 28.11.2026)

Waits on the user's Xesktop session (the user starts the RTX PRO 6000 server and pastes the setup over RDP; the
machine dials out to hazync-proof's restricted `xtunnel` account, which the user approves and which is deleted
afterwards). Nothing here is measured yet: every number below is a Linux reference to compare against.

## What runs on the Windows machine

| part | where it comes from |
|---|---|
| `hzengine.exe` + `cudart64_12.dll` | engine-windows.yml run 46125613543 (28.11.2026, head 082daf2d, sm_61..sm_120), artifact `hzengine-windows-x86_64-cuda` |
| `hzengine.exe` (CPU) | engine-windows.yml's new CPU step (28.11.2026), artifact `hzengine-windows-x86_64-cpu`: built only by a run after 1be0a1fd-or-later; PENDING its first run |
| key folders | from the build box (hazync-coord, `/root/engine/keys-*`), packed as `hazync-keys-<name>.tar.gz` would be: the header image (keys-hdr29, exe a080482e..., vm 00e00077...) and the core image under test |
| block 170's export | the product host's `engine-export` for block 170 (the GPU gate's b170 entry), as a .tgz |
| the header-chain stream | the headers zAPP's input for 0..975,278 (~78 MB; built from `/root/mmr-v027/headers_0_975278.txt.gz` as engine/results/zapp_headers did) |

## Steps

1. **Block 170, CUDA.** `release-gpu-gate-windows.ps1 -Export export_170.tgz -Keys hazync-keys-core.tar.gz -Zip
   hazync-engine-x86_64-windows-cuda.zip` (it hashes every input, proves with `hzengine prove-block`, packs the HZE1
   file and writes gate-windows.json). Bring back the report, `out\block_170.hze`, prove.log.
2. **Verify on Linux.** The proof file under the core image's pinned key with the core verifier (`swiftsync-verify
   --image ... --vk ... --vk-sha256 ...`): DEMO expected (block 170 is far below the checkpoint); a flipped byte refused.
3. **The whole header chain, CUDA.** `hzengine prove --dir <keys-hdr29> --input <stream>` then `hzengine pack`.
   Reference: **268.0 s on the RTX PRO 6000 Blackwell Workstation Edition** (Linux, 26.11.2026, the same SKU as
   RISC Zero's 5,969.5 s run; 266.2 s on the first run; engine/results/zapp_headers). Compare the prove time (Windows
   runs under WDDM; RISC Zero's Windows run of block 170 on this card was 319 s, 09.11.2026). Verify on Linux: the
   journal sha256 must be 20bb754b... (the same as every Linux run), the end hash, chain work and MMR root bb22154a....
4. **The app.** Install with the installer (when an engine release exists) or lay out `prover\host.exe` (the product
   host, the driver), `prover\hzengine.exe` + cudart, `prover\keys\` by hand; the app's Setup tab must say "proves with
   the CUDA build of hzengine"; against a board (or the dry-run coordinator) its image check must name the engine image
   (tools/win-gui supervisor.engine_sidecar / check_engine; tools/win-gui/test_engine.py has the headless cases).
5. **CPU only.** With `prover-cpu\` (the CPU hzengine.exe), block 170 on the processor: record the time, nothing claimed
   before it is measured (the app's CPU sentence still quotes RISC Zero's block-1 figure; replace it).
6. **The 1050 Ti laptop** (2.8 / 2.9 d): the small-card option, after the Xesktop run.

## Still pending (not built yet)

- The product host for Windows with the engine path: the installer's driver is `hazync-host-x86_64-windows-cpu.zip`
  (windows-prove.yml's CPU host, today still the RISC Zero build), and it is NOT in scripts/release-assets.txt's engine
  list: the engine release needs that line (a Windows driver host) or the installer cannot be built (the workflow
  refuses an engine release without it). A release-contents point for the user.
- A release step that fetches the CPU hzengine.exe (`hazync-engine-x86_64-windows-cpu.zip`).
- The app's CPU sentence and the CPU_BLOCK1_S figure are RISC Zero's (tools/win-gui/supervisor.py): replace with step
  5's measurement.
- CANONICAL_ENGINE_IMAGE_ID (supervisor.py) is today's core image e3729469..., not a published one: it follows
  reproduce/engine-release.tsv at the Stage 4 freeze.
