#!/usr/bin/env python3
"""exp/undo-commit (#729): independent reference for the prototype's block undo hash (format U0), from a block fixture.
   per input of every non-coinbase tx, block order:
     rec = code u32 LE (height*2 + coinbase) | amount u64 LE | len(spk) u32 LE | spk;  d = SHA-256(rec)
   undo_hash = SHA-256(d_1 | ... | d_n)
Also prints the size of Core's CBlockUndo serialisation estimate (uncompressed scripts upper bound is the U0 total).
usage: undo_ref.py <block.json>..."""
import hashlib, json, struct, sys
for f in sys.argv[1:]:
    j = json.load(open(f))
    ds, nbytes, n = b"", 0, 0
    for t in j["txs"]:
        for p in t["prevouts"]:
            spk = bytes.fromhex(p["spk"])
            rec = struct.pack("<IQI", p["coin_height"] * 2 + (p["coin_is_coinbase"] & 1), p["value"], len(spk)) + spk
            ds += hashlib.sha256(rec).digest(); nbytes += len(rec); n += 1
    print(f"undo_ref: block {j['height']} inputs {n} hash {hashlib.sha256(ds).digest().hex()} u0_bytes {nbytes}")
