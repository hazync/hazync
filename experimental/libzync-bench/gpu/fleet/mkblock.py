#!/usr/bin/env python3
"""mkblock.py <height> <out.json> — block JSON for HAZYNC_BLOCK from a local node (bitcoin-cli getblock 3).

Fields as build_full() reads them. coin_mtp = MTP of the block BEFORE the coin's block (BIP68's
nCoinTime, Core: chain[coin_height-1]->GetMedianTimePast()). recent_times = timestamps of the 11
blocks before this one, NEWEST first (as the fixtures hold them). Validate by regenerating a block we already hold.
"""
import json, subprocess, sys
from decimal import Decimal

def cli(*a):
    return subprocess.run(["bitcoin-cli", *map(str, a)], check=True, capture_output=True, text=True).stdout.strip()

h, out = int(sys.argv[1]), sys.argv[2]
bh = cli("getblockhash", h)
b = json.loads(cli("getblock", bh, 3), parse_float=Decimal)
mtp_cache = {}
def mtp(height):
    if height not in mtp_cache:
        mtp_cache[height] = json.loads(cli("getblockheader", cli("getblockhash", height)))["mediantime"]
    return mtp_cache[height]

txs = []
for t in b["tx"][1:]:
    pv = []
    for vin in t["vin"]:
        p = vin["prevout"]
        ch = p["height"]
        pv.append({"value": int(p["value"] * 100_000_000), "spk": p["scriptPubKey"]["hex"],
                   "coin_height": ch, "coin_is_coinbase": 1 if p["generated"] else 0,
                   "coin_mtp": mtp(max(ch - 1, 0))})
    txs.append({"raw": t["hex"], "prevouts": pv})

recent = []
for k in range(h - 1, max(h - 12, -1), -1):   # clamps at genesis for h < 12
    recent.append(json.loads(cli("getblockheader", cli("getblockhash", k)))["time"])

j = {"height": h, "bits": int(b["bits"], 16), "time": b["time"], "nonce": b["nonce"], "version": b["version"],
     "prev": b["previousblockhash"], "merkle": b["merkleroot"], "coinbase_hex": b["tx"][0]["hex"],
     "txs": txs, "recent_times": recent}
json.dump(j, open(out, "w"))
print(f"{h}: {len(txs)} txs, {sum(len(t['prevouts']) for t in txs)} inputs -> {out}")
