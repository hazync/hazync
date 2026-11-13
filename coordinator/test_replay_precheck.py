#!/usr/bin/env python3
"""A replayed public proof is refused by a lookup, before any STARK verification (review 11-12, board #6).

Every proof the board serves (/api/proof/<id>, the spine) is the verified proof of something, recorded by its sha256.
Before, a validly signed replay of one under another range id, or of the spine, cost a full verification (two for the
spine) and held a verify slot, so a few IPs could queue honest submits behind nginx's read timeout.

A fake host records every call. The test submits a block and a spine (each verified once), then replays both: the
replays are refused with 409, and the host is NOT called again.

  python3 test_replay_precheck.py            # must PASS
  python3 test_replay_precheck.py --control  # server.py with the pre-check taken out; MUST FAIL
"""
import base64, os, stat, sys, tempfile, types

CONTROL = "--control" in sys.argv
HERE = os.path.dirname(os.path.abspath(__file__))
T = tempfile.mkdtemp(prefix="replay_")
HOST, ARGS = os.path.join(T, "host"), os.path.join(T, "args")
os.makedirs(os.path.join(T, "leaves"))
os.environ.update(HAZYNC_LIBZYNC="1", VERIFY_MODE="real", HAZYNC_HOST=HOST, HAZYNC_BRIDGE_OUT=os.path.join(T, "leaves"),
                  WITNESS_DIR=os.path.join(T, "w"), COORD_DB=os.path.join(T, "c.db"), COORD_STATE=os.path.join(T, "st"),
                  COORD_PROOFS=os.path.join(T, "p"), COORD_SPINE=os.path.join(T, "sp"), SEED_RANGES="0", RATE_EXEMPT="",
                  TIP_HEIGHT="100")
sys.path.insert(0, HERE)
src = open(os.path.join(HERE, "server.py")).read()
if CONTROL:
    a = "        _known = known_receipt(c, sha)\n"
    assert src.count(a) == 1
    src = src.replace(a, "        _known = None\n")
    a = "        _known = known_receipt(c, hashlib.sha256(receipt).hexdigest())\n"
    assert src.count(a) == 1
    src = src.replace(a, "        _known = None\n")
    print("CONTROL: the replay pre-check taken out -- the checks below MUST fail")
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

# a block: verified once
fake_host(5, 5)
rc = b"proof of block 5"
code, out = s.submit(body(rc, range="5"))
check(code == 200 and out.get("ok") and calls() == 1, f"block 5 accepted, verified once ({code}, {calls()} host call(s))")
# replayed under another range id: refused, no verification
code, out = s.submit(body(rc, range="6"))
check(code == 409 and "already the verified proof of 5" in str(out) and calls() == 1,
      f"block 5's proof replayed as block 6: 409 with no verification ({code}, {calls()} host call(s), {str(out)[:70]})")

# a spine: verified once
fake_host(1, 40)
sp = b"spine 1..40"
code, out = s.submit_spine(body(sp))
n = calls()
check(code == 200, f"spine 1..40 accepted ({code}, {str(out)[:60]})")
code, out = s.submit_spine(body(sp))
check(code == 409 and "already the verified proof of spine:1-40" in str(out) and calls() == n,
      f"the spine replayed: 409 with no verification ({code}, {calls() - n} more host call(s), {str(out)[:70]})")

if CONTROL:
    if fails >= 2:
        print(f"CONTROL OK: {fails} checks fail without the pre-check, as they must"); sys.exit(0)
    print(f"CONTROL BROKEN: only {fails} failed -- this test proves nothing"); sys.exit(1)
print("PASS" if fails == 0 else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
