#!/usr/bin/env python3
"""rig_reclassify.py <rig.log> <rigroot/out>: re-read a core-rules rig run on the ENGINE with the rig's own rules.

On the engine a guest's output goes to the engine's work dir (rigroot/out/<case>/engine/s*/*.log), not to the host's
output as RISC Zero's executor prints it, and the host's panic on a failed aggregate carried no guest text before
engine_backend's fix (26.11.2026). So a case the guest refused read HOST-FAIL. Here each such case is judged again
exactly as rig.py judges it (its in_guest test and its reason_ok, imported from rig.py), with the host's line PLUS
the case's guest logs as the text. Cases rig.py did not reach are listed as not run.
"""
import glob, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
RIG = os.path.join(HERE, "../../experimental/libzync-bench/core_rules/rig.py")
args = sys.argv[1:]
# rig.py runs its whole rig at import (a bitcoind, the base chain): never import it. Take its rules from its source
# instead, verbatim: the OURS table and reason_ok (from "OURS = {" up to "CASES ="), and the case names (def c_*).
src = open(RIG).read()
rules = src[src.index("OURS = {"):src.index("\nCASES = ")]
rig = type("rig", (), {})()
ns = {"re": re}
exec(rules, ns)
rig.reason_ok = ns["reason_ok"]
rig.CASES = [(n, None) for n in re.findall(r"^def c_(\w+)\(", src, re.M)]

log, out = args
rows = []
for l in open(log, errors="replace"):
    m = re.match(r"^(AGREE|DISAGREE|HOST-FAIL|REASON|CHECK|DEFERRED-OK)\s+(\S+)\s+core=(\S+)\s+guest=(\S+)\s*(.*)$", l.rstrip("\n"))
    if m:
        rows.append(m.groups())
res = {}
for state, name, core, guest, why in rows:
    new = state
    # the engine image has a coin log: rig.py's RIG_COINLOG=1 case, where a missing coin is refused at the block and
    # judged like any other case (not deferred to H)
    if state == "CHECK" and name in ("missing_coin",):
        state = "HOST-FAIL" if guest == "reject" else state
    if state == "HOST-FAIL":
        glog = "\n".join(open(f, errors="replace").read() for f in sorted(glob.glob(f"{out}/{name}/engine/s*/*.log")))
        text = why + "\n" + glog
        # rig.py's test, plus the one mapping the engine needs: RISC Zero's host reports a guest panic as "Guest
        # panicked"; on the engine the guest's own panic line ("panicked at") is in the guest's log
        in_guest = "Guest panicked" in text or "block INVALID" in text or "prove chunk" in text or "panicked at" in glog
        if not in_guest:
            new = "HOST-FAIL"
        elif not rig.reason_ok(core, text):
            new = "REASON"
        else:
            new = "AGREE"
    res[name] = (state, new, core)
    print(f"{new:12} (was {state:10}) {name:24} core={core}")
ran = set(res)
names = [n for n, _ in rig.CASES]
missing = [n for n in names if n not in ran]
from collections import Counter
c = Counter(v[1] for v in res.values())
print(f"\n{len(res)} of {len(names)} cases ran: " + ", ".join(f"{k} {v}" for k, v in sorted(c.items())))
print(f"not run ({len(missing)}): {' '.join(missing)}")
