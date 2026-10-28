"""Fit the host packer's HAZYNC_COST_MODEL=fit coefficients to measured per-transaction cycles.

Usage: python3 -I fit_packer.py <chunk-profile log, 1 chunk, HAZYNC_TX_CYCLES build> <features.csv>

The log holds one "TXCYC <tx_len> <n_owned> <cycles>" line per transaction in block order and one
"TXFIN <cycles>" for the end-of-chunk batch check. features.csv comes from HAZYNC_DUMP_INPUT_FEATURES.
Per-transaction feature sums are fitted by non-negative least squares, weighted by 1/cycles so the
small transactions (most of a block) count as much as the few big ones. TXFIN is spread over the
predicted verifies and added to the ECDSA / SCHNORR coefficients, because the batch check is paid
per signature at the END of the chunk that owns it. Prints HAZYNC_FIT_* env assignments.
"""
import csv
import re
import sys
from collections import OrderedDict

import numpy as np
from scipy.optimize import nnls

log, feats = sys.argv[1], sys.argv[2]
txcyc = [tuple(map(int, m.groups())) for line in open(log, errors="replace")
         for m in [re.search(r"TXCYC (\d+) (\d+) (\d+)", line)] if m]
fin = [int(m.group(1)) for line in open(log, errors="replace") for m in [re.search(r"TXFIN (\d+)", line)] if m]
rows = list(csv.DictReader(open(feats)))
names = list(rows[0].keys())[3:]
by_tx = OrderedDict()
for r in rows:
    t = by_tx.setdefault(r["tx_idx"], {"len": int(r["tx_len"]), "n": 0, "f": np.zeros(len(names))})
    t["n"] += 1
    t["f"] += np.array([float(r[k]) for k in names])
txs = list(by_tx.values())
assert len(txs) == len(txcyc), f"{len(txs)} transactions in features, {len(txcyc)} TXCYC lines"
for t, (ln, n, _) in zip(txs, txcyc):
    assert t["len"] == ln and t["n"] == n, (t["len"], ln, t["n"], n)

A = np.array([t["f"] for t in txs])
y = np.array([c for (_, _, c) in txcyc], dtype=float)
w = 1.0 / np.sqrt(y)
coef, _ = nnls(A * w[:, None], y * w)
pred = A @ coef
print(f"{len(txs)} transactions, measured {y.sum()/1e6:.1f} M, fitted {pred.sum()/1e6:.1f} M, "
      f"median |err| {np.median(np.abs(pred - y) / y) * 100:.1f}%")
sigs = A[:, names.index("ECDSA")].sum() + A[:, names.index("SCHNORR")].sum()
per_sig = (sum(fin) / sigs) if fin and sigs else 0.0
print(f"batch check {sum(fin)/1e6:.1f} M over {sigs:.0f} predicted verifies = {per_sig:.0f} per verify")
for name, c in zip(names, coef):
    if name in ("ECDSA", "SCHNORR"):
        c += per_sig
    print(f"HAZYNC_FIT_{name}={c:.0f}")
# The largest misses, so a missing feature shows up by name.
err = sorted(range(len(txs)), key=lambda i: -abs(pred[i] - y[i]))[:8]
for i in err:
    print(f"  tx#{i} len={txs[i]['len']} n={txs[i]['n']} measured={y[i]/1e6:.2f}M fitted={pred[i]/1e6:.2f}M "
          + " ".join(f"{k}={v:g}" for k, v in zip(names, A[i]) if v))
