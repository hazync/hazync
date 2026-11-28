# Release tooling for the engine release (ROADMAP 3.6; 28.11.2026)

Prep only: nothing is tagged, released or uploaded. The asset list is a PROPOSAL for the user to go through (the rule:
no release without discussing its contents). The user's decisions it follows are in RELEASE_PLAN.md ("Decided",
27.11.2026): full package (every zAPP CLI and wasm module, in the signed manifest), Windows in, v1.0.0 if the reviews
go well, release before the public re-prove.

## The published records: `reproduce/engine-release.tsv` (written at the Stage 4 freeze)

```
# <name> <network> <image id> <key pin>
core mainnet <SHA-256(exe || vm)> <SHA-256 of vk.bin>
core-signet signet ...
headers mainnet ...
```

One line per image the release publishes. Every release-time check reads it: release.sh (core id for the GPU gate),
release_gpu_gate.py (each gate entry proves and verifies under its record), release-sign.yml (each key tarball must be
its record). It does not exist yet, so all of them refuse: that is the intended state before the freeze.

## What changed (commits on engine/openvm, 28.11.2026)

| file | change |
|---|---|
| scripts/release-assets.txt | the engine list (below) |
| scripts/package-release.sh | stages every zAPP CLI (build-linux-verifier.sh) and wasm module + glue (each build.sh, clean + published), the key tarballs (from ENGINE_KEYS_ROOT, checked against the records, packed reproducibly), hzengine (from HZENGINE_CPU / HZENGINE_CUDA); skips with a named reason what this machine cannot make |
| scripts/build-linux-verifier.sh | optional `<crate dir> <bin>`: any verifier-side CLI the same way (default unchanged: swiftsync-verify) |
| scripts/release.sh | reads the records; REFUSES at preflight until its engine build and identity steps exist; step 4b runs the engine gate on hzengine and the core record |
| scripts/release_gpu_gate.py, release-gpu-gate-pod.sh | the engine gate: hzengine + key tarballs pushed to each pod; GATE entries `block` (prove-block) and `headers` (prove), each packed to HZE1; the laptop verifies each with the staged verifier for its kind under the record; tamper control per verifier; times against the measured reference on the same card (information only: 268.0 s headers on the RTX PRO 6000 Blackwell Workstation Edition, 722.0 s on the RTX A6000; the reference cards are now first in the card lists) |
| scripts/release-gpu-gate-windows.ps1 | hzengine.exe: unpacks the key tarball and the b170 export with tar.exe, prove-block, pack; `-SegMemGb` / HZ_SEG_MEM_GB replace the RISC Zero segment table |
| .github/workflows/release-sign.yml | identity: each key tarball = its record (image.json's id, vk.bin's pin); core verifiers carry the core id; the Windows zip holds one hzengine.exe; the tag's engine-release.tsv is checked out |
| tests | test_release_gpu_gate.py, test-release-gpu-gate.sh, test-release-gpu-gate-windows.sh, test-release-sign.sh updated (synthetic records and fixtures); all pass, every control fails |

## The asset list (scripts/release-assets.txt)

| assets | build route | state |
|---|---|---|
| hazync-host-x86_64-linux-gnu | prover/build-release.sh cpu (release.sh step 2) | PENDING cutover C8 (RISC Zero backend removed first); the CUDA product host is dropped (proving is hzengine's): to confirm |
| hazync-engine-x86_64-linux-gnu, -cuda | none reproducible; package-release.sh stages a given binary | PENDING: a container build of hzengine CPU + CUDA on the hosts' glibc floor |
| hazync-engine-x86_64-windows-cuda.zip | engine-windows.yml artifact | PENDING: a release step fetching it for the release commit; 2.9 (a real Windows prove) |
| hazync-verify-x86_64-linux-gnu, .wasm, .js | release.sh step 3, package-release.sh | as before (engine verifier) |
| hazync-zapp-<8>-x86_64-linux-gnu | build-linux-verifier.sh with the zAPP crate | built in package-release.sh (dry run below) |
| hazync-zapp-<7>.wasm / .js | each wasm/build.sh | zeroconf has no wasm module: PENDING, so not listed |
| hazync-keys-<7>.tar.gz | package-release.sh from ENGINE_KEYS_ROOT | PENDING: reproduce/engine-ids.sh does not export the key files or vk.bin (`hzengine pack --key`); the set of published images is the freeze's |
| worker, coordinator, run-workers | package-release.sh | PENDING 3.2 (engine-aware board) |
| sign: hazync-verify-aarch64 | release-sign.yml build-aarch64-verifier | PENDING: that job still checks the RISC Zero id (marked in the workflow) |
| sign: hazync-setup-x86_64-windows.exe | windows-installer.yml | PENDING 2.9: the Windows app (tools/win-gui) drives the RISC Zero host |
| sign: SHA256SUMS.txt(.asc) | release-sign.yml | engine identity check written and tested |

## Still to write before a release (and why it is not done here)

- **release.sh steps 2, 2b, 4**: they build and check the RISC Zero product host (method-id, LIBZYNC_IDS,
  check-dist.sh, check-release-verifiers.sh on RISC Zero receipts). The engine's need the hzengine release build, the
  Windows fetch step, and check-release-verifiers.sh on engine proofs. Until then release.sh refuses at preflight.
- **The engine gate's fixtures** (engine/gpu-gate: GATE, SHA256SUMS, the b170 export and its anchor state, a large
  block's export, the header chain's stream): made from the frozen image. The header stream is ~78 MB of input (the
  whole chain), too big for git: to fetch by sha256 (Hazync's B2) or build on the pod. engine/gpu-gate/README.md.
- **What each zAPP verifier embeds** (its own record, the core's): fixed at the freeze; release-sign.yml then checks it
  as it checks the core verifiers.
- **docs/BUILD_AND_RELEASE.md** ("The real-GPU gate", the asset list): docs group B (DOCS_SWEEP.md), after Stage 3.
- **Windows app** (supervisor.py's segment table, the installer): 2.9.

## Test results (laptop, 28.11.2026)

| test | result |
|---|---|
| python3 scripts/test_release_gpu_gate.py | 0 failures; `--control` fails (18) as it must |
| scripts/test-release-gpu-gate.sh | 0 failures; `--control` fails (28) |
| scripts/test-release-gpu-gate-windows.sh | 0 failures, including the real Windows PowerShell 5.1 runs under WSL (a fake hzengine.exe; tar.exe unpacking; exit 3 and a hang recorded); without PowerShell (as CI): 0 failures; `--control` fails (9 with PowerShell, 2 without) |
| scripts/test-release-sign.sh | PASS; `--control` fails (8) |
| scripts/test-release-attest.sh, test-release-ci-gate.sh, test-glibc-floor.sh | unchanged, pass |
| shellcheck -S warning on every changed shell file | clean |
