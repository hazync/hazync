#!/usr/bin/env python3
"""#550's tail-vs-bandwidth comparison is REACHED, and refuses to guess (hazync#550).

⛔⛔ THE BUG THIS GUARDS IS "WRITTEN BUT NEVER CALLED". `tail_ranking`, `tail_cut` and
`cut_agreement` shipped with ZERO callers — not in the run, not in `report()`, nowhere but their own
test. A funded run would have produced data for #567 and #598 and nothing at all for #550, and the
money would have been spent before anyone noticed. tip_banks' own comment records the SAME failure
one round earlier: `cohort_effect` and `MIN_FLEET` were unused, and the fix for that added these
three, also unwired.

⇒ So the first thing asserted here is simply that the CLI reaches them.

    python3 test_cut_comparison.py             # the comparison runs and reports honestly
    python3 test_cut_comparison.py --control   # three naive versions, each of which must break
"""
import json
import os
import random
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import join_levels  # noqa: E402
import tip_banks  # noqa: E402

CARDS = [f"hz-smoke-{i}" for i in range(1, 9)]
SLOW = ["hz-smoke-7", "hz-smoke-8"]

fails = 0


def check(ok, what):
    global fails
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails += 1


def make_log(path, thin=(), n=60):
    """A log in join_levels' real format. `thin` cards get too few joins to judge."""
    random.seed(7)
    out = []
    for i, c in enumerate(CARDS[1:], start=2):
        base = 900.0 if c in SLOW else 120.0
        count = 3 if c in thin else n
        for k in range(count):
            tag = 0x80000000 | (1 << 16) | k
            out.append(f"  [rtt] card={c} peer=10.0.0.{i}:41288 kind=join "
                       f"tag=0x{tag:08x} rtt_ms={base + random.random() * 20:.1f} bytes_out=4096\n")
    with open(path, "w") as fh:
        fh.writelines(out)
    return out


def samples_of(path):
    with open(path) as fh:
        return join_levels.parse(fh.readlines())


def probe(dropped):
    return {"t": 0, "elected": CARDS[0], "dropped": sorted(dropped),
            "worker_min_mbit": 0.0, "candidates": {c: {"mbit": 50} for c in CARDS}}


def main():
    d = tempfile.mkdtemp()
    log = os.path.join(d, "fleet.log")
    make_log(log)
    s = samples_of(log)

    print("── 1. the samples and the ranking ──")
    check(len(s) == 420, f"the log parses in the real format ({len(s)} samples)")
    order = [CARDS[0]] + sorted(CARDS[1:])
    ranked, thin = tip_banks.tail_ranking(order, s)
    measured = [(c, v) for c, v in ranked if v is not None]
    check(len(measured) == 7, f"all seven workers are measured ({len(measured)})")
    check([c for c, _ in measured[:2]] == SLOW,
          "the two slow cards rank worst-tail FIRST")
    check(CARDS[0] not in [c for c, _ in ranked],
          "⛔ the AGGREGATE is never ranked — it is not droppable and a check that forgot "
          "once killed a 30-card run")

    print("── 2. the comparison reaches cut_agreement and names the disagreement ──")
    bw = ["hz-smoke-2", "hz-smoke-3"]
    need = len(order) - len(bw)
    tail_drop = sorted(tip_banks.tail_cut(order, s, need=need))
    check(tail_drop == SLOW, f"a tail cut drops the slow pair ({' '.join(tail_drop)})")
    both, bw_only, tail_only = tip_banks.cut_agreement(sorted(bw), tail_drop)
    check((both, bw_only, tail_only) == ([], sorted(bw), SLOW),
          "total disagreement is reported as such, not averaged away")

    print("── 3. and AGREEMENT is a result, not a failure ──")
    both, bw_only, tail_only = tip_banks.cut_agreement(SLOW, tail_drop)
    check(both == SLOW and not bw_only and not tail_only,
          "when both criteria pick the same cards it says they agree")

    print("── 4. an UNMEASURED card is never treated as fast ──")
    log2 = os.path.join(d, "thin.log")
    make_log(log2, thin=SLOW)
    s2 = samples_of(log2)
    ranked2, thin2 = tip_banks.tail_ranking(order, s2)
    check(sorted(thin2) == SLOW, f"cards with 3 joins are UNMEASURED, not ranked ({thin2})")
    check(all(v is None for c, v in ranked2 if c in SLOW),
          "and they carry no p90, so they cannot read as fast")
    check([c for c, v in ranked2 if v is not None][0] not in SLOW,
          "the worst MEASURED card leads instead of a card nobody could judge")

    print("── 5. ⛔ THE WIRING ITSELF: the CLI must reach the comparison ──")
    pj = os.path.join(d, "probe.json")
    with open(pj, "w") as fh:
        json.dump(probe(bw), fh)
    import contextlib
    import io
    buf = io.StringIO()
    argv = sys.argv
    sys.argv = ["tip_banks.py", "--probe", pj, log]
    try:
        with contextlib.redirect_stdout(buf):
            tip_banks.main()
    finally:
        sys.argv = argv
    out = buf.getvalue()
    check("#550" in out and "would a TAIL-based cut" in out,
          "⛔ `--probe` REACHES the #550 section (this is the bug: it had no callers)")
    check("a tail cut would drop" in out, "it prints what a tail cut would have dropped")
    check("needs the block times" in out,
          "and refuses to say which cut was RIGHT — that needs block times, not a list")

    print("── 6. without --probe, nothing changes ──")
    buf2 = io.StringIO()
    sys.argv = ["tip_banks.py", log]
    try:
        with contextlib.redirect_stdout(buf2):
            tip_banks.main()
    finally:
        sys.argv = argv
    check("#550: would a TAIL-based cut" not in buf2.getvalue(),
          "no probe record means no comparison, rather than an invented one")

    print()
    if fails:
        print(f"FAIL: {fails} check(s)")
        return 1
    print("All checks passed.")
    return 0


def control():
    """Three naive versions, each of which must BREAK. Exits 0 when they all do."""
    d = tempfile.mkdtemp()
    log = os.path.join(d, "fleet.log")
    make_log(log, thin=SLOW)          # the slow cards are also the THIN ones here
    s = samples_of(log)
    order = [CARDS[0]] + sorted(CARDS[1:])
    stats = join_levels.by_card(s)
    stats.pop("?", None)
    broke = 0

    print("── control: the naive rankings, on a fleet where the slow cards are also thin ──")

    # ⛔ naive 1: a cut taken over `order` itself, which includes the aggregate at [0]
    real_ranked, _ = tip_banks.tail_ranking(order, s)
    ok = CARDS[0] in order and CARDS[0] not in [c for c, _ in real_ranked]
    print(f"  {'ok  ' if ok else 'FAIL'} ⛔ control: `order` CONTAINS the aggregate, so any cut "
          f"ranking it directly can drop it; tail_ranking returns only order[1:]")
    broke += 1 if ok else 0

    # ⛔ naive 2: zero-fill an unmeasured p90. Both cuts then keep the slow cards — they are
    # unmeasurable here — but the naive ranking SILENTLY SORTS THEM FASTEST, while tail_ranking
    # hands them back as `thin` so a caller can say "we could not tell" out loud.
    def naive_p90(c):
        st = stats.get(c)
        return st["p90"] if st and st["n"] >= tip_banks.MIN_SAMPLES else 0.0
    naive_rank = sorted(CARDS[1:], key=lambda c: -naive_p90(c))
    real_ranked2, real_thin = tip_banks.tail_ranking(order, s)
    naive_says_fastest = naive_rank[-2:]
    ok = sorted(naive_says_fastest) == SLOW and sorted(real_thin) == SLOW
    print(f"  {'ok  ' if ok else 'FAIL'} ⛔ control: zero-filling ranks the UNMEASURED cards as the "
          f"FASTEST two ({' '.join(sorted(naive_says_fastest))}); tail_ranking returns them as "
          f"thin={' '.join(sorted(real_thin))} instead of a number")
    broke += 1 if ok else 0

    # ⛔ naive 3: compare drop-lists of DIFFERENT lengths — measures depth, not choice
    shallow = sorted(tip_banks.tail_cut(order, s, need=len(order) - 1))
    deep = sorted(tip_banks.tail_cut(order, s, need=len(order) - 3))
    _, bw_only, tail_only = tip_banks.cut_agreement(shallow, deep)
    ok = bool(bw_only or tail_only)
    print(f"  {'ok  ' if ok else 'FAIL'} ⛔ control: comparing a 1-card cut with a 3-card cut shows "
          f"{len(bw_only) + len(tail_only)} 'disagreement(s)' that are only a depth difference — "
          f"which is why `need` is DERIVED from the probe")
    broke += 1 if ok else 0

    print()
    if broke < 3:
        print(f"⛔ CONTROL BROKEN: only {broke} of 3 naive versions failed as expected")
        return 1
    print("CONTROL OK: all three naive versions break on a fleet the real code handles")
    return 0


if __name__ == "__main__":
    sys.exit(control() if "--control" in sys.argv else main())
