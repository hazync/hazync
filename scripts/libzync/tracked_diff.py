#!/usr/bin/env python3
"""tracked_diff.py <prep log> <reproduce/libzync-tracked.json> <H>: the tracked transactions prep derived (its PREP_TRACKED
h=<height> txid=<txid> lines, prover/host/src/tracked.rs) against the list's, up to height H. Prints MATCH or DIFFER (exit 1)."""
import json, re, sys
log, lst, H = sys.argv[1], sys.argv[2], int(sys.argv[3])
got = set()
for l in open(log, errors="replace"):
    m = re.search(r"PREP_TRACKED h=(\d+) txid=([0-9a-f]{64})", l)
    if m: got.add((int(m.group(1)), m.group(2)))
j = json.load(open(lst))
want = {(o[2], o[0]) for o in j["outputs"] if o[2] <= H}
cbw = {(o[2], o[0]) for o in j["outputs"] if o[2] <= H and o[4] == 1}
print(f"prep logged {len(got)} tracked txs to {H}; the list has {len(want)} ({len(cbw)} coinbases, {len({h for h,_ in cbw})} coinbase heights)")
print("in prep only:", sorted(got - want)[:10]); print("in list only:", sorted(want - got)[:10])
print("MATCH" if got == want else "DIFFER")
sys.exit(0 if got == want else 1)
