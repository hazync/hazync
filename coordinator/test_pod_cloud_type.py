#!/usr/bin/env python3
"""Every RunPod deploy path asks for SECURE hosts, not ALL/COMMUNITY (hazync#523).

⛔ WHY. `cloudType: ALL` includes RunPod's community hosts, and those mostly never start: RunPod
accepts the deploy and never publishes a port, so the caller waits out its full SSH timeout having
paid for a pod that never existed. Measured 2026-09-20 across two runs, and the split is by PRICE,
which is the tell:

    $0.34/hr    1 of 6 started   (17%)
    $0.49/hr    1 of 1 started
    $0.74/hr   14 of 14 started  (100%)

A 4-card run died outright: three of five never started, so the run could not reach its minimum.

⛔ AND THIS TEST EXISTS BECAUSE THE FIX ONLY LANDED IN ONE OF TWO PLACES. tip_smoke.py was corrected
with that measurement written beside it; sponsor_bot.py kept `cloudType: ALL` for another six days --
even though tip_smoke.py imports sponsor_bot.IMAGE and sponsor_bot.GPU_TYPES, so the two deploy paths
are siblings by construction. One file was fixed, its sibling was not, and nothing compared them.

⚠ For the BOT the consequence is worse than a slow run. A sponsor has already paid. The sponsorship
sits in `proving` while pods that never boot burn down RUNPOD_REFUSALS_MAX, and the person who paid
watches nothing happen.

  python3 test_pod_cloud_type.py            # must PASS
  python3 test_pod_cloud_type.py --control  # a deploy path put back to COMMUNITY; MUST be caught
"""
import os
import pathlib
import re
import sys

CONTROL = "--control" in sys.argv
ROOT = pathlib.Path(os.path.dirname(os.path.abspath(__file__))).parent

fails = []


def check(ok, what):
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails.append(what)


# ⛔ Search the WHOLE repo, with no filename filter. The bug was that somebody fixed the file they
# were looking at; a test that checks the two files we happen to know about would have the same blind
# spot as the fix did. Any new deploy path is caught by construction.
#
# ⚠ ANCHOR ON THE CALL, NOT ON THE WORD. A bare search for `cloudType:` also matches this file's own
# docstring and the comments that explain the bug, so the first version of this test failed on its
# own prose -- four "failures", none of them code. Find each podFindAndDeployOnDemand and read the
# cloudType belonging to THAT mutation.
#
# ⚠ And match the CALL SIGNATURE `podFindAndDeployOnDemand(input:`, not the bare name: the bare name
# also appears where the RESPONSE is unwrapped, `data["podFindAndDeployOnDemand"]["id"]`, which is
# not a deploy and has no cloudType to check.
SKIP_DIRS = {".git", "node_modules", "target", "__pycache__", "venv", ".venv"}
# GraphQL (retiring early 2027): `podFindAndDeployOnDemand(input: { cloudType: SECURE, ... })`; the cloud
# follows the call.
CALL = re.compile(r"podFindAndDeployOnDemand\s*\(\s*input\s*:")
CLOUD = re.compile(r"cloudType\s*:\s*([A-Z_]+)")
WINDOW = 400          # characters after the call in which its cloudType must appear
# ⭐ REST v2: a create is a POST to /pods whose JSON body carries `"cloud"`. The body is BUILT BEFORE the
# call -- sponsor_bot.create_pod's dict, the `Q=$(python3 -c ...)` line before `rp POST /pods "$Q"` -- so its
# cloud is read from the window BEFORE the call, nearest first.
# ⛔ v2 defaults an absent `cloud` to SECURE today, but a default can change under us and an explicit value
# cannot: a v2 create with no `cloud` in its body is reported UNSET and fails, exactly like GraphQL.
# ⚠ The bench scripts create through `rp_create "$Q" <name>`; the helper's own `rp POST /pods "$1"` is generic
# (its body is the caller's) and is not a deploy path, so it is skipped -- each CALLER is checked instead.
CALL_V2 = re.compile(r"""(?:_req\(\s*["']POST["']\s*,\s*["']/pods["']|\brp\s+POST\s+["']?/pods\b(?!["']?\s+"\$1")|\brp_create\s+")""")
CLOUD_V2 = re.compile(r"""["']cloud["']\s*:\s*["']([A-Z_]+)["']""")
WINDOW_V2 = 900       # characters BEFORE the call in which its body's cloud must appear

# ⚠ And skip THIS file. It is the scanner, so its prose necessarily quotes both the call signature
# and the bad value; without this it reports itself. That is not the blind spot the whole-repo sweep
# exists to avoid -- this file deploys nothing -- but every other file stays in scope. The fake RunPod
# server in test_sponsor_bot.py and test_runpod_v2.py RECEIVE creates; they make none.
SELF = os.path.basename(os.path.abspath(__file__))
NOT_CALLERS = {SELF, "test_runpod_v2.py"}

paths = []
for p in ROOT.rglob("*"):
    if p.is_dir() or p.suffix not in (".py", ".sh", ".js", ".ts"):
        continue
    if any(part in SKIP_DIRS for part in p.parts) or p.name in NOT_CALLERS:
        continue
    paths.append(p)

found = []
for p in paths:
    try:
        src = p.read_text(errors="ignore")
    except OSError:
        continue
    hits = []
    for m in CALL.finditer(src):
        cm = CLOUD.search(src, m.start(), m.start() + WINDOW)
        hits.append((m.start(), cm.group(1) if cm else "UNSET"))
    for m in CALL_V2.finditer(src):
        before = list(CLOUD_V2.finditer(src, max(0, m.start() - WINDOW_V2), m.start()))
        hits.append((m.start(), before[-1].group(1) if before else "UNSET"))
    for pos, val in hits:
        line = src[:pos].count("\n") + 1
        # ⛔ A deploy with no cloud at all is NOT a pass: RunPod's GraphQL default included community
        # hosts, so silence here buys exactly the pods that never start.
        # ⚠ The control restores the bug in memory only -- it never writes to the tree.
        if CONTROL and p.name == "sponsor_bot.py":
            val = "COMMUNITY"
        found.append((p.relative_to(ROOT), line, val))

# ⛔ A test that finds nothing must FAIL, not pass. If the mutation's shape ever changes -- a
# different key name, a templateId instead -- this file would otherwise go green by checking nothing,
# which is the failure mode it was written to prevent.
check(len(found) >= 2,
      f"found {len(found)} RunPod deploy path(s) to check (expected at least 2)")
# ⛔ The shared create (sponsor_bot.create_pod, which tip_smoke and fold_rent also go through) must be among
# them: if its call shape changes and the scanner stops seeing it, the one path that spends sponsor money is
# unchecked while the experimental scripts keep the count up.
check(any(str(rel).endswith("coordinator/sponsor_bot.py") for rel, _, _ in found),
      "coordinator/sponsor_bot.py's create is among the deploy paths checked")

for rel, line, val in found:
    check(val == "SECURE", f"{rel}:{line} asks for {val}"
          + ("" if val == "SECURE" else "  ⛔ community hosts mostly never start"))

print()
if CONTROL:
    if any("sponsor_bot.py" in f for f in fails):
        print("CONTROL OK — a deploy path put back to COMMUNITY is caught:")
        for f in fails:
            print(f"  - {f}")
        sys.exit(0)
    print("CONTROL FAILED — the bug was restored but nothing failed")
    sys.exit(1)
if fails:
    print(f"⛔ {len(fails)} check(s) FAILED")
    for f in fails:
        print(f"   - {f}")
    sys.exit(1)
print(f"all {len(found)} RunPod deploy paths ask for SECURE hosts")
