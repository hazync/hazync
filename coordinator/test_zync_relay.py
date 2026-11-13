#!/usr/bin/env python3
"""The coordinator's zyncAPP relay (/api/zync/*) passes only read-only fact paths to the local zync-serve.

  1. with no ZYNC_SERVE_URL the endpoint is 404 and nothing is contacted;
  2. a fact path (/tip, /trie/balance|history/<spk>, /trie/coin/<spk>/<txid>/<vout>) is relayed, body unchanged;
  3. anything else (other paths, traversal, non-hex) is 400 and never reaches zync-serve; a query string is dropped;
  4. the fact service's 404 stays a 404; it being down is a 502, not a hang or a 500;
  5. /api/zync/proof serves ZYNC_PROOF's bytes from the coordinator (404 unset or missing), never relayed.

  python3 test_zync_relay.py            # must PASS
  python3 test_zync_relay.py --control  # the path filter accepts anything; MUST FAIL
"""
import http.server, json, os, re, sys, tempfile, threading, urllib.request, urllib.error

CONTROL = "--control" in sys.argv
T = tempfile.mkdtemp(prefix="zync_")
for k, v in dict(COORD_DB=f"{T}/c.db", COORD_STATE=f"{T}/st", COORD_PROOFS=f"{T}/p", COORD_SPINE=f"{T}/sp",
                 WITNESS_DIR=f"{T}/w", VERIFY_MODE="mock", COORD_ALLOW_MOCK="1").items():
    os.environ[k] = v
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import server  # noqa: E402
server.init_db()
if CONTROL:
    server._ZYNC_PATH = re.compile(r".*")
    print("CONTROL: the relay's path filter accepts anything -- the checks below MUST fail")

seen = []
class Fake(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        seen.append(self.path)
        if self.path.startswith("/trie/balance/dead"):
            self.send_response(404); self.end_headers(); self.wfile.write(b'{"error":"unknown"}'); return
        body = json.dumps({"path": self.path, "tip": 200}).encode()
        self.send_response(200); self.send_header("Content-Type", "application/json"); self.end_headers(); self.wfile.write(body)
    def log_message(self, *a): pass
fake = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fake)
threading.Thread(target=fake.serve_forever, daemon=True).start()
coord = server.ThreadingHTTPServer(("127.0.0.1", 0), server.H)
threading.Thread(target=coord.serve_forever, daemon=True).start()
C = f"http://127.0.0.1:{coord.server_address[1]}"

def get(path):
    try:
        with urllib.request.urlopen(C + path, timeout=30) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()

fails = 0
def check(cond, what):
    global fails
    print(("  ok   " if cond else "  FAIL ") + what); fails += 0 if cond else 1

server.ZYNC_SERVE = ""
code, _ = get("/api/zync/tip")
check(code == 404 and not seen, "no ZYNC_SERVE_URL: 404, nothing contacted")

server.ZYNC_SERVE = f"http://127.0.0.1:{fake.server_address[1]}"
SPK, TX = "76a914" + "ab" * 20 + "88ac", "cd" * 32
for p in ("/tip", f"/trie/balance/{SPK}", f"/trie/history/{SPK}", f"/trie/coin/{SPK}/{TX}/0"):
    code, body = get("/api/zync" + p)
    check(code == 200 and json.loads(body).get("path") == p, f"relayed unchanged: {p[:40]}")
seen.clear()
code, _ = get(f"/api/zync/trie/balance/{SPK}?x=1&y=/admin")
check(code == 200 and seen == [f"/trie/balance/{SPK}"], f"a query string never reaches zync-serve (it saw {seen})")
seen.clear()
for p in ("/coin/tip/5", "/trie/balance/xyz", "/trie/../tip", "/trie/history/",
          f"/trie/coin/{SPK}/{TX}/0/extra", "/find/" + "ab" * 32, "/admin"):
    code, _ = get("/api/zync" + p)
    check(code in (400, 404) and not seen, f"refused before zync-serve: {p[:40]} ({code})")
    seen.clear()
code, _ = get("/api/zync/trie/balance/dead")
check(code == 404, "the fact service's 404 stays 404")
# /api/zync/proof: the receipt the facts are at, served from ZYNC_PROOF by the coordinator itself (never relayed)
seen.clear()
server.ZYNC_PROOF = ""
code, _ = get("/api/zync/proof")
check(code == 404 and not seen, "no ZYNC_PROOF: /api/zync/proof is 404 and zync-serve is not asked")
pf = os.path.join(T, "zapp-proof.rcpt"); open(pf, "wb").write(b"\x00receipt bytes\xff" * 3)
server.ZYNC_PROOF = pf
code, body = get("/api/zync/proof")
check(code == 200 and body == open(pf, "rb").read() and not seen, "ZYNC_PROOF set: its bytes, unchanged, not relayed")
server.ZYNC_PROOF = pf + ".missing"
code, _ = get("/api/zync/proof")
check(code == 404, "ZYNC_PROOF names a missing file: 404")
fake.shutdown(); fake.server_close()
code, _ = get("/api/zync/tip")
check(code == 502, f"the fact service down: 502 ({code})")

print(f"{'CONTROL: ' if CONTROL else ''}{fails} failure(s)")
coord.shutdown()
if CONTROL:
    sys.exit(0 if fails else 1)
sys.exit(1 if fails else 0)
