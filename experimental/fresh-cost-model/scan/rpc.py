"""Minimal Bitcoin Core JSON-RPC client (stdlib only), cookie auth."""
import base64, http.client, json, os

class RPC:
    def __init__(self, datadir="/root/.bitcoin", host="127.0.0.1", port=8332):
        cookie = open(os.path.join(datadir, ".cookie")).read().strip()
        self.auth = "Basic " + base64.b64encode(cookie.encode()).decode()
        self.host, self.port = host, port
        self.conn = None

    def call(self, method, *params):
        body = json.dumps({"jsonrpc": "1.0", "id": 0, "method": method, "params": list(params)})
        for attempt in range(3):
            try:
                if self.conn is None:
                    self.conn = http.client.HTTPConnection(self.host, self.port, timeout=600)
                self.conn.request("POST", "/", body, {"Authorization": self.auth, "Content-Type": "application/json"})
                r = json.loads(self.conn.getresponse().read())
                if r.get("error"):
                    raise RuntimeError(f"{method}{params}: {r['error']}")
                return r["result"]
            except (http.client.HTTPException, OSError):
                self.conn = None
                if attempt == 2:
                    raise
