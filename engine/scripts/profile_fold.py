#!/usr/bin/env python3
"""profile_fold.py <metrics.json> [top]: guest instruction counts by function from an hzengine HZ_METRICS run
(`--features profile`). Prints self and inclusive counts per function, and writes <metrics>.folded (flamegraph input)."""
import json, sys, collections
m = json.load(open(sys.argv[1])); top = int(sys.argv[2]) if len(sys.argv) > 2 else 40
rows = m.get("counter", m if isinstance(m, list) else [])
stacks = collections.Counter()
for r in rows:
    if r.get("metric") != "frequency": continue
    lab = dict(r.get("labels", []))
    sp = lab.get("cycle_tracker_span")
    if sp is None: continue
    stacks[sp] += int(r["value"])
total = sum(stacks.values())
selfc, incl = collections.Counter(), collections.Counter()
for sp, v in stacks.items():
    fs = sp.split(";") if sp else ["<root>"]
    selfc[fs[-1]] += v
    for f in set(fs): incl[f] += v
with open(sys.argv[1] + ".folded", "w") as o:
    for sp, v in stacks.items(): o.write(f"{sp or '<root>'} {v}\n")
print(f"total instructions {total:,}")
print(f"{'self %':>7} {'incl %':>7}  function")
for f, v in selfc.most_common(top):
    print(f"{100*v/total:7.2f} {100*incl[f]/total:7.2f}  {f[:110]}")
