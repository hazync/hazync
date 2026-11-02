#!/usr/bin/env python3
"""zutxo_keys.py <block.json> <out.bin>: the coins one block touches in the Zutxo tree. Deletes = every input's
prevout; inserts = every spendable output; a coin created and spent in the same block cancels and touches nothing.
Each record: kind u8 (0 delete, 1 insert) + outpoint (txid internal byte order, vout u32 LE)."""
import json, os, struct, sys
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "../../../scripts/libzync"))
from utreexo_ref import parse_tx, spendable
j = json.load(open(sys.argv[1]))
raws = [bytes.fromhex(j["coinbase_hex"])] + [bytes.fromhex(t["raw"]) for t in j["txs"]]
dels, ins = [], []
for k, raw in enumerate(raws):
    txid, vin, vout = parse_tx(raw)
    if k > 0:
        dels += [pt + struct.pack("<I", pv) for pt, pv in vin]
    ins += [txid + struct.pack("<I", v) for v, (val, spk) in enumerate(vout) if spendable(spk)]
both = set(dels) & set(ins)
dels = [d for d in dels if d not in both]; ins = [i for i in ins if i not in both]
assert len(set(dels)) == len(dels) and len(set(ins)) == len(ins)
open(sys.argv[2], "wb").write(b"".join(b"\x00" + d for d in dels) + b"".join(b"\x01" + i for i in ins))
print(f"height {j['height']}: {len(raws)} txs, {len(dels)} deletes, {len(ins)} inserts, {len(both)} created and spent in the block")
