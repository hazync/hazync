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

# ---- variant B (branch exp/undo-commit-leaves): SHA-256 of the spent coin leaves (UTREEXO_FORMAT.md §2), block order.
def cs(b, o):
    f = b[o]; o += 1
    if f < 0xfd: return f, o
    n = {0xfd: 2, 0xfe: 4, 0xff: 8}[f]
    return int.from_bytes(b[o:o + n], "little"), o + n
def outpoints(raw):
    o = 4
    if raw[4] == 0 and raw[5] == 1: o += 2
    n, o = cs(raw, o); r = []
    for _ in range(n):
        r.append((raw[o:o + 32], int.from_bytes(raw[o + 32:o + 36], "little"))); o += 36
        l, o = cs(raw, o); o += l + 4
    return r
def leaves_hash(j, tracked):
    ls = b""
    for t in j["txs"]:
        for (txid, vout), p in zip(outpoints(bytes.fromhex(t["raw"])), t["prevouts"]):
            spk = bytes.fromhex(p["spk"])
            fl = (p["coin_is_coinbase"] & 1) | (2 if (txid[::-1].hex(), vout) in tracked else 0)
            pre = b"\x00" + txid + struct.pack("<IQI", vout, p["value"], len(spk)) + spk + struct.pack("<IBI", p["coin_height"], fl, p["coin_mtp"])
            ls += hashlib.sha256(pre).digest()
    return hashlib.sha256(ls).digest().hex()
if __name__ == "__main__":
    import os
    tj = os.path.join(os.path.dirname(os.path.abspath(__file__)), "../../reproduce/libzync-tracked.json")
    tracked = {(x[0], x[1]) for x in json.load(open(tj))["outputs"]}
    for f in sys.argv[1:]:
        j = json.load(open(f))
        print(f"undo_ref_leaves: block {j['height']} hash {leaves_hash(j, tracked)}")
