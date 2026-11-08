# history

Code from the original Hazync design, which proved each signature with Bitcoin Core's code in a per-signature
CORE guest (image `37987b85`, last release v0.22.1). libzync replaced it on `main` in #670 (2026-11-08); the chain is
to be re-proved from genesis under libzync's published images, so nothing here is on a current proving path.

Kept, not deleted, because it is the record: the evidence the CORE design's numbers rest on, and the experiments
that were measured and not adopted. Paths mirror where each item used to live.

| | |
|---|---|
| `prover/evidence/` | the CORE era's committed measurements and run logs, with provenance (`prover/evidence/README.md`) |
| `prover/*.sh`, `prover/*.py` | CORE harnesses and demos run by hand: node sync from a proof, IBD equivalence, reorg and skip-gate probes, snapshot binding, the bundle round trip, multi-GPU fan-out, fold concurrency, the CORE SNARK and `verify-any` gates |
| `fuzz-native/` | the native fuzzer for the CORE guest's C++ |
| `tools/utxo-evidence/` | the UTXO-set evidence at block 136,432 (`docs/history/UTXO_EVIDENCE_136432_2026-10-14.md`) |
| `scripts/` | CORE-only tooling: the CORE reproducible-build wrapper, the deps-image publisher, the stranger test |
| `experimental/` | rejected experiments (the `sys_bigint` field backend, the field-op profile) and libzync's retired design-B documents and scripts |

The CORE design's documents are in [`../docs/history/`](../docs/history/README.md). Still at their old paths until
the CORE fleet is retired (they are wired into CI and the release): the CORE verifiers (`verifier*`, `rangestate`),
the CORE bridge tooling in `coordinator/`, and the CORE-mode branches of the guest, which go at the next deliberate
image change because any edit to the guest source moves the published image id.
