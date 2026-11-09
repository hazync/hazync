#!/usr/bin/env python3
"""Two independent libzync prep runs must agree on the chain state at every height both have reached.

The board pass (libzync-board-prep, from genesis under the published image, leaf bundles for the board) and the
paper run (libzync-prep) compute the same states by separate runs over separate stores. Every field of
state_<h>.json must match except `self_id`, the image a state is stamped with (the paper run switched images at
378,500; the board pass ran 863c9ce3 from block 1). A difference means one run computed a wrong state: the unit
fails (and alerts).

Incremental: it remembers the last height checked (--state) and checks the new shared heights each run.

  libzync-xcheck.py A_OUT B_OUT [--state FILE] [--from H] [--max N]
Exit: 0 all checked heights identical, 1 a difference, 2 could not check.
"""
import argparse, json, os, re, sys

def newest(d):
    best = 0
    with os.scandir(d) as it:
        for e in it:
            m = re.fullmatch(r"state_(\d+)\.json", e.name)
            if m: best = max(best, int(m.group(1)))
    return best

def load(p):
    s = json.load(open(p))
    s.pop("self_id", None)
    return s

def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("a"); ap.add_argument("b")
    ap.add_argument("--state", help="file holding the last height checked")
    ap.add_argument("--from", dest="lo", type=int, default=1)
    ap.add_argument("--max", type=int, default=200000, help="at most this many heights per run")
    a = ap.parse_args(argv)
    for d in (a.a, a.b):
        if not os.path.isdir(d):
            print(f"[xcheck] no directory {d}"); return 2
    lo = a.lo
    if a.state and os.path.exists(a.state):
        lo = max(lo, int(open(a.state).read().strip() or 0) + 1)
    # the newest file of each run may be mid-write (a plain write between save points): stop one short of it
    hi = min(newest(a.a), newest(a.b)) - 1
    hi = min(hi, lo + a.max - 1)
    if hi < lo:
        print(f"[xcheck] nothing new to check (next {lo}, shared up to {hi + 1})"); return 0
    same = 0
    for h in range(lo, hi + 1):
        pa, pb = f"{a.a}/state_{h}.json", f"{a.b}/state_{h}.json"
        if not (os.path.exists(pa) and os.path.exists(pb)):
            print(f"[xcheck] state_{h}.json missing from {'A' if not os.path.exists(pa) else 'B'}: cannot check {h}")
            return 2
        sa, sb = load(pa), load(pb)
        if sa != sb:
            diff = sorted(k for k in set(sa) | set(sb) if sa.get(k) != sb.get(k))
            print(f"[xcheck] DIFFER at height {h}: {diff} ({pa} vs {pb})")
            return 1
        same += 1
        if a.state and (h % 1000 == 0 or h == hi):
            tmp = a.state + ".tmp"
            open(tmp, "w").write(str(h)); os.replace(tmp, a.state)
    print(f"[xcheck] heights {lo}..{hi}: {same} identical (all fields but self_id)")
    return 0

if __name__ == "__main__":
    sys.exit(main())
