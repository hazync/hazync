#!/usr/bin/env python3
"""The coordinator API layer and the block map's geometry, tested offline.

    python3 test_api.py             # the real logic, on fixtures shaped like the live API
    python3 test_api.py --control   # naive versions, each of which must break
    python3 test_api.py --live      # additionally cross-check against the real coordinator

⛔ NO NETWORK IN THE DEFAULT RUN. Fixtures are the shape the live API actually returned on
2026-10-04, recorded here so the tests keep working when the coordinator is unreachable and so a
shape change shows up as a failing test rather than a quietly empty dashboard.
"""

import os as _os, sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
import winconsole as _wc  # noqa: E402
_wc.fix()   # ⛔ BEFORE anything prints: a ✅ on a cp1252 console raises, not degrades

import sys

sys.path.insert(0, __file__.rsplit("/", 1)[0] if "/" in __file__ else ".")
import hazync_api as api  # noqa: E402

fails = 0

# The real shape, trimmed: a genesis-anchored run, the contiguous proven region up to the frontier,
# small proven runs above it with GAPS between them, and tip blocks proved far above the frontier.
RUNS = [
    (1, 136360, 5),
    (136361, 142140, 3),
    (142142, 142147, 3),
    (142149, 142161, 3),
    (969119, 969122, 4),
    (969305, 969306, 3),
]
TIP, SPINE_HI, FRONTIER = 969911, 136360, 142140


def check(ok, what):
    global fails
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails += 1


def main():
    print("── 1. the state codes come from server.py, not from guessing ──")
    check((api.PROVEN, api.FOLDED, api.ANCHORED) == (3, 4, 5), "3 proven, 4 folded, 5 anchored")
    check(api.OPEN == 0 and api.OPEN not in (3, 4, 5),
          "OPEN is ours — the API emits no code for it, a block is open by ABSENCE")

    print("── 2. one block's state ──")
    for h, want in ((1, api.ANCHORED), (SPINE_HI, api.ANCHORED), (SPINE_HI + 1, api.PROVEN),
                    (142141, api.OPEN), (969120, api.FOLDED), (TIP, api.OPEN)):
        got = api.state_at(RUNS, h)
        check(got == want, f"{h:,} -> {api.STATE_NAMES[got]} (expected {api.STATE_NAMES[want]})")

    print("── 3. ⛔ open ranges must be CLIPPED, not merely stopped at ──")
    # ⛔⛔ THE REGRESSION. Tip blocks are proved far above the frontier, so the span between the
    # frontier region and those runs is a gap by construction. Unclipped, that reported 826,567
    # "open" blocks below a frontier of 142,140 — impossible on its face, which is how it was caught.
    gaps = api.open_ranges(RUNS, FRONTIER)
    check(all(hi <= FRONTIER for _, hi in gaps), f"every gap ends at or below the frontier ({gaps})")
    check(api.count_open(RUNS, FRONTIER) <= FRONTIER,
          f"open count ({api.count_open(RUNS, FRONTIER):,}) cannot exceed the frontier")
    unbounded = api.open_ranges(RUNS, None)
    big = [g for g in unbounded if g[1] - g[0] > 100000]
    check(bool(big), f"unbounded, the fake mega-gap IS present ({big}) — so clipping is load-bearing")

    # with a ceiling above the tip runs, the gaps are real again
    gaps_tip = api.open_ranges(RUNS, TIP)
    check(any(lo == 142141 for lo, _ in gaps_tip), "with a tip ceiling, 142141 is open")
    check(all(hi <= TIP for _, hi in gaps_tip), "and nothing exceeds the ceiling")

    print("── 4. the next work is the LOWEST open block ──")
    nxt = api.first_open(RUNS, TIP, count=3)
    check(nxt == [142141, 142148, 142162], f"lowest-first: {nxt}")
    check(nxt == sorted(nxt), "⭐ lowest first — a proof only joins the spine when all below it are proved")

    print("── 5. the map buckets honestly ──")
    rows, per_cell = api.map_rows(RUNS, TIP, per_row=100, rows_max=20)
    check(len(rows) <= 20, f"{len(rows)} rows, {per_cell:,} blocks per cell — the chain fits a window")
    check(rows[0][0][0] == api.ANCHORED, "the genesis region reads anchored")
    # ⛔ a bucket containing ONE open block must read open, not proven
    mixed = api.worst_state([(1, 50, 3), (52, 100, 3)], 1, 100)
    check(mixed == api.OPEN, "a 100-block bucket with ONE open block (51) reads OPEN, not proven")
    full = api.worst_state([(1, 100, 3)], 1, 100)
    check(full == api.PROVEN, "a fully covered bucket reads its real state")
    layered = api.worst_state([(1, 100, 3), (1, 100, 5)], 1, 100)
    check(layered == api.PROVEN, "and a layered bucket reads the LEAST advanced state present")

    print("── 6. errors a person can act on ──")
    try:
        api.fetch("/api/state", coord="http://127.0.0.1:1", timeout=2)
        check(False, "an unreachable coordinator should raise")
    except api.ApiError as e:
        check("could not reach" in str(e), f"unreachable -> actionable message ({str(e)[:46]}…)")

    print("── 7. the headline line ──")
    s = api.summarise_progress({"proven": 142740, "folded": 121406, "spine_hi": 136360,
                                "pct": 14.655, "tip": 969911, "contributors": 11})
    for frag in ("142,740 proven", "121,406 folded", "136,360 anchored", "14.655%", "11 contributors"):
        check(frag in s, f"says {frag!r}")

    print()
    if fails:
        print(f"FAIL: {fails} check(s)")
        return 1
    print("All checks passed.")
    return 0


def control():
    broke = 0
    print("── control: the naive dashboard ──")

    # ⛔ naive 1: open = "stop at the ceiling" instead of clipping
    def naive_open(runs, upto):
        rs = sorted((a, b) for a, b, _ in runs)
        gaps, cursor = [], 1
        for lo, hi in rs:
            if lo > cursor:
                gaps.append((cursor, lo - 1))
            cursor = max(cursor, hi + 1)
        return gaps                       # no clipping at all
    n = sum(hi - lo + 1 for lo, hi in naive_open(RUNS, FRONTIER))
    r = api.count_open(RUNS, FRONTIER)
    ok = n > FRONTIER and r <= FRONTIER
    print(f"  {'ok  ' if ok else 'FAIL'} ⛔ control: unclipped gaps report {n:,} open below a "
          f"frontier of {FRONTIER:,} — impossible; the real answer is {r:,}")
    broke += 1 if ok else 0

    # ⛔ naive 2: a bucket shows its BEST state
    best = max(st for a, b, st in [(1, 50, 3), (52, 100, 3)])
    real = api.worst_state([(1, 50, 3), (52, 100, 3)], 1, 100)
    ok = best == api.PROVEN and real == api.OPEN
    print(f"  {'ok  ' if ok else 'FAIL'} ⛔ control: 'best state in the bucket' paints a bucket with "
          f"an open block as proven — hiding the gaps a prover is looking for")
    broke += 1 if ok else 0

    # ⛔ naive 3: newest open block first
    newest = api.first_open(RUNS, TIP, count=3)[::-1]
    ok = newest[0] > api.first_open(RUNS, TIP, count=1)[0]
    print(f"  {'ok  ' if ok else 'FAIL'} ⛔ control: handing out the NEWEST open block "
          f"({newest[0]:,}) yields proofs that cannot be anchored until everything below is proved")
    broke += 1 if ok else 0

    # ⛔ naive 4: trust the frontier as the ceiling for claimable work
    ok = api.count_open(RUNS, FRONTIER) == 0 and api.count_open(RUNS, TIP) > 0
    print(f"  {'ok  ' if ok else 'FAIL'} ⛔ control: bounding open work by the FRONTIER gives zero "
          f"claimable blocks right now, yet blocks above it are open — which is why allocation is "
          f"left to /api/pick and not re-derived here")
    broke += 1 if ok else 0

    print()
    if broke < 4:
        print(f"⛔ CONTROL BROKEN: only {broke} of 4 naive versions failed as expected")
        return 1
    print("CONTROL OK: all four naive versions break where the real logic holds")
    return 0


def live():
    """Cross-check the fixtures against the real coordinator. Network required."""
    print("── live: does the real API still have the shape these tests assume? ──")
    bad = 0
    try:
        p = api.progress()
        for k in ("proven", "folded", "spine_hi", "frontier", "tip", "pct", "contributors"):
            print(f"  {'ok  ' if k in p else 'FAIL'} /api/state progress has {k!r}"
                  + (f" = {p[k]:,}" if isinstance(p.get(k), int) else ""))
            bad += 0 if k in p else 1
        meta, runs = api.block_status()
        print(f"  ok   /api/blockstatus: {len(runs)} runs, tip={meta['tip']:,}")
        codes = sorted({st for _, _, st in runs})
        known = all(c in (api.PROVEN, api.FOLDED, api.ANCHORED) for c in codes)
        print(f"  {'ok  ' if known else 'FAIL'} every state code is one of 3/4/5: {codes}")
        bad += 0 if known else 1
        # ⭐ the anchored run must agree with spine_hi, or our reading of code 5 is wrong
        anchored_hi = max((hi for _, hi, st in runs if st == api.ANCHORED), default=0)
        agree = anchored_hi == meta["spine_hi"]
        print(f"  {'ok  ' if agree else 'FAIL'} the anchored run ends at spine_hi "
              f"({anchored_hi:,} vs {meta['spine_hi']:,}) — confirms 5 means anchored")
        bad += 0 if agree else 1
    except api.ApiError as e:
        print(f"  SKIP live checks: {e}")
        return 0
    print()
    print("live shape OK" if not bad else f"⛔ {bad} live check(s) failed — the API shape moved")
    return 1 if bad else 0


if __name__ == "__main__":
    if "--control" in sys.argv:
        sys.exit(control())
    rc = main()
    if "--live" in sys.argv:
        rc = live() or rc
    sys.exit(rc)
