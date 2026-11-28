# Local CI inventory of engine/openvm (28.11.2026)

Branch head 9d8c02a2, as a detached worktree (the working tree's uncommitted work excluded). Runner: runci.py here
(each `run:` step of a job in order, bash -eo pipefail, GITHUB_OUTPUT / STEP_SUMMARY / ENV / RUNNER_TEMP set,
event = push to main so the `changes` job runs everything, CARGO_BUILD_JOBS=4, git clean between jobs keeping target
dirs). Unlike CI, every step runs even after an earlier one failed (an inventory, not a verdict). shim/: `sudo apt-get`
and `rustup update` are no-ops (tools preinstalled; the local stable toolchain is rustc 1.99.0, CI takes whatever
stable is current: a known difference). Laptop: 8 GB RAM.

## Workflows

| workflow | triggers | here |
|---|---|---|
| adversarial.yml | push to main, pull_request, schedule, dispatch | 6 light jobs RUN (below); 8 heavy jobs NOT runnable locally |
| windows-installer.yml | pull_request (tools/win-gui/**, coordinator/hazync, itself), dispatch, call | NOT runnable (windows-2022; builds from the latest release, today v0.27.1, RISC Zero). The PR WILL trigger it (both paths changed) |
| engine-windows.yml | push to engine/openvm on engine/ci/windows-build.request, dispatch | not re-run (it runs on the branch already: runs 46032649187, 46125613543, 46136261146 green) |
| release-sign.yml | release published, dispatch | not triggered by a PR; build-aarch64-verifier step 4 asserts the CANONICAL (RISC Zero) guest id: C8.5 |
| windows-prove.yml | dispatch | RISC Zero (guest job builds the RISC Zero ELF; 10 matching steps): C8.5 or retire |
| cuda-arch-audit, cuda-probe, publish-deps-image, publish-wasm-image | dispatch only | not triggered |

## adversarial.yml, the jobs run here

| job | steps run | pass | fail |
|---|---|---|---|
| changes | 1 | 1 | 0 |
| dependency-audit (cargo-audit, every lockfile) | 2 | 2 | 0 |
| utreexo-follower (run_ci.sh) | 1 | 1 | 0 |
| zapps (every zapps/*/test.sh, 1,047 s) | 2 | 2 | 0 |
| libzync-tests | 11 | 9 | 2 |
| accumulator-tests | 426 | 419 | 7 |
| **total** | **443** | **434** | **9** |

## The 9 failures (first real error line)

| step | error | kind |
|---|---|---|
| accumulator-tests[2] path-filter selftest | CI_CHANGES_SELFTEST_FAILED 1: `reproduce/METHOD_ID` not among the include targets of the prover's crates (the closure the heavy jobs build no longer reaches prover/*) | RISC Zero-shaped (C8.5) |
| accumulator-tests[62] dark mode is actually dark | `timeout: failed to run command 'xvfb-run'` | local tool (no xvfb here); not a code failure |
| accumulator-tests[245] sponsor engine POSITIVE CONTROL | test_sponsor_engine.py --control prints "CONTROL: 9 failure(s)" but exits 0 (line 158 `sys.exit(0 if fails else 1)`), while the CI step now expects a non-zero exit | REAL (3.2 / 3.1 commits disagree on the control's exit convention) |
| accumulator-tests[415] versions agree | 9 FAILs, e.g. "engine/results/board_engine/RESULTS.md:12 claims guest id 'c7fe0eb8', which is neither canonical nor listed in reproduce/METHOD_ID" | RISC Zero-shaped (the check knows only RISC Zero ids: needs the engine records / dated results exempt) |
| accumulator-tests[416] coordinator reference | "environment variable `launcher:HAZYNC_ENGINE` is read but has no description in ENV" (and HAZYNC_ENGINE_KEYS); docs/COORDINATOR_REFERENCE.md stale | REAL |
| accumulator-tests[420] test surfaces connected | 15 FAILs: "engine/guest/batch-test contains #[test] but no workflow runs it", msm-test, engine/host, engine/journal, ... | REAL (the engine's own test crates are not in any workflow) |
| accumulator-tests[426] shellcheck | 66 warnings, nearly all in engine/results/* and engine/scripts/pod/* (SC1090, SC2155, SC2034); also swiftsync-verify/test.sh (2) | REAL (fix, or exclude dated records as experimental/ is) |
| libzync-tests[2] libzync tests | `an_opening_below_the_tip_on_the_real_run` panics, tests/liveness_format.rs:82: zapps/liveness/vectors/mainnet_1_200_history.bin not found (removed by Cutover C4, 418b6210) | REAL (C4 fallout: the test or the vector) |
| libzync-tests[9] release gate | "CLI on the real receipt: exit 1, want 2 with DEMO, image 46666b6f..." ("the file is an engine (OpenVM) proof (HZE1); a RISC Zero receipt is checked with v0.27.1's verifier"); 3 of 7 checks | RISC Zero-shaped (the gate feeds RISC Zero receipts to the engine-only verifier) |

The release-gate CONTROL (libzync-tests[10]) passed here only because the gate itself fails; it names the RISC Zero
image (reproduce/LIBZYNC_IDS): RISC Zero-shaped too.

## Not runnable locally (22 run steps in 8 heavy jobs, plus the Windows installer's 10)

reproducible-image-id, libzync-hosts and everything needing its hosts (libzync-chain, -blocks, -vectors, -rig,
-signet, soundness-suite): built in reproduce/libzync.Dockerfile, the RISC Zero container (hours; and dead once C8
step 4 removes the RISC Zero backend). All RISC Zero-shaped: replaced in C8.5 by engine jobs, not run here.

## Counts

- run locally: 443 steps; pass 434; fail 9 (1 of them a local tool gap: xvfb)
- not runnable locally: 22 heavy steps (adversarial.yml) + 10 (windows-installer.yml, Windows runner)
- RISC Zero-shaped (C8.5): the 8 heavy jobs, accumulator-tests[2] and [415], libzync-tests[9] and [10],
  release-sign.yml build-aarch64-verifier step 4, windows-prove.yml, windows-installer.yml (builds from the latest
  RISC Zero release)
- real, to fix: accumulator-tests[245], [416], [420], [426]; libzync-tests[2]
