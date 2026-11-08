# experimental

**`libzync-bench/`** is libzync's lab book: the benchmarks, test rigs and result files behind every number in
[`docs/libzync/`](../docs/libzync/LIBZYNC.md) and the top-level README, each with the script that produced it. It is
not compiled into the guest, the host or any release; the code that ships is in `libzync/`, `prover/` and the
`swiftsync-verify*` crates. Where to look:

| | |
|---|---|
| [`libzync-bench/results/`](libzync-bench/results/CYCLES_REAL_STATE.md) | real mainnet state: libzync against the original CORE guest, cycles and card time on one card |
| [`libzync-bench/zapp/`](libzync-bench/zapp/RESULT_zapp.txt) | the real genesis..200 proof by the published image, and the coin-check page checked end to end |
| [`libzync-bench/h7/`](libzync-bench/h7/RESULT_h7.txt) | the reproducible build of the published images |
| `libzync-bench/gpu/` | GPU runs and their drivers (`GPU_RESULTS.md`) |
| `libzync-bench/handover/`, `join/`, `leaf/`, `prep/`, `coord/` | the full test passes, joins, leaf bundles, prep and coordinator checks |
| `libzync-bench/core_vectors/`, `core_rules/`, `neg/` | Core's own vectors, rule-by-rule tests, negative tests |
| `libzync-bench/cost/` | whole-chain cost estimates (ESTIMATED, with their method) |

Experiments that were built, measured and not adopted (the field backend, the field-op profile, libzync's retired
design-B documents) are in [`../history/experimental/`](../history/experimental/README.md).
