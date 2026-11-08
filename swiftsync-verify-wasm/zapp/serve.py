#!/usr/bin/env python3
"""The coin-check zyncAPP page, served from one origin (a demo; ROADMAP R4).

    python3 zapp/serve.py <proof.rcpt> <zync-serve addr, e.g. 127.0.0.1:8601> [port, default 8600]

Serves swiftsync-verify-wasm/ (the page, the glue, the .wasm), the proof at /zapp/proof.rcpt, and forwards
GET /zapp/api/<path> to zync-serve's /<path>. It adds nothing to the trust model: the page verifies the proof and
checks every fact itself, so this server, like zync-serve, is untrusted.
"""
import http.server, os, sys, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROOF, UPSTREAM = sys.argv[1], sys.argv[2]
PORT = int(sys.argv[3]) if len(sys.argv) > 3 else 8600


class H(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **k):
        super().__init__(*a, directory=ROOT, **k)

    def end_headers(self):
        if self.path.endswith('.wasm'):
            self.send_header('Content-Type', 'application/wasm')
        super().end_headers()

    def do_GET(self):
        if self.path == '/zapp/proof.rcpt':
            b = open(PROOF, 'rb').read()
            self.send_response(200); self.send_header('Content-Type', 'application/octet-stream')
            self.send_header('Content-Length', str(len(b))); self.end_headers(); self.wfile.write(b); return
        if self.path.startswith('/zapp/api/'):
            sub = self.path[len('/zapp/api'):]
            try:
                with urllib.request.urlopen(f'http://{UPSTREAM}{sub}', timeout=60) as r:
                    code, body = r.status, r.read()
            except urllib.error.HTTPError as e:
                code, body = e.code, e.read()
            except Exception as e:
                code, body = 502, ('{"error": "%s"}' % str(e).replace('"', "'")).encode()
            self.send_response(code); self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body); return
        return super().do_GET()

    def guess_type(self, path):
        return 'application/wasm' if str(path).endswith('.wasm') else super().guess_type(path)


if __name__ == '__main__':
    print(f'coin check: http://127.0.0.1:{PORT}/zapp/  (proof {PROOF}, facts from {UPSTREAM})', flush=True)
    http.server.ThreadingHTTPServer(('127.0.0.1', PORT), H).serve_forever()
