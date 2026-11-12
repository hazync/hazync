#!/usr/bin/env python3
"""Free keys cannot stall the board (Fable review, 2026-11-12).

  1. Junk submissions (random signatures, or any key other than the live claimant) do not count as failures of the
     block, so three of them cannot park a block and take a working prover's claim away. A failure from the claimant
     itself still counts.
  2. CLAIM_NEW_IP_MAX: keys with no proven block share one budget of live claims per IP, so made-up keys from one
     address cannot take every low block. Another address, and a key that has proven a block, are not held back.

  python3 test_claim_sybil.py            # must PASS
  python3 test_claim_sybil.py --control  # server.py with both guards taken out; MUST FAIL
"""
import os, secrets, sys, tempfile, time, types

CONTROL = "--control" in sys.argv
HERE = os.path.dirname(os.path.abspath(__file__))
T = tempfile.mkdtemp(prefix="sybil_")
os.makedirs(os.path.join(T, "leaves"))
for h in range(1, 60):
    open(os.path.join(T, "leaves", f"leaf_{h}.bin"), "wb").write(b"LZLEAF02")
os.environ.update(COORD_DB=os.path.join(T, "c.db"), HAZYNC_LIBZYNC="1", HAZYNC_BRIDGE_OUT=os.path.join(T, "leaves"),
                  WITNESS_DIR=os.path.join(T, "w"), COORD_STATE=os.path.join(T, "st"), COORD_PROOFS=os.path.join(T, "p"),
                  COORD_SPINE=os.path.join(T, "sp"), SEED_RANGES="0", VERIFY_MODE="mock", COORD_ALLOW_MOCK="1",
                  TIP_HEIGHT="100", CLAIM_NEW_IP_MAX="8", RATE_EXEMPT="")
sys.path.insert(0, HERE)
src = open(os.path.join(HERE, "server.py")).read()
if CONTROL:
    for a, b in (("if sig_ok and not _other:", "if True:"),
                 ("if CLAIM_NEW_IP_MAX > 0 and ip and ip not in RATE_EXEMPT:", "if False:")):
        assert a in src, a
        src = src.replace(a, b)
    print("CONTROL: both guards taken out -- the checks below MUST fail")
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

def key():
    sk = Ed25519PrivateKey.generate()
    return sk, sk.public_key().public_bytes(ser.Encoding.Raw, ser.PublicFormat.Raw).hex()

def signed_claim(sk, pk, handle, ip):
    n, ts = secrets.token_hex(8), int(time.time())
    return s.claim({"pubkey": pk, "handle": handle, "nonce": n, "ts": ts, "_client_ip": ip,
                    "sig": sk.sign(f"claim:{n}:{ts}".encode()).hex()})

def row(rid):
    c = s.db(); r = dict(c.execute("SELECT status, assignee, attempts FROM ranges WHERE id=?", (rid,)).fetchone()); c.close()
    return r

# 1. junk submissions against a live claim (every receipt here fails verification, as junk does)
import base64
s.verify_receipt = lambda receipt, rng: (False, "receipt rejected — not a valid proof", None)
vsk, vpk = key()
code, body = signed_claim(vsk, vpk, "victim", "10.0.0.1")
rid = body["range"]
for _ in range(3):
    s.submit({"range": rid, "pubkey": "11" * 32, "sig": secrets.token_hex(64), "receipt": "AAAA", "handle": "x",
              "_client_ip": "10.9.9.9"})
osk, opk = key()   # a real key, signing its own junk, that does not hold the claim
for _ in range(3):
    s.submit({"range": rid, "pubkey": opk, "sig": osk.sign(base64.b64decode("AAAA")).hex(),
              "receipt": "AAAA", "handle": "o", "_client_ip": "10.9.9.8"})
r = row(rid)
check(r["status"] == "claimed" and r["assignee"] == vpk and not r["attempts"],
      f"six junk submissions (random signatures, another key's) leave the victim's claim alone ({r})")
# the claimant's own failure still counts
rc = base64.b64encode(b"bad-proof").decode()
s.submit({"range": rid, "pubkey": vpk, "sig": vsk.sign(b"bad-proof").hex(), "receipt": rc, "handle": "victim",
          "_client_ip": "10.0.0.1"})
check(row(rid)["attempts"] == 1, f"the claimant's own failed proof still counts ({row(rid)['attempts']})")

# 2. made-up keys from one address
got = []
for i in range(12):
    sk, pk = key()
    code, body = signed_claim(sk, pk, f"sq{i}", "10.6.6.6")
    got.append(code)
check(got.count(200) == 8 and got[8:] == [429] * 4, f"new keys from one IP get 8 live claims, then 429 ({got})")
sk, pk = key()
check(signed_claim(sk, pk, "elsewhere", "10.7.7.7")[0] == 200, "a new key from another IP still gets a block")
wsk, wpk = key()
c = s.db()
c.execute("INSERT INTO contributors(pubkey,handle,first_seen,blocks) VALUES(?,?,?,?)", (wpk, "worker", time.time(), 3))
c.commit(); c.close()
check(signed_claim(wsk, wpk, "worker", "10.6.6.6")[0] == 200, "a key with proven blocks on the same IP still gets a block")

if CONTROL:
    if fails >= 2:
        print(f"CONTROL OK: {fails} checks fail without the guards, as they must"); sys.exit(0)
    print(f"CONTROL BROKEN: only {fails} failed without the guards -- this test proves nothing"); sys.exit(1)
print("PASS" if fails == 0 else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
