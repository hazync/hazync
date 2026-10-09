#!/usr/bin/env python3
"""What the window says is happening, tested on the worker's real output.

    python3 test_activity.py             # the real reader
    python3 test_activity.py --control   # a naive reader, which must get these wrong

⛔ THE LINES BELOW ARE REAL. They were copied from `worker_N.log` on two rented cards proving and
folding as a contributor on 2026-10-09, and from the worker's own print statements for the anchor
job. A reader tested on invented lines passes for ever and reads nothing.
"""

import os as _os, sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
import winconsole as _wc  # noqa: E402
_wc.fix()   # ⛔ BEFORE anything prints: a ✅ on a cp1252 console raises, not degrades

import sys

import activity  # noqa: E402

fails = 0

PROVE = """claimed block 144263 (yours for 60 min; if you stop, it reopens by itself)
using archive-node bundles (no replay) for blocks 144263..144263
proving range 144263 (1 block) — the expensive step…
range [144263..144263] (bridge): executed, 61 segments at po2 21 -- proving
    segment 1/61  4s elapsed, ~191s left
    segment 30/61  101s elapsed, ~104s left
    segment 61/61  205s elapsed, ~0s left
    assembling 61 segment receipts (lift + join)  30s elapsed
    assembled 61 segment receipts in 43s
proved range [144263..144263] from bridge bundle in 263.0s -> range_144263.hzk
receipt: /root/.hazync/receipts/144263.bin
✓ range 144263: the coordinator re-verified your proof and put it on the board as 'G H O S T'.
""".splitlines()

FOLD = """folding 137121-137124 + 137125-137128 -> 137121-137128 …
✓ 137121-137128: folded and verified on the board as 'G H O S T'.
folded 1 pair(s).
""".splitlines()

SPINE = """absorbing 136433-136448 into [1..136432] — one fold, no re-proving…
✓ spine now [1..136448] — genesis-anchored, 223456 bytes, as 'G H O S T'.
""".splitlines()


def check(ok, what):
    global fails
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails += 1


def run(job, lines, cls=None):
    a = (cls or activity.Activity)(job)
    events, fractions = [], []
    for line in lines:
        e = a.feed(line)
        if e:
            events.append(e)
        fractions.append(a.fraction)
    return a, events, fractions


def checks(cls=None):
    print("── 1. a prove, start to finish ──")
    a, events, fr = run("run", PROVE, cls)
    check(a.done == 1, f"one block landed, counted once (got {a.done})")
    check("144,263" in a.headline and "on the board" in a.headline,
          f"the headline says which block landed: {a.headline!r}")
    check(a.fraction == 1.0, "the bar is full only once the coordinator has accepted it")
    nums = [f for f in fr if f is not None]
    check(nums == sorted(nums), "⛔ the bar never moves backwards")
    check(len(set(nums)) >= 6, f"the bar moves through the prove ({len(set(nums))} distinct stops)")
    check(len(events) == 3, f"three things HAPPENED — took, finished, accepted (got {len(events)})")
    check(not any("segment" in e.lower() or "piece" in e.lower() for e in events),
          "progress inside a step is not an event, or the feed would be nothing else")

    print("── 2. mid-prove, a person can see how far along it is ──")
    a, _, _ = run("run", PROVE[:6], cls)
    check("30" in a.detail and "61" in a.detail, f"which piece of how many: {a.detail!r}")
    check("1m 44s" in a.detail, f"and how long is left, as a duration: {a.detail!r}")
    check(0.3 < (a.fraction or 0) < 0.6, f"about half way along the bar ({a.fraction})")
    check(a.done == 0, "nothing is counted as landed while it is still proving")

    print("── 3. ⛔ a finished proof is not a landed block ──")
    a, _, _ = run("run", PROVE[:10], cls)
    check(a.done == 0 and (a.fraction or 0) < 1.0,
          "proved but not yet accepted: not counted, and the bar is not full")

    print("── 4. folding has no measurable length, so it claims none ──")
    a, events, _ = run("fold", FOLD[:1], cls)
    check(a.fraction is None, "⛔ no invented percentage while a fold runs")
    check("137,121–137,128" in a.headline, f"it names the range: {a.headline!r}")
    a, events, _ = run("fold", FOLD, cls)
    check(a.done == 1 and len(events) == 1, "one fold landed, one event")

    print("── 5. anchoring ──")
    a, events, _ = run("spine", SPINE, cls)
    check(a.done == 1 and "136,448" in a.headline, f"the chain's new reach: {a.headline!r}")

    print("── 6. nothing to do is said plainly, and is not a failure ──")
    a, events, _ = run("run", ["nothing to claim right now: this key already holds 4 claimed "
                               "blocks that are not finished"], cls)
    check(a.waiting and a.done == 0 and len(events) == 1, "waiting is a state of its own")
    a, _, _ = run("spine", ["  block 136449 is not proven yet (no retained receipt) — the spine "
                            "is up to date"], cls)
    check(a.waiting and "136,449" in a.detail, f"and says what it is waiting for: {a.detail!r}")

    print("── 7. a line it does not know changes nothing ──")
    a = (cls or activity.Activity)("run")
    before = (a.headline, a.detail, a.fraction, a.done)
    for junk in ("", "   ", "WARNING: something new the worker prints next year",
                 "receipt: /root/.hazync/receipts/144263.bin"):
        a.feed(junk)
    check((a.headline, a.detail, a.fraction, a.done) == before,
          "unknown output leaves the headline stale rather than wrong")

    print("── 8. the small formatters ──")
    check(activity.pretty("144263") == "144,263" and activity.pretty("1-20") == "1–20"
          and activity.pretty("abc") == "abc", "block numbers get separators; anything else is left alone")
    check((activity.clock(45), activity.clock(185), activity.clock(4320)) ==
          ("45s", "3m 05s", "1h 12m"), "durations read at a glance")
    check(activity.tally("run", 1) == "1 block proven" and activity.tally("fold", 3) == "3 folds done",
          "the session count is in the job's own words")
    check(set(activity.JOB_NAMES) == set(activity.JOBS) == set(activity.JOB_BLURBS),
          "every job has a name and a sentence, so no choice on screen is unexplained")


class Naive(activity.Activity):
    """The obvious reader: count a block when the proof finishes, and always show some percentage."""

    def feed(self, line):
        e = super().feed(line)
        if "proved range" in line:
            self.done += 1                 # counts a block the coordinator has not accepted yet
            self.fraction = 1.0
        if self.fraction is None:
            self.fraction = 0.5            # a number where there is nothing to measure
        if "segment" in line:
            return line.strip()            # every progress line becomes an event
        return e


def control():
    global fails
    print("── control: a naive reader ──")
    checks(Naive)
    broke, fails = fails, 0
    print()
    if broke < 4:
        print(f"⛔ CONTROL BROKEN: only {broke} checks failed against the naive reader")
        return 1
    print(f"CONTROL OK: the naive reader fails {broke} checks the real one passes")
    return 0


def main():
    checks()
    print()
    if fails:
        print(f"FAIL {fails}")
        return 1
    print("All checks passed.")
    return 0


if __name__ == "__main__":
    if "--control" in sys.argv:
        sys.exit(control())
    sys.exit(main())
