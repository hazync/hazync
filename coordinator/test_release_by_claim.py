#!/usr/bin/env python3
"""POST /api/release frees the claim it NAMES, never a later claim of the same key on the same block.

WHY THIS EXISTS. #725's release went by key + block. A release in flight for an OLD claim could therefore free a
block the same key had just claimed AGAIN (found in a review of the Windows app):
  - the worker hands its block back on SIGTERM, a new worker of the same key claims that block again, and then the
    app's own `hazync-worker release <n>` for the first claim lands (a fresh release nonce, so the replay memory does
    not catch it) and frees the NEW claim;
  - a claim lapses, the same key claims the block again, and a slow release for the lapsed claim lands.
Each is a block taken from under a running prover and offered to the next worker: duplicate work.
A release now names its claim (`claim` = the nonce the claim was made with, signed into the message) and the
coordinator releases only if the live claim is that one; otherwise 409 "not this claim", the re-claim untouched.

  python3 test_release_by_claim.py            # must PASS
  python3 test_release_by_claim.py --control  # the claim named is not checked: MUST FAIL
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
GRACE = 600
os.environ["CLAIM_OPEN_MAX"] = "4"
os.environ["CLAIM_GRACE"] = str(GRACE)
os.environ["CLAIM_TTL"] = "3600"
os.environ["BEAT_SKEW"] = "120"
os.environ["CLAIM_RETAKE_WAIT"] = "0"     # a key may re-take its own lapsed block at once: the lapse race below
os.environ.pop("CLAIM_REQUIRE_SIG", None)

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import server  # noqa: E402
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey  # noqa: E402
from cryptography.hazmat.primitives import serialization  # noqa: E402

server.init_db()
server.provable_tip = lambda: 1_000
server.witness_available = lambda h: True
if CONTROL:
    server._CONTROL_RELEASE_ANY_CLAIM = True
    print("CONTROL: a release frees the key's claim whichever claim it names -- the checks below MUST fail")

fails = []


def check(ok, what):
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails.append(what)


SK = Ed25519PrivateKey.generate()
PK = SK.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw).hex()


def claim():
    """(block, claim id) as the worker claims: one fresh nonce per claim."""
    nonce, ts = os.urandom(16).hex(), int(time.time())
    code, r = server.claim({"pubkey": PK, "handle": "a", "nonce": nonce, "ts": ts,
                            "sig": SK.sign(f"claim:{nonce}:{ts}".encode()).hex()})
    assert code == 200, (code, r)
    return r["range"], nonce


def release_body(rng, claim_id):
    """A release as the worker signs it: a fresh release nonce every time, as `hazync release` mints one per run."""
    nonce, ts = os.urandom(16).hex(), int(time.time())
    return {"range": str(rng), "claim": claim_id, "pubkey": PK, "nonce": nonce, "ts": ts,
            "sig": SK.sign(f"release:{rng}:{claim_id}:{nonce}:{ts}".encode()).hex()}


def row(rng):
    c = server.db()
    r = c.execute("SELECT status, assignee, claim_nonce FROM ranges WHERE id=?", (str(rng),)).fetchone()
    c.close()
    return (r["status"], r["claim_nonce"]) if r else None


def reset():
    c = server.db()
    c.execute("DELETE FROM ranges")
    c.commit()
    c.close()
    server._release_seen.clear()


print("== released, re-claimed by the same key, then a late release of the first claim ==")
reset()
b, first = claim()
code, r = server.release(release_body(b, first))          # the worker's own release on SIGTERM
check(code == 200 and row(b)[0] == "open", f"the first claim is released ({code}, {row(b)})")
b2, second = claim()                                       # a new worker of the same key
check(b2 == b and row(b) == ("claimed", second), f"the same key claims block {b} again ({b2}, {row(b)})")
code, r = server.release(release_body(b, first))           # the app's release for the FIRST claim, landing late
check(code == 409 and "not this claim" in str(r.get("error")) and row(b) == ("claimed", second),
      f"a late release of the first claim is refused and the re-claim survives ({code} {r.get('error')}, {row(b)})")
code, r = server.release(release_body(b, second))
check(code == 200 and row(b)[0] == "open", f"a release naming the re-claim frees it ({code}, {row(b)})")

print("== a claim that lapsed and was re-taken, then a slow release of the lapsed claim ==")
reset()
b, first = claim()
c = server.db()
c.execute("UPDATE ranges SET claimed_at=? WHERE id=?", (time.time() - GRACE - 60, b))   # never beat, past the grace
c.commit()
c.close()
b2, second = claim()
check(b2 == b and row(b) == ("claimed", second), f"the lapsed block {b} is claimed again by the same key ({b2})")
code, r = server.release(release_body(b, first))
check(code == 409 and row(b) == ("claimed", second),
      f"the lapsed claim's release does not free the re-claim ({code} {r.get('error')}, {row(b)})")

print("== a re-claim that has itself lapsed reports the lapse, not a re-claim ==")
reset()
b, first = claim()
server.release(release_body(b, first))
b2, second = claim()
c = server.db()
c.execute("UPDATE ranges SET claimed_at=? WHERE id=?", (time.time() - GRACE - 60, b))
c.commit()
c.close()
code, r = server.release(release_body(b, first))
check(code == 409 and r.get("status") == "lapsed", f"the old claim's release on a lapsed re-claim: 'lapsed' ({code} {r})")

print("== a claim's id has the shape a release can name ==")
reset()
nonce, ts = "retry-me", int(time.time())
code, r = server.claim({"pubkey": PK, "handle": "a", "nonce": nonce, "ts": ts,
                        "sig": SK.sign(f"claim:{nonce}:{ts}".encode()).hex()})
check(code == 400 and "hex" in str(r.get("error")), f"a signed claim whose nonce is not 16-64 hex: 400 ({code} {r})")
code, r = server.claim({"pubkey": PK, "handle": "a", "nonce": "n1"})          # unsigned: any nonce, as before
check(code == 200, f"an unsigned claim is still taken ({code})")
code, r2 = server.release(release_body(r.get("range"), os.urandom(16).hex()))
check(code == 409 and r2.get("status") == "unreleasable" and row(r.get("range"))[0] == "claimed",
      f"releasing an unsigned claim says it cannot be released, not 'reclaimed' ({code} {r2})")

print("== the claim named is part of what is signed ==")
reset()
b, first = claim()
body = release_body(b, os.urandom(16).hex())
body["claim"] = first                                       # someone swaps in the live claim's id after signing
code, r = server.release(body)
check(code == 403 and row(b) == ("claimed", first), f"a release whose claim was changed after signing: 403 ({code})")

print("== over HTTP, through the real handler ==")
reset()
httpd = server.ThreadingHTTPServer(("127.0.0.1", 0), server.H)
threading.Thread(target=httpd.serve_forever, daemon=True).start()
url = f"http://127.0.0.1:{httpd.server_address[1]}/api/release"


def post(body):
    req = urllib.request.Request(url, data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            return resp.status, json.load(resp)
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read().decode())


b, first = claim()
post(release_body(b, first))
b2, second = claim()
code, r = post(release_body(b, first))
check(b2 == b and code == 409 and row(b) == ("claimed", second), f"POST /api/release of the old claim: {code}, {row(b)}")
httpd.shutdown()

if CONTROL:
    if fails:
        print(f"CONTROL OK — the claim named is unchecked, and {len(fails)} assertion(s) failed, as they must.")
        sys.exit(0)
    print("CONTROL FAILED — the claim named was unchecked and every test still passed.")
    sys.exit(1)
if fails:
    print(f"{len(fails)} failure(s).")
    sys.exit(1)
print("a release frees only the claim it names; a re-claim of the same block by the same key survives it.")
