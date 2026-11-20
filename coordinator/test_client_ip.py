#!/usr/bin/env python3
"""The coordinator sets `_client_ip` itself on every POST; a client cannot choose it (15.11.2026).

Before the fix, do_POST filled `_client_ip` only for /api/sponsor, so claim()'s CLAIM_NEW_IP_MAX read whatever the
CLIENT put in the body. Real workers send none, so the per-IP cap on new keys never applied; a client could also name
any address. test_claim_sybil.py called claim() directly with its own `_client_ip`, which is why it never showed.

Drives the REAL HTTP handler on a loopback port and records the body each handler receives.

  python3 test_client_ip.py            # must PASS
  python3 test_client_ip.py --control  # do_POST as it was before the fix; MUST FAIL
"""
import json
import os
import sys
import tempfile
import threading
import types
import urllib.error
import urllib.request

CONTROL = "--control" in sys.argv
HERE = os.path.dirname(os.path.abspath(__file__))
os.environ.update(COORD_DB=tempfile.NamedTemporaryFile(suffix=".db", delete=False).name,
                  COORD_SPINE=tempfile.mkdtemp(prefix="spine_"), COORD_PROOFS=tempfile.mkdtemp(prefix="proofs_"),
                  VERIFY_MODE="mock", COORD_ALLOW_MOCK="1", TIP_CACHE_TTL="0", RATE_EXEMPT="")
os.environ.setdefault("COORD_WEB", HERE)

src = open(os.path.join(HERE, "server.py")).read()
FIXED = '        if isinstance(body, dict):\n            # set here, always, for every POST'
assert FIXED in src, "the fix is not in server.py"
if CONTROL:
    # put back the pre-fix wiring: only /api/sponsor gets the server-side address
    src = src.replace(FIXED, '        if isinstance(body, dict) and p == "/api/sponsor":\n            # set here, always, for every POST', 1)
    print("CONTROL: do_POST as before the fix -- the checks below MUST fail")
server = types.ModuleType("server")
server.__file__ = os.path.join(HERE, "server.py")
sys.modules["server"] = server
exec(compile(src, server.__file__, "exec"), server.__dict__)
server.init_db()

seen = {}
def recorder(name):
    def fn(body):
        seen[name] = dict(body) if isinstance(body, dict) else body
        return 200, {"ok": True}
    return fn
# the handler looks the functions up by name at call time, so replacing them records exactly what do_POST passes on
for name in ("claim", "submit", "beat", "fold_claim", "rotate", "submit_spine"):
    setattr(server, name, recorder(name))

httpd = server.ThreadingHTTPServer(("127.0.0.1", 0), server.H)
threading.Thread(target=httpd.serve_forever, daemon=True).start()
URL = f"http://127.0.0.1:{httpd.server_address[1]}"

def post(path, body):
    req = urllib.request.Request(URL + path, data=json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json"}, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=10) as r:
            return r.status
    except urllib.error.HTTPError as e:
        return e.code

fails = 0
def check(name, ok, got):
    global fails
    fails += not ok
    print(f"  {'ok  ' if ok else 'FAIL'} {name}  ({got})")

for path, name in (("/api/claim", "claim"), ("/api/submit", "submit"), ("/api/beat", "beat"),
                   ("/api/foldclaim", "fold_claim")):
    seen.clear()
    code = post(path, {"pubkey": "11" * 32, "_client_ip": "6.6.6.6"})
    ip = (seen.get(name) or {}).get("_client_ip")
    check(f"{path}: a client-sent _client_ip is replaced by the real address", code == 200 and ip == "127.0.0.1",
          f"http {code}, handler saw _client_ip={ip!r}")
    seen.clear()
    post(path, {"pubkey": "11" * 32})
    ip = (seen.get(name) or {}).get("_client_ip")
    check(f"{path}: with none sent, the real address is set", ip == "127.0.0.1", f"_client_ip={ip!r}")

httpd.shutdown()
print("CLIENT_IP_TESTS_OK" if not fails else f"{fails} FAILED")
sys.exit(1 if fails else 0)
