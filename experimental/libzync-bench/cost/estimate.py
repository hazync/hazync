"""estimate.py: whole-chain proving cost from the 19-block survey (execute-mode cycles) + real tx counts per era.

Model, per era (the span of blocks each sample stands for, boundaries at the midpoints):
    cycles = blocks * base + txs * per_tx(sample)
    per_tx(sample) = (sample_total - base) / sample_txs, base = the 100,000 sample (3 txs: fixed per-block work)
GPU time = cycles * s_per_Mcycle (measured 2026-10-29: PRO 6000 ~1.2, A40 ~2.7)
         + blocks * fixed_per_block (today one chunk proof + one aggregate proof per block, each ~8.5 s of
           process start + assembly on a PRO 6000 and ~17 s on an A40 -- measured from the chunk runs)
"Batched" drops the per-block fixed term: many blocks per proof session. That is NOT built.
All rough: one sample per era, cycles->seconds from 962,000-sized work.
"""
import re

R = "/home/defenwycke/libzync-gpu"
surv = {}
for l in open(f"{R}/r17/survey.out"):
    p = l.split("\t")
    if len(p) >= 5 and p[0] in ("on", "off"):
        surv[(p[0], int(p[1]))] = int(p[4])
ntx = {}
for l in open(f"{R}/sample/mk.log"):
    m = re.match(r"(\d+): (\d+) txs", l)
    if m: ntx[int(m.group(1))] = int(m.group(2)) + 1   # + coinbase
tc = dict((int(a), int(b)) for a, b in (l.split() for l in open(f"{R}/sample/txcounts.txt")))
bounds = sorted(tc)
samples = sorted(ntx)
eras = list(zip(samples, bounds[:-1], bounds[1:]))   # sample h stands for [lo, hi)

CARDS = {"PRO 6000": (1.2, 8.5 * 2, 2.49), "A40": (2.7, 17.0 * 2, 0.59)}
for mode in ("on", "off"):
    base = surv[(mode, 100000)]
    cyc = blk = 0
    for h, lo, hi in eras:
        per_tx = max(0.0, (surv[(mode, h)] - base) / ntx[h])
        b, t = hi - lo, tc[hi] - tc[lo]
        cyc += b * base + t * per_tx
        blk += b
    print(f"\nmainnet blocks 1..970,000, scripts {mode.upper()}: {cyc/1e12:.1f} T cycles over {blk} blocks")
    for card, (spm, fixed, price) in CARDS.items():
        s_cyc = cyc / 1e6 * spm
        s_fix = blk * fixed
        for label, s in (("per-block proofs (today)", s_cyc + s_fix), ("batched (to build)", s_cyc)):
            print(f"  {card:9s} {label:26s} {s/3600:10,.0f} card-hours  ${s/3600*price:10,.0f}")
    # signet: 320,000 blocks, ~25.9 M txs (sampled from mempool.space), per-tx cost of the 970,000 sample
    base_s = base
    per_tx = (surv[(mode, 970000)] - base) / ntx[970000]
    scyc = 320000 * base_s + 25.9e6 * per_tx
    print(f"  signet to 320,000, scripts {mode.upper()}: {scyc/1e12:.2f} T cycles")
    for card, (spm, fixed, price) in CARDS.items():
        s_cyc = scyc / 1e6 * spm
        for label, s in (("per-block proofs (today)", s_cyc + 320000 * fixed), ("batched (to build)", s_cyc)):
            print(f"    {card:9s} {label:26s} {s/3600:10,.0f} card-hours  ${s/3600*price:10,.0f}")
