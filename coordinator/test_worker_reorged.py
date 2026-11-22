#!/usr/bin/env python3
"""The worker's submit (coordinator/hazync) and the board's D1 answers (REORG.md §19, review rounds 2 and 3):

  1. 503 {"retry": true} (the board could not read a state just then; nothing counted): sent again, then verified: ok
  2. a 503 that persists past HAZYNC_SUBMIT_RETRIES: UNSENT, not rejected; `submit` keeps the receipt, marks it in
     RCPTDIR/.unsent and exits 75; the next run's resend_unsent sends it (and clears the mark when it lands)
  3. 409 {"reorged": true} (the chain moved under the block): not the worker's failure, ok
  4. a real rejection stays one
  5. HAZYNC_TIP=1 puts "tip": true in the signed claim (claim_body), unset it does not
  6. a proxy's HTML 503 page is retryable on the submit path, a SystemExit elsewhere
  7. HAZYNC_SUBMIT_RETRIES=-1 or "x": clamped (one post, no unbound result)

  python3 test_worker_reorged.py                    # must PASS
  python3 test_worker_reorged.py --control-retry    # no retry on 503: MUST FAIL
  python3 test_worker_reorged.py --control-reorged  # a reorged proof a failure: MUST FAIL
"""
import importlib.machinery, importlib.util, io, os, sys, tempfile, urllib.error

HERE = os.path.dirname(os.path.abspath(__file__))
PATCHES = {
    "--control-retry": ('        if not res.get("retry") or k == tries:\n            break\n', "        break\n"),
    "--control-reorged": ('    if res.get("reorged"):\n', "    if False:\n"),
}
CONTROL = next((a for a in sys.argv[1:] if a in PATCHES), None)
os.environ["HAZYNC_HOME"] = tempfile.mkdtemp(prefix="worker_reorged_")
os.environ["HAZYNC_SUBMIT_RETRIES"] = "2"
src = open(os.path.join(HERE, "hazync")).read()
if CONTROL:
    a, b = PATCHES[CONTROL]
    assert src.count(a) == 1, CONTROL
    src = src.replace(a, b)
    print(f"CONTROL {CONTROL}: the checks below MUST fail")
path = os.path.join(os.environ["HAZYNC_HOME"], "hazync_under_test.py")
open(path, "w").write(src)
loader = importlib.machinery.SourceFileLoader("hz", path)
hz = importlib.util.module_from_spec(importlib.util.spec_from_loader("hz", loader))
loader.exec_module(hz)
hz.time.sleep = lambda s: None
REAL_POST = hz.post

fails = 0
def check(cond, what):
    global fails
    print(("  ok   " if cond else "  FAIL ") + what); fails += 0 if cond else 1

class SK:
    def sign(self, b): return bytes(64)
def run(answers, rng="977001"):
    seen = []
    def post(path, body):
        seen.append(body)
        return answers[min(len(seen) - 1, len(answers) - 1)]
    hz.post = post
    return hz._post_receipt(rng, b"rcpt", SK(), "ab" * 32, "h"), len(seen)

r503 = {"error": "the board's state at block 977000 cannot be read right now", "retry": True}
(ok, err), n = run([r503, r503, {"ok": True}])
check(ok and n == 3, f"1. 503 retry twice, then verified: ok after {n} posts")
(ok, err), n = run([r503])
check(not ok and n == 3 and err.startswith(hz.UNSENT), f"2. a 503 past HAZYNC_SUBMIT_RETRIES=2: UNSENT after {n} posts, not a rejection")
hz.RCPTDIR.mkdir(parents=True, exist_ok=True)
(hz.RCPTDIR / "977001.bin").write_bytes(b"rcpt")
try:
    hz.identity = lambda: (SK(), "ab" * 32, "h")
    hz.warn_if_default_handle = lambda brief=False: None
    hz.cmd_submit(["977001"]); code = 0
except SystemExit as e:
    code = e.code
check(code == hz.EX_TEMPFAIL and (hz.UNSENT_DIR / "977001").exists() and (hz.RCPTDIR / "977001.bin").exists(),
      f"2. `submit`: exit {code} (75), the receipt kept and marked unsent")
run([r503])
sent, left = hz.resend_unsent(SK(), "ab" * 32, "h")
check((sent, left) == (0, 1) and (hz.UNSENT_DIR / "977001").exists(), f"2. the next run while the board still cannot check it: kept ({sent}, {left})")
run([{"ok": True}])
sent, left = hz.resend_unsent(SK(), "ab" * 32, "h")
check((sent, left) == (1, 0) and not (hz.UNSENT_DIR / "977001").exists(), f"2. ... and once it can: sent, the mark cleared ({sent}, {left})")
(ok, err), n = run([{"error": "reorged: the prep run holding block 977001 stopped", "reorged": True}])
check(ok and n == 1, "3. 409 reorged: not the worker's failure")
(ok, err), n = run([{"error": "receipt rejected - not a valid proof"}])
check(not ok and "not a valid proof" in err and not err.startswith(hz.UNSENT), "4. a real rejection stays one")
os.environ["HAZYNC_TIP"] = "1"
b1 = hz.claim_body(SK(), "ab" * 32, "h", "n" * 32, 1)
os.environ.pop("HAZYNC_TIP")
b0 = hz.claim_body(SK(), "ab" * 32, "h", "n" * 32, 1)
check(b1.get("tip") is True and "tip" not in b0 and b0["sig"] == b1["sig"], "5. HAZYNC_TIP=1: tip: true in the claim; unset: absent")
# 6. a proxy's HTML page for a 503
def html_503(req, timeout=None):
    raise urllib.error.HTTPError("u", 503, "Service Unavailable", {}, io.BytesIO(b"<html>503</html>"))
hz._open = html_503
hz._guard_dev_writes = lambda p: None
hz.post = REAL_POST
r = hz.post("/api/submit", {})
check(r.get("retry") is True, f"6. an HTML 503 on the submit path: retryable ({r})")
try:
    hz.post("/api/claim", {}); raised = False
except SystemExit:
    raised = True
check(raised, "6. ... and still a SystemExit on other paths (the claim's own retry)")
for v in ("-1", "x"):
    os.environ["HAZYNC_SUBMIT_RETRIES"] = v
    (ok, err), n = run([r503])
    check(n == (1 if v == "-1" else 6) and err.startswith(hz.UNSENT), f"7. HAZYNC_SUBMIT_RETRIES={v}: {n} post(s), clamped")

# the tip rig's claim (tip_board.py) carries the same flag
sys.path.insert(0, HERE)
import tip_board
seen_tb = []
os.environ["HAZYNC_TIP"] = "1"
tip_board.claim(post_fn=lambda p, b: (seen_tb.append(b), {"ok": True, "range": "977001"})[1], ident=(SK(), "ab" * 32, "h"), sleep=lambda s: None)
os.environ.pop("HAZYNC_TIP")
tip_board.claim(post_fn=lambda p, b: (seen_tb.append(b), {"ok": True, "range": "977002"})[1], ident=(SK(), "ab" * 32, "h"), sleep=lambda s: None)
check(seen_tb[0].get("tip") is True and "tip" not in seen_tb[1], "5. tip_board.py's claim: tip with HAZYNC_TIP=1, not without")

if CONTROL:
    if fails:
        print(f"CONTROL OK: {fails} check(s) fail under {CONTROL}, as they must"); sys.exit(0)
    print(f"CONTROL BROKEN: nothing failed under {CONTROL} -- this test proves nothing"); sys.exit(1)
print("PASS" if not fails else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
