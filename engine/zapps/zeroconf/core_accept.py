#!/usr/bin/env python3
"""core_accept.py <bitcoind> <raw.txt> <tip height> <T.json> <port>: a throwaway regtest node (Core), the harness's
main branch submitted to <tip height>, then testmempoolaccept on T. Prints the tip and Core's verdict."""
import json, os, subprocess, sys, tempfile, time, base64, urllib.request
B, RAW, TIP, TJ, PORT = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4], int(sys.argv[5])
d = tempfile.mkdtemp(prefix="zc-core-")
p = subprocess.Popen([B, "-regtest", f"-datadir={d}", f"-rpcport={PORT}", "-rpcuser=u", "-rpcpassword=p", "-listen=0",
                      "-testactivationheight=bip34@110", "-fallbackfee=0.0001"], stdout=subprocess.DEVNULL)
def rpc(m, *a):
    r = urllib.request.Request(f"http://127.0.0.1:{PORT}", json.dumps({"id": 0, "method": m, "params": list(a)}).encode(),
                               {"Authorization": "Basic " + base64.b64encode(b"u:p").decode()})
    return json.load(urllib.request.urlopen(r))["result"]
try:
    for _ in range(60):
        try: rpc("getblockcount"); break
        except Exception: time.sleep(0.5)
    for l in open(RAW):
        br, h, _, hx = l.split()
        if br == "main" and int(h) <= TIP:
            assert rpc("submitblock", hx) is None, f"block {h} refused"
    tip = rpc("getbestblockhash")
    print("tip", rpc("getblockcount"), tip)
    t = json.load(open(TJ))
    print("testmempoolaccept:", json.dumps(rpc("testmempoolaccept", [t["raw"]])))
finally:
    p.terminate(); p.wait()
