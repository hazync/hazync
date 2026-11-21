"""project.py <table_a.tsv> <table_b.tsv> <txcounts_5k.txt> <tip> <chainsample_1k.tsv>: whole-chain projection from the measured curve.
EVERYTHING printed here past the input tables is a PROJECTION (modelled), with the arithmetic shown.

Curve: one point per sample height, cycles per block (user = executed, padded = sum of 2^po2 over segments at po2 21).
  A (real state) where it exists, else B (state modelled). Variant "B adj": B's trie + aggregate scaled by the A/B
  ratio measured at the highest height with both (the model's error on real state there), chunk unchanged.
Interpolation, two methods (their difference is one of the stated uncertainties):
  per-tx  each sample stands for its era (boundaries at midpoints between samples); era cycles =
          blocks x base + era transactions x (sample - base) / sample txs; base = the fixed per-block cost (the A
          sample with no input, block 1,000); era transactions from getchaintxstats (txcounts_5k.txt, linear between
          5,000-block points). The method of ../cost/estimate.py.
  height  per-block cycles linear in height between samples (flat before the first and after the last); summed.
  per-input  each sample stands for its era; every block costs base + (sample - base) / sample inputs x its inputs,
          with the inputs of the chain taken from one block per 1,000 (chainsample_1k.tsv, each standing for its
          1,000 blocks; the last stratum for the blocks to the tip). Cycles track inputs (r = 0.96 over the 29 measured
          blocks with >100 inputs; transactions r = 0.74), so this is the HEADLINE method.
  pooled  one least-squares fit over all measured blocks with >100 inputs (A where both): cycles = a + b x inputs
          + c x outputs, applied to every stratum block with >100 inputs (per-input of its era otherwise).
Card time per block on ONE RTX PRO 6000, from MEASURED rates already in the repo (none measured here):
  R1  23.49 s + 0.9326 s per padded Mcycle: least-squares fit to ../results run1 (8 real-state leaf blocks 200k-300k,
      c14 bbf82800, 1 chunk, one PRO 6000, wall incl. CPU hint collection; residuals <= 1.1 s; 29-174 padded Mcycles).
  R2  1.100 s per user Mcycle all-in: block 962,000 proved in 537 s (LIBZYNC.md s4, c14, 16 chunks, quiet host) for
      375.7 + 64.9 M (chunks + aggregate, handover/RESULT_c14.txt) + 47.6 M trie (c10e5, modelled) = 488.2 M user.
  R3  1.604 s per user Mcycle all-in: run2, 5 full 2026 blocks 969,118-122, 4,194 s for 2,615 user Mcycles (modelled
      state, 1 chunk, one PRO 6000).
  R2/R3 are applied per era as max(R1's 23.49 s floor, rate x mean user cycles per block of the era).
  R4  23.77 s + 0.9940 s per padded Mcycle: MEASURED on image 46666b6f (clean v0.27.0 build), one RTX PRO 6000,
      11 blocks proved for real 170..976,000, 2 chunks, po2 21 (gpu_fit.py, raw/gpu_fit.out; residuals -6.6..+9.0 s).
      THE HEADLINE RATE; R1-R3 are kept for comparison.
Price: $2.49 per card-hour (RunPod PRO 6000, the board run's and run1's measured price)."""
import sys
TA, TB, TXC, TIP = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
CS = sys.argv[5]  # chainsample_1k.tsv: one block per 1,000 (h = 500, 1,500, ...): txs, ins, outs, weight
strata = [tuple(map(int, l.split()[:4])) for l in open(CS) if l[0].isdigit()]  # (h, txs, ins, outs)
def load(p):
    L = open(p).read().split("\n"); hd = L[0].split("\t")
    return {int(r[0]): dict(zip(hd, r)) for r in (l.split("\t") for l in L[1:] if l)}
A, B = load(TA), load(TB)
# the dispersion blocks 800,001..800,005 are not curve points (they are reported by dispersion below)
DISP = {h: B.pop(h) for h in list(B) if 800000 < h < 800010}
tc = sorted(tuple(map(int, l.split())) for l in open(TXC) if l.strip())
def cum(h):  # transactions in blocks 0..h
    for (h0, c0), (h1, c1) in zip(tc, tc[1:]):
        if h0 <= h <= h1: return c0 + (c1 - c0) * (h - h0) / (h1 - h0)
    return tc[-1][1]
I = lambda r, k: int(r[k])
both = sorted(set(A) & set(B))
hb = max(h for h in both if I(A[h], "ins") > 100) if both else None
adj = {}
for k in ("user", "padded"):
    ab = sum(I(A[hb], f"{p}_{k}") for p in ("trie", "agg")); bb = sum(I(B[hb], f"{p}_{k}") for p in ("trie", "agg"))
    adj[k] = ab / bb
print(f"A/B trie+aggregate ratio at {hb:,}: user {adj['user']:.3f}, padded {adj['padded']:.3f} (applied to B points in 'B adj')")
base = {k: I(A[1000], f"total_{k}") for k in ("user", "padded")}
print(f"base (block 1,000, no input, real state): {base['user']:,} user / {base['padded']:,} padded cycles\n")
def curve(variant):
    pts = {}
    for h in sorted(set(A) | set(B)):
        if h in A: r, src = A[h], "A"
        else: r, src = B[h], "B"
        v = {}
        for k in ("user", "padded"):
            t = I(r, f"total_{k}")
            if src == "B" and variant == "B adj":
                t = I(r, f"chunk_{k}") + adj[k] * (I(r, f"trie_{k}") + I(r, f"agg_{k}"))
            v[k] = t
        pts[h] = (v, I(r, "txs"), src, I(r, "ins"))
    return pts
def eras(pts):
    hs = sorted(pts); bounds = [1] + [(a + b) // 2 for a, b in zip(hs, hs[1:])] + [TIP + 1]
    return [(h, bounds[i], bounds[i + 1]) for i, h in enumerate(hs)]  # sample h stands for [lo, hi)
def per_tx(pts, k):
    out = []
    for h, lo, hi in eras(pts):
        v, txs, _, _ = pts[h]; n = hi - lo; t = cum(hi - 1) - cum(lo - 1)
        ptx = max(0.0, (v[k] - base[k]) / txs) if txs > 1 else 0.0
        out.append((h, lo, hi, n, n * base[k] + t * ptx))
    return out
def by_height(pts, k):
    hs = sorted(pts); out = []
    for h, lo, hi in eras(pts):
        s = 0.0
        for x in range(lo, hi):
            j = max(i for i, y in enumerate(hs) if y <= x) if x >= hs[0] else 0
            if x <= hs[0] or j == len(hs) - 1: s += pts[hs[j]][0][k]
            else:
                a, b = hs[j], hs[j + 1]; s += pts[a][0][k] + (pts[b][0][k] - pts[a][0][k]) * (x - a) / (b - a)
        out.append((h, lo, hi, hi - lo, s))
    return out
def strat(pts, k, perblock):
    """sum over 1,000-block strata of 1,000 x perblock(era sample, stratum block); the era of a stratum = its midpoint's"""
    E = eras(pts); out = {h: [h, lo, hi, 0, 0.0] for h, lo, hi in E}
    for i, (sh, tx, ins, outs) in enumerate(strata):
        lo = sh - 500 if sh > 500 else 1; hi = min(sh + 500, TIP + 1) if i < len(strata) - 1 else TIP + 1
        n = hi - lo
        h = next(h for h, a, b in E if a <= sh < b)
        out[h][3] += n; out[h][4] += n * perblock(pts[h], k, ins, outs)
    return [tuple(v) for v in out.values()]
def per_input(pts, k):
    def f(p, k, ins, outs):
        v, txs, src, sins = p
        if sins == 0: return base[k] + 0.0
        return base[k] + max(0.0, (v[k] - base[k]) / sins) * ins
    return strat(pts, k, f)
import numpy as np
def pooled_fit(k):
    rows = {h: r for h, r in B.items()}; rows.update(A)
    rows = [r for r in rows.values() if I(r, "ins") > 100]
    X = np.array([[1, I(r, "ins"), I(r, "outs")] for r in rows], float); Y = np.array([I(r, f"total_{k}") for r in rows], float)
    return np.linalg.lstsq(X, Y, rcond=None)[0]
FIT = {k: pooled_fit(k) for k in ("user", "padded")}
print(f"pooled fit (user):   {FIT['user'][0]/1e6:.2f} M + {FIT['user'][1]/1e3:.1f} k x inputs + {FIT['user'][2]/1e3:.1f} k x outputs")
print(f"pooled fit (padded): {FIT['padded'][0]/1e6:.2f} M + {FIT['padded'][1]/1e3:.1f} k x inputs + {FIT['padded'][2]/1e3:.1f} k x outputs\n")
def pooled(pts, k):
    a, b, c = FIT[k]
    def f(p, k, ins, outs):
        if ins > 100: return a + b * ins + c * outs
        v, txs, src, sins = p
        return base[k] + (max(0.0, (v[k] - base[k]) / sins) * ins if sins else 0.0)
    return strat(pts, k, f)
F, S1 = 23.49, 0.9326
res = {}
for variant in ("as measured", "B adj"):
    pts = curve(variant)
    for meth, fn in (("per-input", per_input), ("pooled", pooled), ("height", by_height), ("per-tx", per_tx)):
        eu, ep = fn(pts, "user"), fn(pts, "padded")
        U, P, N = sum(e[4] for e in eu), sum(e[4] for e in ep), sum(e[3] for e in eu)
        r1 = N * F + S1 * P / 1e6
        r2 = sum(max(F * e[3], 1.100 * e[4] / 1e6) for e in eu)
        r3 = sum(max(F * e[3], 1.604 * e[4] / 1e6) for e in eu)
        r4 = N * 23.77 + 0.9940 * P / 1e6
        res[(variant, meth)] = (U, P, N, r1, r2, r3, eu, ep)
        print(f"[{variant}, {meth}] blocks 1..{TIP:,} = {N:,}: {U/1e12:.1f} T user cycles, {P/1e12:.1f} T padded")
        for lab, s in (("R4 (measured, 46666b6f)", r4), ("R1", r1), ("R2", r2), ("R3", r3)):
            print(f"    {lab}: {s/3600:>9,.0f} PRO 6000 card-hours  ${s/3600*2.49:>9,.0f}")
v, m = "as measured", "per-input"
U, P, N, r1, r2, r3, eu, ep = res[(v, m)]
print(f"\nPer era [{v}, {m}] (user / padded cycles; R4 card-hours):")
print(f"  {'sample':>8} {'src':>3} {'era':>17} {'blocks':>8} {'era txs':>12} {'T user':>8} {'T padded':>9} {'R4 card-h':>10} {'share':>6}")
pts = curve(v)
for (h, lo, hi, n, cu), (_, _, _, _, cp) in zip(eu, ep):
    t = cum(hi - 1) - cum(lo - 1); r = (n * 23.77 + 0.9940 * cp / 1e6) / 3600
    print(f"  {h:>8,} {pts[h][2]:>3} {lo:>8,}-{hi-1:<8,} {n:>8,} {t:>12,.0f} {cu/1e12:>8.2f} {cp/1e12:>9.2f} {r:>10,.0f} {100*cu/U:5.1f}%")

# dispersion: how well the 800,000 sample's per-tx cost predicts its next five blocks (an era is one sample)
b0 = B[800000]
print("\nDispersion around 800,000 (B, modelled state): each model of 800,000 vs the next five blocks")
for h, r in sorted(DISP.items()):
    for k in ("user",):
        ptx = (I(b0, f"total_{k}") - base[k]) / I(b0, "txs")
        pred = base[k] + ptx * I(r, "txs"); m = I(r, f"total_{k}")
        print(f"  {h:,}: {I(r,'txs'):>5} txs {I(r,'ins'):>5} ins  measured {m/1e6:7.1f} M  per-tx model {pred/1e6:7.1f} M  ({100*(pred/m-1):+5.1f}%)"
              f"   flat (= 800,000's {I(b0,'total_user')/1e6:.1f} M): {100*(I(b0,'total_user')/m-1):+5.1f}%")
        pin = base[k] + (I(b0, f"total_{k}") - base[k]) / I(b0, "ins") * I(r, "ins")
        a_, b_, c_ = FIT[k]; po = a_ + b_ * I(r, "ins") + c_ * I(r, "outs")
        print(f"           per-input model {pin/1e6:7.1f} M ({100*(pin/m-1):+5.1f}%)   pooled fit {po/1e6:7.1f} M ({100*(po/m-1):+5.1f}%)")

# the pooled fit's own residuals over the measured blocks
print("\nPooled fit residuals (user), measured blocks with >100 inputs:")
rows = {h: r for h, r in B.items()}; rows.update(A)
for h in sorted(rows):
    r = rows[h]
    if I(r, "ins") <= 100: continue
    a_, b_, c_ = FIT["user"]; p = a_ + b_ * I(r, "ins") + c_ * I(r, "outs"); m = I(r, "total_user")
    print(f"  {h:>8,} {r['src'][0]} measured {m/1e6:7.1f} M  fit {p/1e6:7.1f} M  {100*(p/m-1):+6.1f}%   {m/I(r,'ins')/1e3:6.1f} k cycles/input")
