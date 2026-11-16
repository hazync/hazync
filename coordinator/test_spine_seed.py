#!/usr/bin/env python3
"""A fresh board's spine can be seeded with block 1's own proof (16.11.2026, v0.26.0 board).

`hazync spine` seeds an empty spine with the receipt from /api/proof/1. The replay pre-check (review 11-12, board #6)
refused it with 409 as "already the verified proof of 1", so on a fresh board the spine stayed empty for good; the
workaround was re-proving block 1 for new receipt bytes. A verified genesis range's proof is a genesis-anchored head,
not a replay, as long as it would advance the spine.

A fake host records every call. On a fresh board:
  - block 5's proof offered as the spine: still 409 with no verification (not a genesis range)
  - block 1's proof offered as the spine: accepted, one verification (the fix; 409 on main)
  - the same receipt again, now the spine head: 409 with no verification (a true replay)
  - a verified genesis range [1..4] above the head: accepted (it advances the spine)
  - block 1's proof again, below the head: 409 with no verification

  python3 test_spine_seed.py            # must PASS
  python3 test_spine_seed.py --control  # server.py with the seed exception taken out; MUST FAIL
"""
import base64, os, stat, sys, tempfile, types

CONTROL = "--control" in sys.argv
HERE = os.path.dirname(os.path.abspath(__file__))
T = tempfile.mkdtemp(prefix="spineseed_")
HOST, ARGS = os.path.join(T, "host"), os.path.join(T, "args")
os.makedirs(os.path.join(T, "leaves"))
os.environ.update(HAZYNC_LIBZYNC="1", VERIFY_MODE="real", HAZYNC_HOST=HOST, HAZYNC_BRIDGE_OUT=os.path.join(T, "leaves"),
                  WITNESS_DIR=os.path.join(T, "w"), COORD_DB=os.path.join(T, "c.db"), COORD_STATE=os.path.join(T, "st"),
                  COORD_PROOFS=os.path.join(T, "p"), COORD_SPINE=os.path.join(T, "sp"), SEED_RANGES="0", RATE_EXEMPT="",
                  TIP_HEIGHT="100")
sys.path.insert(0, HERE)
src = open(os.path.join(HERE, "server.py")).read()
if CONTROL:
    a = "        if _known and spine_seed_allowed(c, _sha):\n"
    assert src.count(a) == 1
    src = src.replace(a, "        if False:\n")
    print("CONTROL: the spine seed exception taken out -- the checks below MUST fail")
s = types.ModuleType("server"); s.__file__ = os.path.join(HERE, "server.py"); sys.modules["server"] = s
exec(compile(src, s.__file__, "exec"), s.__dict__)
s.init_db()
if not s.HAVE_ED:
    print("SKIP: no ed25519 library"); sys.exit(0)
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization as ser

fails = 0
def check(cond, what):
    global fails
    print(("  ok   " if cond else "  FAIL ") + what); fails += 0 if cond else 1

def fake_host(lo, hi):
    tip = s.GENESIS_TIP if lo == 1 else "ab" * 32
    with open(HOST, "w") as f:
        f.write("#!/bin/sh\necho \"$@\" >> %s\n" % ARGS
                + f"echo 'RANGE-OK lo={lo} hi={hi} in_tip={tip} out_tip={'cd' * 32} out_leaves=3 range_work=0 "
                + f"in_bhash={'00' * 32} out_bhash={'11' * 32} anchored={int(lo == 1)} end=board'\n")
    os.chmod(HOST, os.stat(HOST).st_mode | stat.S_IEXEC)

def calls():
    return len(open(ARGS).read().splitlines()) if os.path.exists(ARGS) else 0

sk = Ed25519PrivateKey.generate()
pk = sk.public_key().public_bytes(ser.Encoding.Raw, ser.PublicFormat.Raw).hex()
def body(receipt, **kw):
    return dict(pubkey=pk, sig=sk.sign(receipt).hex(), receipt=base64.b64encode(receipt).decode(), handle="t",
                _client_ip="10.1.1.1", **kw)

# a non-genesis range's proof is not a spine seed: refused by the lookup, empty spine or not
fake_host(5, 5)
r5 = b"proof of block 5"
code, out = s.submit(body(r5, range="5"))
check(code == 200 and calls() == 1, f"block 5 accepted ({code}, {calls()} host call(s))")
code, out = s.submit_spine(body(r5))
check(code == 409 and "already the verified proof of 5" in str(out) and calls() == 1 and s.spine_head() is None,
      f"block 5's proof as the spine: 409 with no verification ({code}, {calls()} host call(s), {str(out)[:70]})")

# block 1 proven on a fresh board, then its own proof seeds the spine (what `hazync spine` does)
fake_host(1, 1)
r1 = b"proof of block 1"
code, out = s.submit(body(r1, range="1"))
check(code == 200 and calls() == 2, f"block 1 accepted ({code}, {calls()} host call(s))")
n = calls()
code, out = s.submit_spine(body(r1))
head = s.spine_head()
check(code == 200 and head and head.get("hi") == 1 and calls() == n + 1,
      f"block 1's proof seeds the empty spine, verified once ({code}, {calls() - n} host call(s), {str(out)[:80]})")

# the same receipt again is now a true replay of the spine head
n = calls()
code, out = s.submit_spine(body(r1))
check(code == 409 and "already the verified proof of" in str(out) and calls() == n,
      f"the seed replayed: 409 with no verification ({code}, {calls() - n} host call(s), {str(out)[:70]})")

# a verified genesis range above the head advances the spine with its own proof
for b in (2, 3, 4):              # a fold is accepted only over blocks the board holds
    fake_host(b, b)
    code, out = s.submit(body(b"proof of block %d" % b, range=str(b)))
    check(code == 200, f"block {b} accepted ({code}, {str(out)[:60]})")
fake_host(1, 4)
r14 = b"proof of 1..4"
code, out = s.submit(body(r14, range="1-4"))
check(code == 200, f"range 1-4 accepted ({code}, {str(out)[:60]})")
n = calls()
code, out = s.submit_spine(body(r14))
head = s.spine_head()
check(code == 200 and head and head.get("hi") == 4 and calls() == n + 1,
      f"range 1-4's proof advances the spine to 4 ({code}, {calls() - n} host call(s), {str(out)[:70]})")

# block 1's proof is below the head now: a replay that would not advance it, refused with no verification
n = calls()
code, out = s.submit_spine(body(r1))
check(code == 409 and calls() == n and (s.spine_head() or {}).get("hi") == 4,
      f"block 1's proof below the head: 409 with no verification ({code}, {calls() - n} host call(s), {str(out)[:70]})")

if CONTROL:
    if fails >= 2:
        print(f"CONTROL OK: {fails} checks fail without the seed exception, as they must"); sys.exit(0)
    print(f"CONTROL BROKEN: only {fails} failed -- this test proves nothing"); sys.exit(1)
print("PASS" if fails == 0 else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
