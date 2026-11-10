#!/usr/bin/env python3
"""deploy/libzync/libzync-xcheck.py: two prep runs must agree on every shared height, all fields but self_id.

  1. identical states (self_id aside) pass, and the state file records the last height checked;
  2. a later run checks only the new heights;
  3. any other field that differs fails (exit 1) and names the field and height;
  4. a missing state at a shared height is "could not check" (exit 2), not a pass;
  5. the newest state of either run (possibly mid-write) is not checked yet.

  python3 test_libzync_xcheck.py            # must PASS
  python3 test_libzync_xcheck.py --control  # the check ignores every field (compares nothing); MUST FAIL
"""
import importlib.util, io, json, os, sys, tempfile
from contextlib import redirect_stdout

CONTROL = "--control" in sys.argv
HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("xc", os.path.join(HERE, "deploy", "libzync", "libzync-xcheck.py"))
xc = importlib.util.module_from_spec(spec); spec.loader.exec_module(xc)
if CONTROL:
    xc.load = lambda p: {}
    print("CONTROL: the check compares nothing -- the checks below MUST fail")

fails = 0
def check(cond, what):
    global fails
    print(("  ok   " if cond else "  FAIL ") + what); fails += 0 if cond else 1

def run(*args):
    buf = io.StringIO()
    with redirect_stdout(buf):
        rc = xc.main(list(args))
    return rc, buf.getvalue()

t = tempfile.mkdtemp(prefix="xcheck_")
A, B, ST = os.path.join(t, "a"), os.path.join(t, "b"), os.path.join(t, "last")
os.makedirs(A); os.makedirs(B)
def put(d, h, **kw):
    s = {"height": h, "trie_root": [h % 7] * 4, "utxo_leaves": h * 3, "self_id": [1] * 8}
    s.update(kw); json.dump(s, open(f"{d}/state_{h}.json", "w"))
for h in range(1, 11):
    put(A, h); put(B, h, self_id=[2] * 8)          # different images, same states

rc, out = run(A, B, "--state", ST)
check(rc == 0 and "1..9: 9 identical" in out, f"identical states (self_id aside) pass, newest left (rc={rc}: {out.strip()})")
check(open(ST).read() == "9", "the state file records the last height checked")
for h in range(11, 16):
    put(A, h); put(B, h)
rc, out = run(A, B, "--state", ST)
check(rc == 0 and "10..14: 5 identical" in out, f"a later run checks only the new heights ({out.strip()})")

put(B, 15, utxo_leaves=-1); put(A, 16); put(B, 16)
rc, out = run(A, B, "--state", ST)
check(rc == 1 and "DIFFER at height 15" in out and "utxo_leaves" in out, f"a differing field fails and is named (rc={rc})")

t2 = tempfile.mkdtemp(prefix="xcheck_")
A2, B2 = os.path.join(t2, "a"), os.path.join(t2, "b"); os.makedirs(A2); os.makedirs(B2)
for h in (1, 2, 3, 4): put(A2, h)
for h in (1, 3, 4): put(B2, h)
rc, out = run(A2, B2)
check(rc == 2 and "state_2.json missing from B" in out, f"a missing shared height is could-not-check, exit 2 (rc={rc})")
rc, out = run(A2, os.path.join(t2, "nope"))
check(rc == 2, "a missing directory is exit 2")

print(f"{'CONTROL: ' if CONTROL else ''}{fails} failure(s)")
if CONTROL:
    sys.exit(0 if fails else 1)
sys.exit(1 if fails else 0)
