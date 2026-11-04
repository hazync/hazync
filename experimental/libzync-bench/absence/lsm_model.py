#!/usr/bin/env python3
# E1 options B (epoch batch) and C (log-structured tree) from measured coin lifetimes (lifetimes.py output).
# Tree updates per block are what the guest applies today: spends of coins not created in the same block (removes) and
# creates not spent in the same block (inserts). Option B: one batch per epoch; updates = spends of pre-epoch coins +
# creates alive at epoch end. Option C: per-block sorted runs merged F at a time (aligned groups); a coin's insert and
# tombstone cancel at the first merge whose span holds both. Levels past the window: no further cancellation assumed.
# usage: lsm_model.py <lifetimes .json>
import json, sys
rows = json.load(open(sys.argv[1]))
H = [r[0] for r in rows]; n = len(rows)
born = {}
for h, cr, sp in rows:
    for c in cr: born[c] = h
spent_at = {}
for h, cr, sp in rows:
    for s in sp:
        if s in born: spent_at[s] = h
# per-block updates (today's tree)
upd = []
for h, cr, sp in rows:
    rem = sum(1 for s in sp if born.get(s) != h)
    ins = sum(1 for c in cr if spent_at.get(c) != h)
    upd.append(rem + ins)
U = sum(upd)
print(f"window {H[0]}..{H[-1]} ({n} blocks): {U} tree updates, {U/n:.0f} per block")
# lifetimes of coins created in the first half
half = H[0] + n // 2
first = [c for c, b in born.items() if b < half]
for d in (0, 1, 15, 143, 255, n // 2 - 1):
    k = sum(1 for c in first if c in spent_at and spent_at[c] - born[c] <= d)
    print(f"  coins spent within {d+1:5d} blocks of creation: {100*k/len(first):5.1f}%")
# option B
for E in (144, 1008):
    tot = 0; cnt = 0
    for e0 in range(H[0], H[0] + n - E + 1, E):
        e1 = e0 + E
        for h, cr, sp in rows[e0 - H[0]:e1 - H[0]]:
            tot += sum(1 for s in sp if not (s in born and e0 <= born[s] < e1))
            tot += sum(1 for c in cr if not (c in spent_at and spent_at[c] < e1))
        cnt += E
    pb = sum(upd[:cnt])
    print(f"option B epoch {E}: {tot} updates vs {pb} per-block ({100*tot/pb:.1f}%)")
# option C: entries per block = upd; simulate aligned merges of F blocks, then F^2 blocks
def survive(span):
    # entries alive after merging all blocks in each aligned span of `span` blocks (within the window)
    alive = 0; total = 0
    for g0 in range(H[0], H[0] + n - span + 1, span):
        g1 = g0 + span
        for h, cr, sp in rows[g0 - H[0]:g1 - H[0]]:
            for s in sp:
                if born.get(s) == h: continue
                total += 1
                if not (s in born and g0 <= born[s] < g1): alive += 1
            for c in cr:
                if spent_at.get(c) == h: continue
                total += 1
                if not (c in spent_at and spent_at[c] < g1): alive += 1
    return alive / total
for F, levels, mc in ((16, 4, 1408), (4, 9, 1112)):
    s = [1.0]
    L = 1
    while F ** L <= n and L <= levels:
        s.append(survive(F ** L)); L += 1
    while len(s) < levels: s.append(s[-1])  # beyond the window: no more cancellation
    merges = sum(s[:levels])                 # entries entering merge level 1..levels
    per = 400 + mc * merges
    print(f"option C fanout {F}: survival after levels {[round(x,3) for x in s[1:]]}; merges per update {merges:.2f}; "
          f"~{per:.0f} cycles per update (block run 400 + {mc}/merge)")
