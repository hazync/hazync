#!/usr/bin/env python3
"""A fresh board's spine can be seeded with block 1's own proof (16.11.2026, v0.26.0 board), at a bounded cost.

`hazync spine` seeds an empty spine with the receipt from /api/proof/1. The replay pre-check (review 11-12, board #6)
refused it with 409 as "already the verified proof of 1", so on a fresh board the spine stayed empty for good; the
workaround was re-proving block 1 for new receipt bytes. A verified genesis range's proof is a genesis-anchored head,
not a replay, as long as it would advance the spine. Letting it through must not reopen what the pre-check closed
(unbounded STARK verifications of a public receipt), nor hand the spine's credit to whoever re-submits it.

A fake host records every call. Ranges are proven by key P; the spine is submitted by key Q.
  - block 5's proof offered as the spine: 409 with no verification (not a genesis range)
  - a [1..4] fold whose spine verification FAILS: 400 once, then 409 with no verification (review M1)
  - block 1's proof, sent 5 times at once to an empty spine: ONE verification, one 200 (review M2; the fix itself)
  - the spine names P, who proved block 1, with P's signature over the receipt, not Q (review L1)
  - block 1's proof again, now the head: 409 with no verification (a true replay)
  - the head moved, so the failed [1..4] is tried again: it verifies and advances the spine to 4
  - block 1's proof below the head: 409 with no verification

  python3 test_spine_seed.py                  # must PASS
  python3 test_spine_seed.py --control        # the seed exception taken out; MUST FAIL
  python3 test_spine_seed.py --control-race   # no in-flight guard: racing copies all verify; MUST FAIL
  python3 test_spine_seed.py --control-failed # no failure memory: a failing seed is verified again; MUST FAIL
  python3 test_spine_seed.py --control-credit # the submitter is credited, not the prover; MUST FAIL
"""
import base64, os, stat, sys, tempfile, threading, types

PATCHES = {
    "--control": ("        _row = spine_seed_row(c, _sha) if _known else None\n", "        _row = None\n"),
    "--control-race": ("            if _sha in _spine_seed_inflight:\n", "            if False:\n"),
    "--control-failed": ("            if _spine_seed_failed.get(_sha) == _spine_hi(spine_head()):\n",
                         "            if False:\n"),
    "--control-credit": ('    if seed is not None and seed["pubkey"]:\n', "    if False:\n"),
}
CONTROL = next((a for a in sys.argv[1:] if a in PATCHES), None)
HERE = os.path.dirname(os.path.abspath(__file__))
T = tempfile.mkdtemp(prefix="spineseed_")
HOST, ARGS = os.path.join(T, "host"), os.path.join(T, "args")
os.makedirs(os.path.join(T, "leaves"))
os.environ.update(HAZYNC_LIBZYNC="1", VERIFY_MODE="real", HAZYNC_HOST=HOST, HAZYNC_BRIDGE_OUT=os.path.join(T, "leaves"),
                  WITNESS_DIR=os.path.join(T, "w"), COORD_DB=os.path.join(T, "c.db"), COORD_STATE=os.path.join(T, "st"),
                  COORD_PROOFS=os.path.join(T, "p"), COORD_SPINE=os.path.join(T, "sp"), SEED_RANGES="0", RATE_EXEMPT="",
                  TIP_HEIGHT="100", VERIFY_CONCURRENCY="8")
sys.path.insert(0, HERE)
src = open(os.path.join(HERE, "server.py")).read()
if CONTROL:
    a, b = PATCHES[CONTROL]
    assert src.count(a) == 1, CONTROL
    src = src.replace(a, b)
    print(f"CONTROL {CONTROL}: the checks below MUST fail")
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

def fake_host(lo, hi, fail=False, delay=0):
    tip = s.GENESIS_TIP if lo == 1 else "ab" * 32
    with open(HOST, "w") as f:
        f.write("#!/bin/sh\necho \"$@\" >> %s\n" % ARGS + (f"sleep {delay}\n" if delay else "")
                + ("echo 'verification failed' >&2; exit 1\n" if fail else
                   f"echo 'RANGE-OK lo={lo} hi={hi} in_tip={tip} out_tip={'cd' * 32} out_leaves=3 range_work=0 "
                   f"in_bhash={'00' * 32} out_bhash={'11' * 32} anchored={int(lo == 1)} end=board'\n"))
    os.chmod(HOST, os.stat(HOST).st_mode | stat.S_IEXEC)

def calls():
    return len(open(ARGS).read().splitlines()) if os.path.exists(ARGS) else 0

def key():
    sk = Ed25519PrivateKey.generate()
    return sk, sk.public_key().public_bytes(ser.Encoding.Raw, ser.PublicFormat.Raw).hex()
P_SK, P = key()          # proves the ranges
Q_SK, Q = key()          # submits the spine
def body(sk, pk, receipt, **kw):
    return dict(pubkey=pk, sig=sk.sign(receipt).hex(), receipt=base64.b64encode(receipt).decode(),
                handle="prover" if pk == P else "submitter", _client_ip="10.1.1.1", **kw)
def spine(receipt):
    return s.submit_spine(body(Q_SK, Q, receipt))
def hi():
    return (s.spine_head() or {}).get("hi")

# a non-genesis range's proof is not a spine seed: refused by the lookup, empty spine or not
fake_host(5, 5)
r5 = b"proof of block 5"
code, out = s.submit(body(P_SK, P, r5, range="5"))
check(code == 200 and calls() == 1, f"block 5 accepted ({code}, {calls()} host call(s))")
code, out = spine(r5)
check(code == 409 and "already the verified proof of 5" in str(out) and calls() == 1 and hi() is None,
      f"block 5's proof as the spine: 409 with no verification ({code}, {calls()} host call(s), {str(out)[:70]})")

# blocks 1..4 and their fold [1..4], proven by P
for b in (1, 2, 3, 4):
    fake_host(b, b)
    code, out = s.submit(body(P_SK, P, b"proof of block %d" % b, range=str(b)))
    check(code == 200, f"block {b} accepted ({code}, {str(out)[:60]})")
fake_host(1, 4)
r14 = b"proof of 1..4"
code, out = s.submit(body(P_SK, P, r14, range="1-4"))
check(code == 200, f"range 1-4 accepted ({code}, {str(out)[:60]})")

# M1: a genesis range whose spine verification fails is not verified again and again
fake_host(1, 4, fail=True)
n = calls()
code, out = spine(r14)
check(code == 400 and calls() == n + 1, f"[1..4] failing as the spine: 400 after one verification ({code}, {calls() - n})")
n = calls()
code, out = spine(r14)
check(code == 409 and calls() == n and hi() is None,
      f"sent again at the same head: 409 with no verification ({code}, {calls() - n} host call(s), {str(out)[:70]})")

# M2 + the fix: block 1's proof, five copies at once, seeds the empty spine with ONE verification
fake_host(1, 1, delay=1)
r1 = b"proof of block 1"
n = calls()
results = []
ts = [threading.Thread(target=lambda: results.append(spine(r1)[0])) for _ in range(5)]
[t.start() for t in ts]; [t.join() for t in ts]
check(results.count(200) == 1 and calls() == n + 1 and hi() == 1,
      f"block 1's proof x5 at once: one 200, ONE verification, spine at 1 ({sorted(results)}, {calls() - n} host call(s))")

# L1: the spine is credited to who proved block 1, under their own signature over the receipt
head = s.spine_head() or {}
c = s.db()
row = c.execute("SELECT pubkey, sig FROM submissions WHERE range_id='spine:1-1'").fetchone()
c.close()
check(head.get("pubkey") == P and row and row["pubkey"] == P and s.verify_sig(P, row["sig"], r1),
      f"the spine names the prover P, with P's signature, not the submitter Q "
      f"(head {str(head.get('pubkey'))[:8]}, row {str(row['pubkey'] if row else None)[:8]}; P {P[:8]}, Q {Q[:8]})")

# the same receipt again is now a true replay of the spine head
fake_host(1, 1)
n = calls()
code, out = spine(r1)
check(code == 409 and "already the verified proof of" in str(out) and calls() == n,
      f"the seed replayed: 409 with no verification ({code}, {calls() - n} host call(s), {str(out)[:70]})")

# the head moved, so the [1..4] that failed before may be tried again; this time it verifies
fake_host(1, 4)
n = calls()
code, out = spine(r14)
check(code == 200 and hi() == 4 and calls() == n + 1,
      f"the head moved: [1..4] is verified again and advances the spine to 4 ({code}, {calls() - n}, {str(out)[:60]})")

# block 1's proof is below the head now: a replay that would not advance it, refused with no verification
n = calls()
code, out = spine(r1)
check(code == 409 and calls() == n and hi() == 4,
      f"block 1's proof below the head: 409 with no verification ({code}, {calls() - n} host call(s), {str(out)[:70]})")

if CONTROL:
    if fails >= 1:
        print(f"CONTROL OK: {fails} check(s) fail under {CONTROL}, as they must"); sys.exit(0)
    print(f"CONTROL BROKEN: nothing failed under {CONTROL} -- this test proves nothing"); sys.exit(1)
print("PASS" if fails == 0 else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
