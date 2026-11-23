#!/usr/bin/env python3
"""mkcore.py <getblock-3.json> <flags> <out.bin>: [u32 flags][u32 ntx] then per non-coinbase tx
[u32 len][tx bytes, with witness][u32 len][prevouts: Core std::vector<CTxOut> serialisation][u32 n_inputs]."""
import json, struct, sys
from decimal import Decimal
def cs(n): return bytes([n]) if n < 253 else (b"\xfd"+struct.pack("<H",n) if n < 65536 else b"\xfe"+struct.pack("<I",n))
j = json.load(open(sys.argv[1]), parse_float=Decimal)
out = bytearray(struct.pack("<II", int(sys.argv[2]), len(j["tx"]) - 1))
for t in j["tx"][1:]:
    tx = bytes.fromhex(t["hex"]); pv = bytearray(cs(len(t["vin"])))
    for v in t["vin"]:
        spk = bytes.fromhex(v["prevout"]["scriptPubKey"]["hex"])
        pv += struct.pack("<q", int(Decimal(v["prevout"]["value"]) * 100000000)) + cs(len(spk)) + spk
    out += struct.pack("<I", len(tx)) + tx + struct.pack("<I", len(pv)) + pv + struct.pack("<I", len(t["vin"]))
open(sys.argv[3], "wb").write(out)
print("txs", len(j["tx"]) - 1, "bytes", len(out))
