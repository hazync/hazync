# Epochs: spend-age measurement (2026-11-02)

`spendage.py` (read-only, any Core node with the default undo data, `getblock` verbosity 3) samples 600 blocks evenly over
the last 8,064 before tip 974,274 (966,211..974,274) and records each input's coin creation height.
`spendage_974274.json` is its output: totals, per-block counts and the full spend-age histogram.

MEASURED: 4,735,086 inputs (7,892 per block), 6,483,471 outputs.

| spent within | share of inputs |
|---|---|
| 1 block | 45.3% |
| 6 blocks | 57.7% |
| 144 | 78.6% |
| 1,008 | 86.5% |
| 2,016 | 88.5% |
| 4,032 | 90.3% |
| 52,560 (~1 year) | 95.0% |

Spends that cross an epoch boundary (coin created in an earlier epoch than the spending block; epoch of block x =
ceil(x / L)), i.e. the spends that would need a deletion path in a durable-coin tree:

| L | crossing | per block mean / median / p90 |
|---|---|---|
| 144 | 28.1% | 2,220 / 1,964 / 4,512 |
| 1,008 | 17.6% | 1,385 / 1,118 / 2,815 |
| 2,016 | 15.1% | 1,191 / 854 / 2,418 |
| 4,032 | 12.9% | 1,018 / 678 / 2,232 |

ESTIMATED (not measured): with 929 cycles per SHA-256 parent (measured in the guest) and a ~1.7e8-leaf tree, a deletion
is ~2 x 27.3 hashes; paths in one block sharing upper levels (2 x 929 x k x (log2(n/k) + 1)) give per block ~69 M
(L=144), ~45 M (1,008), ~39 M (2,016), ~33 M (4,032) cycles, against ~427 M for block 962,000. To measure next: one path's
real cycles in the guest and the batching saving, for an append-only forest with deletions and a compact
outpoint-keyed tree.
