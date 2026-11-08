#!/usr/bin/env python3
"""coinlog_keys.py <block.json> <out.txt>: what one block does to the coin log. Line 1 height; line 2 appends (spendable
outputs not spent in the same block); line 3 the creation heights of the coins it spends from earlier blocks."""
import json, os, struct, sys
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "../../../scripts/libzync"))
from utreexo_ref import parse_tx, spendable
j = json.load(open(sys.argv[1])); h = j["height"]
raws = [bytes.fromhex(j["coinbase_hex"])] + [bytes.fromhex(t["raw"]) for t in j["txs"]]
spent_here, created, olds = set(), [], []
for k, raw in enumerate(raws):
    txid, vin, vout = parse_tx(raw)
    if k > 0:
        for (pt, pv), p in zip(vin, j["txs"][k - 1]["prevouts"]):
            if p["coin_height"] == h: spent_here.add(pt + struct.pack("<I", pv))
            else: olds.append(p["coin_height"])
    created += [txid + struct.pack("<I", v) for v, (val, spk) in enumerate(vout) if spendable(spk)]
apps = sum(1 for c in created if c not in spent_here)
open(sys.argv[2], "w").write(f"{h}\n{apps}\n{' '.join(map(str, olds))}\n")
print(f"height {h}: {len(olds)} spends of earlier coins, {apps} appends, {len(spent_here)} created and spent in the block")
