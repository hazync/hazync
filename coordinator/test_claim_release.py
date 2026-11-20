#!/usr/bin/env python3
"""POST /api/release: a worker that stops gives its claimed block back at once (#725).

WHY THIS EXISTS. A stopped worker's claim held its block until it lapsed: up to CLAIM_TTL after its last beat,
CLAIM_GRACE if it never beat. On 14.11.2026 block 1807 sat that way, held by a stopped Windows app.

What is checked, against the real handler functions and then once over HTTP:
- the holder's signed release frees the block: it is no longer claimed, another key is offered it straight
  away, and it stops counting against the holder's CLAIM_OPEN_MAX;
- a release signed by another key, or by a key for someone else's claim, is refused and the claim is untouched;
- the same release sent twice gets the first answer and changes nothing, so a replay cannot release a claim the
  same key made on that block afterwards; a nonce reused for another block is refused;
- a proven block, a lapsed claim, a block never claimed, a stale timestamp and a malformed body are refused.

  python3 test_claim_release.py            # must PASS
  python3 test_claim_release.py --control  # ownership and signatures not checked: MUST FAIL
"""
import json
import os
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request

CONTROL = "--control" in sys.argv

_tmpdb = tempfile.NamedTemporaryFile(suffix=".db", delete=False)
os.environ["COORD_DB"] = _tmpdb.name
os.environ["COORD_SPINE"] = tempfile.mkdtemp(prefix="spine_")
os.environ["COORD_PROOFS"] = tempfile.mkdtemp(prefix="proofs_")
os.environ["VERIFY_MODE"] = "mock"
os.environ["COORD_ALLOW_MOCK"] = "1"
os.environ["TIP_CACHE_TTL"] = "0"
os.environ.setdefault("COORD_WEB", os.path.dirname(os.path.abspath(__file__)))
CAP, GRACE, TTL, SKEW = 4, 600, 3600, 120
os.environ["CLAIM_OPEN_MAX"] = str(CAP)
os.environ["CLAIM_GRACE"] = str(GRACE)
os.environ["CLAIM_TTL"] = str(TTL)
os.environ["BEAT_SKEW"] = str(SKEW)
os.environ.pop("CLAIM_REQUIRE_SIG", None)

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import server  # noqa: E402
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey  # noqa: E402
from cryptography.hazmat.primitives import serialization  # noqa: E402

server.init_db()
server.provable_tip = lambda: 1_000
server.witness_available = lambda h: True
if CONTROL:
    server._CONTROL_RELEASE_UNCHECKED = True
    print("CONTROL: any signed release frees any claim -- the checks below MUST fail")

fails = []


def check(ok, what):
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails.append(what)


def key():
    sk = Ed25519PrivateKey.generate()
    return sk, sk.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw).hex()


A_SK, A = key()
B_SK, B = key()


CLAIM_OF = {}                            # block -> the nonce of the latest claim made on it here


def claim(sk, pk, handle="a"):
    nonce, ts = os.urandom(8).hex(), int(time.time())
    code, r = server.claim({"pubkey": pk, "handle": handle, "nonce": nonce, "ts": ts,
                            "sig": sk.sign(f"claim:{nonce}:{ts}".encode()).hex()})
    assert code == 200, (code, r)
    CLAIM_OF[str(r["range"])] = nonce
    return r["range"]


def release_body(sk, pk, rng, nonce=None, ts=None, claim_id=None):
    """A release of the latest claim on rng (or of claim_id), as the worker signs it."""
    nonce = nonce or os.urandom(16).hex()
    ts = int(time.time()) if ts is None else ts
    claim_id = claim_id or CLAIM_OF.get(str(rng)) or os.urandom(8).hex()
    return {"range": str(rng), "claim": claim_id, "pubkey": pk, "nonce": nonce, "ts": ts,
            "sig": sk.sign(f"release:{rng}:{claim_id}:{nonce}:{ts}".encode()).hex()}


def q(sql, args=()):
    c = server.db()
    rows = c.execute(sql, args).fetchall()
    c.commit()
    c.close()
    return rows


def holder(rng):
    r = q("SELECT status, assignee FROM ranges WHERE id=?", (str(rng),))
    return (r[0]["status"], r[0]["assignee"]) if r else None


def open_claims(pk):
    return q("SELECT COUNT(*) FROM ranges WHERE assignee=? AND " + server.LIVE_CLAIM_SQL,
             (pk,) + server._live_claim_args(time.time()))[0][0]


def board_claims():
    return [c["lo"] for c in q("SELECT lo FROM ranges WHERE status='claimed'")]


def reset():
    q("DELETE FROM ranges")
    server._release_seen.clear()


print("== the holder releases its claim ==")
reset()
b = claim(A_SK, A)
before = open_claims(A)
code, r = server.release(release_body(A_SK, A, b))
check(code == 200 and r.get("released") == b, f"the holder's signed release is accepted ({code} {r})")
check(holder(b) == ("open", A) and int(b) not in board_claims(),
      f"the block is no longer claimed ({holder(b)}, board claims {board_claims()})")
check(open_claims(A) == before - 1, f"and it stops counting against the holder's cap ({before} -> {open_claims(A)})")
got = claim(B_SK, B, "b")
check(got == b, f"another key is offered the released block straight away ({b} -> {got})")

print("== a release the key holder did not sign ==")
reset()
b = claim(A_SK, A)
body = release_body(B_SK, B, b)
body["pubkey"] = A                       # B's signature presented under A's key
code, r = server.release(body)
check(code == 403 and holder(b) == ("claimed", A), f"signed by another key: refused, claim untouched ({code} {holder(b)})")

print("== someone else's claim ==")
reset()
b = claim(A_SK, A)
code, r = server.release(release_body(B_SK, B, b))
check(code == 409 and holder(b) == ("claimed", A),
      f"a key releasing a block another key holds: refused, claim untouched ({code} {holder(b)})")

print("== a repeat or replay changes nothing ==")
reset()
b = claim(A_SK, A)
body = release_body(A_SK, A, b)
code1, r1 = server.release(dict(body))
code2, r2 = server.release(dict(body))
check(code1 == 200 and code2 == 200 and r2.get("repeat") is True,
      f"the same release sent again (a lost response retried) gets the first answer ({code1}, {code2} {r2})")
again = claim(A_SK, A)
check(again == b, f"the holder takes the block again ({again})")
code3, r3 = server.release(dict(body))
check(holder(b) == ("claimed", A), f"a replay of the old release does not free the new claim ({code3} {holder(b)})")
other = claim(A_SK, A)
nonce_reuse = release_body(A_SK, A, other, nonce=body["nonce"])
code4, r4 = server.release(nonce_reuse)
check(code4 == 409 and holder(other) == ("claimed", A),
      f"a nonce already used for another block is refused ({code4} {r4.get('error')})")

print("== nothing to release ==")
reset()
b = claim(A_SK, A)
q("UPDATE ranges SET status='verified', verified_at=? WHERE id=?", (time.time(), b))
code, r = server.release(release_body(A_SK, A, b))
check(code == 409 and r.get("status") == "verified" and holder(b)[0] == "verified",
      f"an already proven block: refused, still verified ({code} {r})")
reset()
b = claim(A_SK, A)
q("UPDATE ranges SET claimed_at=? WHERE id=?", (time.time() - GRACE - 60, b))      # never beat, past the grace
code, r = server.release(release_body(A_SK, A, b))
check(code == 409 and r.get("status") == "lapsed", f"a claim that has already lapsed: refused ({code} {r})")
code, r = server.release(release_body(A_SK, A, 987))
check(code == 404, f"a block that was never claimed: 404 ({code} {r})")
b = claim(A_SK, A)
code, r = server.release(release_body(A_SK, A, b, ts=int(time.time()) - SKEW - 60))
check(code == 400 and holder(b) == ("claimed", A), f"a stale timestamp: refused ({code} {r.get('error')})")
for what, mangle in (("no signature", lambda d: d.pop("sig")),
                     ("no nonce", lambda d: d.pop("nonce")),
                     ("no claim named", lambda d: d.pop("claim")),
                     ("a float timestamp", lambda d: d.update(ts=float(d["ts"]))),
                     ("no range", lambda d: d.pop("range"))):
    d = release_body(A_SK, A, b)
    mangle(d)
    code, r = server.release(d)
    check(code == 400 and holder(b) == ("claimed", A), f"{what}: 400, claim untouched ({code})")

print("== over HTTP, through the real handler ==")
reset()
httpd = server.ThreadingHTTPServer(("127.0.0.1", 0), server.H)
threading.Thread(target=httpd.serve_forever, daemon=True).start()
url = f"http://127.0.0.1:{httpd.server_address[1]}/api/release"
b = claim(A_SK, A)


def post(body):
    req = urllib.request.Request(url, data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            return resp.status, json.load(resp)
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read().decode())


code, r = post(release_body(B_SK, B, b))
check(code == 409 and holder(b) == ("claimed", A), f"POST /api/release by another key: {code}")
code, r = post(release_body(A_SK, A, b))
check(code == 200 and holder(b)[0] == "open", f"POST /api/release by the holder: {code}, now {holder(b)}")
httpd.shutdown()

if CONTROL:
    if fails:
        print(f"CONTROL OK — ownership and signatures unchecked, and {len(fails)} assertion(s) failed, as they must.")
        sys.exit(0)
    print("CONTROL FAILED — releases were unchecked and every test still passed.")
    sys.exit(1)
if fails:
    print(f"{len(fails)} failure(s).")
    sys.exit(1)
print("only the holder releases its own live claim, once; the block is open to anyone at once.")
