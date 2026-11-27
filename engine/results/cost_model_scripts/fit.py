# fit.py <report_v1.txt> <measured.txt> <dump dir>: least squares (rows weighted 1/measured) of each measured chunk's
# engine cost on the chunk's sums of: base+signatures+2*share (the model's other terms), legacy_pos, script_extra,
# and a constant per chunk. Prints the coefficients and the rates in the model's own units.
import sys, collections, numpy as np
rep, meas, d = sys.argv[1:4]
bounds = collections.defaultdict(dict)
for l in open(rep):
    p = l.split()
    if len(p) > 4 and p[2] == 'chunk':
        lo, hi = p[4].strip('[)').split('..'); bounds[p[1]][int(p[3])] = (int(lo), int(hi))
m = collections.defaultdict(dict)
for l in open(meas):
    b, c, v = l.split(); m[b][int(c.split('_')[1])] = int(v)
rows, ys, names = [], [], []
skip = set(sys.argv[4].split(',')) if len(sys.argv) > 4 else set()
for b in bounds:
    if b in skip or b not in m or len(m[b]) != len(bounds[b]): continue
    inp = [list(map(int, l.split())) for l in open(f"{d}/{b}.inputs")]
    for c, (lo, hi) in bounds[b].items():
        s = np.array(inp[lo:hi]).sum(axis=0)          # v1, base+ops, whole, share, legacy_pos, script_extra
        x = [s[1] + 2 * s[3], s[4], s[5], 1.0]
        rows.append(x); ys.append(m[b][c]); names.append(f"{b}:{c}")
A = np.array(rows, float); y = np.array(ys, float); W = 1 / y
coef, *_ = np.linalg.lstsq(A * W[:, None], y * W, rcond=None)
a, bl, r, g = coef
print(f"alpha (engine per model unit) {a:.4f}  legacy_pos {bl:.1f}  script_extra {r:.1f}  per-chunk {g:,.0f}")
print(f"=> COST_PER_LEGACY_POS {bl/a:.1f}  COST_PER_SCRIPT_BYTE {r/a:.1f} (model units)")
pred = A @ coef; err = pred / y - 1
for n_, e in sorted(zip(names, err), key=lambda t: -abs(t[1]))[:8]: print(f"  worst {n_:16} {e:+.3f}")
print(f"  rows {len(y)}  median |err| {np.median(abs(err)):.3f}")
