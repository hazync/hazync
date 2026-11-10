# history

Code from the original Hazync design, which proved each signature with Bitcoin Core's code in a per-signature
CORE guest (image `37987b85`, last release v0.22.1). libzync replaced it on `main` in #670 (2026-11-08); the chain is
to be re-proved from genesis under libzync's published images, so nothing here is on a current proving path.

Kept, not deleted, because it is the record: the evidence the CORE design's numbers rest on, and the experiments
that were measured and not adopted. Paths mirror where each item used to live. Nothing here is built, tested or
run by CI, and relative paths inside it (Cargo path dependencies, scripts) still point at the old layout.

| | |
|---|---|
| `prover/evidence/` | the CORE era's committed measurements and run logs, with provenance (`prover/evidence/README.md`) |
| `prover/*.sh`, `prover/*.py` | CORE harnesses and demos run by hand: node sync from a proof, IBD equivalence, reorg and skip-gate probes, snapshot binding, the bundle round trip, multi-GPU fan-out, fold concurrency, the CORE SNARK and `verify-any` gates; and the CORE CI gates: the reject-path negative tests (`ci_negative_tests.sh`, its generator `make_negative_tests.py` and `block_741000_badwit.json`) and the activation-boundary blocks (`ci_boundary_tests.sh`; its fixtures stay in `prover/testdata/boundaries/`, which libzync's CI also reads) |
| `prover/range_170.bin`, `prover/range_14592.bin` | two CORE range receipts |
| `prover/testdata/snark/`, `prover/testdata/ffi_adversarial.cpp` | the CORE Groth16 fixtures and the C++ adversarial suite for the CORE FFI verifier |
| `verifier/`, `verifier-ffi/`, `verifier-wasm/` | the CORE verifiers: standalone CLI, C ABI and browser build of a CORE range proof. libzync's are `swiftsync-verify/`, `swiftsync-verify-ffi/` and `swiftsync-verify-wasm/` |
| `rangestate/` | the CORE range-proof journal (`RangeState`) shared by those verifiers |
| `audit-fuzz/` | the fuzz harness for the CORE guest's Utreexo delete (`utreexo.rs`), with its SEC-2 control seed |
| `leaf-differential/` | the CORE coin leaf built by the guest's C++ and the host's Rust, compared byte for byte |
| `fuzz-native/` | the native fuzzer for the CORE guest's C++ |
| `tools/utxo-evidence/` | the UTXO-set evidence at block 136,432 (`docs/history/UTXO_EVIDENCE_136432_2026-10-14.md`) |
| `scripts/` | CORE-only tooling: the CORE reproducible-build wrapper, the deps-image publisher, the stranger test, the CORE re-baseline tool (`rebaseline-id.sh`), and the checks of the CORE spec (`check-spec.sh`) and of the `RangeState` mirrors (`check-rangestate.sh`) |
| `.github/workflows/cross-platform-guest.yml` | the manual workflow that embedded the CORE guest off Linux; `windows-prove.yml` does it for libzync |
| `experimental/` | rejected experiments (the `sys_bigint` field backend, the field-op profile) and libzync's retired design-B documents and scripts |

The CORE design's documents are in [`../docs/history/`](../docs/history/README.md); its operator's reference is still
`docs/OPERATIONS.md`, which serves the CORE board until the board switches to libzync.

**Retired from CI with it** (2026-11-09; the job definitions are in git history before that date): the CORE-mode
`soundness-suite` (block-170 regression, adversarial suite, negative and boundary tests, BIP30, script-flag schedule,
bundle round trip, leaf differential, the host unit tests in CORE mode), `boundary-heavy`, `deployed-verifier`
(the browser verifier the site served) and the CORE verifier steps of `accumulator-tests`. `soundness-suite` is now
the gate over libzync's soundness jobs.

**Removed from the host** (`prover/host` is libzync-only and refuses to build without `HAZYNC_LIBZYNC`): every command
that drove a guest mode a libzync image refuses (1 block, 2 chain step, 3 multi-check, 6 range, 7 fold, 12 stages:
`prove-block`, `prove-chain`, `prove-snark`, `prove-full`, `check-full`, `check-ibd`, `prove-ibd`, `prove-range`,
`prove-seg`, `extend-spine`, `regress`, `adversarial`, `check-bip30`, `prove-chain-bad`, `vb-stages`, and the default
demonstration), the CORE range verifiers (`verify-range`, `verify-snark`, `verify-chain`, `snark-wrap`,
`snapshot-verify`), the CORE bundle tests (`bundle-roundtrip-test`, `script-flags-test`), seg-serve's mode-6 path, and
the CORE branches of `fold-range`, `verify-any` and `prove-range-bridge` (each now the libzync command, `lz_coord.rs`).

**Still at their old paths**: the CORE board tooling in `coordinator/` and the host's CORE bridge (`host bridge`,
`dump-snapshot`, `snapshot-emit`), which go with the board's switch to libzync; the CORE release tooling
(`scripts/release.sh`, `scripts/package-release.sh`, `.github/workflows/release-sign.yml`, `prover/build-release.sh`,
`reproduce/Dockerfile`), which changes only with a release decision; and the CORE-mode branches of the guest, which go
at the next deliberate image change because any edit to the guest source moves the published image id.
