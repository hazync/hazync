#!/usr/bin/env python3
"""The worker's submit (coordinator/hazync `_post_receipt`) and the board's D1 answers (REORG.md §19, review round 2):

  1. 503 {"retry": true} (the board could not read a state just then; nothing counted): sent again, then verified: ok
  2. a 503 that persists past HAZYNC_SUBMIT_RETRIES: a rejection (not a hang)
  3. 409 {"reorged": true} (the chain moved under the block): not the worker's failure, ok
  4. a real rejection stays one
  5. HAZYNC_TIP=1 puts "tip": true in the claim (the board's TIP_CLAIM_FIRST offers tip blocks first only to such workers)

  python3 test_worker_reorged.py            # must PASS
  python3 test_worker_reorged.py --control  # the worker before (no retry, a reorged proof a failure): MUST FAIL
"""
import importlib.machinery, importlib.util, os, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
CONTROL = "--control" in sys.argv[1:]
os.environ["HAZYNC_HOME"] = tempfile.mkdtemp(prefix="worker_reorged_")
os.environ["HAZYNC_SUBMIT_RETRIES"] = "2"
src = open(os.path.join(HERE, "hazync")).read()
if CONTROL:
    a = '        if not res.get("retry") or k == tries:\n            break\n'
    b = '    if res.get("reorged"):\n'
    assert src.count(a) == 1 and src.count(b) == 1
    src = src.replace(a, "        break\n").replace(b, "    if False:\n")
    print("CONTROL: no retry on 503, a reorged proof a failure; the checks below MUST fail")
path = os.path.join(os.environ["HAZYNC_HOME"], "hazync_under_test.py")
open(path, "w").write(src)
loader = importlib.machinery.SourceFileLoader("hz", path)
hz = importlib.util.module_from_spec(importlib.util.spec_from_loader("hz", loader))
loader.exec_module(hz)
hz.time.sleep = lambda s: None

fails = 0
def check(cond, what):
    global fails
    print(("  ok   " if cond else "  FAIL ") + what); fails += 0 if cond else 1

class SK:
    def sign(self, b): return bytes(64)
def run(answers):
    seen = []
    def post(path, body):
        seen.append(body)
        return answers[min(len(seen) - 1, len(answers) - 1)]
    hz.post = post
    return hz._post_receipt("977001", b"rcpt", SK(), "ab" * 32, "h"), len(seen)

r503 = {"error": "the board's state at block 977000 cannot be read right now", "retry": True}
(ok, err), n = run([r503, r503, {"ok": True}])
check(ok and n == 3, f"1. 503 retry twice, then verified: ok after {n} posts")
(ok, err), n = run([r503])
check(not ok and n == 3, f"2. a 503 past HAZYNC_SUBMIT_RETRIES=2: a rejection after {n} posts")
(ok, err), n = run([{"error": "reorged: the prep run holding block 977001 stopped", "reorged": True}])
check(ok and n == 1, "3. 409 reorged: not the worker's failure")
(ok, err), n = run([{"error": "receipt rejected - not a valid proof"}])
check(not ok and "not a valid proof" in err, "4. a real rejection stays one")
check('body["tip"] = True' in src and 'os.environ.get("HAZYNC_TIP") == "1"' in src, "5. HAZYNC_TIP=1 sends tip: true in the claim")

if CONTROL:
    if fails:
        print(f"CONTROL OK: {fails} check(s) fail, as they must"); sys.exit(0)
    print("CONTROL BROKEN: nothing failed -- this test proves nothing"); sys.exit(1)
print("PASS" if not fails else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
