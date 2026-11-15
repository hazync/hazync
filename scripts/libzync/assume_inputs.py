#!/usr/bin/env python3
"""assume_inputs.py <H> --datadir <dir> [--rpcport <port>] --headers <out.bin> [--tracked <out.txt>]

The two side inputs of `swiftsync-verify assumeutxo` (docs/libzync/ASSUME_ZAPPS.md §3), from a node, over JSON-RPC
batches (cookie auth from the datadir; the cookie is read, never printed):

  --headers  the raw 80-byte headers of blocks 0..H, oldest first (each coin's creation MTP; the verifier links them
             and requires the last one to be the proven tip)
  --tracked  the BIP30-tracked coins unspent at H, `<txid>:<vout>` per line (leaf flag bit 1, UTREEXO_FORMAT.md §2a),
             by utreexo_ref.py's own `Tracked` rule over every block 1..H (`getblock <hash> 0`): a full block scan

Both are side inputs: a wrong one can only make a true snapshot fail, never a false one pass.
"""
import argparse, base64, json, os, sys, urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from utreexo_ref import Tracked, parse_tx, varint, spendable, BIP30_UNSPENDABLE  # noqa: E402


class Rpc:
    def __init__(self, datadir, port):
        user, pw = open(os.path.join(datadir, ".cookie")).read().strip().split(":", 1)
        self.url = f"http://127.0.0.1:{port}/"
        self.auth = "Basic " + base64.b64encode(f"{user}:{pw}".encode()).decode()

    def batch(self, method, params_list):
        body = json.dumps([{"jsonrpc": "1.0", "id": i, "method": method, "params": p} for i, p in enumerate(params_list)])
        req = urllib.request.Request(self.url, body.encode(), {"Authorization": self.auth, "Content-Type": "application/json"})
        out = json.load(urllib.request.urlopen(req, timeout=600))
        out.sort(key=lambda r: r["id"])
        for r in out:
            if r.get("error"):
                raise SystemExit(f"rpc {method}: {r['error']}")
        return [r["result"] for r in out]


def tx_len(raw):
    """Length of the serialised transaction at the start of raw."""
    j = 4
    seg = raw[4] == 0 and raw[5] == 1
    j += 2 if seg else 0
    n, j = varint(raw, j)
    for _ in range(n):
        j += 36
        sl, j = varint(raw, j)
        j += sl + 4
    m, j = varint(raw, j)
    for _ in range(m):
        sl, j = varint(raw, j + 8)
        j += sl
    if seg:
        for _ in range(n):
            c, j = varint(raw, j)
            for _ in range(c):
                l, j = varint(raw, j)
                j += l
    return j + 4


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("H", type=int)
    ap.add_argument("--datadir", required=True)
    ap.add_argument("--rpcport", type=int, default=8332)
    ap.add_argument("--headers", required=True)
    ap.add_argument("--tracked")
    a = ap.parse_args()
    rpc = Rpc(a.datadir, a.rpcport)
    STEP = 2000
    hashes = []
    with open(a.headers, "wb") as f:
        for lo in range(0, a.H + 1, STEP):
            hs = rpc.batch("getblockhash", [[h] for h in range(lo, min(lo + STEP, a.H + 1))])
            hashes += hs
            for hx in rpc.batch("getblockheader", [[h, False] for h in hs]):
                f.write(bytes.fromhex(hx))
    if not a.tracked:
        return
    trk = Tracked(hashes[0])
    live = set()
    STEP = 200
    for lo in range(1, a.H + 1, STEP):
        hi = min(lo + STEP, a.H + 1)
        for h, bx in zip(range(lo, hi), rpc.batch("getblock", [[hashes[h], 0] for h in range(lo, hi)])):
            b = bytes.fromhex(bx)
            n, i = varint(b, 80)
            for k in range(n):
                L = tx_len(b[i:])
                raw = b[i:i + L]
                i += L
                txid, ins, outs = parse_tx(raw)
                if k > 0:
                    live.difference_update(ins)
                t = trk.tx(h, k, raw, txid, ins if k > 0 else [])
                if k == 0 and BIP30_UNSPENDABLE.get(h) == hashes[h]:
                    continue
                if t:
                    for v, (_, spk) in enumerate(outs):
                        trk.created((txid, v))
                        if spendable(spk):
                            live.add((txid, v))
            assert i == len(b), f"block {h}: {len(b) - i} bytes after {n} transactions"
    with open(a.tracked, "w") as f:
        f.write(f"# BIP30-tracked coins unspent at height {a.H} (scripts/libzync/assume_inputs.py)\n")
        for (txid, v) in sorted(live):
            f.write(f"{txid[::-1].hex()}:{v}\n")


if __name__ == "__main__":
    main()
